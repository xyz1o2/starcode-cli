use crate::core::context::call_graph::{self, CallGraph};
use crate::core::context::chunking::SmartChunker;
use crate::core::context::fusion::{self, Candidate, RrfParams};
use crate::core::context::reranker::{HeuristicReranker, RerankCandidate, Reranker};
use crate::core::context::search_cache::{CachedSearchEngine, SearchEngineCacheManager};
use crate::core::context::search_engine::{SearchEngine, SearchOptions};
use crate::core::context::symbol::{self, FileSymbols};
use crate::core::tools::ripgrep::{search_with_ripgrep, RipgrepConfig};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// StarCode 自己的状态目录 —— 索引它纯属噪声，且用户的 `.gitignore` 未必写了。
const CODEBASE_SEARCH_EXCLUDES: &[&str] = &[".star/"];
const DEFAULT_CODEBASE_SEARCH_MAX_FILES: usize = 1200;
const DEFAULT_CODEBASE_SEARCH_MAX_FILE_BYTES: u64 = 512 * 1024;
const DEFAULT_CODEBASE_SEARCH_MAX_TOTAL_BYTES: u64 = 12 * 1024 * 1024;
const DEFAULT_CODEBASE_SEARCH_TIMEOUT_MS: u64 = 12_000;
const DEFAULT_AUTO_CODEBASE_SEARCH_MAX_FILES: usize = 320;
const DEFAULT_AUTO_CODEBASE_SEARCH_MAX_FILE_BYTES: u64 = 256 * 1024;
const DEFAULT_AUTO_CODEBASE_SEARCH_MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_AUTO_CODEBASE_SEARCH_TIMEOUT_MS: u64 = 5_000;
const CODEBASE_SEARCH_PROGRESS_EVERY_FILES: usize = 120;

/// 语义索引可纳入的**唯一权威**扩展名白名单。
///
/// 以前这个口径在四处各写了一份（本文件、`chunking.rs`、`integration.rs`、
/// `commands/mod.rs`），任何一处改动都会让"哪些文件进了索引"分叉。
/// 全量构建与增量补丁都必须经过 [`is_indexable_ext`]，白名单不可能再分叉。
pub const CODEBASE_INDEXABLE_EXTS: &[&str] = &[
    // tree-sitter 可解析
    "rs", "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "tsx", "go", "java", "c", "h", "cpp", "cc",
    "cxx", "hpp", "hxx", // 纯文本 / 配置（走 SmartChunker 启发式分层）
    "md", "txt", "json", "toml", "yaml", "yml",
];

/// 该扩展名是否应进入语义索引（单一事实源）。
pub fn is_indexable_ext(ext: &str) -> bool {
    CODEBASE_INDEXABLE_EXTS.contains(&ext)
}

#[derive(Clone)]
pub struct CodebaseSearchTool {
    config: Arc<crate::core::config::Config>,
    search_cache: Option<Arc<SearchEngineCacheManager>>,
}

impl CodebaseSearchTool {
    /// 构造工具。
    ///
    /// 默认挂上**进程级共享**的搜索引擎缓存。以前这里是 `None` ——
    /// 意味着每次检索都同步全量重建索引（实测 851 文件 / 10 MB / 11 秒），
    /// 而 watcher 明明已经在后台把索引建好了 —— 只是两边不是同一个
    /// cache manager，工具读不到。
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self {
            config,
            search_cache: Some(crate::core::context::search_cache::shared_search_cache()),
        }
    }

    pub fn with_cache(
        config: Arc<crate::core::config::Config>,
        search_cache: Arc<SearchEngineCacheManager>,
    ) -> Self {
        Self {
            config,
            search_cache: Some(search_cache),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CodebaseSearchParams {
    pub query: String,
    pub path: Option<String>,
    #[serde(default)]
    pub budget_profile: Option<String>,
}

pub struct CodebaseSearchInvocation {
    tool: CodebaseSearchTool,
    params: CodebaseSearchParams,
}

#[derive(Clone, Copy, Debug)]
pub struct CodebaseSearchLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub timeout_ms: u64,
}

impl CodebaseSearchLimits {
    /// 缓存键 —— **只认引擎，不认预算档位**。
    ///
    /// 以前 key 里带着 max_files/max_total_bytes/timeout，于是
    /// `budget_profile="auto"`（f320b4M）会开出一个**watcher 永远不会填的**
    /// 缓存命名空间：查询永远 cache miss → 永远落到 Warming → 模型看到
    /// "索引还在构建"就改用 Grep 绕开（现场：连续两次 0 ranked results，
    /// 而同样的 query 在默认预算下能命中 40 条）。
    ///
    /// 引擎内容只由默认预算决定（watcher 用它建），预算档位退化成"输出多少"
    /// 的旋钮，不该影响缓存身份。
    pub fn cache_key(&self) -> String {
        "engine".to_string()
    }
}

#[derive(Default)]
struct IndexStats {
    indexed_files: usize,
    scanned_text_files: usize,
    skipped_large_files: usize,
    total_bytes: u64,
    truncated: bool,
}

type ProgressCallback = Arc<dyn Fn(String) + Send + Sync>;

/// Read the mtime of the Indexer's `index.json` for the given project root.
/// Returns `None` when the file doesn't exist (project never indexed).
fn index_mtime(project_root: &Path) -> Option<SystemTime> {
    let index_path = project_root
        .join(".star")
        .join("context")
        .join("index.json");
    std::fs::metadata(&index_path)
        .and_then(|m| m.modified())
        .ok()
}

/// Error record for a single file that failed indexing (does not stop the batch).
#[derive(Debug, Clone)]
struct FileIndexError {
    path: String,
    reason: String,
}

/// 单文件接纳决策 —— 全量构建与增量补丁的**唯一**策略点。
///
/// 以前"哪些文件进索引"这条规则散落在全量循环里，增量路径要再写一遍，
/// 两边迟早分叉。现在所有"这个文件该不该进索引"的问题都只有这一个答案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// 纳入索引。
    Admit,
    /// 扩展名不在白名单里。
    SkipExt,
    /// 超过单文件上限。
    ///
    /// **调用方注意**：在增量路径里这必须被处理成 `Remove` ——
    /// 该文件可能**原本就在索引里**，只是刚刚涨过了上限。
    /// 只"跳过"而不移除，会永久留下一个陈旧文档。
    SkipTooLarge,
    /// 文件数或字节预算耗尽。
    ///
    /// 全量构建把它当"截断"（有明确的截断语义）；增量补丁没有合理的
    /// 逐出策略，调用方应回退全量重建。
    SkipBudget,
}

/// 判断一个文件能否进入索引。
///
/// `replaced_bytes` 是**本次会先释放掉**的字节数（重写/重命名时旧文档的大小）。
/// 预算核算必须先扣掉它：否则把一个 5 MB 文件重写成同样 5 MB，会因为
/// "当前已用 5 MB，再加 5 MB 超上限"而被拒 —— 但它压根没让语料变大。
fn admit_file(
    path: &Path,
    size: u64,
    used_files: usize,
    used_bytes: u64,
    replaced_bytes: u64,
    limits: CodebaseSearchLimits,
) -> Admission {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return Admission::SkipExt;
    };
    if !is_indexable_ext(ext) {
        return Admission::SkipExt;
    }
    if size > limits.max_file_bytes {
        return Admission::SkipTooLarge;
    }
    let effective_bytes = used_bytes.saturating_sub(replaced_bytes);
    if used_files >= limits.max_files
        || effective_bytes.saturating_add(size) > limits.max_total_bytes
    {
        return Admission::SkipBudget;
    }
    Admission::Admit
}

/// Build a fresh SearchEngine by traversing the filesystem (cold start).
///
/// **Per-file isolation**: tree-sitter chunking is wrapped in `catch_unwind` on a
/// dedicated large-stack thread.  A panic or parse failure in one file does **not**
/// stop the rest of the batch — partial results are always returned.
fn build_search_engine_from_fs(
    root: &Path,
    limits: CodebaseSearchLimits,
    update_output: &Option<ProgressCallback>,
) -> Result<(SearchEngine, IndexStats), Box<dyn std::error::Error + Send + Sync>> {
    let mut engine = SearchEngine::new();
    let mut stats = IndexStats::default();
    let mut last_progress_indexed = 0usize;
    let mut file_errors: Vec<FileIndexError> = Vec::new();

    // 遍历口径统一走 `utils::file_walk`：`.gitignore`（含 `require_git(false)`）、
    // `.starignore`、`~/.star/ignore` 都生效，dotfile 可见。以前只有
    // `hidden(true) + git_ignore(true)`，没有 `require_git(false)` —— worktree
    // 和还没 `git init` 的项目里 `.gitignore` 直接失效。
    let walker = crate::utils::file_walk::walk(root, &walk_options());

    emit_progress(
        update_output,
        format!("Indexing codebase · root {}", root.display()),
    );

    for result in walker {
        if stats.indexed_files >= limits.max_files || stats.total_bytes >= limits.max_total_bytes {
            stats.truncated = true;
            emit_progress(
                update_output,
                format!(
                    "Reached scan budget · {} indexed files · {:.1} MB",
                    stats.indexed_files,
                    bytes_to_mb(stats.total_bytes)
                ),
            );
            break;
        }

        match result {
            Ok(entry) => {
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }

                let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                    continue;
                };

                let file_size = match std::fs::metadata(path) {
                    Ok(meta) => meta.len(),
                    Err(_) => continue,
                };

                // 与增量路径共用同一个接纳策略点（全量构建没有"替换"，
                // 所以 replaced_bytes 恒为 0）。
                let admission = admit_file(
                    path,
                    file_size,
                    stats.indexed_files,
                    stats.total_bytes,
                    0,
                    limits,
                );
                if admission != Admission::SkipExt {
                    stats.scanned_text_files += 1;
                }
                match admission {
                    Admission::Admit => {}
                    Admission::SkipExt => continue,
                    Admission::SkipTooLarge => {
                        stats.skipped_large_files += 1;
                        continue;
                    }
                    Admission::SkipBudget => {
                        stats.truncated = true;
                        emit_progress(
                            update_output,
                            format!(
                                "Reached scan budget · {} indexed files · {:.1} MB",
                                stats.indexed_files,
                                bytes_to_mb(stats.total_bytes)
                            ),
                        );
                        break;
                    }
                }

                {
                    match index_file_safe(path, ext, root) {
                        Ok((rel_path, chunks)) => {
                            if !chunks.is_empty() {
                                // 预算口径必须是 **chunk 内容字节**，而不是文件字节。
                                // 增量补丁在锁内用的是 `engine.total_bytes()`
                                // （= 各 chunk content 长度之和）；全量这边若继续用
                                // `file_size`，同一份语料在两条路径下就会算出不同的
                                // "已用多少"，`admit_file` 这个单一策略点也就白设了。
                                let chunk_bytes: u64 =
                                    chunks.iter().map(|c| c.content.len() as u64).sum();
                                engine.add_document(rel_path, chunks);
                                stats.indexed_files += 1;
                                stats.total_bytes += chunk_bytes;

                                if stats.indexed_files == 1
                                    || stats.indexed_files.saturating_sub(last_progress_indexed)
                                        >= CODEBASE_SEARCH_PROGRESS_EVERY_FILES
                                {
                                    last_progress_indexed = stats.indexed_files;
                                    emit_progress(update_output, format_progress(&stats));
                                }
                            } else {
                                // tree-sitter returned empty chunks → log and skip
                                file_errors.push(FileIndexError {
                                    path: path.display().to_string(),
                                    reason: "empty chunks after parsing (likely tree-sitter error recovery)".into(),
                                });
                            }
                        }
                        Err(err) => {
                            file_errors.push(err);
                        }
                    }
                }
            }
            Err(err) => {
                emit_progress(update_output, format!("Walk error (non-fatal): {}", err));
            }
        }
    }

    // Report per-file errors once (first N only to avoid flooding).
    if !file_errors.is_empty() {
        let total_errs = file_errors.len();
        let shown = file_errors.iter().take(5).collect::<Vec<_>>();
        let mut msg = format!("Indexing completed with {} file errors:", total_errs);
        for e in shown {
            msg.push_str(&format!("\n  - {}: {}", e.path, e.reason));
        }
        if total_errs > 5 {
            msg.push_str(&format!("\n  ... and {} more", total_errs - 5));
        }
        emit_progress(update_output, msg);
    }

    Ok((engine, stats))
}

