// ── Search Engine Cache Manager ──────────────────────────────────────────────
//
// Replaces the global static Mutex caches that were in semantic_search.rs.
// Uses parking_lot::RwLock (no poisoning) + LRU eviction for bounded memory.
//
// Architecture decision: caches are owned by ContextEngine and passed to
// semantic search functions via Arc. When None (standalone/test), caching
// is skipped entirely and search engine is built fresh each call.

use super::search_engine::SearchEngine;
use lru::LruCache;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::SystemTime;

/// Cached search engine with validity signal (mtime of index.json).
#[derive(Clone)]
pub struct CachedSearchEngine {
    pub engine: SearchEngine,
    pub index_mtime: Option<SystemTime>,
}

/// Maximum number of cached search engines (one per project + budget profile).
const MAX_CACHED_ENGINES: usize = 3;
/// Maximum number of cached query results.
const MAX_CACHED_QUERIES: usize = 64;

/// 引擎的轻量元信息（不克隆整个引擎），供状态展示与预算核算。
#[derive(Debug, Clone, Copy)]
pub struct EngineMeta {
    pub doc_count: usize,
    pub total_bytes: u64,
    pub index_mtime: Option<SystemTime>,
}

/// `patch_engine` 的结果。
///
/// `Missing` / `Stale` 都意味着"缓存里的引擎不可作为补丁基底"，
/// 调用方应回退全量重建。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOutcome {
    /// 补丁已在写锁内应用（无论是否真的改动了引擎）。
    Applied,
    /// 该 key 没有缓存引擎。
    Missing,
    /// 缓存引擎的 mtime 与调用方持有的令牌不符 —— 有别的构建抢先落地。
    Stale,
}

/// Thread-safe cache manager for search engines and query results.
///
/// Uses parking_lot locks exclusively: they do not track poisoning, so a
/// panic in a lock-holding thread simply releases the lock. This is the
/// correct behaviour for caches where data can always be rebuilt.
pub struct SearchEngineCacheManager {
    /// Cached pre-built SearchEngines, keyed by (canonical_project_root, budget_profile_key).
    engine_cache: RwLock<HashMap<(PathBuf, String), CachedSearchEngine>>,
    /// LRU query result cache, keyed by (canonical_root, query, index_mtime).
    query_cache: RwLock<LruCache<(PathBuf, String, Option<SystemTime>), String>>,
}

