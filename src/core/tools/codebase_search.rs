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
use std::collections::HashMap;
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
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self {
            config,
            search_cache: None,
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
    /// A compact key for cache lookups, encoding the budget profile.
    pub fn cache_key(&self) -> String {
        format!(
            "f{}b{}t{}",
            self.max_files, self.max_total_bytes, self.timeout_ms
        )
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
        format!("Semantic Search: {}", self.params.query)
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
        "ACE-POWERED codebase search. PRIMARY tool for conceptual/functional queries (architecture, flow, ownership, tests, config, permissions, providers, UI). Returns ranked code context with match signals."
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
                    "description": "Natural language query (e.g. 'how is authentication handled', 'where are user settings stored')"
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
    let cache_key = (canonical_root, limits.cache_key());

    if let Some(manager) = cache {
        if let Some(engine) = manager.get_engine(&cache_key, current_mtime) {
            return Ok((engine, IndexStats::default(), EngineSource::Fresh));
        }

        if let Some((stale_engine, _)) = manager.get_engine_stale(&cache_key) {
            crate::core::context::watcher::request_refresh(root);
            return Ok((stale_engine, IndexStats::default(), EngineSource::Stale));
        }

        // 冷启动：协调器可用且未发生过失败 → 后台构建 + Warming 兜底
        let coordinator_engaged = crate::core::context::watcher::is_started();
        if coordinator_engaged && !crate::core::context::watcher::last_rebuild_failed() {
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
    let (engine, stats, source) = resolve_engine(root, limits, &update_output, cache)?;

    if matches!(source, EngineSource::Warming) {
        let mut output = String::new();
        output.push_str(&format!(
            "Semantic index is building in the background for '{}' (first build; typically finishes in seconds).\n",
            root.display()
        ));
        output.push_str("No results available from the semantic index this turn. For exact symbols/strings use `Grep` now; re-run `CodebaseSearch` shortly for conceptual queries.\n");
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
        TOP_N_CANDIDATE,
        &SearchOptions::expanded_no_diversity(),
    );

    // Strategy B: exact terms only, no diversity
    let exact_results = engine.search_with_options(query, TOP_N_CANDIDATE, &SearchOptions::exact());

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

    // RRF fusion
    let fused = fusion::fuse(
        &[expanded_candidates, exact_candidates],
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
    let rerank_candidates: Vec<RerankCandidate> = fused
        .iter()
        .filter_map(|f| result_lookup.get(&f.id).map(|r| (*r).clone()))
        .map(|r| RerankCandidate::from_search_result(&r))
        .take(TOP_N_FUSED)
        .collect();

    // ── Reranker ──────────────────────────────────────────────────────────────
    let reranker = HeuristicReranker;
    let reranked = reranker.rerank(query, rerank_candidates, TOP_N_FINAL);

    // ── Format Output ────────────────────────────────────────────────────────
    let mut output = String::new();
    output.push_str(&format!(
        "Semantic Search Results for '{}' (Top {})\nRoot: {}\n",
        query,
        reranked.len(),
        root.display(),
    ));
    if matches!(source, EngineSource::Stale) {
        output.push_str(
            "Index: serving previous revision — files changed since; rebuild running in background (results may be slightly stale)\n",
        );
    }
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
    output.push_str(&format!(
        "Search: RRF fusion (K=60) · expanded + exact · reranked ({})\n\n",
        reranked.len()
    ));
    if stats.truncated {
        output.push_str(&format!(
            "Note: search was capped for responsiveness (max_files={}, max_total_bytes={}).\n\n",
            limits.max_files, limits.max_total_bytes
        ));
    }

    if reranked.is_empty() {
        output.push_str("No relevant code found.");
    } else {
        for r in &reranked {
            let c = &r.candidate;
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
    }

    // ── Store in query cache ─────────────────────────────────────────────────
    // LRU eviction is handled automatically by lru::LruCache.
    if let Some(manager) = cache {
        manager.put_query(query_cache_key, output.clone());
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

    match search_with_ripgrep(query, root.to_string_lossy().as_ref(), config) {
        Ok(results) => format_ripgrep_results(query, &results),
        Err(e) => format!("Fallback grep failed: {}", e),
    }
}

fn format_ripgrep_results(
    query: &str,
    results: &[crate::core::tools::tools::SearchResult],
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "Fallback grep results for '{}' ({} hits):\n\n",
        query,
        results.len()
    ));

    if results.is_empty() {
        output.push_str("No matches found.");
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

        let language = language_for_call_graph(ext);
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

/// Map file extension to Tree-sitter language for call graph extraction.
fn language_for_call_graph(ext: &str) -> Option<tree_sitter::Language> {
    match ext {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" | "pyi" => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        _ => None,
    }
}

/// Trace the call chain for a symbol matching `name_hint`.
///
/// Returns a formatted call chain string suitable for the LLM.
pub fn trace_call_chain(
    root: &Path,
    name_hint: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let graph = build_call_graph(root)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::chunking::CodeChunk;
    use crate::core::context::indexer::{IndexResult, Indexer};
    use crate::core::context::search_cache::{CachedSearchEngine, SearchEngineCacheManager};
    use std::fs;

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
}