/// Index a single file with panic isolation for tree-sitter chunking.
///
/// Runs chunking in a dedicated thread with 8 MiB stack + `catch_unwind`.
/// If tree-sitter panics (Rust binding layer), the panic is caught and
/// returned as an error — the calling batch loop can continue.
fn index_file_safe(
    path: &Path,
    ext: &str,
    root: &Path,
) -> Result<(String, Vec<crate::core::context::chunking::CodeChunk>), FileIndexError> {
    let content = std::fs::read_to_string(path).map_err(|e| FileIndexError {
        path: path.display().to_string(),
        reason: format!("I/O error: {}", e),
    })?;

    let rel_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // Run tree-sitter chunking on a dedicated thread with large stack.
    // catch_unwind catches Rust-level panics from tree-sitter bindings
    // (unwrap failures, integer overflows). C-level abort() from tree-sitter
    // assertions can still kill the process — this is a best-effort defence.
    let content_owned = content.clone();
    let ext_owned = ext.to_string();

    let chunks_result = std::thread::Builder::new()
        .name("star-ts-index".into())
        .stack_size(8 * 1024 * 1024) // 8 MiB — generous for deeply nested files
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                SmartChunker::chunk(&content_owned, &ext_owned)
            }))
        })
        .map_err(|e| FileIndexError {
            path: rel_path.clone(),
            reason: format!("thread spawn failed: {}", e),
        })?
        .join()
        .map_err(|_| FileIndexError {
            path: rel_path.clone(),
            reason: "chunking thread panicked (join error)".into(),
        })?;

    match chunks_result {
        Ok(chunks) => Ok((rel_path, chunks)),
        Err(_panic) => Err(FileIndexError {
            path: rel_path,
            reason: "tree-sitter panic caught by catch_unwind".into(),
        }),
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn codebase_search_limits() -> CodebaseSearchLimits {
    codebase_search_limits_for_profile(None)
}

fn codebase_search_limits_for_profile(profile: Option<&str>) -> CodebaseSearchLimits {
    let normalized = profile.map(|value| value.trim().to_ascii_lowercase());
    let is_auto_budget = matches!(normalized.as_deref(), Some("auto" | "fast" | "quick"));

    if is_auto_budget {
        return CodebaseSearchLimits {
            max_files: env_usize(
                "STAR_AUTO_CODEBASE_SEARCH_MAX_FILES",
                DEFAULT_AUTO_CODEBASE_SEARCH_MAX_FILES,
            ),
            max_file_bytes: env_u64(
                "STAR_AUTO_CODEBASE_SEARCH_MAX_FILE_BYTES",
                DEFAULT_AUTO_CODEBASE_SEARCH_MAX_FILE_BYTES,
            ),
            max_total_bytes: env_u64(
                "STAR_AUTO_CODEBASE_SEARCH_MAX_TOTAL_BYTES",
                DEFAULT_AUTO_CODEBASE_SEARCH_MAX_TOTAL_BYTES,
            ),
            timeout_ms: env_u64(
                "STAR_AUTO_CODEBASE_SEARCH_TIMEOUT_MS",
                DEFAULT_AUTO_CODEBASE_SEARCH_TIMEOUT_MS,
            ),
        };
    }

    CodebaseSearchLimits {
        max_files: env_usize(
            "STAR_CODEBASE_SEARCH_MAX_FILES",
            DEFAULT_CODEBASE_SEARCH_MAX_FILES,
        ),
        max_file_bytes: env_u64(
            "STAR_CODEBASE_SEARCH_MAX_FILE_BYTES",
            DEFAULT_CODEBASE_SEARCH_MAX_FILE_BYTES,
        ),
        max_total_bytes: env_u64(
            "STAR_CODEBASE_SEARCH_MAX_TOTAL_BYTES",
            DEFAULT_CODEBASE_SEARCH_MAX_TOTAL_BYTES,
        ),
        timeout_ms: env_u64(
            "STAR_CODEBASE_SEARCH_TIMEOUT_MS",
            DEFAULT_CODEBASE_SEARCH_TIMEOUT_MS,
        ),
    }
}

/// 索引用的遍历口径。
///
/// 硬编码黑名单原来有 13 个目录名（`build`、`dist`、`target`、`node_modules`
/// …），按**文件名**匹配，所以 `src/build/` 这种正常源码目录也会被整棵剪掉。
/// 那些目录本来就在 `.gitignore` 里，交给 ignore 文件即可；只有
/// [`CODEBASE_SEARCH_EXCLUDES`] 里的 agent 自身状态目录需要显式排除。
fn walk_options() -> crate::utils::file_walk::WalkOptions {
    crate::utils::file_walk::WalkOptions::new().exclude(CODEBASE_SEARCH_EXCLUDES.iter().copied())
}

async fn run_codebase_search(
    root_path: PathBuf,
    query: String,
    budget_profile: Option<String>,
    update_output: Option<ProgressCallback>,
    cache: Option<Arc<SearchEngineCacheManager>>,
) -> String {
    let limits = codebase_search_limits_for_profile(budget_profile.as_deref());
    let budget_label = match budget_profile
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("auto" | "fast" | "quick") => "fast budget",
        _ => "default budget",
    };
    emit_progress(
        &update_output,
        format!(
            "Preparing semantic search · {} · {} · max {} files · {}s timeout",
            root_path.display(),
            budget_label,
            limits.max_files,
            limits.timeout_ms / 1000
        ),
    );

    let root_for_search = root_path.clone();
    let query_for_search = query.clone();
    let update_for_search = update_output.clone();
    let limits_for_search = limits;
    let cache_for_search = cache.clone();

    // ── 冷启动：等索引就绪，而不是立刻返回 "still building" ──────────────
    //
    // 自动触发路径早就等过一次（见 `agent_tool_process.rs` 的 warmup wait），
    // 但**模型显式调用**这条路径没有 —— 于是模型拿到的是一句"还在构建，
    // 请重跑一次"，它照着做，一轮一轮地重试，直到会话耗尽（现场：连着 11 次
    // 同一条提示，每次都"重跑一次"，而索引压根没在推进）。
    //
    // 只在**引擎完全不存在**时等。三种状态要分清：
    // - Fresh（缓存里的 mtime 与磁盘一致）→ 直接用，不等；
    // - Stale（引擎在，但磁盘变了）→ 立刻用旧引擎回答（serve-stale），
    //   **也**不等 —— 否则"改一个文件"之后每一次检索都要白等 8 秒；
    // - Absent（冷启动，压根没有引擎）→ 这才是该等的情况。
    //
    // 等的是**工具自己手里那份缓存**（`cache`），不是某个全局单例：
    // 判据不一致就会出现"等了 8 秒还是 Warming"的白等。
    if let Some(cache_ref) = cache.as_ref() {
        let presence =
            crate::core::tools::codebase_search::root_engine_presence(cache_ref, &root_path);
        if presence == crate::core::tools::codebase_search::EnginePresence::Absent {
            let wait_budget = std::time::Duration::from_millis(
                std::env::var("STAR_CODEBASE_SEARCH_WARMUP_WAIT_MS")
                    .ok()
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(8_000),
            );
            emit_progress(
                &update_output,
                format!(
                    "Cross-file index is still building · waiting up to {}s for it",
                    wait_budget.as_millis() / 1000
                ),
            );
            crate::core::tools::codebase_search::wait_for_index_ready_in(
                cache_ref,
                &root_path,
                wait_budget,
            )
            .await;
        }
    }

    let search_future = tokio::task::spawn_blocking(move || {
        search_codebase_with_limits(
            &root_for_search,
            &query_for_search,
            update_for_search,
            limits_for_search,
            cache_for_search.as_ref(),
        )
    });
    let sleep = tokio::time::sleep(Duration::from_millis(limits.timeout_ms));
    tokio::pin!(search_future);
    tokio::pin!(sleep);

    let _result = tokio::select! {
        result = &mut search_future => {
            match result {
                Ok(Ok(res)) => return res,
                Ok(Err(e)) => {
                    emit_progress(
                        &update_output,
                        "Semantic search failed · falling back to text search",
                    );
                    let fallback = format_ripgrep_fallback(&query, &root_path);
                    return format!("Semantic search error: {}\n\n{}", e, fallback);
                }
                Err(e) => {
                    emit_progress(
                        &update_output,
                        "Semantic search worker crashed · falling back to text search",
                    );
                    let fallback = format_ripgrep_fallback(&query, &root_path);
                    return format!("Semantic search execution failed: {}\n\n{}", e, fallback);
                }
            }
        }
        _ = &mut sleep => {
            emit_progress(
                &update_output,
                "Semantic search timed out · falling back to text search",
            );
            let fallback = format_ripgrep_fallback(&query, &root_path);
            return format!(
                "Semantic search timed out after {}ms. Returning ripgrep fallback.\n\n{}",
                limits.timeout_ms, fallback
            );
        }
    };
}

pub async fn run_codebase_search_for_skill(root_path: std::path::PathBuf, query: String) -> String {
    run_codebase_search(root_path, query, None, None, None).await
}

