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
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const IGNORED_SEMANTIC_SEARCH_DIRS: &[&str] = &[
    ".git",
    ".next",
    ".star",
    ".turbo",
    ".venv",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "vendor",
    "venv",
];
const DEFAULT_SEMANTIC_SEARCH_MAX_FILES: usize = 1200;
const DEFAULT_SEMANTIC_SEARCH_MAX_FILE_BYTES: u64 = 512 * 1024;
const DEFAULT_SEMANTIC_SEARCH_MAX_TOTAL_BYTES: u64 = 12 * 1024 * 1024;
const DEFAULT_SEMANTIC_SEARCH_TIMEOUT_MS: u64 = 12_000;
const DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_FILES: usize = 320;
const DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_FILE_BYTES: u64 = 256 * 1024;
const DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_AUTO_SEMANTIC_SEARCH_TIMEOUT_MS: u64 = 5_000;
const SEMANTIC_SEARCH_PROGRESS_EVERY_FILES: usize = 120;

#[derive(Clone)]
pub struct SemanticSearchTool {
    config: Arc<crate::core::config::Config>,
    search_cache: Option<Arc<SearchEngineCacheManager>>,
}

impl SemanticSearchTool {
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
pub struct SemanticSearchParams {
    pub query: String,
    pub path: Option<String>,
    #[serde(default)]
    pub budget_profile: Option<String>,
}

pub struct SemanticSearchInvocation {
    tool: SemanticSearchTool,
    params: SemanticSearchParams,
}

#[derive(Clone, Copy)]
struct SemanticSearchLimits {
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
    timeout_ms: u64,
}

impl SemanticSearchLimits {
    /// A compact key for cache lookups, encoding the budget profile.
    fn cache_key(&self) -> String {
        format!(
            "f{}b{}t{}",
            self.max_files, self.max_total_bytes, self.timeout_ms
        )
    }
}

#[derive(Default)]
struct SemanticSearchStats {
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

/// Returns a cached SearchEngine if the index hasn't changed, otherwise builds a new one.
///
/// Uses the `SearchEngineCacheManager` (parking_lot RwLock, no poisoning) when
/// provided.  Falls back to building a fresh engine every call when `cache` is None.
fn get_or_build_search_engine(
    root: &Path,
    limits: SemanticSearchLimits,
    update_output: &Option<ProgressCallback>,
    cache: Option<&Arc<SearchEngineCacheManager>>,
) -> Result<(SearchEngine, SemanticSearchStats), Box<dyn std::error::Error + Send + Sync>> {
    let cache_key = (
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        limits.cache_key(),
    );

    let current_mtime = index_mtime(root);

    // Fast path: check the parking_lot-based cache (no poisoning risk).
    if let Some(manager) = cache {
        if let Some(engine) = manager.get_engine(&cache_key, current_mtime) {
            let stats = SemanticSearchStats {
                indexed_files: 0,
                ..Default::default()
            };
            return Ok((engine, stats));
        }
    }

    // Slow path: (re)build the engine from filesystem.
    let (engine, stats) = build_search_engine_from_fs(root, limits, update_output)?;

    // Store in cache if available.
    if let Some(manager) = cache {
        manager.put_engine(
            cache_key,
            CachedSearchEngine {
                engine: engine.clone(),
                index_mtime: current_mtime,
            },
        );
    }

    Ok((engine, stats))
}

/// Error record for a single file that failed indexing (does not stop the batch).
#[derive(Debug, Clone)]
struct FileIndexError {
    path: String,
    reason: String,
}

/// Build a fresh SearchEngine by traversing the filesystem (cold start).
///
/// **Per-file isolation**: tree-sitter chunking is wrapped in `catch_unwind` on a
/// dedicated large-stack thread.  A panic or parse failure in one file does **not**
/// stop the rest of the batch — partial results are always returned.
fn build_search_engine_from_fs(
    root: &Path,
    limits: SemanticSearchLimits,
    update_output: &Option<ProgressCallback>,
) -> Result<(SearchEngine, SemanticSearchStats), Box<dyn std::error::Error + Send + Sync>> {
    let mut engine = SearchEngine::new();
    let mut stats = SemanticSearchStats::default();
    let mut last_progress_indexed = 0usize;
    let mut file_errors: Vec<FileIndexError> = Vec::new();

    let mut builder = WalkBuilder::new(root);
    builder.hidden(true).git_ignore(true);
    builder.filter_entry(|entry| !should_skip_semantic_search_entry(entry.path()));
    let walker = builder.build();

    emit_semantic_progress(
        update_output,
        format!("Indexing codebase · root {}", root.display()),
    );

    for result in walker {
        if stats.indexed_files >= limits.max_files || stats.total_bytes >= limits.max_total_bytes {
            stats.truncated = true;
            emit_semantic_progress(
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

                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ![
                        "rs", "py", "js", "ts", "tsx", "go", "java", "c", "cpp", "md", "txt",
                        "json", "toml",
                    ]
                    .contains(&ext)
                    {
                        continue;
                    }

                    stats.scanned_text_files += 1;

                    let file_size = match std::fs::metadata(path) {
                        Ok(meta) => meta.len(),
                        Err(_) => continue,
                    };

                    if file_size > limits.max_file_bytes {
                        stats.skipped_large_files += 1;
                        continue;
                    }

                    if stats.total_bytes + file_size > limits.max_total_bytes {
                        stats.truncated = true;
                        break;
                    }

                    match index_file_safe(path, ext, root) {
                        Ok((rel_path, chunks)) => {
                            if !chunks.is_empty() {
                                engine.add_document(rel_path, chunks);
                                stats.indexed_files += 1;
                                stats.total_bytes += file_size;

                                if stats.indexed_files == 1
                                    || stats.indexed_files.saturating_sub(last_progress_indexed)
                                        >= SEMANTIC_SEARCH_PROGRESS_EVERY_FILES
                                {
                                    last_progress_indexed = stats.indexed_files;
                                    emit_semantic_progress(
                                        update_output,
                                        format_semantic_progress(&stats),
                                    );
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
                emit_semantic_progress(update_output, format!("Walk error (non-fatal): {}", err));
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
        emit_semantic_progress(update_output, msg);
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

fn semantic_search_limits() -> SemanticSearchLimits {
    semantic_search_limits_for_profile(None)
}

fn semantic_search_limits_for_profile(profile: Option<&str>) -> SemanticSearchLimits {
    let normalized = profile.map(|value| value.trim().to_ascii_lowercase());
    let is_auto_budget = matches!(normalized.as_deref(), Some("auto" | "fast" | "quick"));

    if is_auto_budget {
        return SemanticSearchLimits {
            max_files: env_usize(
                "STAR_AUTO_SEMANTIC_SEARCH_MAX_FILES",
                DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_FILES,
            ),
            max_file_bytes: env_u64(
                "STAR_AUTO_SEMANTIC_SEARCH_MAX_FILE_BYTES",
                DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_FILE_BYTES,
            ),
            max_total_bytes: env_u64(
                "STAR_AUTO_SEMANTIC_SEARCH_MAX_TOTAL_BYTES",
                DEFAULT_AUTO_SEMANTIC_SEARCH_MAX_TOTAL_BYTES,
            ),
            timeout_ms: env_u64(
                "STAR_AUTO_SEMANTIC_SEARCH_TIMEOUT_MS",
                DEFAULT_AUTO_SEMANTIC_SEARCH_TIMEOUT_MS,
            ),
        };
    }

    SemanticSearchLimits {
        max_files: env_usize(
            "STAR_SEMANTIC_SEARCH_MAX_FILES",
            DEFAULT_SEMANTIC_SEARCH_MAX_FILES,
        ),
        max_file_bytes: env_u64(
            "STAR_SEMANTIC_SEARCH_MAX_FILE_BYTES",
            DEFAULT_SEMANTIC_SEARCH_MAX_FILE_BYTES,
        ),
        max_total_bytes: env_u64(
            "STAR_SEMANTIC_SEARCH_MAX_TOTAL_BYTES",
            DEFAULT_SEMANTIC_SEARCH_MAX_TOTAL_BYTES,
        ),
        timeout_ms: env_u64(
            "STAR_SEMANTIC_SEARCH_TIMEOUT_MS",
            DEFAULT_SEMANTIC_SEARCH_TIMEOUT_MS,
        ),
    }
}

fn should_skip_semantic_search_entry(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| IGNORED_SEMANTIC_SEARCH_DIRS.contains(&name))
        .unwrap_or(false)
}

async fn run_semantic_search(
    root_path: PathBuf,
    query: String,
    budget_profile: Option<String>,
    update_output: Option<ProgressCallback>,
    cache: Option<Arc<SearchEngineCacheManager>>,
) -> String {
    let limits = semantic_search_limits_for_profile(budget_profile.as_deref());
    let budget_label = match budget_profile
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("auto" | "fast" | "quick") => "fast budget",
        _ => "default budget",
    };
    emit_semantic_progress(
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
                    emit_semantic_progress(
                        &update_output,
                        "Semantic search failed · falling back to text search",
                    );
                    let fallback = format_ripgrep_fallback(&query, &root_path);
                    return format!("Semantic search error: {}\n\n{}", e, fallback);
                }
                Err(e) => {
                    emit_semantic_progress(
                        &update_output,
                        "Semantic search worker crashed · falling back to text search",
                    );
                    let fallback = format_ripgrep_fallback(&query, &root_path);
                    return format!("Semantic search execution failed: {}\n\n{}", e, fallback);
                }
            }
        }
        _ = &mut sleep => {
            emit_semantic_progress(
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

pub async fn run_semantic_search_for_skill(root_path: std::path::PathBuf, query: String) -> String {
    run_semantic_search(root_path, query, None, None, None).await
}

impl ToolInvocation for SemanticSearchInvocation {
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
            let root_path = resolve_semantic_search_root(&self.tool.config, path.as_deref());

            let results =
                run_semantic_search(root_path, query, budget_profile, update_output, cache).await;

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

impl BaseDeclarativeTool for SemanticSearchTool {
    fn name(&self) -> &str {
        "SemanticSearch"
    }

    fn display_name(&self) -> &str {
        "Semantic Search"
    }

    fn description(&self) -> &str {
        "ACE-POWERED Semantic Search. PRIMARY tool for conceptual/functional queries (architecture, flow, ownership, tests, config, permissions, providers, UI). Returns ranked code context with match signals."
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
        let params: SemanticSearchParams = serde_json::from_value(params)?;
        Ok(Box::new(SemanticSearchInvocation {
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
    search_codebase_with_limits(root, query, update_output, semantic_search_limits(), None)
}

fn search_codebase_with_limits(
    root: &Path,
    query: &str,
    update_output: Option<ProgressCallback>,
    limits: SemanticSearchLimits,
    cache: Option<&Arc<SearchEngineCacheManager>>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // ── Query result cache ────────────────────────────────────────────────────
    // Check the parking_lot-based LRU cache (no poisoning risk).
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let current_mtime = index_mtime(root);
    let query_cache_key = (canonical_root.clone(), query.to_string(), current_mtime);

    if let Some(manager) = cache {
        if let Some(cached_output) = manager.get_query(&query_cache_key) {
            emit_semantic_progress(&update_output, "Query cache hit · returning cached result");
            return Ok(cached_output);
        }
    }

    // ── Build or reuse cached search engine ──────────────────────────────────
    // Leverages the Indexer's .star/context/index.json mtime as a validity signal:
    // if no files have changed since the last build, the in-memory index is reused.
    let (engine, stats) = get_or_build_search_engine(root, limits, &update_output, cache)?;

    let was_cached = stats.indexed_files == 0 && !stats.truncated;

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
    emit_semantic_progress(&update_output, progress_msg);

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
    if was_cached {
        output.push_str("Index: cached (no files changed)\n");
    } else {
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

fn emit_semantic_progress(update_output: &Option<ProgressCallback>, message: impl Into<String>) {
    if let Some(cb) = update_output.as_ref() {
        cb(message.into());
    }
}

fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn format_semantic_progress(stats: &SemanticSearchStats) -> String {
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

fn resolve_semantic_search_root(
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

    let mut builder = WalkBuilder::new(root);
    builder.hidden(true).git_ignore(true);
    builder.filter_entry(|entry| !should_skip_semantic_search_entry(entry.path()));

    for result in builder.build() {
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