impl SearchEngineCacheManager {
    pub fn new() -> Self {
        Self {
            engine_cache: RwLock::new(HashMap::new()),
            query_cache: RwLock::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_QUERIES).unwrap(),
            )),
        }
    }

    /// Look up a cached search engine. Returns None if not found or mtime mismatch.
    pub fn get_engine(
        &self,
        key: &(PathBuf, String),
        current_mtime: Option<SystemTime>,
    ) -> Option<SearchEngine> {
        let cache = self.engine_cache.read();
        cache.get(key).and_then(|cached| {
            if cached.index_mtime == current_mtime && current_mtime.is_some() {
                Some(cached.engine.clone())
            } else {
                None
            }
        })
    }

    /// 取最近一次构建的引擎（无论 mtime 是否匹配），供 serve-stale-while-rebuild：
    /// 索引过期时先拿旧引擎立即回答，后台重建完成后自然切换。
    pub fn get_engine_stale(
        &self,
        key: &(PathBuf, String),
    ) -> Option<(SearchEngine, Option<SystemTime>)> {
        let cache = self.engine_cache.read();
        cache
            .get(key)
            .map(|cached| (cached.engine.clone(), cached.index_mtime))
    }

    /// 引擎的元信息快照（不克隆引擎本体）。
    pub fn engine_meta(&self, key: &(PathBuf, String)) -> Option<EngineMeta> {
        let cache = self.engine_cache.read();
        cache.get(key).map(|cached| EngineMeta {
            doc_count: cached.engine.doc_count(),
            total_bytes: cached.engine.total_bytes(),
            index_mtime: cached.index_mtime,
        })
    }

    /// 在写锁内对一个已缓存引擎施加增量补丁。
    ///
    /// `expected_mtime` 就是并发令牌：它在**写锁内**与缓存当前 mtime 比对，
    /// 因此 `Stale` 检测天然 race-free，不需要引入额外的锁。
    ///
    /// **`Applied` 时无条件把 mtime 盖成 `new_mtime`** —— 即使 `f` 返回 `false`
    /// （无实际改动，例如变更文件全是非索引类型）。否则 `get_engine` 永远不命中，
    /// 查询路径每轮都会触发一次刷新，形成死循环。这是本设计最隐蔽的正确性点。
    pub fn patch_engine<F>(
        &self,
        key: &(PathBuf, String),
        expected_mtime: Option<SystemTime>,
        new_mtime: Option<SystemTime>,
        f: F,
    ) -> PatchOutcome
    where
        F: FnOnce(&mut SearchEngine) -> bool,
    {
        let mut cache = self.engine_cache.write();
        let Some(cached) = cache.get_mut(key) else {
            return PatchOutcome::Missing;
        };
        if cached.index_mtime != expected_mtime {
            return PatchOutcome::Stale;
        }
        let _changed = f(&mut cached.engine);
        cached.index_mtime = new_mtime;
        PatchOutcome::Applied
    }

    /// Store a search engine in the cache. Evicts oldest entry if at capacity.
    pub fn put_engine(&self, key: (PathBuf, String), cached: CachedSearchEngine) {
        let mut cache = self.engine_cache.write();
        if cache.len() >= MAX_CACHED_ENGINES && !cache.contains_key(&key) {
            // Evict a random entry (simple FIFO via drain)
            if let Some(oldest_key) = cache.iter().next().map(|(k, _)| k.clone()) {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(key, cached);
    }

    /// Look up a cached query result.
    pub fn get_query(&self, key: &(PathBuf, String, Option<SystemTime>)) -> Option<String> {
        self.query_cache.write().get(key).cloned()
    }

    /// Store a query result in the LRU cache.
    pub fn put_query(&self, key: (PathBuf, String, Option<SystemTime>), output: String) {
        self.query_cache.write().put(key, output);
    }

    /// Clear all caches.
    pub fn clear(&self) {
        self.engine_cache.write().clear();
        self.query_cache.write().clear();
    }
}

impl Default for SearchEngineCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::chunking::CodeChunk;
    use std::time::Duration;

    fn test_engine(paths: &[&str]) -> SearchEngine {
        let mut engine = SearchEngine::new();
        for path in paths {
            engine.add_document(
                path.to_string(),
                vec![CodeChunk {
                    content: format!("fn marker_for_{}() {{}}", path.replace(['/', '.'], "_")),
                    start_line: 1,
                    end_line: 1,
                    context_header: None,
                }],
            );
        }
        engine
    }

    fn key(name: &str) -> (PathBuf, String) {
        (PathBuf::from(name), "f1b1t1".to_string())
    }

    /// 本设计最隐蔽的正确性点：**无操作补丁也必须盖 mtime**。
    ///
    /// 变更文件全是不可索引类型时（比如只动了 `target/` 或 `.svg`），补丁
    /// 什么都没改。如果此时不盖 mtime，`get_engine` 就永远不命中 →
    /// 查询路径每轮都判定"引擎过期" → 触发一次刷新 → 又什么都没改 →
    /// 下一轮再触发。这是活锁，不是慢。
    #[test]
    fn no_op_patch_still_stamps_mtime() {
        let cache = SearchEngineCacheManager::new();
        let k = key("/proj/noop");
        let t0 = SystemTime::now();
        let t1 = t0 + Duration::from_secs(5);

        cache.put_engine(
            k.clone(),
            CachedSearchEngine {
                engine: test_engine(&["a.rs"]),
                index_mtime: Some(t0),
            },
        );
        assert!(
            cache.get_engine(&k, Some(t1)).is_none(),
            "新 mtime 下不该命中"
        );

        // 闭包返回 false：没有任何实际改动。
        let outcome = cache.patch_engine(&k, Some(t0), Some(t1), |_| false);
        assert_eq!(outcome, PatchOutcome::Applied);

        assert!(
            cache.get_engine(&k, Some(t1)).is_some(),
            "无操作补丁后必须以新 mtime 命中，否则查询路径会陷入刷新活锁"
        );
        assert_eq!(cache.engine_meta(&k).unwrap().index_mtime, Some(t1));
    }

    #[test]
    fn stale_token_is_rejected_without_touching_the_engine() {
        let cache = SearchEngineCacheManager::new();
        let k = key("/proj/stale");
        let t0 = SystemTime::now();
        cache.put_engine(
            k.clone(),
            CachedSearchEngine {
                engine: test_engine(&["a.rs"]),
                index_mtime: Some(t0),
            },
        );

        // 调用方手里的令牌与缓存当前 mtime 不符 —— 说明有别的构建抢先落地。
        let wrong = Some(t0 + Duration::from_secs(1));
        assert_eq!(
            cache.patch_engine(&k, wrong, wrong, |_| true),
            PatchOutcome::Stale
        );

        // Stale 时引擎与 mtime 都不能被碰，否则会覆盖掉赢家那次的成果。
        assert_eq!(cache.engine_meta(&k).unwrap().index_mtime, Some(t0));
        assert_eq!(cache.engine_meta(&k).unwrap().doc_count, 1);
    }

    #[test]
    fn missing_key_reports_missing() {
        let cache = SearchEngineCacheManager::new();
        assert_eq!(
            cache.patch_engine(&key("/proj/absent"), None, None, |_| true),
            PatchOutcome::Missing
        );
    }

    /// mtime 为 None（index.json 从未写过）时 `get_engine` 不应命中 ——
    /// 没有令牌就没有并发校验，此时必须回退全量重建。
    #[test]
    fn engine_without_mtime_never_hits() {
        let cache = SearchEngineCacheManager::new();
        let k = key("/proj/nomtime");
        cache.put_engine(
            k.clone(),
            CachedSearchEngine {
                engine: test_engine(&["a.rs"]),
                index_mtime: None,
            },
        );

        assert!(cache.get_engine(&k, None).is_none());
        assert!(cache.get_engine(&k, Some(SystemTime::now())).is_none());
        // 但 serve-stale 仍然拿得到它 —— 旧引擎总比没有强。
        assert!(cache.get_engine_stale(&k).is_some());
    }
}