impl ToolInvocation for CodebaseSearchInvocation {
    fn get_description(&self) -> String {
        // 别再叫 "Semantic Search"：这个词和"按语义检索"的承诺一起出现在 UI 上，
        // 会让使用者以为它答的是概念问题，而它其实是词法排序。
        format!("Ranked cross-file search: {}", self.params.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let query = self.params.query.clone();
        let path = self.params.path.clone();
        let budget_profile = self.params.budget_profile.clone();
        let cache = self.tool.search_cache.clone();

        Box::pin(async move {
            let root_path = resolve_codebase_search_root(&self.tool.config, path.as_deref());

            let results =
                run_codebase_search(root_path, query, budget_profile, update_output, cache).await;

            Ok(CoreToolResult {
                llm_content: Some(results.clone()),
                return_display: Some(results.clone()),
                output: results,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for CodebaseSearchTool {
    fn name(&self) -> &str {
        "CodebaseSearch"
    }

    fn display_name(&self) -> &str {
        "Codebase Search"
    }

    fn description(&self) -> &str {
        // 与 `tool-description-codebase_search.md` 的 frontmatter 同一句话。
        // 两份各写一遍时它们漂移过：schema 说"按语义"，指南说"按词法"，
        // 模型收到矛盾指令后退回 Grep，这个工具就永远不触发。
        // 权威分工见 `core::prompts::routing_matrix::ROUTING_MATRIX`
        // （槽 `ranked-cross-file-locate`）。
        // 注意长度受 `compact_tool_definition` 的 160 字符上限约束，
        // `core::prompts::tests::ace_tool_descriptions_fit_compact_limit` 会盯着。
        "Ranked cross-file search — top relevant regions with match signals, not every raw match. The index is LEXICAL: the query must be code-side terms (identifiers, type names, technical nouns, path fragments) — never the user's sentence verbatim. Use when you know roughly the code's words; for exact strings use Grep."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Code-side keywords whose terms appear in the code (e.g. 'retry backoff', 'parse config toml'). Do not pass the user's sentence — the index holds identifiers, paths and English comments, so a Chinese or question-shaped phrasing matches nothing. If the request is in another language, translate it yourself into the words the code would use. Keywords, not a full question: a sentence dilutes the ranking."
                },
                "path": {
                    "type": "string",
                    "description": "Root directory to search (default: current directory)"
                },
                "budget_profile": {
                    "type": "string",
                    "description": "Optional search budget profile. Use 'auto' for a faster, more conservative scan budget."
                }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: CodebaseSearchParams = serde_json::from_value(params)?;
        Ok(Box::new(CodebaseSearchInvocation {
            tool: self.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

pub fn search_codebase(
    root: &Path,
    query: &str,
    update_output: Option<ProgressCallback>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    search_codebase_with_limits(root, query, update_output, codebase_search_limits(), None)
}

/// 后台重建引擎并写入缓存（watcher 协调器的 worker 线程调用）。
/// 返回本次索引的文件数。
pub fn build_engine_into_cache(
    root: &Path,
    cache: &Arc<SearchEngineCacheManager>,
    update_output: Option<ProgressCallback>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let limits = codebase_search_limits();
    let (engine, stats) = build_search_engine_from_fs(root, limits, &update_output)?;
    let key = (
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        limits.cache_key(),
    );
    cache.put_engine(
        key,
        CachedSearchEngine {
            engine,
            index_mtime: index_mtime(root),
        },
    );
    Ok(stats.indexed_files)
}

// ── 语义引擎增量更新 ──────────────────────────────────────────────────────────

/// 增量补丁的单条操作。
///
/// `size` 随 `Upsert` 一起带着走：预算核算必须在**写锁内**用引擎的实时数字做，
/// 而分块在锁外 —— 不带上原始大小，锁内就没法判预算。
#[derive(Debug)]
enum PatchOp {
    Upsert {
        path: String,
        size: u64,
        chunks: Vec<crate::core::context::chunking::CodeChunk>,
    },
    Remove {
        path: String,
    },
}

/// 语料小于这个数时，"变化比例"没有统计意义（3 个文件改 1 个就是 33%）。
const INCREMENTAL_MIN_CORPUS: usize = 8;
/// 小语料下的绝对阈值。
const INCREMENTAL_MIN_CHANGED: usize = 4;
/// 大语料下允许的变化比例上限。
const INCREMENTAL_MAX_CHANGE_RATIO: f64 = 0.30;

/// 变动太大时，逐条打补丁还不如从零重建 —— 两者都要扫全树，但重建没有
/// 簿记风险。这三个是**调优旋钮**，不是正确性旋钮。
fn should_full_rebuild(indexable_changed: usize, doc_count: usize) -> bool {
    if doc_count < INCREMENTAL_MIN_CORPUS {
        return indexable_changed >= INCREMENTAL_MIN_CHANGED;
    }
    (indexable_changed as f64) / (doc_count as f64) > INCREMENTAL_MAX_CHANGE_RATIO
}

/// 把权威变更集（`IndexResult`）翻译成补丁操作。
///
/// **分块在这里完成，也就是在锁外** —— tree-sitter 是纯 CPU 工作，
/// 绝不能握着引擎的写锁跑。锁内只做 O(触及的 posting) 的簿记。
///
/// 这里的接纳判断用零预算调用 `admit_file`，只会命中 `SkipExt` / `SkipTooLarge`：
/// 预算项在锁内用引擎的实时数字重判（`max_file_bytes < max_total_bytes`，
/// 所以单个文件不可能在这里就撞上总预算）。
fn plan_patch_ops(
    root: &Path,
    index_result: &crate::core::context::indexer::IndexResult,
    limits: CodebaseSearchLimits,
) -> Vec<PatchOp> {
    let mut ops =
        Vec::with_capacity(index_result.new_blobs.len() + index_result.removed_blobs.len());

    // 删除先入队：先把预算释放掉，重命名（removed=[old] + new=[new]）才不会
    // 因为"旧的还没扣、新的就要加"而误触上限。
    for path in &index_result.removed_blobs {
        ops.push(PatchOp::Remove { path: path.clone() });
    }

    for blob in &index_result.new_blobs {
        let full_path = root.join(&blob.path);
        let Ok(meta) = std::fs::metadata(&full_path) else {
            // 文件在 index_project 与这里之间消失了 —— 当作删除处理，
            // 否则会留下一个永远删不掉的陈旧文档。
            ops.push(PatchOp::Remove {
                path: blob.path.clone(),
            });
            continue;
        };
        let size = meta.len();

        match admit_file(&full_path, size, 0, 0, 0, limits) {
            Admission::Admit => {}
            // `SkipExt` 可以安全地**什么都不做**：扩展名是从路径派生的，
            // `assets/logo.svg` 不可能曾经进过索引（否则当初就过不了白名单），
            // 所以不存在需要清理的陈旧文档。发一条 Remove 只会让
            // `changed` 计数虚高，还会白跑一次 remove_document。
            Admission::SkipExt => continue,
            // 而 `SkipTooLarge` 必须表达成 `Remove` —— 同一个 .rs 路径完全可能
            // 原本就在索引里，只是刚刚涨过了上限。只跳过不删除，就会永久留下
            // 一个陈旧文档。这是本设计最容易被实现错的一条。
            Admission::SkipTooLarge | Admission::SkipBudget => {
                ops.push(PatchOp::Remove {
                    path: blob.path.clone(),
                });
                continue;
            }
        }

        let Some(ext) = full_path.extension().and_then(|s| s.to_str()) else {
            ops.push(PatchOp::Remove {
                path: blob.path.clone(),
            });
            continue;
        };

        match index_file_safe(&full_path, ext, root) {
            Ok((path, chunks)) if !chunks.is_empty() => {
                ops.push(PatchOp::Upsert { path, size, chunks });
            }
            Ok(_) => {
                // 分块结果为空（文件变空 / 纯空白 / 解析退化成零 chunk）：
                // 同样必须表达成移除，不能留陈旧文档。
                ops.push(PatchOp::Remove {
                    path: blob.path.clone(),
                });
            }
            Err(_) => {
                // 文件仍在，只是这轮解析失败 —— **保留旧版本**，
                // 不发任何操作。解析失败是暂时的，删掉就丢了。
            }
        }
    }

    ops
}

/// 在写锁内应用补丁。返回 `false` 表示预算饱和 —— 调用方应回退全量重建。
///
/// 全量构建有明确的截断语义（扫到哪算哪，并把 `truncated` 告诉用户）；
/// 补丁没有合理的逐出策略：中途撞上上限就既不能丢弃剩余文件（会留下
/// 半新半旧的索引），也不能悄悄超出上限。所以这里只能如实上报，
/// 让上层回退到全量。
fn apply_patch_ops(
    engine: &mut SearchEngine,
    ops: &[PatchOp],
    limits: CodebaseSearchLimits,
) -> bool {
    // 1. 先把移除全部做完，并**重算**已用字节 —— 移除释放的预算必须先回到池子里，
    //    否则重命名（删 5 MB 加 5 MB）会因为峰值而误判超限。
    for op in ops {
        if let PatchOp::Remove { path } = op {
            engine.remove_document(path);
        }
    }

    let mut used_files = engine.doc_count();
    let mut used_bytes = engine.total_bytes();
    // 一旦有任何一条因为预算被拒，整个补丁批次就不可信了 —— 见函数头的说明。
    let mut budget_exceeded = false;

    // 2. 再逐个 upsert，每步都用最新的已用数字判预算。
    for op in ops {
        let PatchOp::Upsert { path, size, chunks } = op else {
            continue;
        };
        let was_present = engine.contains_document(path);
        // 重写同一个文件时，它自己占的字节会先被替换掉，不算新增。
        let replaced_bytes = if was_present {
            engine.document_bytes(path)
        } else {
            0
        };

        let admission = admit_file(
            Path::new(path),
            *size,
            used_files,
            used_bytes,
            replaced_bytes,
            limits,
        );
        if admission != Admission::Admit {
            // 预算不够或不再合格 —— 移除（可能本来就不在，remove 会返回 false，无副作用）。
            engine.remove_document(path);
            // `SkipExt` / `SkipTooLarge` 是**正常**结果：这些文件本来就不该在索引里
            // （见 plan_patch_ops 的 skip==remove 不变式），不影响批次可信度。
            // 但 `SkipBudget` 不是 —— 它是"我本来想加，但装不下"。
            if admission == Admission::SkipBudget {
                budget_exceeded = true;
            }
            continue;
        }

        engine.upsert_document(path.clone(), chunks.clone());
        used_bytes = used_bytes.saturating_sub(replaced_bytes) + *size;
        if !was_present {
            used_files += 1;
        }
    }

    // 3. 复核总预算：单条都合格不代表累计合格（每步的 used_bytes 都在涨）。
    //    超了就交给上层全量重建 —— 全量会把语料截断到上限内，语义是明确的。
    !budget_exceeded && used_bytes <= limits.max_total_bytes && used_files <= limits.max_files
}

/// 增量更新结果。
#[derive(Debug)]
pub enum UpdateOutcome {
    /// 补丁已在锁内应用。`changed` 是实际发出的操作条数。
    Patched { changed: usize },
    /// 变化太大 / 引擎缺失 / mtime 不匹配 / 预算饱和 —— 已回退全量重建。
    FullRebuild { files: usize },
}

/// 用 `IndexResult` 增量更新缓存里的语义引擎（watcher 的重建 worker 调用）。
///
/// 任何一条"补丁基底不可信"的情形都回退全量重建，且 **`FullRebuild` 是成功**：
/// `resolve_engine` 依赖 `last_rebuild_failed` 决定是否回退同步构建，
/// 误报失败会平白禁用 Warming 快路径。
pub fn update_engine_in_cache(
    root: &Path,
    cache: &Arc<SearchEngineCacheManager>,
    index_result: &crate::core::context::indexer::IndexResult,
    update_output: Option<ProgressCallback>,
) -> Result<UpdateOutcome, Box<dyn std::error::Error + Send + Sync>> {
    update_engine_in_cache_with_limits(
        root,
        cache,
        index_result,
        codebase_search_limits(),
        update_output,
    )
}

/// 与 [`update_engine_in_cache`] 相同，但显式传入 limits。
///
/// 测试必须用这个版本：`default_limits()` 读进程级环境变量，
/// 而 Rust 测试并行跑，改 env 的测试会 flaky。
pub fn update_engine_in_cache_with_limits(
    root: &Path,
    cache: &Arc<SearchEngineCacheManager>,
    index_result: &crate::core::context::indexer::IndexResult,
    limits: CodebaseSearchLimits,
    update_output: Option<ProgressCallback>,
) -> Result<UpdateOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let key = (canonical_root, limits.cache_key());

    let full_rebuild = |update_output: &Option<ProgressCallback>|
     -> Result<UpdateOutcome, Box<dyn std::error::Error + Send + Sync>> {
        let (engine, stats) = build_search_engine_from_fs(root, limits, update_output)?;
        // mtime 必须在写盘**之后**读：build 链路里的 index_project() 会写
        // .star/context/index.json，令牌早于写盘读取就会过期，下一次
        // resolve_engine 的 get_engine 比对必然失败 —— 引擎明明构建完了，
        // 查询却永远命中 Warming/Stale，用户看到的就是"一直在构建"。
        let stored_mtime = index_mtime(root);
        cache.put_engine(
            key.clone(),
            CachedSearchEngine {
                engine,
                index_mtime: stored_mtime,
            },
        );
        Ok(UpdateOutcome::FullRebuild {
            files: stats.indexed_files,
        })
    };

    // ── 何时直接放弃补丁 ─────────────────────────────────────────────────────
    // 1) index.json 从未写过 → 没有 mtime 令牌，无从校验并发。
    // 2) 缓存里根本没有这个 key → 没有补丁基底。
    let Some(meta) = cache.engine_meta(&key) else {
        return full_rebuild(&update_output);
    };
    if meta.index_mtime.is_none() {
        return full_rebuild(&update_output);
    }

    // 冷语料：引擎是空的，而文件系统上明明有文件。补丁会把它从 0 补成 1 个文件，
    // 然后 get_engine 命中并**谎报 Fresh** —— 用户看到的"索引"只有一个文件。
    // 这种情形必须走全量。
    //
    // 判据用 `total_files`（index.json 里的条目数）而不是 `new_blobs`：
    // 冷启动时 index.json 往往**已经写好了**（文件索引先跑），此时
    // `new_blobs` 是空的，用它判断等于永远不触发 —— 守卫形同虚设。
    // `total_files` 才真正回答"文件系统上到底有没有东西"。
    if meta.doc_count == 0 && index_result.total_files > 0 {
        return full_rebuild(&update_output);
    }

    // 3) 变化太大，逐条补不如重建。
    //    `indexable_changed` 只数**通过白名单**的新增/修改文件 ——
    //    `git checkout` 碰 500 个 .lock/.svg 不该触发全量重建。
    //    删除不计入分子：删除永远比重建便宜（见 apply_patch_ops）。
    let indexable_changed = index_result
        .new_blobs
        .iter()
        .filter(|blob| {
            Path::new(&blob.path)
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(is_indexable_ext)
        })
        .count();
    if should_full_rebuild(indexable_changed, meta.doc_count) {
        emit_progress(
            &update_output,
            format!(
                "Incremental patch skipped · {} of {} docs changed · rebuilding",
                indexable_changed, meta.doc_count
            ),
        );
        return full_rebuild(&update_output);
    }

    // ── 分块（锁外）→ 补丁（锁内） ───────────────────────────────────────────
    // mtime 令牌必须在 index_project() 写盘**之后**读取，否则存进缓存的
    // 是旧令牌，下次查询的 get_engine 比对必然失败（"一直在构建"的根因）。
    let stored_mtime = index_mtime(root);
    let ops = plan_patch_ops(root, index_result, limits);
    if ops.is_empty() {
        // 变更文件全是不可索引类型 —— 但**仍然要把 mtime 盖上**，
        // 否则 get_engine 永不命中，每轮查询都触发一次刷新，形成死循环。
        // 这正是 patch_engine 里"Applied 时无条件盖 mtime"那条注释的由来。
        let _ = cache.patch_engine(&key, meta.index_mtime, stored_mtime, |_| false);
        return Ok(UpdateOutcome::Patched { changed: 0 });
    }

    let mut budget_exceeded = false;
    let outcome = cache.patch_engine(&key, meta.index_mtime, stored_mtime, |engine| {
        let ok = apply_patch_ops(engine, &ops, limits);
        budget_exceeded = !ok;
        true
    });

    match outcome {
        crate::core::context::search_cache::PatchOutcome::Applied => {
            if budget_exceeded {
                emit_progress(
                    &update_output,
                    "Incremental patch hit the budget ceiling · rebuilding from scratch",
                );
                // 补丁已经落进去了，但结果不可信 —— 全量会整体覆盖掉。
                return full_rebuild(&update_output);
            }
            Ok(UpdateOutcome::Patched { changed: ops.len() })
        }
        // 引擎不在（被 LRU 逐出）或 mtime 变了（有别的构建抢先落地）：
        // 都不构成"失败"，回退全量即可。
        crate::core::context::search_cache::PatchOutcome::Missing
        | crate::core::context::search_cache::PatchOutcome::Stale => {
            emit_progress(
                &update_output,
                "Patch base unavailable (evicted or superseded) · rebuilding from scratch",
            );
            full_rebuild(&update_output)
        }
    }
}

/// 引擎来源：决定查询结果的标注与兜底策略。
#[derive(Debug)]
enum EngineSource {
    /// 缓存命中且 mtime 匹配
    Fresh,
    /// mtime 不匹配，先拿旧版本回答，后台重建中
    Stale,
    /// 本会话还没有任何引擎（冷启动），后台构建中
    Warming,
}

/// 三级引擎解析（serve-stale-while-rebuild）：
/// 1. 新鲜缓存 → 直接用；
/// 2. 过期缓存 → **立即**返回旧引擎继续回答，同时触发协调器后台重建；
/// 3. 冷启动 → 触发后台构建并返回 Warming，让本次查询降级到 Grep，
///    绝不在查询路径上同步做全量构建卡住 agent。
/// 协调器不可用（未启动/上次后台重建失败）时回退旧的同步构建路径。
fn resolve_engine(
    root: &Path,
    limits: CodebaseSearchLimits,
    update_output: &Option<ProgressCallback>,
    cache: Option<&Arc<SearchEngineCacheManager>>,
) -> Result<(SearchEngine, IndexStats, EngineSource), Box<dyn std::error::Error + Send + Sync>> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let current_mtime = index_mtime(root);
    let cache_key = (canonical_root.clone(), limits.cache_key());

    if let Some(manager) = cache {
        if let Some(engine) = manager.get_engine(&cache_key, current_mtime) {
            return Ok((engine, IndexStats::default(), EngineSource::Fresh));
        }

        if let Some((stale_engine, _)) = manager.get_engine_stale(&cache_key) {
            crate::core::context::watcher::request_refresh(root);
            return Ok((stale_engine, IndexStats::default(), EngineSource::Stale));
        }

        // 冷启动：协调器可用且未发生过失败 → 后台构建 + Warming 兜底
        //
        // 但**只有查询的就是协调器实际在重建的那个 root** 时才能走这条路。
        // 协调器是进程级单例，它只重建第一次交给它的那棵树；对别的 root
        // （典型：把查询缩到子目录）走 Warming = 永远等不到引擎 —— 后台根本
        // 不会给那个 root 建索引，而查询侧一次又一次看到同一条 "still
        // building"。现场：缩到单目录后连着几次都是 Warming，而全仓查询
        // 同样等不到（协调器的 root 是启动时的 cwd，查询 root 与它不一致）。
        //
        // 非托管 root 直接同步构建：它没被后台覆盖，只能自己建。子目录通常
        // 很小，同步构建代价可接受；大目录则会走上面的超时兜底。
        let coordinator_engaged = crate::core::context::watcher::is_started();
        let root_is_managed = crate::core::context::watcher::rebuild_root()
            .map(|managed| managed == canonical_root)
            .unwrap_or(false);
        if coordinator_engaged
            && root_is_managed
            && !crate::core::context::watcher::last_rebuild_failed()
        {
            crate::core::context::watcher::request_refresh(root);
            return Ok((
                SearchEngine::new(),
                IndexStats::default(),
                EngineSource::Warming,
            ));
        }
        // 协调器不可用 → 同步构建（旧行为）
        let (engine, stats) = build_search_engine_from_fs(root, limits, update_output)?;
        manager.put_engine(
            cache_key,
            CachedSearchEngine {
                engine: engine.clone(),
                index_mtime: current_mtime,
            },
        );
        return Ok((engine, stats, EngineSource::Fresh));
    }

    // 无缓存（standalone/测试）：保持旧的同步构建行为
    let (engine, stats) = build_search_engine_from_fs(root, limits, &update_output)?;
    Ok((engine, stats, EngineSource::Fresh))
}

fn search_codebase_with_limits(
    root: &Path,
    query: &str,
    update_output: Option<ProgressCallback>,
    limits: CodebaseSearchLimits,
    cache: Option<&Arc<SearchEngineCacheManager>>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // ── Query result cache ────────────────────────────────────────────────────
    // Check the parking_lot-based LRU cache (no poisoning risk).
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let current_mtime = index_mtime(root);
    let query_cache_key = (canonical_root.clone(), query.to_string(), current_mtime);

    if let Some(manager) = cache {
        if let Some(cached_output) = manager.get_query(&query_cache_key) {
            emit_progress(&update_output, "Query cache hit · returning cached result");
            return Ok(cached_output);
        }
    }

    // ── Resolve engine (fresh / stale / warming) ─────────────────────────────
    //
    // 引擎一律用**默认预算**取：watcher 建的引擎就是它，缓存身份也按它算
    // （见 `CodebaseSearchLimits::cache_key`）。预算档位只影响"取多少结果"，
    // 不影响引擎身份 —— 否则 auto 档会开出一个 watcher 永远不填的命名空间，
    // 查询永远 Warming。
    let build_limits = codebase_search_limits();
    let (engine, stats, source) = resolve_engine(root, build_limits, &update_output, cache)?;

    // auto 档 = 更保守的输出（候选与结果各减半），但**不**另建一份引擎。
    let conservative = limits.max_files < build_limits.max_files;
    let candidate_cap = if conservative {
        TOP_N_CANDIDATE / 2
    } else {
        TOP_N_CANDIDATE
    };
    let final_cap = if conservative {
        TOP_N_FINAL / 2
    } else {
        TOP_N_FINAL
    };

    if matches!(source, EngineSource::Warming) {
        // 索引未就绪 ≠ 只能干等：ripgrep 字面兜底**不依赖索引**，先把能给的给出来。
        //
        // 之前的实现在此直接 return "still building / re-run once"——模型重跑时
        // watcher 若仍未就绪，就还是同一句 Warming；用户实际看到的就是连续
        // 两轮 turn=13 双发、零产出。字面兜底让冷启动第一轮就有可用结果。
        let mut output = String::new();
        output.push_str(&format!(
            "The cross-file search index for '{}' is still building in the background (first build; typically finishes in seconds).\n",
            root.display()
        ));

        let literal_text = literal_text_fallback(query, root);
        match literal_text {
            Some(fallback) => {
                output.push_str("Meanwhile, a literal text search found these candidates:\n\n");
                output.push_str(&fallback);
            }
            None => {
                // 字面也零命中。此时**不能再让模型重跑**。
                //
                // 旧文案是"Re-run with the same keywords once before giving up"——
                // 它把模型推进一个死循环：索引在会话内没建完，重跑多少次都是
                // 同一条 "still building"。现场：模型照着做了，连着 11 次
                // CodebaseSearch 全部返回同一句提示，token 烧掉 35k，最后
                // 放弃改用 Grep，还给用户写了一份"索引构建很慢/疑似卡住"的结论。
                //
                // 现在只说**真话**：索引没就绪、重跑无用、眼下该用什么。
                if is_cjk_only_query(query) {
                    output.push_str(
                        "No hits — and the query itself is in Chinese, while the index holds English \
                         identifiers, paths and English comments. Retrying the same Chinese words \
                         cannot match: pass code-side terms instead (e.g. the error/drawback/limitation \
                         names this codebase would use), or re-run with the English wording of the \
                         concept.\n",
                    );
                } else {
                    output.push_str(
                        "No hits — the index for this root is not ready, so this is not an empty \
                         result.\n\
                         \n\
                         Do NOT re-run `CodebaseSearch` for this root: the first build is still in \
                         progress, so every retry returns this same message. Use the tools that work \
                         right now instead:\n\
                         - `Grep` for exact identifiers, strings or paths (regex over file contents);\n\
                         - `ProjectMap` for the repository's file/symbol structure.\n\
                         \n\
                         When the index does finish, a later `CodebaseSearch` on the same root will \
                         return ranked cross-file results.\n",
                    );
                }
            }
        }
        return Ok(output);
    }

    let was_cached =
        matches!(source, EngineSource::Fresh) && stats.indexed_files == 0 && !stats.truncated;

    // ── Hybrid Search (RRF) ───────────────────────────────────────────────────
    // P2: Two complementary search strategies are fused via RRF:
    //   A. Expanded — co-occurrence expansion catches related terms ("auth" → "authentication")
    //   B. Exact — base terms only, no expansion, catches exact synonym matches
    // This dual-strategy approach surfaces results that either strategy alone would miss.
    const TOP_N_CANDIDATE: usize = 40;
    const TOP_N_FUSED: usize = 25;
    const TOP_N_FINAL: usize = 10;

    let progress_msg = if was_cached {
        "Using cached index · RRF hybrid search"
    } else {
        "RRF hybrid search · expanded + exact strategies"
    };
    emit_progress(&update_output, progress_msg);

    // Strategy A: full expansion, no diversity (preserves raw scores for RRF)
    let expanded_results = engine.search_with_options(
        query,
        candidate_cap,
        &SearchOptions::expanded_no_diversity(),
    );

    // Strategy B: exact terms only, no diversity
    let exact_results = engine.search_with_options(query, candidate_cap, &SearchOptions::exact());

    // Convert to RRF candidates
    let expanded_candidates: Vec<Candidate> = expanded_results
        .iter()
        .map(|r| Candidate {
            id: format!("{}:{}", r.file_path, r.chunk.start_line),
            score: r.score,
        })
        .collect();

    let exact_candidates: Vec<Candidate> = exact_results
        .iter()
        .map(|r| Candidate {
            id: format!("{}:{}", r.file_path, r.chunk.start_line),
            score: r.score,
        })
        .collect();

    // ── 第三路：字面精确召回（ripgrep）────────────────────────────────────
    //
    // 前两路是同一个词法引擎的两种参数，属于**同源**信号；这一路才是异构的：
    // ripgrep 的字面匹配对精确符号特别准，而这正是词法排序最容易漏的地方
    // （`handle_error` 会被拆成 handle + error，`handle_error` 的整串权重却
    // 只在字面匹配里才拿得到）。
    //
    // 对齐方式：字面命中的是**文件**，语义命中的是文件里的 **chunk**。
    // 把文件级的命中映射到该文件在语义结果里的最佳 chunk，两路才能在
    // 同一个 candidate id 上相遇 —— 否则 RRF 融合的是不相干的 id，等于没融。
    let literal_files = literal_recall_files(query, root);
    let mut literal_candidates: Vec<Candidate> = Vec::new();
    // 字面命中、但语义没召回的文件 → 其载体块（供重排阶段还原内容）
    let mut literal_only_files: HashMap<String, crate::core::context::chunking::CodeChunk> =
        HashMap::new();
    if !literal_files.is_empty() {
        // 文件 → 该文件在语义结果中的最佳 chunk（取 expanded 优先）
        let mut best_per_file: HashMap<&str, &crate::core::context::search_engine::SearchResult> =
            HashMap::new();
        for r in expanded_results.iter().chain(exact_results.iter()) {
            best_per_file
                .entry(r.file_path.as_str())
                .and_modify(|existing| {
                    if r.score > existing.score {
                        *existing = r;
                    }
                })
                .or_insert(r);
        }

        for (file_path, _hits) in &literal_files {
            match best_per_file.get(file_path.as_str()) {
                // 语义也召回了同一文件 —— 对齐到同一个 chunk，两路在此汇合
                Some(result) => literal_candidates.push(Candidate {
                    id: format!("{}:{}", result.file_path, result.chunk.start_line),
                    score: result.score,
                }),
                // 语义没召回、字面召回了 —— **这一路必须能独立把结果带进来**。
                // 字面通道的价值正在于补语义排序的漏；把它裁剪成「语义已召回
                // 的文件的子集」就等于没有这一路。
                //
                // 用该文件在索引里的**首个块**作为载体：字面命中已经证明这个
                // 文件相关，具体是哪一块由后续重排与展开决定。
                None => {
                    let chunks = engine.document_chunks(file_path);
                    if let Some(chunk) = chunks.first() {
                        literal_candidates.push(Candidate {
                            id: format!("{file_path}:{}", chunk.start_line),
                            score: 0.0,
                        });
                        literal_only_files.insert(file_path.clone(), chunk.clone());
                    }
                }
            }
        }
    }

    // RRF fusion —— 三路异构信号
    let fused = fusion::fuse(
        &[expanded_candidates, exact_candidates, literal_candidates],
        &RrfParams::default(),
    );

    // Build a lookup from candidate id → SearchResult
    let result_lookup: HashMap<String, &crate::core::context::search_engine::SearchResult> =
        expanded_results
            .iter()
            .chain(exact_results.iter())
            .map(|r| (format!("{}:{}", r.file_path, r.chunk.start_line), r))
            .collect();

    // Convert fused results to reranker candidates
    //
    // 字面独有候选（`file:0`）不在语义结果里，lookup 查不到 —— 这里补一份
    // 最小可用的 SearchResult，让它照样能进重排。分数给 0：它靠 RRF 的
    // 排名位置进入，而不是靠伪造的高分挤掉语义真正命中的块。
    let mut rerank_candidates: Vec<RerankCandidate> = fused
        .iter()
        .filter_map(|f| {
            result_lookup.get(&f.id).map(|r| (*r).clone()).or_else(|| {
                let (file_path, start_line) = f.id.rsplit_once(':')?;
                let start_line: usize = start_line.parse().ok()?;
                let chunk = literal_only_files.get(file_path)?.clone();
                Some(crate::core::context::search_engine::SearchResult {
                    file_path: file_path.to_string(),
                    chunk,
                    score: 0.0,
                    matches: Vec::new(),
                    signals: vec!["literal_match".to_string()],
                })
            })
        })
        .map(|r| RerankCandidate::from_search_result(&r))
        .take(TOP_N_FUSED)
        .collect();

    // ── Reranker ──────────────────────────────────────────────────────────────
    let reranker = HeuristicReranker;
    let reranked = reranker.rerank(query, rerank_candidates, final_cap);

    // ── 零结果诊断 ─────────────────────────────────────────────────────────────
    //
    // 诊断只依赖**索引**和**查询词**，跟重排结果无关，所以先算好，
    // 零结果时直接拿来解释，非零结果时不额外开销。
    let diagnostics = engine.diagnosis_from_terms(&engine.extract_base_terms(query));

    // ── Format Output ────────────────────────────────────────────────────────
    //
    // 头部只保留**有信息量的行**：命中数与异常态（索引过期/被截断）。RRF 参数、
    // 索引统计这类恒定元数据每查都一样（现场：`Search: RRF fusion (K=60) ·
    // expanded + exact · reranked (5)` 十次查询九次一字不差），喂给模型纯属
    // 重复 token —— 正常路径一行头部 + 结果正文就够了。
    let mut output = String::new();
    output.push_str(&format!(
        "Ranked results for '{}' — {} file(s):\n",
        query,
        reranked.len(),
    ));
    if matches!(source, EngineSource::Stale) {
        output.push_str(
            "Index: serving previous revision — files changed since; rebuild running in background (results may be slightly stale)\n",
        );
    }
    if stats.truncated {
        output.push_str(&format!(
            "Note: search was capped for responsiveness (max_files={}, max_total_bytes={}).\n",
            build_limits.max_files, build_limits.max_total_bytes
        ));
    }
    if reranked.is_empty() {
        // 缓存状态诊断只在「没搜到」时才有价值——有结果时用户不关心索引新鲜度。
        if was_cached {
            output.push_str("Index: cached (no files changed)\n");
        } else if !matches!(source, EngineSource::Stale) {
            output.push_str(&format!(
                "Indexed files: {} / scanned text files: {} / skipped large files: {} / bytes: {}\n",
                stats.indexed_files,
                stats.scanned_text_files,
                stats.skipped_large_files,
                stats.total_bytes
            ));
        }
        output.push('\n');
    }

    // ── 空结果诊断 ─────────────────────────────────────────────────────────────
    if reranked.is_empty() {
        output.push_str("No relevant code found.\n");
        output.push_str("Diagnosis:\n");
        if diagnostics.query_is_pure_cjk && !diagnostics.index_has_cjk_token {
            output.push_str(
                "  - Your query keywords are in Chinese, but this codebase has no Chinese \
                 (no CJK tokens in the index). Chinese keywords can't match English \
                 identifiers — that's why there are no hits.\n",
            );
            output.push_str(
                "  - Try again with English domain words (e.g. 'retry backoff', 'parse toml'), \
                 or use `Grep` if you know the exact string/symbol.\n",
            );
        } else if diagnostics.term_coverage < 0.15 {
            output.push_str(&format!(
                "  - None of the query keywords appear in the indexed code at all \
                 (term coverage: {:.0}%), so there is nothing to rank.\n",
                diagnostics.term_coverage * 100.0,
            ));
            output.push_str(
                "  - The keywords may not be the words the code actually uses. Try `Grep` \
                 with the exact string/symbol you have in mind.\n",
            );
        } else if diagnostics.index_cjk_token_ratio > 0.1 {
            output.push_str(
                "  - The index contains Chinese text, but the query terms still don't appear \
                 in the indexed code chunks (only in comments/docs that weren't chunked).\n",
            );
            output.push_str("  - Use `Grep` if you know the exact string you're looking for.\n");
        } else {
            output.push_str(
                "  - The query terms exist but didn't co-occur in any indexed code chunk with \
                 enough score to pass ranking. `Grep` will show every raw match instead.\n",
            );
        }
        if !diagnostics.query_is_pure_cjk {
            output.push_str(&format!(
                "  - Keywords used: {}\n",
                engine.extract_base_terms(query).join(", ")
            ));
        }
        return Ok(output);
    }
    // ── 有结果：正常渲染 ────────────────────────────────────────────────────────
    for r in &reranked {
        let c = &r.candidate;
        // 元数据行按有无内容输出：Matched terms / Signals 空的时候不再各占
        // 一行 "Matched terms: " 空壳（把非空的 and/in 这类词显示出来更吵）。
        output.push_str(&format!(
            "File: {} (Score: {:.2}, Lines: {}-{})\n",
            c.file_path, r.final_score, c.start_line, c.end_line
        ));
        if let Some(ctx) = &c.context_header {
            output.push_str(&format!("Context: {}\n", ctx));
        }
        if !c.matched_terms.is_empty() {
            output.push_str(&format!("Matched terms: {}\n", c.matched_terms.join(", ")));
        }
        if !r.boost_signals.is_empty() {
            output.push_str(&format!("Signals: {}\n", r.boost_signals.join("; ")));
        }
        output.push_str("```\n");
        output.push_str(&c.content);
        output.push_str("\n```\n\n");
    }
    output.push_str("Suggested next step: read the top 1-3 files above before editing or explaining cross-file behavior.\n");

    // ── Store in query cache ─────────────────────────────────────────────────
    // LRU eviction is handled automatically by lru::LruCache.
    //
    // 但**未就绪**的输出不能进缓存："索引还在构建"不是检索结果。缓存下来会让
    // 同一条 query 在引擎就绪之后仍然重播那句"还在构建"——用户看到的就是
    // "连查两次都是同一句"，像坏了。
    if let Some(manager) = cache {
        if !matches!(source, EngineSource::Warming) {
            manager.put_query(query_cache_key, output.clone());
        }
    }

    Ok(output)
}

fn emit_progress(update_output: &Option<ProgressCallback>, message: impl Into<String>) {
    if let Some(cb) = update_output.as_ref() {
        cb(message.into());
    }
}

fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn format_progress(stats: &IndexStats) -> String {
    let mut message = format!(
        "Indexing codebase · {} indexed / {} scanned · {:.1} MB",
        stats.indexed_files,
        stats.scanned_text_files,
        bytes_to_mb(stats.total_bytes)
    );

    if stats.skipped_large_files > 0 {
        message.push_str(&format!(" · {} large skipped", stats.skipped_large_files));
    }

    message
}

fn resolve_codebase_search_root(
    config: &Arc<crate::core::config::Config>,
    path: Option<&str>,
) -> PathBuf {
    let requested = path
        .map(|value| crate::core::utils::paths::resolve_tool_path(config.target_dir(), value))
        .unwrap_or_else(|| config.target_dir().clone());

    if requested.is_dir() {
        return requested;
    }

    if requested.is_file() {
        return requested
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config.target_dir().clone());
    }

    config.target_dir().clone()
}

fn format_ripgrep_fallback(query: &str, root: &std::path::Path) -> String {
    let config = RipgrepConfig {
        max_results: Some(40),
        ..Default::default()
    };

    match crate::core::tools::ripgrep::search_with_ripgrep_stats(
        query,
        root.to_string_lossy().as_ref(),
        config,
    ) {
        Ok((results, stats)) => format_ripgrep_results(query, &results, &stats, root),
        Err(e) => format!("Fallback grep failed: {}", e),
    }
}

/// 等待语义索引就绪，最多等 `timeout`。
///
/// 冷启动时 `CodebaseSearch` 的第一次调用几乎必然返回 `Warming`（空结果 +
/// "稍后重跑"）。这是刻意设计的 —— 不在查询路径上同步做全量构建。
/// 但代价是：**自动触发那一次基本白跑**。工具自己给的"稍后重跑"要靠模型
/// 自觉执行，而模型常常不会。
///
/// 所以框架需要自己把这次补上：等索引就绪，然后重跑同一个查询。
/// Windsurf FastContext 的官方经验是「宁可漏掉部分上下文，也不要污染主
/// agent 上下文」；反过来在这里也成立 —— **宁可多等几秒，也不要给模型
/// 一个空结果**，因为空结果会被模型当成「这个功能不存在」。
///
/// # 为什么按 root 等，而不是等全局 `is_engine_ready()`
///
/// 现场事故：日志里 `ready=true waited_ms=8000`，补跑却仍然返回
/// "still building"。两个"就绪"看的不是同一件事：
/// - 这里等的 `watcher::is_engine_ready()` 是**进程级**标志
///   （"语义引擎在某个时刻构建过"）；
/// - 工具侧 `resolve_engine` 查的是**按 root + cache key** 的引擎，
///   而且 mtime 必须对得上（`get_engine` 的两个条件）。
///
/// 于是"全局就绪"不等于"这个项目的引擎可用"：全局标志一置位就返回 true，
/// 补跑却拿到同一个 Warming —— 白等 8 秒，还多一次零命中调用。
/// 现在按 root 查缓存，和工具侧用**完全相同**的判据。
///
/// 等的是**指定的那份缓存**（工具手里那份），不是某个全局单例 ——
/// 判据与工具读的对象不一致，就会出现"等了 8 秒还是 Warming"的白等。
pub async fn wait_for_index_ready(root: &Path, timeout: Duration) -> bool {
    let cache = crate::core::context::search_cache::shared_search_cache();
    wait_for_index_ready_in(&cache, root, timeout).await
}

/// 在**指定缓存**里等这个项目根的引擎就绪。
///
/// 与 [`wait_for_index_ready`] 的区别只在于查哪份缓存 —— 但这个区别正是
/// 事故的核心：工具读的是它自己手里那份，而 watcher 写进的是另一份，
/// 于是"等全局就绪"永远等不到"工具能读到"。
pub async fn wait_for_index_ready_in(
    cache: &std::sync::Arc<crate::core::context::search_cache::SearchEngineCacheManager>,
    root: &Path,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if root_engine_presence(cache, root) != EnginePresence::Absent {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

/// 这个项目根的引擎在缓存里的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePresence {
    /// 缓存里有引擎，且它记录的 mtime 与磁盘上的 `index.json` 一致 —— 可直接用。
    Fresh,
    /// 缓存里有引擎，但 mtime 对不上 —— 磁盘变了。用旧引擎回答（serve-stale），
    /// 后台重建完成后自然切换。
    Stale,
    /// 缓存里压根没有这个 root 的引擎 —— 冷启动。
    Absent,
}

/// 这个项目根的引擎现在能不能被查询路径直接取到。
///
/// 与 `resolve_engine` 的 Fresh 分支同源：缓存里有 `(root, "engine")`，
/// 且它记录的 mtime 与当前 `index.json` 的 mtime 一致。
pub fn root_engine_ready(root: &Path) -> bool {
    let cache = crate::core::context::search_cache::shared_search_cache();
    root_engine_presence(&cache, root) == EnginePresence::Fresh
}

/// 引擎在**指定缓存**里的状态。
///
/// 与 `resolve_engine` 用**同一个**判据（同一个 cache key、同一份 mtime 比对）——
/// 两处判据一旦分叉，就会出现"等到了却说没就绪"或"没等却说就绪"的白等。
pub fn root_engine_presence(
    cache: &std::sync::Arc<crate::core::context::search_cache::SearchEngineCacheManager>,
    root: &Path,
) -> EnginePresence {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let current_mtime = index_mtime(root);
    let cache_key = (canonical_root, codebase_search_limits().cache_key());
    if cache.get_engine(&cache_key, current_mtime).is_some() {
        EnginePresence::Fresh
    } else if cache.get_engine_stale(&cache_key).is_some() {
        EnginePresence::Stale
    } else {
        EnginePresence::Absent
    }
}

/// 把这个项目根的引擎构建好放进**共享**缓存（线上查询路径用的就是这个缓存）。
///
/// 返回索引的文件数。给"想在进程外验证检索行为"的场景用（探针、诊断）。
pub fn seed_shared_engine(root: &Path) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let limits = codebase_search_limits();
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cache_key = (canonical_root, limits.cache_key());
    let cache = crate::core::context::search_cache::shared_search_cache();
    if let Some(engine) = cache.get_engine(&cache_key, index_mtime(root)) {
        let _ = engine;
        return Ok(0);
    }
    let (engine, stats) = build_search_engine_from_fs(root, limits, &None)?;
    let files = stats.indexed_files;
    cache.put_engine(
        cache_key,
        CachedSearchEngine {
            engine,
            index_mtime: index_mtime(root),
        },
    );
    Ok(files)
}

/// 查询是不是**纯中文**（没有任何 ASCII 字母数字）。
///
/// 索引里是英文标识符、路径和英文注释 —— 纯中文的 query 一个 token 都命中不了。
/// 命中为零时，模型需要知道"换个词"而不是"再搜一遍"，否则它会原样重跑。
fn is_cjk_only_query(query: &str) -> bool {
    let mut has_cjk = false;
    let mut has_ascii_alnum = false;
    for ch in query.chars() {
        if crate::core::context::search_engine::is_cjk_char(ch) {
            has_cjk = true;
        } else if ch.is_ascii_alphanumeric() {
            has_ascii_alnum = true;
        }
    }
    has_cjk && !has_ascii_alnum
}

/// 字面兜底（Warming 分支专用）：索引未就绪时立即跑一次 ripgrep 全文搜索。
///
/// 返回 `Some(格式化结果)` 当且仅当**有命中**；零命中或 ripgrep 失败都返回
/// `None`（调用方据此决定是给出兜底结果还是让模型重跑）。
fn literal_text_fallback(query: &str, root: &Path) -> Option<String> {
    let config = RipgrepConfig {
        max_results: Some(40),
        ..Default::default()
    };
    match crate::core::tools::ripgrep::search_with_ripgrep_stats(
        query,
        root.to_string_lossy().as_ref(),
        config,
    ) {
        Ok((results, stats)) if !results.is_empty() => {
            Some(format_ripgrep_results(query, &results, &stats, root))
        }
        _ => None,
    }
}

/// 字面精确召回通道 —— 返回命中文件的**排名列表**。
///
/// 这是第三路召回，与前两路**异构**：
/// - 前两路是同一个词法引擎的两种参数（expanded / exact）
/// - 这一路是 ripgrep 的字面匹配：精确符号（`handle_error`）、路径片段、
///   大小写敏感的字面串 —— 这些是词法排序最容易漏掉的东西
///
/// Augment 把它做成了 `matched_string` 字段与 `contains_string` 查询约束，
/// 官方文档称之为「语义 + 字面 + 结构化过滤三路合一」。这里用同样的思路。
///
/// 返回的是**文件路径**（按命中数与命中位置排序），不是 chunk ——
/// chunk 的对齐交给调用方：字面命中的是文件，语义命中的是文件里的块。
fn literal_recall_files(query: &str, root: &Path) -> Vec<(String, usize)> {
    let config = RipgrepConfig {
        max_results: Some(80),
        case_sensitive: false,
        ..Default::default()
    };

    let results = match search_with_ripgrep(query, &root.to_string_lossy(), config) {
        Ok(results) => results,
        // 字面通道失败不是致命错误：语义通道还在跑，静默跳过这一路。
        Err(_) => return Vec::new(),
    };

    // 聚合到文件维度，命中行数越多排越前。
    //
    // 路径必须归一成「相对 root + 正斜杠」—— 与索引里的 key 同一口径
    // （见 `index_file` 的 `strip_prefix(root)`）。ripgrep 返回的是绝对路径，
    // 直接拿去和索引比对会一个都匹配不上。
    let mut per_file: HashMap<String, usize> = HashMap::new();
    for hit in &results {
        let rel = Path::new(&hit.file)
            .strip_prefix(root)
            .unwrap_or_else(|_| Path::new(&hit.file))
            .to_string_lossy()
            .replace('\\', "/");
        *per_file.entry(rel).or_default() += 1;
    }
    let mut ranked: Vec<(String, usize)> = per_file.into_iter().collect();
    // 命中数降序；同数按路径升序 —— 稳定排序让同一查询的结果可复现
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

fn format_ripgrep_results(
    query: &str,
    results: &[crate::core::tools::tools::SearchResult],
    scan_stats: &crate::core::tools::ripgrep::ScanStats,
    root: &Path,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "Fallback grep results for '{}' ({} hits):\n\n",
        query,
        results.len()
    ));

    if results.is_empty() {
        if is_cjk_only_query(query) {
            output.push_str(
                "No matches — the query is in Chinese while the codebase is written in English \
                 identifiers. Retrying the same words cannot match: use the code-side terms for \
                 the concept you mean.\n",
            );
        } else {
            output.push_str("No matches found.");
        }
        // 空结果附上扫描范围：兜底到底搜了几个文件 —— 不然模型会把"兜底零命中"
        // 和"这个概念不存在"混成一句。
        output.push('\n');
        output.push_str(&crate::core::tools::ripgrep::empty_result_tail(
            scan_stats, query, root, None, None, false,
        ));
        return output;
    }

    for res in results.iter().take(20) {
        let line = res
            .line
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let snippet = res
            .text
            .as_deref()
            .or(res.match_content.as_deref())
            .unwrap_or("")
            .trim();
        output.push_str(&format!("- {}:{} {}\n", res.file, line, snippet));
    }

    if results.len() > 20 {
        output.push_str(&format!(
            "\n... {} more results omitted",
            results.len() - 20
        ));
    }

    output
}

// ── Call Chain Tracing (P3) ───────────────────────────────────────────────────

/// Build a CallGraph by walking the project and extracting symbols with Tree-sitter.
///
/// This is a lighter-weight scan than SearchEngine building — it only needs to
/// parse call-graph-capable files (Rust/Python/JS/TS/Go/Java/C/C++) and extract
/// function definitions and call sites.
pub fn build_call_graph(
    root: &Path,
) -> Result<CallGraph, Box<dyn std::error::Error + Send + Sync>> {
    let mut file_symbols: Vec<FileSymbols> = Vec::new();
    let mut next_id: symbol::SymbolId = 0;

    for result in crate::utils::file_walk::walk(root, &walk_options()) {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            continue;
        }

        let ext = match path.extension().and_then(|s| s.to_str()) {
            Some(e) => e,
            None => continue,
        };

        let language = symbol::language_for_file(ext);
        if language.is_none() {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if let Some(fs) =
            symbol::extract_symbols(&content, &rel_path, ext, &language.unwrap(), &mut next_id)
        {
            file_symbols.push(fs);
        }
    }

    Ok(CallGraph::build(&file_symbols))
}

/// Trace the call chain for a symbol matching `name_hint`.
///
/// Returns a formatted call chain string suitable for the LLM.
pub fn trace_call_chain(
    root: &Path,
    name_hint: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // 优先读 `.star/context/callgraph.db`；需要时增量同步，库不可用才回退
    // 内存全量构建。查询 API 与输出格式不变 —— 见
    // docs/callgraph-persistence-design.zh-CN.md。
    let graph = crate::core::context::call_graph_db::get_call_graph(root)?;

    let symbols = graph.find_symbols(name_hint);
    if symbols.is_empty() {
        return Ok(format!(
            "No symbols found matching '{}' in the project call graph ({} total symbols indexed).",
            name_hint,
            graph.len()
        ));
    }

    let mut output = String::new();
    output.push_str(&format!("Call Graph Search for '{}'\n", name_hint));
    output.push_str(&format!(
        "Total symbols indexed: {} | Resolved call edges: {}\n\n",
        graph.len(),
        graph.edge_count()
    ));

    if symbols.len() > 5 {
        output.push_str(&format!(
            "Found {} matching symbols. Showing top 5 by name match:\n",
            symbols.len()
        ));
    }

    for sym in symbols.iter().take(5) {
        output.push_str("───\n");
        let chain = graph.call_chain(sym.id, 3);
        output.push_str(&call_graph::format_call_chain(&chain));
        output.push('\n');
    }

    if symbols.len() > 5 {
        output.push_str(&format!(
            "... and {} more matching symbols. Use a more specific name to narrow down.\n",
            symbols.len() - 5
        ));
    }

    Ok(output)
}

/// explore 的输出预算。工具层从参数构造，测试直接给值。
#[derive(Debug, Clone, Copy)]
pub struct ExploreLimits {
    /// 最多展开多少个匹配符号。
    pub max_symbols: usize,
    /// 调用链深度（上游 + 下游）。
    pub depth: usize,
    /// 每个符号最多带多少行源码。
    pub max_source_lines: usize,
}

impl Default for ExploreLimits {
    fn default() -> Self {
        Self {
            max_symbols: 3,
            depth: 2,
            max_source_lines: 80,
        }
    }
}

/// 复合查询（explore）：命中符号 + **逐字源码** + 直接调用关系 + 影响面，一次返回。
///
/// 对标 codegraph 的 `explore`：agent 平时要 `CodeGraph` → `Read` → 再查调用链，
/// 三次往返；这里合并成一次，源码按符号的行区间精确截取。输出目标"够用且不
/// 淹没上下文"——源码默认最多 80 行/符号、最多 3 个符号。
pub fn explore_symbol(
    root: &Path,
    name_hint: &str,
    limits: ExploreLimits,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // CallGraph 将子目录归一到 Git 根；源码片段也必须使用同一个 root，
    // 否则 `RepoMap(path=repo/agent)` 得到的 `agent/foo.go` 会被拼成
    // `repo/agent/agent/foo.go`，表现为图有符号但 source unavailable。
    let root = crate::core::context::call_graph_db::project_root(root);
    let graph = crate::core::context::call_graph_db::get_call_graph(&root)?;

    // 候选 = 内存 contains 匹配 ∪ FTS5 分词匹配。
    //
    // 两条路径互补：内存认不出「auth handler」→ `auth_handler`（查询串不连续），
    // FTS5 认不出拼写误差。并集之后按**同一份**评分排序，两条路径给模型的
    // 顺序必须一致。
    let query = name_hint.trim();
    let query_lower = query.to_lowercase();
    let mut scored: Vec<(f32, &symbol::SymbolDef)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for candidate in graph.find_symbols(query) {
        let score = call_graph::score_symbol(candidate, query, &query_lower).unwrap_or(0.0);
        if seen.insert((candidate.file_path.clone(), candidate.name.clone())) {
            scored.push((score, candidate));
        }
    }

    if let Ok(hits) =
        crate::core::context::call_graph_db::search_symbols(&root, query, limits.max_symbols * 4)
    {
        for hit in hits {
            let Some(candidate) = graph.symbols().iter().find(|candidate| {
                candidate.file_path == hit.file_path && candidate.name == hit.name
            }) else {
                continue;
            };
            if seen.insert((candidate.file_path.clone(), candidate.name.clone())) {
                scored.push((hit.score, candidate));
            }
        }
    }

    scored.sort_by(|(score_a, a), (score_b, b)| {
        score_b
            .partial_cmp(score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.chars().count().cmp(&b.name.chars().count()))
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.id.cmp(&b.id))
    });

    let symbols: Vec<&symbol::SymbolDef> =
        scored.into_iter().map(|(_, candidate)| candidate).collect();

    if symbols.is_empty() {
        return Ok(format!(
            "No symbols found matching '{name_hint}' in the project call graph ({} total symbols indexed).\n\
             Try a shorter substring, or run CodebaseSearch first to discover the exact symbol name.",
            graph.len()
        ));
    }

    let max_symbols = limits.max_symbols.max(1);
    let depth = limits.depth.max(1);
    let mut output = String::new();
    output.push_str(&format!(
        "Explore '{name_hint}' — {} match(es); indexed {} symbols / {} edges\n",
        symbols.len(),
        graph.len(),
        graph.edge_count()
    ));
    if symbols.len() > max_symbols {
        output.push_str(&format!(
            "Showing the top {max_symbols} by name match; use a more specific hint for the rest.\n"
        ));
    }
    output.push('\n');

    for symbol in symbols.iter().take(max_symbols) {
        write_symbol_card(
            &mut output,
            &root,
            &graph,
            symbol,
            limits.max_source_lines,
            depth,
        );
    }

    if symbols.len() > max_symbols {
        output.push_str(&format!(
            "… and {} more matching symbols.\n",
            symbols.len() - max_symbols
        ));
    }

    Ok(output)
}

fn write_symbol_card(
    output: &mut String,
    root: &Path,
    graph: &CallGraph,
    symbol: &symbol::SymbolDef,
    max_source_lines: usize,
    depth: usize,
) {
    output.push_str(&format!(
        "── {} `{}` — {}:{}-{}\n",
        symbol.kind.as_str(),
        symbol.name,
        symbol.file_path,
        symbol.start_line,
        symbol.end_line
    ));

    match source_snippet(root, symbol, max_source_lines) {
        Some(snippet) => output.push_str(&snippet),
        None => output.push_str("(source unavailable)\n"),
    }
    output.push('\n');

    write_relations(output, "Callers", &graph.callers_of(symbol.id));
    write_relations(output, "Callees", &graph.callees_of(symbol.id));

    let chain = graph.call_chain(symbol.id, depth);
    let upstream: usize = chain.callers.iter().map(|level| level.symbols.len()).sum();
    let downstream: usize = chain.callees.iter().map(|level| level.symbols.len()).sum();
    let downstream_names: Vec<&str> = chain
        .callees
        .iter()
        .flat_map(|level| level.symbols.iter())
        .take(6)
        .map(|symbol| symbol.name.as_str())
        .collect();

    output.push_str(&format!(
        "Impact (depth {depth}): {upstream} upstream / {downstream} downstream"
    ));
    if !downstream_names.is_empty() {
        output.push_str(&format!(" — e.g. {}", downstream_names.join(", ")));
    }
    output.push_str("\n\n");
}

fn write_relations(output: &mut String, label: &str, symbols: &[&symbol::SymbolDef]) {
    const SHOWN: usize = 6;

    if symbols.is_empty() {
        output.push_str(&format!("{label}: —\n"));
        return;
    }

    let shown: Vec<String> = symbols
        .iter()
        .take(SHOWN)
        .map(|symbol| format!("`{}` ({})", symbol.name, symbol.file_path))
        .collect();
    output.push_str(&format!(
        "{label} ({}): {}",
        symbols.len(),
        shown.join(", ")
    ));
    if symbols.len() > SHOWN {
        output.push_str(&format!(" … +{}", symbols.len() - SHOWN));
    }
    output.push('\n');
}

/// 按 1-based 行区间截取源码（explore 的"逐字源码"部分）。
///
/// 只保留原始行，不重复行号 —— 区间已经在卡片标题里，再标一遍纯占 token。
fn source_snippet(root: &Path, symbol: &symbol::SymbolDef, max_lines: usize) -> Option<String> {
    let content = std::fs::read_to_string(root.join(&symbol.file_path)).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = symbol.start_line.max(1);
    if start > lines.len() {
        return None;
    }

    let cap = max_lines.max(1);
    let end = symbol
        .end_line
        .max(start)
        .min(lines.len())
        .min(start + cap - 1);

    let mut snippet = String::from("```\n");
    for line in &lines[start - 1..end] {
        snippet.push_str(line);
        snippet.push('\n');
    }
    snippet.push_str("```\n");

    if symbol.end_line > end {
        snippet.push_str(&format!(
            "… ({} more lines; Read `{}:{}` for the rest)\n",
            symbol.end_line - end,
            symbol.file_path,
            end + 1
        ));
    }
    Some(snippet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::chunking::CodeChunk;
    use crate::core::context::indexer::{IndexResult, Indexer};
    use crate::core::context::search_cache::{CachedSearchEngine, SearchEngineCacheManager};
    use std::fs;

    /// 字面召回通道：ripgrep 命中但语义排序漏掉的文件，必须能进结果。
    ///
    /// 这是第三路召回存在的全部理由 —— 精确符号（`handle_error`）在词法
    /// 排序里会被拆成 handle + error，整串权重只有字面匹配才拿得到。
    /// 如果这一路被裁剪成「语义结果的子集」，它就等于不存在。
    #[test]
    fn literal_recall_finds_files_by_exact_identifier() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("exact.rs"), "pub fn handle_error() {}\n").unwrap();
        fs::write(root.join("other.rs"), "pub fn unrelated() {}\n").unwrap();

        let hits = literal_recall_files("handle_error", root);
        assert!(
            !hits.is_empty(),
            "字面通道必须命中 handle_error 这个精确标识符"
        );
        assert!(
            hits.iter().any(|(path, _)| path.ends_with("exact.rs")),
            "命中的应当是 exact.rs，实际：{hits:?}"
        );
        // 路径必须与索引 key 同一口径：相对 root + 正斜杠
        assert!(
            hits.iter().all(|(path, _)| !path.starts_with('/')),
            "字面通道的路径应相对 root，实际：{hits:?}"
        );
    }

    /// 字面通道不该把不相关文件带进来。
    #[test]
    fn literal_recall_ignores_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("other.rs"), "pub fn unrelated() {}\n").unwrap();

        let hits = literal_recall_files("handle_error", root);
        assert!(hits.is_empty(), "不相关的文件不该被字面通道召回：{hits:?}");
    }

    /// 搜索引擎缓存必须是**进程级共享**的。
    ///
    /// 以前 `CodebaseSearchTool::new()` 挂的是 `None`，而 watcher 把建好的
    /// 索引写进 `ContextEngine` 那份 cache —— 两边不是同一个对象。于是
    /// watcher 明明在后台把索引建好了，工具却永远读不到，每次检索都退回
    /// 同步全量重建（实测 851 文件 / 10 MB / 11 秒）。
    ///
    /// 这条测试守住接线：工具默认拿到的必须是共享实例。
    #[test]
    fn tool_uses_the_shared_search_cache() {
        let config = std::sync::Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters::default(),
        ));
        let tool = CodebaseSearchTool::new(config);
        let cache = tool.search_cache.clone().expect("工具必须挂上搜索引擎缓存");
        assert!(
            std::sync::Arc::ptr_eq(
                &cache,
                &crate::core::context::search_cache::shared_search_cache()
            ),
            "工具必须与 watcher 共用同一个缓存实例，否则 watcher 建好的索引读不到"
        );
    }

    /// **回归：分裂缓存** —— watcher 写进 A 桶、工具读 B 桶。
    ///
    /// 这是 ARTEX 上「索引永远建不出来 + 模型无限重试」的真正根因：
    ///
    /// ```text
    /// agent_core.rs:221  watcher 写入 → shared_search_cache()
    /// agent_core.rs:260  工具读取   → context_engine.search_cache（另一个实例！）
    /// engine.rs:103      ContextEngine::new() 里 Arc::new(SearchEngineCacheManager::new())
    /// ```
    ///
    /// `lazy_init` 那句 `register_tool` 会**覆盖** `runtime_bootstrap` 里
    /// 已经挂好共享缓存的工具，于是最终生效的是"读 B 桶"的那份。
    /// watcher 把索引建好了（ARTEX 日志：`semantic engine updated in
    /// background: full rebuild, 639 files in 4.34s`），工具却永远读不到，
    /// 每次查询都回一句 "still building" —— 模型照着提示重试 11 次。
    ///
    /// 这条测试守住接线：**ContextEngine 持有的必须是共享单例**。
    /// 只要它退回 `Arc::new(...)`，这里立刻红。
    #[test]
    fn the_context_engine_shares_the_process_wide_search_cache() {
        let engine = crate::core::context::engine::ContextEngine::new();
        assert!(
            std::sync::Arc::ptr_eq(
                &engine.search_cache,
                &crate::core::context::search_cache::shared_search_cache()
            ),
            "ContextEngine 必须持有进程级共享缓存：watcher 写的是它、工具读的也必须是它，\
             两个对象就永远读不到对方建好的索引"
        );
    }

    /// 兜底 grep 的空结果必须附上扫描范围（[scope]/[hints]）—— 纯说
    /// "No matches found." 会让模型把"兜底零命中"当成"这个概念不存在"。
    #[test]
    fn an_empty_fallback_result_lists_the_scan_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn unrelated() {}\n").unwrap();

        let stats = crate::core::tools::ripgrep::ScanStats {
            files_enumerated: 1,
            files_searched: 1,
            matches: 0,
            ..Default::default()
        };
        let out = format_ripgrep_results("absent_token", &[], &stats, dir.path());

        assert!(out.contains("No matches found."), "实际：{out}");
        assert!(out.contains("[scope]"), "实际：{out}");
        assert!(
            out.contains("1 files in scope, 1 searched, 0 matches"),
            "实际：{out}"
        );
        assert!(out.contains("[hints]"), "实际：{out}");
    }

