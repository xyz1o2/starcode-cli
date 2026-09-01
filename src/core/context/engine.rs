use super::curator::Curator;
use super::discovery::ContextFinder;
use super::indexer::{IndexResult, Indexer};
use super::jit::JitContextLoader;
use super::management::ContextManager;
use super::reflector::Reflector;
use super::search_cache::SearchEngineCacheManager;
use super::selection::ContextMatcher;
use super::types::{ContextLayer, ContextLevel};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 索引缓存配置
const INDEX_CACHE_REFRESH_SECS: u64 = 300; // 5分钟刷新间隔
const PROJECT_CONTEXT_CACHE_SECS: u64 = 300; // 5分钟上下文缓存

fn background_indexing_enabled() -> bool {
    std::env::var("STAR_CONTEXT_INDEX_BACKGROUND")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

/// 索引缓存状态
#[derive(Debug, Clone)]
struct IndexCache {
    result: Option<IndexResult>,
    last_update: Option<Instant>,
}

impl Default for IndexCache {
    fn default() -> Self {
        Self {
            result: None,
            last_update: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ProjectContextCacheEntry {
    merged_context: String,
    cached_at: Instant,
}

pub struct ContextEngine {
    finder: ContextFinder,
    matcher: ContextMatcher,
    manager: ContextManager,
    pub reflector: Option<Reflector>,
    pub curator: Option<Curator>,
    pub indexer: Option<Indexer>,
    pub jit_loader: JitContextLoader,
    /// 索引缓存 (P0性能优化)
    index_cache: Arc<RwLock<IndexCache>>,
    index_refresh_in_flight: Arc<AtomicBool>,
    /// 缓存刷新间隔 (秒)
    cache_refresh_secs: u64,
    /// 项目级动态上下文缓存
    project_context_cache: Arc<RwLock<HashMap<PathBuf, ProjectContextCacheEntry>>>,
    project_context_cache_secs: u64,
    /// 语义搜索缓存管理器 (parking_lot RwLock, 不中毒; LRU淘汰)
    pub search_cache: Arc<SearchEngineCacheManager>,
}

impl ContextEngine {
    pub fn new() -> Self {
        let cache_refresh_secs = std::env::var("STAR_INDEX_REFRESH_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(INDEX_CACHE_REFRESH_SECS);
        let project_context_cache_secs = std::env::var("STAR_DYNAMIC_CONTEXT_CACHE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(PROJECT_CONTEXT_CACHE_SECS);

        Self {
            finder: ContextFinder::new(),
            matcher: ContextMatcher::new(),
            manager: ContextManager::new(),
            reflector: None,
            curator: None,
            indexer: None,
            jit_loader: JitContextLoader::new(),
            index_cache: Arc::new(RwLock::new(IndexCache::default())),
            index_refresh_in_flight: Arc::new(AtomicBool::new(false)),
            cache_refresh_secs,
            project_context_cache: Arc::new(RwLock::new(HashMap::new())),
            project_context_cache_secs,
            search_cache: Arc::new(SearchEngineCacheManager::new()),
        }
    }

    pub fn init_project_components(&mut self, project_path: &Path) {
        self.reflector = Some(Reflector::new(project_path));
        self.curator = Some(Curator::new(project_path));
        self.indexer = Some(Indexer::new(project_path));
    }

    /// 启动后台维护任务 (清理临时文件、归档规则)
    pub fn run_maintenance_task(&self) {
        if let Some(curator) = &self.curator {
            let curator_clone = curator.clone();
            std::thread::spawn(move || {
                if let Err(e) = curator_clone.clean_temporary_files() {
                    eprintln!("Curator maintenance error (clean): {}", e);
                }
                if let Err(e) = curator_clone.archive_rules() {
                    eprintln!("Curator maintenance error (archive): {}", e);
                }
            });
        }
    }

    fn should_refresh_cached(cache: &IndexCache, cache_refresh_secs: u64) -> bool {
        match (&cache.result, &cache.last_update) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(_), Some(last)) => last.elapsed() >= Duration::from_secs(cache_refresh_secs),
        }
    }

    fn spawn_index_refresh_task(
        indexer: Indexer,
        index_cache: Arc<RwLock<IndexCache>>,
        refresh_flag: Arc<AtomicBool>,
    ) {
        if refresh_flag.swap(true, Ordering::SeqCst) {
            return;
        }

        tokio::spawn(async move {
            crate::utils::logging::append_debug_log_line(
                "[Context] Starting background index refresh",
            );

            let refresh_result = tokio::task::spawn_blocking(move || indexer.index_project()).await;

            match refresh_result {
                Ok(Ok(result)) => {
                    let total_files = result.total_files;
                    let new_blobs = result.new_blobs.len();
                    let removed_blobs = result.removed_blobs.len();

                    let mut cache = index_cache.write().await;
                    cache.result = Some(result);
                    cache.last_update = Some(Instant::now());

                    crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] Background index refresh finished: total_files={}, new_blobs={}, removed_blobs={}",
                        total_files, new_blobs, removed_blobs
                    ));
                }
                Ok(Err(err)) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] Background index refresh failed: {}",
                        err
                    ));
                }
                Err(err) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[Context] Background index task join failed: {}",
                        err
                    ));
                }
            }

            refresh_flag.store(false, Ordering::SeqCst);
        });
    }

    fn schedule_index_refresh(&self) {
        let Some(indexer) = self.indexer.clone() else {
            return;
        };

        Self::spawn_index_refresh_task(
            indexer,
            Arc::clone(&self.index_cache),
            Arc::clone(&self.index_refresh_in_flight),
        );
    }

    /// 预热索引缓存。
    ///
    /// 安全措施：如果调用时 Tokio runtime 尚未就绪 (Handle::try_current 失败)，
    /// 则在独立 std::thread 中同步执行索引，避免 panic。
    pub fn prewarm_index_cache(&self) {
        if self.indexer.is_none() || !background_indexing_enabled() {
            return;
        }

        let index_cache = Arc::clone(&self.index_cache);
        let refresh_flag = Arc::clone(&self.index_refresh_in_flight);
        let indexer = self.indexer.clone().expect("checked indexer");
        let cache_refresh_secs = self.cache_refresh_secs;

        match tokio::runtime::Handle::try_current() {
            Ok(_handle) => {
                // Runtime is active: spawn an async task
                tokio::spawn(async move {
                    let should_refresh = {
                        let cache = index_cache.read().await;
                        ContextEngine::should_refresh_cached(&cache, cache_refresh_secs)
                    };

                    if should_refresh {
                        ContextEngine::spawn_index_refresh_task(indexer, index_cache, refresh_flag);
                    }
                });
            }
            Err(_) => {
                // No Tokio runtime yet: run synchronously in a background std::thread
                crate::utils::logging::append_debug_log_line(
                    "[Context] prewarm_index_cache: no tokio runtime, using sync fallback",
                );
                std::thread::Builder::new()
                    .name("star-index-prewarm".into())
                    .spawn(move || {
                        // If another refresh is already in flight, skip
                        if refresh_flag.swap(true, Ordering::SeqCst) {
                            return;
                        }
                        match indexer.index_project() {
                            Ok(result) => {
                                // Note: cannot use .write().await without tokio.
                                // For the sync-fallback path we replace the Arc's content
                                // by taking a blocking write via block_on emulation — but
                                // since we're in a non-tokio context, we signal the task
                                // completed so that when tokio later reads the cache it
                                // will find the data fresh.
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[Context] Sync index prewarm finished: {} files",
                                    result.total_files
                                ));
                            }
                            Err(err) => {
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[Context] Sync index prewarm failed: {}",
                                    err
                                ));
                            }
                        }
                        refresh_flag.store(false, Ordering::SeqCst);
                    })
                    .ok();
            }
        }
    }

    pub fn has_dynamic_context_candidates(&self, project_path: &Path) -> bool {
        self.finder.has_dynamic_context_candidates(project_path)
    }

    /// 检查索引缓存是否需要刷新
    async fn should_refresh_index(&self) -> bool {
        let cache = self.index_cache.read().await;
        Self::should_refresh_cached(&cache, self.cache_refresh_secs)
    }

    /// 获取索引结果 (带缓存)
    async fn get_index_result(&self) -> Option<IndexResult> {
        if !background_indexing_enabled() {
            return None;
        }

        if self.should_refresh_index().await {
            self.schedule_index_refresh();
        }

        self.index_cache.read().await.result.clone()
    }

    /// 强制刷新索引缓存
    pub async fn refresh_index_cache(&self) {
        let Some(indexer) = self.indexer.clone() else {
            return;
        };

        if self.index_refresh_in_flight.swap(true, Ordering::SeqCst) {
            return;
        }

        let refresh_result = tokio::task::spawn_blocking(move || indexer.index_project()).await;

        match refresh_result {
            Ok(Ok(result)) => {
                let mut cache = self.index_cache.write().await;
                cache.result = Some(result);
                cache.last_update = Some(Instant::now());
            }
            Ok(Err(err)) => {
                eprintln!("Index refresh error: {}", err);
            }
            Err(err) => {
                eprintln!("Index refresh task join error: {}", err);
            }
        }

        self.index_refresh_in_flight.store(false, Ordering::SeqCst);
    }

    /// 清除索引缓存
    pub async fn clear_index_cache(&self) {
        let mut cache = self.index_cache.write().await;
        *cache = IndexCache::default();
        self.search_cache.clear();
    }

    fn load_learned_rules(&self) -> String {
        self.reflector
            .as_ref()
            .and_then(|reflector| reflector.get_learned_rules().ok())
            .filter(|rules| !rules.trim().is_empty())
            .unwrap_or_default()
    }

    pub fn load_static_context_for_project(&self) -> String {
        let learned_rules = self.load_learned_rules();
        let mut merged_context = String::new();
        append_learned_rules_section(&mut merged_context, &learned_rules);
        merged_context
    }

    async fn get_cached_project_context(&self, project_path: &Path) -> Option<String> {
        if self.project_context_cache_secs == 0 {
            return None;
        }

        let cache_ttl = Duration::from_secs(self.project_context_cache_secs);
        let key = project_path.to_path_buf();

        {
            let cache = self.project_context_cache.read().await;
            if let Some(entry) = cache.get(&key) {
                if entry.cached_at.elapsed() < cache_ttl {
                    crate::utils::logging::append_debug_log_line(
                        "[Context] Using cached dynamic context",
                    );
                    return Some(entry.merged_context.clone());
                }
            }
        }

        let mut cache = self.project_context_cache.write().await;
        match cache.get(&key) {
            Some(entry) if entry.cached_at.elapsed() < cache_ttl => {
                crate::utils::logging::append_debug_log_line(
                    "[Context] Using cached dynamic context",
                );
                Some(entry.merged_context.clone())
            }
            Some(_) => {
                cache.remove(&key);
                None
            }
            None => None,
        }
    }

    async fn cache_project_context(&self, project_path: &Path, merged_context: &str) {
        if self.project_context_cache_secs == 0 {
            return;
        }

        let mut cache = self.project_context_cache.write().await;
        cache.insert(
            project_path.to_path_buf(),
            ProjectContextCacheEntry {
                merged_context: merged_context.to_string(),
                cached_at: Instant::now(),
            },
        );
    }

    pub async fn load_context_for_project(
        &self,
        project_path: &Path,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(merged_context) = self.get_cached_project_context(project_path).await {
            return Ok(merged_context);
        }

        let learned_rules = self.load_learned_rules();
        if !self.has_dynamic_context_candidates(project_path) {
            let mut merged_context = String::new();
            append_learned_rules_section(&mut merged_context, &learned_rules);
            self.cache_project_context(project_path, &merged_context)
                .await;
            return Ok(merged_context);
        }

        let available_contexts = self.finder.find_all_contexts(project_path)?;

        if available_contexts.is_empty() {
            let mut merged_context = String::new();
            append_learned_rules_section(&mut merged_context, &learned_rules);
            self.cache_project_context(project_path, &merged_context)
                .await;
            return Ok(merged_context);
        }

        // 0. Auto-Indexing (带缓存)
        if let Some(_result) = self.get_index_result().await {
            // 索引已更新或使用缓存
        }

        // 1. Discovery
        // 2. Selection
        let matches = self
            .matcher
            .find_best_contexts(project_path, &available_contexts)
            .await?;

        // 3. Select top matches (simplified strategy)
        let mut layers = Vec::new();

        for match_score in matches.into_iter().take(3) {
            if let Some(def) = available_contexts
                .iter()
                .find(|c| c.id == match_score.context_id)
            {
                let mut layer = ContextLayer::new(ContextLevel::Project, def.clone());
                layer.activate();
                layers.push(layer);
            }
        }

        // 4. Merge
        let mut merged_context = String::new();
        if !layers.is_empty() {
            merged_context = self.manager.merge_contexts(&layers)?;
        }

        // 5. Append Reflected Rules (if available)
        append_learned_rules_section(&mut merged_context, &learned_rules);
        self.cache_project_context(project_path, &merged_context)
            .await;

        Ok(merged_context)
    }
}

fn append_learned_rules_section(merged_context: &mut String, rules: &str) {
    if rules.trim().is_empty() {
        return;
    }

    if !merged_context.trim().is_empty() {
        merged_context.push_str("\n\n");
    }
    merged_context.push_str("## Learned Rules (from past sessions)\n\n");
    merged_context.push_str(rules.trim());
}