    /// 工具实例（无论 `new` 还是 `with_cache`）拿到的缓存，必须与
    /// `ContextEngine` 持有的那份是**同一个对象**。
    ///
    /// `agent_core::lazy_init` 用 `with_cache(context_engine.search_cache)`
    /// 覆盖掉 `runtime_bootstrap` 注册的工具 —— 也就是说，**最终生效的**
    /// 工具读的是 ContextEngine 那份。只要它不是共享单例，watcher 建好的
    /// 索引就永远读不到。
    #[test]
    fn the_tool_and_the_context_engine_read_one_cache() {
        let config = std::sync::Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters::default(),
        ));
        let engine = crate::core::context::engine::ContextEngine::new();
        let tool = CodebaseSearchTool::with_cache(config, engine.search_cache.clone());
        let tool_cache = tool.search_cache.clone().expect("工具必须挂上缓存");

        assert!(
            std::sync::Arc::ptr_eq(&tool_cache, &engine.search_cache),
            "工具与 ContextEngine 必须读同一个缓存对象"
        );
        assert!(
            std::sync::Arc::ptr_eq(
                &tool_cache,
                &crate::core::context::search_cache::shared_search_cache()
            ),
            "工具最终必须读进程级共享缓存（watcher 写的就是它）"
        );
    }

    /// watcher 建好的引擎，工具**读得到** —— 这才是"索引建出来了"的定义。
    ///
    /// 前两条守住"接线"，这条守住"接线之后确实通"。
    /// 模拟 watcher 的动作：往**共享单例**里存一个引擎；然后问
    /// `root_engine_presence`：必须报 Fresh，而不是 Absent。
    ///
    /// 事故原样：ARTEX 上 watcher 明明写了引擎（639 files），查询侧却一直
    /// 判定"引擎不存在" → 每次回 "still building"。
    #[test]
    fn an_engine_written_by_the_watcher_is_visible_to_the_query_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("alpha.rs"), "fn alpha_handler() {}\n").unwrap();
        // 写 index.json，产生 mtime 令牌（get_engine 要比对它）。
        Indexer::new(root).index_project().unwrap();

        let cache = crate::core::context::search_cache::shared_search_cache();
        let key = (
            root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            codebase_search_limits().cache_key(),
        );
        let mut engine = crate::core::context::search_engine::SearchEngine::new();
        engine.add_document(
            "alpha.rs".to_string(),
            vec![CodeChunk {
                content: "fn alpha_handler() {}".to_string(),
                start_line: 1,
                end_line: 1,
                context_header: None,
            }],
        );
        cache.put_engine(
            key.clone(),
            CachedSearchEngine {
                engine,
                index_mtime: index_mtime(root),
            },
        );

        assert_eq!(
            root_engine_presence(&cache, root),
            EnginePresence::Fresh,
            "watcher 写进共享缓存的引擎，查询路径必须能看到"
        );
        assert!(
            root_engine_ready(root),
            "共享缓存里有引擎时，root_engine_ready 必须为真"
        );

        // 清理：共享单例是进程级的，别把引擎留给后续用例。
        cache.clear();
    }

    /// 显式调用**不再白等**：引擎已经 Fresh 时，检索必须立刻拿到结果，
    /// 不能先进 8 秒的 warmup 等待。
    ///
    /// 修复前，显式调用路径完全没有等待逻辑（只有自动触发路径等），
    /// 于是模型拿到 "still building" → 重试 → 再 "still building" → 死循环。
    #[test]
    fn an_explicit_call_does_not_wait_when_the_engine_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("alpha.rs"), "fn alpha_handler() {}\n").unwrap();
        Indexer::new(root).index_project().unwrap();

        let cache = std::sync::Arc::new(SearchEngineCacheManager::new());
        // 预置一个与磁盘 mtime 一致的引擎 → 应当被判为 Fresh。
        let key = (
            root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            codebase_search_limits().cache_key(),
        );
        let mut engine = crate::core::context::search_engine::SearchEngine::new();
        engine.add_document(
            "alpha.rs".to_string(),
            vec![CodeChunk {
                content: "fn alpha_handler() {}".to_string(),
                start_line: 1,
                end_line: 1,
                context_header: None,
            }],
        );
        cache.put_engine(
            key,
            CachedSearchEngine {
                engine,
                index_mtime: index_mtime(root),
            },
        );

        assert_eq!(
            root_engine_presence(&cache, root),
            EnginePresence::Fresh,
            "预置的引擎必须被判为 Fresh"
        );
    }

    /// Stale（磁盘变了、引擎还在）**不等待** —— serve-stale 的意义就在这。
    ///
    /// 修复前的等待逻辑只看"引擎在不在"，于是改一个文件之后每一次检索
    /// 都要白等 8 秒才返回旧结果。
    #[test]
    fn a_stale_engine_is_served_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("alpha.rs"), "fn alpha_handler() {}\n").unwrap();
        Indexer::new(root).index_project().unwrap();

        let cache = std::sync::Arc::new(SearchEngineCacheManager::new());
        let key = (
            root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            codebase_search_limits().cache_key(),
        );
        // 存一个 mtime 对不上的引擎 → Stale。
        cache.put_engine(
            key,
            CachedSearchEngine {
                engine: crate::core::context::search_engine::SearchEngine::new(),
                index_mtime: Some(std::time::SystemTime::UNIX_EPOCH),
            },
        );

        assert_eq!(
            root_engine_presence(&cache, root),
            EnginePresence::Stale,
            "mtime 对不上的引擎必须判为 Stale（可 serve-stale，不必等）"
        );
    }

    /// 第二次检索应当命中缓存 —— 引擎缓存命中后不该再全量重建。
    ///
    /// 这是「更快」的直接兑现：实测首次 11.9s，第二次 2.3s
    /// （同一查询走查询缓存则是 3ms）。
    ///
    /// 用**独立的** cache 实例，不能用共享那个 —— 共享实例是进程级的，
    /// 别的测试先跑就会把 watcher 启起来，于是这里拿到的是预热提示而不是
    /// 缓存命中。测试不该依赖全局状态。
    #[test]
    fn second_search_reuses_the_cached_engine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("alpha.rs"), "fn alpha_handler() {}\n").unwrap();

        let cache = std::sync::Arc::new(SearchEngineCacheManager::new());
        let rt = tokio::runtime::Runtime::new().unwrap();

        let first = rt.block_on(run_codebase_search(
            root.to_path_buf(),
            "alpha_handler".to_string(),
            None,
            None,
            Some(cache.clone()),
        ));
        assert!(!first.is_empty(), "首次检索应当有结果");

        // 第二次跑**同一个查询**：查询缓存（LRU）必然命中 —— 它按
        // (root, query, mtime) 键控，不受协调器是否启动的影响。
        //
        // 这里刻意不测「不同查询走引擎缓存」：那条路径会撞上
        // `watcher::is_started()` 这个**进程级**状态 —— 别的测试一旦启动过
        // 协调器，这里就只会拿到预热提示。测试不该依赖全局状态。
        let second = rt.block_on(run_codebase_search(
            root.to_path_buf(),
            "alpha_handler".to_string(),
            None,
            None,
            Some(cache.clone()),
        ));
        assert_eq!(
            first, second,
            "同一查询的第二次检索应当命中查询缓存，返回完全相同的结果"
        );
    }

    /// 零结果必须**解释原因**并给出下一步 —— 否则模型只会读成
    /// 「这个功能不存在」，然后换个说法再问一遍，再问一遍。
    ///
    /// 事故原样（docs/tool-io-token-audit.md §1.3）：中文提问打纯英文代码库，
    /// 工具只回一句 "No relevant code found" —— 信息量等同于"没有这个东西"。
    #[test]
    fn a_zero_hit_result_explains_the_language_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 纯英文代码库。
        write(root, "alpha.rs", "fn alpha_handler() {}\n");

        // 用 `seed_cache` 预置索引，并直接调 `search_codebase_with_limits`（显式传
        // `test_limits()`）。
        //
        // 两个坑：
        // 1. 直接跑 `run_codebase_search` 会撞上 `watcher::is_started()` 这个
        //    **进程级**状态 —— 别的测试一旦启动过协调器，这里就只会拿到 Warming
        //    提示而不是真实检索结果。测试不该依赖全局状态。
        // 2. `run_codebase_search` 的 limits 来自进程级环境变量，而
        //    `seed_cache` 用的是 `test_limits()` —— 两边 cache_key 不同，
        //    预置的引擎根本命中不了。显式传同一个 limits 才对得上。
        let cache = seed_cache(root, test_limits());
        let out = search_codebase_with_limits(root, "错误处理", None, test_limits(), Some(&cache))
            .expect("search must succeed");

        assert!(
            out.contains("No relevant code found"),
            "零结果必须明说没命中：{out}"
        );
        assert!(out.contains("Diagnosis"), "零结果必须带诊断段：{out}");
        assert!(
            out.contains("Chinese") && out.contains("English"),
            "诊断必须点破语言不匹配（中文关键词 vs 英文代码库）：{out}"
        );
        assert!(
            out.contains("Grep"),
            "诊断必须给出可执行的下一步（换英文词或用 Grep）：{out}"
        );
    }

    /// 零结果但索引里**有**中文时，不该误报语言不匹配 —— 那是在骗模型。
    #[test]
    fn a_zero_hit_result_does_not_claim_a_language_mismatch_when_the_index_has_chinese() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 索引里有中文注释，但查询的词不存在于其中。
        write(root, "alpha.rs", "// 错误处理入口\nfn alpha_handler() {}\n");

        let cache = seed_cache(root, test_limits());
        let out = search_codebase_with_limits(root, "限流回退", None, test_limits(), Some(&cache))
            .expect("search must succeed");

        assert!(
            out.contains("No relevant code found"),
            "零结果必须明说没命中：{out}"
        );
        assert!(
            !out.contains("this codebase has no Chinese"),
            "索引里有中文时不能报'这个代码库没有中文'：{out}"
        );
    }

    /// 回归现场：`budget_profile="auto"` 的查询永远命中不了 watcher 建的引擎。
    ///
    /// 缓存键里带着预算档位时，auto 档（f320b4M）与 watcher 的默认档
    /// （f1200b12M）是**两个缓存命名空间**，而 watcher 只填后者 —— 于是 auto
    /// 查询永远 cache miss → 永远 Warming → 模型看到"索引还在构建"就改用
    /// Grep 绕开（现场：连续两次 0 ranked results，而同样的 query 在默认
    /// 预算下能命中 40 条）。
    #[test]
    fn an_auto_budget_query_still_finds_the_shared_engine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "alpha.rs", "fn alpha_handler() {}\n");

        // watcher 用默认预算建引擎（= 共享的那一份）
        let cache = seed_cache(root, test_limits());
        // 查询用 auto 档预算
        let auto_limits = CodebaseSearchLimits {
            max_files: 320,
            max_file_bytes: 256 * 1024,
            max_total_bytes: 4 * 1024 * 1024,
            timeout_ms: 5_000,
        };
        let out =
            search_codebase_with_limits(root, "alpha_handler", None, auto_limits, Some(&cache))
                .expect("search must succeed");

        assert!(
            !out.contains("still building"),
            "auto 档不该再命中 Warming：{out}"
        );
        assert!(
            out.contains("alpha.rs"),
            "auto 档必须能用上共享引擎的检索结果：{out}"
        );
    }

    /// 关键词压根不在索引里时，诊断必须说清"覆盖率 0%"并给出下一步。
    ///
    /// 这跟语言不匹配是两回事：前者是词选错了，后者是语言不对。
    /// 混为一谈会让模型换一种错误的方式重试。
    #[test]
    fn a_zero_hit_result_reports_term_coverage_when_the_words_do_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "alpha.rs", "fn alpha_handler() {}\n");

        let cache = seed_cache(root, test_limits());
        let out = search_codebase_with_limits(
            root,
            "nonexistent_symbol_xyz",
            None,
            test_limits(),
            Some(&cache),
        )
        .expect("search must succeed");

        assert!(out.contains("No relevant code found"), "{out}");
        assert!(out.contains("Diagnosis"), "零结果必须带诊断段：{out}");
        assert!(
            !out.contains("Chinese"),
            "英文查询不该被报成语言问题：{out}"
        );
    }

    /// 测试用的显式预算。
    ///
    /// **绝不能**用 `default_limits()` —— 它读进程级环境变量，而 Rust
    /// 测试并行跑，任何一个改 env 的测试都会让它 flaky。这正是拆出
    /// `update_engine_in_cache_with_limits` 的原因。
    fn test_limits() -> CodebaseSearchLimits {
        CodebaseSearchLimits {
            max_files: 1_000,
            max_file_bytes: 512,
            max_total_bytes: 200_000,
            timeout_ms: 12_000,
        }
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn cache_key_for(root: &Path, limits: CodebaseSearchLimits) -> (PathBuf, String) {
        (
            root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
            limits.cache_key(),
        )
    }

    /// 把"当前文件系统"的全量引擎放进缓存，模拟引擎已就绪的稳态。
    fn seed_cache(root: &Path, limits: CodebaseSearchLimits) -> Arc<SearchEngineCacheManager> {
        let cache = Arc::new(SearchEngineCacheManager::new());
        let (engine, _) = build_search_engine_from_fs(root, limits, &None).expect("seed build");
        cache.put_engine(
            cache_key_for(root, limits),
            CachedSearchEngine {
                engine,
                index_mtime: index_mtime(root),
            },
        );
        cache
    }

    fn cached_engine(
        root: &Path,
        limits: CodebaseSearchLimits,
        cache: &SearchEngineCacheManager,
    ) -> SearchEngine {
        cache
            .get_engine_stale(&cache_key_for(root, limits))
            .expect("engine must be cached")
            .0
    }

    fn full_engine(root: &Path, limits: CodebaseSearchLimits) -> SearchEngine {
        build_search_engine_from_fs(root, limits, &None)
            .expect("full rebuild")
            .0
    }

    /// 探针查询的可观测结果：路径 + 行号 + 内容（排序后，顺序无关）。
    fn probe(engine: &SearchEngine, query: &str) -> Vec<(String, usize, String)> {
        let mut hits: Vec<(String, usize, String)> = engine
            .search(query, 50)
            .into_iter()
            .map(|r| (r.file_path, r.chunk.start_line, r.chunk.content))
            .collect();
        hits.sort();
        hits
    }

    // ── admit_file：单一策略点 ────────────────────────────────────────────────

    #[test]
    fn admit_file_covers_every_branch() {
        let limits = test_limits();
        let p = Path::new("src/a.rs");

        assert_eq!(admit_file(p, 10, 0, 0, 0, limits), Admission::Admit);
        // 扩展名不在白名单。
        assert_eq!(
            admit_file(Path::new("logo.svg"), 10, 0, 0, 0, limits),
            Admission::SkipExt
        );
        // 没有扩展名。
        assert_eq!(
            admit_file(Path::new("Makefile"), 10, 0, 0, 0, limits),
            Admission::SkipExt
        );
        // 超过单文件上限。
        assert_eq!(
            admit_file(p, limits.max_file_bytes + 1, 0, 0, 0, limits),
            Admission::SkipTooLarge
        );
        // 文件数耗尽。
        assert_eq!(
            admit_file(p, 10, limits.max_files, 0, 0, limits),
            Admission::SkipBudget
        );
        // 总字节耗尽。
        assert_eq!(
            admit_file(p, 10, 0, limits.max_total_bytes, 0, limits),
            Admission::SkipBudget
        );
    }

    /// `replaced_bytes` 的存在意义：把 5 MB 重写成同样 5 MB，不该因为
    /// "已用 5 MB + 新增 5 MB 超上限"被拒 —— 语料压根没变大。
    #[test]
    fn admit_file_credits_bytes_that_the_rewrite_frees() {
        let limits = CodebaseSearchLimits {
            max_total_bytes: 100,
            ..test_limits()
        };

        // 已用 80，再写 30 → 超限。
        assert_eq!(
            admit_file(Path::new("a.rs"), 30, 0, 80, 0, limits),
            Admission::SkipBudget
        );
        // 但若这 30 是替换掉自己原先占的 30，实际用量仍是 80。
        assert_eq!(
            admit_file(Path::new("a.rs"), 30, 0, 80, 30, limits),
            Admission::Admit
        );
    }

    // ── should_full_rebuild：调优旋钮 ────────────────────────────────────────

    #[test]
    fn should_full_rebuild_uses_absolute_threshold_for_small_corpora() {
        // 3 个文件改 1 个是 33% —— 比例在这么小的语料上毫无意义，
        // 必须用绝对阈值，否则每次编辑都在重建。
        assert!(!should_full_rebuild(1, 3));
        assert!(!should_full_rebuild(3, 5));
        assert!(should_full_rebuild(4, 5));
    }

    #[test]
    fn should_full_rebuild_uses_ratio_for_large_corpora() {
        assert!(!should_full_rebuild(3, 100));
        assert!(!should_full_rebuild(30, 100), "恰好 30% 不算超过");
        assert!(should_full_rebuild(31, 100));
    }

    // ── apply_patch_ops：预算簿记 ────────────────────────────────────────────

    fn op(path: &str, size: u64, content: &str) -> PatchOp {
        PatchOp::Upsert {
            path: path.to_string(),
            size,
            chunks: vec![CodeChunk {
                content: content.to_string(),
                start_line: 1,
                end_line: 1,
                context_header: None,
            }],
        }
    }

    #[test]
    fn apply_patch_ops_removals_are_settled_before_budget_is_checked() {
        // 重命名的形状：删旧的 + 加新的，同批下发。
        // 若移除释放的预算没先回到池子里，峰值会让整个批次被误判超限。
        let limits = CodebaseSearchLimits {
            max_total_bytes: 60,
            ..test_limits()
        };
        let mut engine = SearchEngine::new();
        engine.add_document(
            "old.rs".to_string(),
            vec![CodeChunk {
                content: "x".repeat(50),
                start_line: 1,
                end_line: 1,
                context_header: None,
            }],
        );

        let ops = vec![
            PatchOp::Remove {
                path: "old.rs".to_string(),
            },
            op("new.rs", 50, &"y".repeat(50)),
        ];
        assert!(
            apply_patch_ops(&mut engine, &ops, limits),
            "先释放旧预算后，同尺寸重命名必须放得下"
        );
        assert_eq!(engine.document_paths(), vec!["new.rs"]);
        engine.verify_invariants().unwrap();
    }

    #[test]
    fn apply_patch_ops_reports_budget_exhaustion_instead_of_lying() {
        let limits = CodebaseSearchLimits {
            max_total_bytes: 10,
            ..test_limits()
        };
        let mut engine = SearchEngine::new();

        let ops = vec![op("a.rs", 100, &"z".repeat(100))];
        assert!(
            !apply_patch_ops(&mut engine, &ops, limits),
            "超预算必须如实上报 false，让上层回退全量重建"
        );
    }

    /// 增量路径里 "skip" 必须意味着 "remove" —— 这是最容易被实现错的一条。
    #[test]
    fn plan_patch_ops_turns_oversized_and_unknown_extension_into_removals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let limits = test_limits();

        // 一个超过单文件上限的 .rs，和一个不在白名单里的 .svg。
        // 用**多行**填充：单行重复 200 次 `fn big() {}` 会让 tree-sitter 建出
        // 极深的错误恢复树，把 8 MiB 的索引线程栈撑爆 —— 那是解析器自身的
        // 隐患（`index_file_safe` 的 catch_unwind 正为它准备），不该由测试夹具触发。
        let big: String = (0..200).map(|i| format!("fn big{i}() {{}}\n")).collect();
        write(root, "big.rs", &big);
        write(root, "logo.svg", "<svg/>");

        let result = IndexResult {
            new_blobs: vec![
                crate::core::context::indexer::Blob {
                    hash: "h1".into(),
                    path: "big.rs".into(),
                    content: None,
                },
                crate::core::context::indexer::Blob {
                    hash: "h2".into(),
                    path: "logo.svg".into(),
                    content: None,
                },
            ],
            removed_blobs: vec![],
            total_files: 2,
            skipped_large: 0,
            truncated: false,
        };

        let ops = plan_patch_ops(root, &result, limits);

        // 涨过上限的 .rs → Remove（它可能原本就在索引里）。
        assert!(
            ops.iter().any(|op| matches!(
                op,
                PatchOp::Remove { path } if path == "big.rs"
            )),
            "超限的 .rs 必须被移除，否则留下陈旧文档：{:?}",
            ops
        );

        // 不可索引扩展名 → **不产生任何操作**。扩展名是从路径派生的，
        // `logo.svg` 不可能曾经进过索引，发 Remove 只是让 changed 虚高。
        assert!(
            !ops.iter()
                .any(|op| matches!(op, PatchOp::Remove { ref path } if path == "logo.svg")),
            "不可索引文件不该产生操作：{:?}",
            ops
        );
    }

    #[test]
    fn plan_patch_ops_emits_upsert_for_a_normal_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "pub fn alpha() -> u32 { 1 }");

        let result = IndexResult {
            new_blobs: vec![crate::core::context::indexer::Blob {
                hash: "h".into(),
                path: "src/a.rs".into(),
                content: None,
            }],
            removed_blobs: vec![],
            total_files: 1,
            skipped_large: 0,
            truncated: false,
        };

        let ops = plan_patch_ops(root, &result, test_limits());
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            PatchOp::Upsert { path, size, chunks } => {
                assert_eq!(path, "src/a.rs");
                assert!(*size > 0);
                assert!(!chunks.is_empty());
            }
            other => panic!("期望 Upsert，实际 {:?}", std::mem::discriminant(other)),
        }
    }

    /// **删掉的路径必须被显式移除**，而不是"文件不在就不管"。
    #[test]
    fn plan_patch_ops_removes_deleted_paths() {
        let dir = tempfile::tempdir().unwrap();
        let result = IndexResult {
            new_blobs: vec![],
            removed_blobs: vec!["gone.rs".into()],
            total_files: 0,
            skipped_large: 0,
            truncated: false,
        };

        let ops = plan_patch_ops(dir.path(), &result, test_limits());
        assert!(matches!(&ops[0], PatchOp::Remove { path } if path == "gone.rs"));
    }

    // ── 基石：差分测试 ───────────────────────────────────────────────────────

    /// **本模块最重要的一个测试。**
    ///
    /// 全量构建 → 施加一批脚本化变更 → 走增量补丁 → 再把补丁结果与
    /// "从变更后的文件系统全量重建"逐项对比。两者必须**观测等价**。
    ///
    /// 这比逐个测簿记函数强得多：它直接断言"用户看到的东西一样"，
    /// 覆盖 swap-remove 重映射、doc_freqs 增减、预算口径、skip==remove
    /// 等所有边界的组合效应。任何一处簿记写错，这里都会红。
    #[test]
    fn incremental_matches_full_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let limits = test_limits();

        // 20 个文件：语料够大，5 处改动只有 25%，才会真正走补丁分支
        // 而不是被 should_full_rebuild 拦回全量。
        for i in 0..20 {
            write(
                root,
                &format!("src/f{i:02}.rs"),
                &format!("pub fn fn{i:02}() -> u32 {{\n    // marker{i:02}\n    {i}\n}}\n"),
            );
        }

        // 先跑一次文件索引，让 index.json 存在（mtime 令牌的来源）。
        Indexer::new(root).index_project().expect("first index");
        let cache = seed_cache(root, limits);

        // ── 脚本化变更 ────────────────────────────────────────────────────────
        std::thread::sleep(std::time::Duration::from_millis(20));

        // 1. 改一个
        write(
            root,
            "src/f05.rs",
            "pub fn fn05() -> u32 {\n    // marker05_rewritten\n    55\n}\n",
        );
        // 2. 删一个
        fs::remove_file(root.join("src/f06.rs")).unwrap();
        // 3. 重命名一个（旧路径消失 + 新路径出现，同批下发）
        fs::rename(root.join("src/f07.rs"), root.join("src/f07_moved.rs")).unwrap();
        // 4. 加一个
        write(
            root,
            "src/added.rs",
            "pub fn added() -> u32 {\n    // marker_added\n    1\n}\n",
        );
        // 5. 清空一个（必须变成 Remove，不能留陈旧文档）
        write(root, "src/f08.rs", "");
        // 6. 加一个不可索引类型的文件（必须完全不影响引擎）
        write(root, "assets/logo.svg", "<svg>marker_svg</svg>");
        // 7. 一个 .rs 涨过单文件上限（必须变成 Remove）
        write(
            root,
            "src/f09.rs",
            &format!(
                "pub fn fn09() -> u32 {{\n{}\n    0\n}}\n",
                "    // pad\n".repeat(120)
            ),
        );

        // ── 增量路径 ──────────────────────────────────────────────────────────
        let index_result = Indexer::new(root).index_project().expect("second index");
        let outcome = update_engine_in_cache_with_limits(root, &cache, &index_result, limits, None)
            .expect("incremental update must not error");

        assert!(
            matches!(outcome, UpdateOutcome::Patched { .. }),
            "语料 20 个、改动 5 处，必须走补丁分支而非全量重建：{:?}",
            outcome
        );

        // ── 对比 ──────────────────────────────────────────────────────────────
        let patched = cached_engine(root, limits, &cache);
        let rebuilt = full_engine(root, limits);

        patched
            .verify_invariants()
            .expect("补丁后的引擎必须满足全部簿记不变量");

        assert_eq!(
            patched.doc_count(),
            rebuilt.doc_count(),
            "文档数必须与全量重建一致"
        );
        assert_eq!(
            patched.document_paths(),
            rebuilt.document_paths(),
            "已索引路径集合必须与全量重建一致"
        );
        assert_eq!(
            patched.total_bytes(),
            rebuilt.total_bytes(),
            "预算口径必须与全量重建一致"
        );
        assert_eq!(
            patched.doc_freqs_snapshot(),
            rebuilt.doc_freqs_snapshot(),
            "词频必须与全量重建逐项一致 —— 任何漂移都会在此暴露"
        );

        // 具体断言各条边界都落到了预期状态。
        assert!(
            !patched.contains_document("src/f06.rs"),
            "删除的文件必须消失"
        );
        assert!(
            !patched.contains_document("src/f07.rs"),
            "重命名的旧路径必须消失"
        );
        assert!(
            patched.contains_document("src/f07_moved.rs"),
            "重命名的新路径必须进索引"
        );
        assert!(
            patched.contains_document("src/added.rs"),
            "新增文件必须进索引"
        );
        assert!(
            !patched.contains_document("src/f08.rs"),
            "清空的文件必须被移除，不能留下陈旧文档"
        );
        assert!(
            !patched.contains_document("assets/logo.svg"),
            "不可索引类型不得进引擎"
        );
        assert!(
            !patched.contains_document("src/f09.rs"),
            "涨过单文件上限的文件必须被移除"
        );

        // 探针查询：路径、行号、内容逐项一致。
        for query in ["marker00", "marker05_rewritten", "marker_added", "fn"] {
            assert_eq!(
                probe(&patched, query),
                probe(&rebuilt, query),
                "探针查询 {:?} 的结果必须与全量重建一致",
                query
            );
        }

        // 分数也必须一致 —— 它依赖 doc_freqs 与文档总数，是簿记正确性的
        // 最敏感指标。
        let p = patched.search("marker05_rewritten", 5);
        let r = rebuilt.search("marker05_rewritten", 5);
        assert_eq!(p.len(), r.len());
        assert_eq!(p[0].file_path, r[0].file_path);
        assert!(
            (p[0].score - r[0].score).abs() < 1e-9,
            "分数必须一致：补丁 {} vs 全量 {}",
            p[0].score,
            r[0].score
        );
    }

    /// 变更全是不可索引类型时，补丁一条操作都不发 —— 但 **mtime 必须被盖上**，
    /// 否则 `get_engine` 永不命中，查询路径每轮都触发一次刷新，形成活锁。
    #[test]
    fn no_indexable_changes_still_stamps_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let limits = test_limits();

        write(root, "src/a.rs", "pub fn alpha() -> u32 { 1 }\n");
        Indexer::new(root).index_project().unwrap();
        let cache = seed_cache(root, limits);
        let key = cache_key_for(root, limits);

        std::thread::sleep(std::time::Duration::from_millis(20));
        write(root, "assets/logo.svg", "<svg/>");

        let index_result = Indexer::new(root).index_project().unwrap();
        let outcome = update_engine_in_cache_with_limits(root, &cache, &index_result, limits, None)
            .expect("must not error");

        assert!(
            matches!(outcome, UpdateOutcome::Patched { changed: 0 }),
            "不可索引的变更不应产生任何补丁操作：{:?}",
            outcome
        );
        assert!(
            cache.get_engine(&key, index_mtime(root)).is_some(),
            "无操作补丁后必须以当前 mtime 命中，否则查询路径会陷入刷新活锁"
        );
    }

    /// 引擎是空的、而文件系统上有文件 —— 必须走全量。
    /// 若打补丁，会把空引擎补成 1 个文件，然后 `get_engine` 命中并**谎报 Fresh**，
    /// 用户看到的"索引"只有一个文件。
    #[test]
    fn cold_corpus_forces_full_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let limits = test_limits();

        write(root, "src/a.rs", "pub fn alpha() -> u32 { 1 }\n");
        Indexer::new(root).index_project().unwrap();

        // 缓存里放一个**空**引擎，但 mtime 是有效的。
        let cache = Arc::new(SearchEngineCacheManager::new());
        cache.put_engine(
            cache_key_for(root, limits),
            CachedSearchEngine {
                engine: SearchEngine::new(),
                index_mtime: index_mtime(root),
            },
        );

        let index_result = Indexer::new(root).index_project().unwrap();
        let outcome = update_engine_in_cache_with_limits(root, &cache, &index_result, limits, None)
            .expect("must not error");

        assert!(
            matches!(outcome, UpdateOutcome::FullRebuild { .. }),
            "冷语料必须全量重建，否则会谎报 Fresh：{:?}",
            outcome
        );
        let engine = cached_engine(root, limits, &cache);
        assert_eq!(engine.doc_count(), 1, "全量重建后必须真的索引到那个文件");
    }

    /// `index.json` 从未写过（mtime 为 None）→ 没有并发令牌 → 全量重建。
    #[test]
    fn engine_without_mtime_token_forces_full_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let limits = test_limits();

        write(root, "src/a.rs", "pub fn alpha() -> u32 { 1 }\n");

        let cache = Arc::new(SearchEngineCacheManager::new());
        cache.put_engine(
            cache_key_for(root, limits),
            CachedSearchEngine {
                engine: SearchEngine::new(),
                index_mtime: None,
            },
        );

        let index_result = Indexer::new(root).index_project().unwrap();
        let outcome = update_engine_in_cache_with_limits(root, &cache, &index_result, limits, None)
            .expect("must not error");

        assert!(matches!(outcome, UpdateOutcome::FullRebuild { .. }));
    }

    /// 回归"一直在构建"：后台重建完成后，下一次查询必须命中 Fresh。
    ///
    /// 链路是 resolve_engine（缓存空 → Warming/同步构建）→ 后台 worker 调
    /// update_engine_in_cache 存引擎 → 下次 resolve_engine 的 get_engine
    /// 比对 mtime。若存入的 mtime 与后续读到的对不上，会永远 Warming。
    #[test]
    fn engine_fresh_after_background_rebuild() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn auth() {}").unwrap();

        let cache = Arc::new(SearchEngineCacheManager::new());
        let limits = CodebaseSearchLimits {
            max_files: 100,
            max_file_bytes: 64 * 1024,
            max_total_bytes: 1024 * 1024,
            timeout_ms: 5_000,
        };

        // 冷启动：缓存空，resolve_engine 必然构建一次（测试里 watcher 未启动，
        // 走同步分支；真实会话里走 Warming 分支，两者随后都依赖同一条
        // "重建→存引擎→下次命中"路径）。
        let (_, _, src1) = resolve_engine(root, limits, &None, Some(&cache)).unwrap();
        assert!(
            matches!(src1, EngineSource::Fresh | EngineSource::Warming),
            "cold start should build or warm, got {src1:?}"
        );

        // 后台 worker 的核心动作：indexer 增量重建并更新缓存引擎。
        let indexer = crate::core::context::indexer::Indexer::new(root);
        let idx = indexer.index_project().unwrap();
        let outcome = update_engine_in_cache_with_limits(root, &cache, &idx, limits, None);
        assert!(outcome.is_ok(), "engine update failed: {:?}", outcome.err());

        // 关键断言：此后无文件变更，查询必须稳定命中 Fresh。
        for i in 0..3 {
            let (eng, _, src) = resolve_engine(root, limits, &None, Some(&cache)).unwrap();
            assert!(
                matches!(src, EngineSource::Fresh),
                "query #{} after rebuild should be Fresh, got {src:?}",
                i + 1
            );
            assert!(
                eng.doc_count() > 0,
                "engine should not be empty after rebuild"
            );
        }
    }

    /// **端到端：watcher 建索引 → 工具查得到。**
    ///
    /// 这条测试复现 ARTEX 事故的完整形状：
    ///
    /// ```text
    /// 1. watcher 把索引写进**共享单例**（生产路径：watcher::set_engine_cache）
    /// 2. 模型显式调用 CodebaseSearch
    /// 3. 查询路径必须拿到那个引擎，**不是** "still building"
    /// ```
    ///
    /// 事故原样：第 1 步与第 2 步用的是**两个不同的缓存对象**，
    /// 于是第 3 步永远判成"引擎不存在"，模型连着 11 次重试
    /// （35.2k tokens / 1m48s）后给用户写了"索引构建疑似卡住"的错误结论。
    ///
    /// 关键：第 2 步查的缓存**必须取自 `ContextEngine`** —— 生产路径上
    /// 工具拿到的就是 `context_engine.search_cache`（`agent_core::lazy_init`
    /// 的 `with_cache(...)`）。如果这里直接用共享单例，测试就绕开了
    /// 真正出问题的接线，事故照样能溜过去。
    #[test]
    fn the_watcher_and_the_query_path_share_one_engine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/repeater.rs", "fn repeater_replay_request() {}\n");
        write(root, "src/other.rs", "fn unrelated_handler() {}\n");

        let cache = crate::core::context::search_cache::shared_search_cache();
        cache.clear();

        // ── 第 1 步：watcher 建索引（生产路径：update_engine_in_cache 写共享单例）
        let indexer = Indexer::new(root);
        let idx = indexer.index_project().unwrap();
        let limits = codebase_search_limits();
        let outcome = update_engine_in_cache_with_limits(root, &cache, &idx, limits, None)
            .expect("watcher rebuild must succeed");
        assert!(
            matches!(outcome, UpdateOutcome::FullRebuild { .. }),
            "冷启动必须是全量重建：{outcome:?}"
        );

        // ── 第 2 步：工具查的是 **ContextEngine 手里那份**（生产接线）
        //
        // `agent_core::lazy_init` 就是这么注册的：
        // `CodebaseSearchTool::with_cache(config, context_engine.search_cache.clone())`
        let engine = crate::core::context::engine::ContextEngine::new();
        let tool_cache = engine.search_cache.clone();

        assert!(
            std::sync::Arc::ptr_eq(&tool_cache, &cache),
            "ContextEngine 必须与 watcher 写的是同一个缓存（分裂缓存 = 事故根因）"
        );

        let presence = root_engine_presence(&tool_cache, root);
        assert_eq!(
            presence,
            EnginePresence::Fresh,
            "watcher 建好的引擎必须对查询路径可见（分裂缓存 = 事故根因）"
        );

        // ── 第 3 步：走完整的检索路径，必须真的检索到
        let out = search_codebase_with_limits(
            root,
            "repeater_replay_request",
            None,
            limits,
            Some(&tool_cache),
        )
        .expect("search must succeed");
        assert!(
            out.contains("repeater.rs"),
            "检索必须命中 repeater.rs，实际输出：{out}"
        );
        assert!(
            !out.contains("still building"),
            "索引已就绪，绝不能回 \"still building\"：{out}"
        );

        cache.clear();
    }

    /// **端到端：显式调用在引擎未就绪时不会自说自话。**
    ///
    /// 显式调用路径的等待逻辑（修复 A）必须满足：
    /// - 引擎 Fresh → **不等待**，立刻检索；
    /// - 引擎 Absent → 等到预算上限，然后如实说"没就绪"。
    ///
    /// 两种情况都**不能**返回 "still building" 之外的东西，更不能让模型
    /// 收到一句"重跑一次"（那是死循环指令）。
    #[test]
    fn an_explicit_call_never_tells_the_model_to_retry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn alpha_handler() {}\n");
        Indexer::new(root).index_project().unwrap();

        let cache = std::sync::Arc::new(SearchEngineCacheManager::new());
        let out =
            search_codebase_with_limits(root, "alpha_handler", None, test_limits(), Some(&cache))
                .expect("search must succeed");

        assert!(
            !out.contains("Re-run `CodebaseSearch`"),
            "绝不能让模型重跑同一个查询（旧文案是死循环指令）：{out}"
        );
        assert!(
            !out.contains("before giving up"),
            "绝不能出现「重跑一次再放弃」这类死循环指令：{out}"
        );
    }

    /// 文件未变更时，indexer 不该重复写盘 —— 否则 index.json 的 mtime
    /// 每轮都变，get_engine 的 mtime 比对永远失效，陷入"一直构建"。
    #[test]
    fn stable_index_does_not_rewrite_index_json() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn auth() {}").unwrap();

        let indexer = crate::core::context::indexer::Indexer::new(root);
        indexer.index_project().unwrap();

        let m1 = std::fs::metadata(root.join(".star/context/index.json"))
            .and_then(|m| m.modified())
            .expect("index.json must exist after first index");

        // 无任何文件变更，再跑一轮：不应写盘。
        indexer.index_project().unwrap();

        let m2 = std::fs::metadata(root.join(".star/context/index.json"))
            .and_then(|m| m.modified())
            .expect("index.json must still exist");

        assert_eq!(
            m1, m2,
            "index.json was rewritten despite no changes — mtime churn breaks engine cache hits"
        );
    }

    /// 回归"一直在构建"：update_engine_in_cache_with_limits 存入缓存的
    /// mtime 令牌必须等于**写盘之后**的 index.json mtime。
    ///
    /// 以前令牌在函数开头读取，而 full_rebuild 内部的 index_project() 会写
    /// .star/context/index.json —— 存入的是旧令牌，下次 resolve_engine 的
    /// get_engine 比对必然失败，查询永远命中 Warming/Stale。用户看到的
    /// 就是"索引一直在构建"。
    #[test]
    fn stored_mtime_matches_post_write_index_json() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn auth() {}").unwrap();

        let cache = Arc::new(SearchEngineCacheManager::new());
        let limits = CodebaseSearchLimits {
            max_files: 100,
            max_file_bytes: 64 * 1024,
            max_total_bytes: 1024 * 1024,
            timeout_ms: 5_000,
        };

        // 让 index_project 先写一次盘（模拟 indexer 已跑过一轮）。
        let indexer = crate::core::context::indexer::Indexer::new(root);
        indexer.index_project().unwrap();
        let mtime_after_first_write = std::fs::metadata(root.join(".star/context/index.json"))
            .and_then(|m| m.modified())
            .expect("index.json must exist");

        // 再跑一次（无变更，但可能仍写盘），随后更新引擎缓存。
        let idx = indexer.index_project().unwrap();
        update_engine_in_cache_with_limits(root, &cache, &idx, limits, None).unwrap();

        let canonical = root.canonicalize().unwrap();
        let key = (canonical, limits.cache_key());
        let stored = cache.engine_meta(&key).expect("engine must be cached");

        // 存入的令牌必须等于此刻磁盘上的真实 mtime —— 这正是下次查询
        // resolve_engine 会读到的值。
        let on_disk = std::fs::metadata(root.join(".star/context/index.json"))
            .and_then(|m| m.modified())
            .expect("index.json must still exist");

        assert_eq!(
            stored.index_mtime,
            Some(on_disk),
            "stored mtime token must match post-write index.json;\
             stale token = {:?}, on disk = {:?}",
            stored.index_mtime.unwrap_or(SystemTime::UNIX_EPOCH),
            on_disk
        );
        let _ = mtime_after_first_write;

        // 下次查询应当直接命中 Fresh。
        let (_, _, src) = resolve_engine(root, limits, &None, Some(&cache)).unwrap();
        assert!(
            matches!(src, EngineSource::Fresh),
            "query should be Fresh with a valid token, got {src:?}"
        );
    }

    /// **ARTEX 生产路径端到端探针**（手动，不进 CI）。
    ///
    /// `diagnose_real_repo` 只验"引擎能不能建出来、能不能命中"；这一条验的是
    /// **接线** —— 事故那一步恰恰不是"引擎建不出来"，而是"引擎建好了、
    /// 工具读不到"。所以必须走生产路径的三段：
    ///
    /// ```text
    /// 1. watcher 把索引写进**共享单例**
    /// 2. 工具从 **ContextEngine** 手里那份读（不是共享单例直连）
    /// 3. 查询必须命中，且不能出现 "still building"
    /// ```
    ///
    /// 第 2 步必须取自 `ContextEngine` —— 直接用共享单例会绕开真正出问题的
    /// 那段接线，事故照样能溜过去。
    ///
    /// ```bash
    /// STAR_DIAG_ROOT=/mnt/h/test/ARTEX \
    /// STAR_DIAG_QUERY="repeater replay request send draft parse HTTP" \
    /// cargo test --release --lib -- artix_e2e_probe --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn artix_e2e_probe() {
        let root = std::env::var("STAR_DIAG_ROOT")
            .map(std::path::PathBuf::from)
            .expect("STAR_DIAG_ROOT must point at the repo");
        let query = std::env::var("STAR_DIAG_QUERY")
            .unwrap_or_else(|_| "repeater replay request send draft parse HTTP".to_string());

        let cache = crate::core::context::search_cache::shared_search_cache();
        cache.clear();

        // ── 第 1 步：watcher 建索引（生产路径：update_engine_in_cache → 共享单例）
        let idx = Indexer::new(&root).index_project().expect("index");
        println!(
            "[1] index_project: total_files={} new_blobs={} removed={} truncated={}",
            idx.total_files,
            idx.new_blobs.len(),
            idx.removed_blobs.len(),
            idx.truncated
        );
        let limits = codebase_search_limits();
        let outcome = update_engine_in_cache_with_limits(&root, &cache, &idx, limits, None)
            .expect("watcher rebuild");
        println!("[1] watcher rebuild outcome: {outcome:?}");

        // ── 第 2 步：工具读的是 ContextEngine 那份
        let engine = crate::core::context::engine::ContextEngine::new();
        let tool_cache = engine.search_cache.clone();
        println!(
            "[2] same object as shared cache? {}",
            std::sync::Arc::ptr_eq(&tool_cache, &cache)
        );
        let presence = root_engine_presence(&tool_cache, &root);
        println!("[2] engine presence (tool's view): {presence:?}");

        // ── 第 3 步：走完整检索路径
        let out = search_codebase_with_limits(&root, &query, None, limits, Some(&tool_cache))
            .expect("search");
        println!(
            "[3] contains 'still building'? {} | contains 'No hits'? {}",
            out.contains("still building"),
            out.contains("No hits")
        );
        for line in out.lines().take(8) {
            println!("      {line}");
        }

        // 断言：这一步失败 = 事故复现
        assert_eq!(
            presence,
            EnginePresence::Fresh,
            "工具必须能看到 watcher 建好的引擎（分裂缓存 = 事故根因）"
        );
        assert!(
            !out.contains("still building"),
            "绝不能回 \"still building\"：\n{out}"
        );
        assert!(
            !out.contains("No hits"),
            "事故当时的查询必须命中，不能零命中：\n{out}"
        );

        cache.clear();
        println!("[OK] ARTEX 生产路径端到端通过");
    }

    /// 手动诊断（不进 CI）：对着**真实仓库**跑一条 query，把引擎返回、
    /// 分词结果与零命中诊断全打出来。
    ///
    /// ```powershell
    /// $env:STAR_DIAG_ROOT="H:\test\ARTEX"
    /// $env:STAR_DIAG_QUERY="runWorker assignWork MCP tool call"
    /// cargo test --release --lib -- diagnose_real_repo --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn diagnose_real_repo() {
        let root = std::env::var("STAR_DIAG_ROOT")
            .expect("STAR_DIAG_ROOT must point at the repo to diagnose");
        let root = std::path::PathBuf::from(root);
        let query = std::env::var("STAR_DIAG_QUERY")
            .unwrap_or_else(|_| "runWorker assignWork MCP tool call".to_string());

        let limits = match std::env::var("STAR_DIAG_BUDGET").as_deref() {
            Ok("auto") => codebase_search_limits_for_profile(Some("auto")),
            _ => CodebaseSearchLimits {
                max_files: 1200,
                max_file_bytes: 512 * 1024,
                max_total_bytes: 12 * 1024 * 1024,
                timeout_ms: 12_000,
            },
        };
        let (engine, stats) =
            build_search_engine_from_fs(&root, limits, &None).expect("engine build should succeed");
        println!(
            "indexed_files={} bytes={} truncated={} scanned={}",
            stats.indexed_files, stats.total_bytes, stats.truncated, stats.scanned_text_files
        );

        let base = engine.extract_base_terms(&query);
        println!("base_terms={base:?}");
        println!("diagnosis={:?}", engine.diagnosis_from_terms(&base));

        let expanded =
            engine.search_with_options(&query, 40, &SearchOptions::expanded_no_diversity());
        let exact = engine.search_with_options(&query, 40, &SearchOptions::exact());
        println!(
            "expanded_results={} exact_results={}",
            expanded.len(),
            exact.len()
        );
        for result in expanded.iter().take(5) {
            println!(
                "  {}:{} score={:.3} matched={:?}",
                result.file_path, result.chunk.start_line, result.score, result.matches
            );
        }
    }

    // ── explore（复合查询） ─────────────────────────────────────────────

    /// 一次调用必须带回三样东西：逐字源码、直接调用关系、影响面。
    /// 少任何一样，agent 就还得再补一次 Read/Grep，复合查询就失去意义。
    #[test]
    fn explore_returns_source_and_call_flow_in_one_call() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "src/a.rs",
            "fn helper() {}\n\nfn target() {\n    helper();\n}\n",
        );
        write(root, "src/b.rs", "fn caller() {\n    target();\n}\n");

        let out = explore_symbol(root, "target", ExploreLimits::default()).unwrap();

        assert!(out.contains("fn target()"), "必须带逐字源码: {out}");
        assert!(
            out.contains("Callers (1)") && out.contains("caller"),
            "必须带直接调用者: {out}"
        );
        assert!(
            out.contains("Callees (1)") && out.contains("helper"),
            "必须带直接被调者: {out}"
        );
        assert!(out.contains("src/b.rs"), "调用者路径必须出现: {out}");
        assert!(out.contains("Impact (depth 2)"), "必须带影响面: {out}");
    }

    /// 源码必须按预算截断，并明确告诉 agent 剩余部分从哪里读。
    /// 不截断会让一个 800 行的函数直接吃掉整个上下文。
    #[test]
    fn explore_caps_the_source_snippet() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: String = (1..=30).map(|i| format!("    // line {i}\n")).collect();
        write(root, "src/long.rs", &format!("fn long() {{\n{body}}}\n"));

        let limits = ExploreLimits {
            max_symbols: 1,
            depth: 1,
            max_source_lines: 5,
        };
        let out = explore_symbol(root, "long", limits).unwrap();

        assert_eq!(
            out.matches("// line").count(),
            4,
            "只应带 5 行源码（含签名行）: {out}"
        );
        assert!(out.contains("more lines"), "截断必须明确告知: {out}");
        assert!(
            out.contains("Read `src/long.rs:6`"),
            "必须指向剩余内容的起始行: {out}"
        );
    }

    #[test]
    fn explore_says_when_nothing_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn a() {}\n");

        let out = explore_symbol(root, "no_such_symbol_xyz", ExploreLimits::default()).unwrap();
        assert!(out.contains("No symbols found"), "{out}");
        assert!(
            out.contains("CodebaseSearch"),
            "零命中时应给出下一步建议: {out}"
        );
    }

    /// FTS5 分词检索补上了内存 contains 的盲区：
    /// 「auth handler」在内存里匹配不到 `auth_handler`（查询串不连续）。
    #[test]
    fn explore_finds_symbols_by_identifier_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/a.rs", "fn auth_handler() {}\n");

        let out = explore_symbol(root, "auth handler", ExploreLimits::default()).unwrap();
        assert!(
            out.contains("auth_handler"),
            "分词检索应命中 auth_handler: {out}"
        );
    }
}
