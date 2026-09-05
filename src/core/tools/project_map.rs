use crate::core::tools::tools::ToolResult as CoreToolResult;
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Language specification table ────────────────────────────────────────────
// To add a new language: add ONE entry here.  No other code changes needed.
struct LanguageSpec {
    ext: &'static str,
    bucket: &'static str,
    /// Symbol extraction regex (None → file is indexed but not symbol-scanned).
    symbol_pattern: Option<&'static str>,
}

static LANGUAGE_SPECS: &[LanguageSpec] = &[
    LanguageSpec {
        ext: "rs",
        bucket: "rust",
        symbol_pattern: Some(
            r"^\s*(pub\s+)?(fn|struct|enum|trait|impl|mod|type)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "py",
        bucket: "python",
        symbol_pattern: Some(r"^\s*(class|def)\s+([a-zA-Z0-9_]+)"),
    },
    LanguageSpec {
        ext: "js",
        bucket: "javascript",
        symbol_pattern: Some(
            r"^\s*(export\s+)?(class|function|const|let|var|interface|type)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "ts",
        bucket: "typescript",
        symbol_pattern: Some(
            r"^\s*(export\s+)?(class|function|const|let|var|interface|type)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "tsx",
        bucket: "typescript",
        symbol_pattern: Some(
            r"^\s*(export\s+)?(class|function|const|let|var|interface|type)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "jsx",
        bucket: "react",
        symbol_pattern: Some(
            r"^\s*(export\s+)?(class|function|const|let|var|interface|type)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "go",
        bucket: "go",
        symbol_pattern: Some(r"^\s*func\s+([a-zA-Z0-9_]+)"),
    },
    LanguageSpec {
        ext: "java",
        bucket: "java",
        symbol_pattern: Some(
            r"^\s*(public|private|protected)?\s*(class|interface|enum)\s+([a-zA-Z0-9_]+)",
        ),
    },
    LanguageSpec {
        ext: "c",
        bucket: "c/cpp",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "cpp",
        bucket: "c/cpp",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "h",
        bucket: "c/cpp",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "hpp",
        bucket: "c/cpp",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "toml",
        bucket: "toml",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "json",
        bucket: "json",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "yaml",
        bucket: "yaml",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "yml",
        bucket: "yaml",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "md",
        bucket: "markdown",
        symbol_pattern: None,
    },
    LanguageSpec {
        ext: "sh",
        bucket: "shell",
        symbol_pattern: None,
    },
];

/// Manifest file names that always appear in the "Key Files" section and may
/// override the ext-based language bucket label.
static KEY_FILE_BUCKETS: &[(&str, &str)] = &[
    ("cargo.toml", "cargo"),
    ("package.json", "node"),
    ("pyproject.toml", "python-project"),
    ("go.mod", "go-project"),
    ("pnpm-workspace.yaml", "node"),
];

static KEY_FILE_NAMES: &[&str] = &[
    "cargo.toml",
    "package.json",
    "pnpm-workspace.yaml",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "readme.md",
    "star.md",
    "claude.md",
];

/// Compile every symbol-extraction pattern exactly once (lazy, thread-safe).
fn compiled_patterns() -> &'static HashMap<&'static str, Regex> {
    static PATTERNS: OnceLock<HashMap<&'static str, Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        LANGUAGE_SPECS
            .iter()
            .filter_map(|spec| {
                spec.symbol_pattern
                    .map(|pat| (spec.ext, Regex::new(pat).expect("invalid symbol pattern")))
            })
            .collect()
    })
}

const DEFAULT_PROJECT_MAP_DEPTH: usize = 4;
const DEFAULT_PROJECT_MAP_MAX_FILES: usize = 400;
const DEFAULT_PROJECT_MAP_MAX_SYMBOL_FILES: usize = 60;
const DEFAULT_PROJECT_MAP_SYMBOL_LINE_LIMIT: usize = 320;
const DEFAULT_PROJECT_MAP_CACHE_TTL_SECS: u64 = 300;
const MAX_PROJECT_MAP_CACHE_ENTRIES: usize = 24;

#[derive(Clone)]
pub struct ProjectMapTool {
    _config: Arc<crate::core::config::Config>,
}

impl ProjectMapTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { _config: config }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProjectMapParams {
    pub path: Option<String>,
    pub max_depth: Option<usize>,
    pub include_symbols: Option<bool>,
    pub force_refresh: Option<bool>,
    pub max_files: Option<usize>,
}

pub struct ProjectMapInvocation {
    params: ProjectMapParams,
}

#[derive(Debug, Clone)]
struct ProjectMapBuildOptions {
    max_depth: usize,
    include_symbols: bool,
    max_files: usize,
    max_symbol_files: usize,
    symbol_scan_line_limit: usize,
}

impl Default for ProjectMapBuildOptions {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_PROJECT_MAP_DEPTH,
            include_symbols: false,
            max_files: DEFAULT_PROJECT_MAP_MAX_FILES,
            max_symbol_files: DEFAULT_PROJECT_MAP_MAX_SYMBOL_FILES,
            symbol_scan_line_limit: DEFAULT_PROJECT_MAP_SYMBOL_LINE_LIMIT,
        }
    }
}

impl ProjectMapBuildOptions {
    fn from_params(params: &ProjectMapParams) -> Self {
        let mut options = Self::default();
        if let Some(depth) = params.max_depth {
            options.max_depth = depth.clamp(1, 12);
        }
        if let Some(include_symbols) = params.include_symbols {
            options.include_symbols = include_symbols;
        }
        if let Some(max_files) = params.max_files {
            options.max_files = max_files.clamp(50, 5000);
        }
        options
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectMapCacheEntry {
    generated_at_unix: i64,
    output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProjectMapDiskCache {
    entries: HashMap<String, ProjectMapCacheEntry>,
}

fn map_cache() -> &'static Mutex<HashMap<String, ProjectMapCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ProjectMapCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn prewarmed_roots() -> &'static Mutex<HashSet<String>> {
    static PREWARMED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PREWARMED.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn spawn_project_map_prewarm(project_root: PathBuf) {
    if !project_map_prewarm_enabled() {
        return;
    }

    let canonical_root = canonicalize_or_self(&project_root);
    let root_key = canonical_root.to_string_lossy().to_string();

    let should_spawn = {
        let mut roots = prewarmed_roots()
            .lock()
            .expect("project map prewarm roots lock poisoned");
        roots.insert(root_key)
    };
    if !should_spawn {
        return;
    }

    std::thread::spawn(move || {
        let options = ProjectMapBuildOptions::default();
        let cache_key = build_cache_key(&canonical_root, &options);
        let ttl_secs = resolved_cache_ttl_secs();

        if load_from_cache(&cache_key, &canonical_root, ttl_secs).is_some() {
            return;
        }

        if let Ok(output) = generate_project_map(&canonical_root, &options) {
            store_cache_entry(&cache_key, &canonical_root, output);
        }
    });
}

fn project_map_prewarm_enabled() -> bool {
    project_map_prewarm_enabled_from_env(std::env::var("STAR_PROJECT_MAP_PREWARM").ok())
}

fn project_map_prewarm_enabled_from_env(raw: Option<String>) -> bool {
    raw.map(|value| {
        let normalized = value.trim().to_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "on" | "yes")
    })
    .unwrap_or(false)
}

pub async fn run_project_map_for_skill(
    root_path: PathBuf,
    max_depth: usize,
    max_files: usize,
    include_symbols: bool,
) -> String {
    let root_path = canonicalize_or_self(&root_path);
    let options = ProjectMapBuildOptions {
        max_depth: max_depth.clamp(1, 12),
        max_files: max_files.clamp(50, 5000),
        include_symbols,
        ..ProjectMapBuildOptions::default()
    };
    let cache_key = build_cache_key(&root_path, &options);
    let ttl_secs = resolved_cache_ttl_secs();

    if let Some(cached_output) = load_from_cache(&cache_key, &root_path, ttl_secs) {
        return cached_output;
    }

    let root_for_build = root_path.clone();
    let options_for_build = options.clone();
    let generated = match tokio::task::spawn_blocking(move || {
        generate_project_map(&root_for_build, &options_for_build)
    })
    .await
    {
        Ok(Ok(map)) => map,
        Ok(Err(err)) => return format!("Project map error: {}", err),
        Err(err) => return format!("Project map execution failed: {}", err),
    };

    store_cache_entry(&cache_key, &root_path, generated.clone());
    generated
}

impl ToolInvocation for ProjectMapInvocation {
    fn get_description(&self) -> String {
        format!(
            "Project Map: {}",
            self.params.path.as_deref().unwrap_or(".")
        )
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
        let path = self.params.path.clone();
        let options = ProjectMapBuildOptions::from_params(&self.params);
        let force_refresh = self.params.force_refresh.unwrap_or(false);

        Box::pin(async move {
            let root_path = if let Some(p) = path {
                canonicalize_or_self(Path::new(&p))
            } else {
                canonicalize_or_self(&std::env::current_dir()?)
            };
            if let Some(cb) = update_output.as_ref() {
                cb(format!("Preparing project map for {}", root_path.display()));
            }
            let cache_key = build_cache_key(&root_path, &options);
            let ttl_secs = resolved_cache_ttl_secs();

            if !force_refresh {
                if let Some(cached_output) = load_from_cache(&cache_key, &root_path, ttl_secs) {
                    if let Some(cb) = update_output.as_ref() {
                        cb("Loaded cached repository structure".to_string());
                    }
                    let truncated = truncate_map(&cached_output, 16000);
                    return Ok(CoreToolResult {
                        llm_content: Some(truncated.clone()),
                        return_display: Some(truncated.clone()),
                        output: truncated,
                        error: None,
                        data: None,
                    });
                }
            }

            let root_for_build = root_path.clone();
            let options_for_build = options.clone();
            if let Some(cb) = update_output.as_ref() {
                cb(format!(
                    "Scanning repository structure (depth {}, up to {} files)",
                    options.max_depth, options.max_files
                ));
            }
            let map = match tokio::task::spawn_blocking(move || {
                generate_project_map(&root_for_build, &options_for_build)
            })
            .await
            {
                Ok(res) => res.map_err(|e| e as Box<dyn std::error::Error>)?,
                Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
            };

            store_cache_entry(&cache_key, &root_path, map.clone());
            if let Some(cb) = update_output.as_ref() {
                cb("Project map ready".to_string());
            }

            // Truncate large outputs to keep context / rendering manageable.
            // Both llm_content and output MUST be truncated — the raw output goes to
            // the UI renderer and an untruncated 200KB+ map freezes the terminal on
            // every frame due to per-character unicode-width wrapping.
            const MAX_MAP_CHARS: usize = 16000;
            let truncated = truncate_map(&map, MAX_MAP_CHARS);

            Ok(CoreToolResult {
                llm_content: Some(truncated.clone()),
                return_display: Some(truncated.clone()),
                output: truncated,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for ProjectMapTool {
    fn name(&self) -> &str {
        "ProjectMap"
    }

    fn display_name(&self) -> &str {
        "Project Map"
    }

    fn description(&self) -> &str {
        "Generate a high-level map of the project structure, listing files and key symbols (classes, functions) to understand the codebase architecture."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The root directory to map (default: current directory)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum directory depth to traverse (default: 4)"
                },
                "include_symbols": {
                    "type": "boolean",
                    "description": "Whether to include symbol extraction (default: false for speed)"
                },
                "force_refresh": {
                    "type": "boolean",
                    "description": "Bypass cache and rebuild project map"
                },
                "max_files": {
                    "type": "integer",
                    "description": "Maximum number of files to scan (default: 400)"
                }
            }
        })
    }

    fn can_update_output(&self) -> bool {
        true
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ProjectMapParams = serde_json::from_value(params)?;
        Ok(Box::new(ProjectMapInvocation { params }))
    }
}

fn generate_project_map(
    root: &Path,
    options: &ProjectMapBuildOptions,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Which extensions have symbol extraction support (subset of all files).
    // File inclusion is NOT gated on this set — .gitignore handles exclusion.
    let symbol_exts: HashSet<&str> = LANGUAGE_SPECS
        .iter()
        .filter(|s| s.symbol_pattern.is_some())
        .map(|s| s.ext)
        .collect();
    let patterns = compiled_patterns();

    // 遍历口径统一走 `utils::file_walk`：dotfile 可见（`.github/workflows`
    // 是项目结构的一部分）、`~/.star/ignore` 也生效，不再只认 `.starignore`。
    let opts = crate::utils::file_walk::WalkOptions::new().max_depth(options.max_depth);
    let walker = crate::utils::file_walk::walk(root, &opts);

    let mut output = String::new();
    output.push_str(&format!("Project Map for {}\n\n", root.display()));

    let mut scanned_files = 0usize;
    let mut file_limit_reached = false;
    let mut language_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut top_level_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut top_level_samples: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut key_files = Vec::new();
    let mut symbol_sections: Vec<(String, Vec<String>)> = Vec::new();

    for result in walker {
        match result {
            Ok(entry) => {
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }

                let rel_path = match path.strip_prefix(root) {
                    Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                if rel_path.starts_with(".star/") || rel_path.starts_with(".git/") {
                    continue;
                }

                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let ext_lower = ext.to_lowercase();

                scanned_files += 1;
                if scanned_files > options.max_files {
                    file_limit_reached = true;
                    break;
                }

                let top_level = rel_path
                    .split('/')
                    .next()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ".".to_string());
                *top_level_counts.entry(top_level.clone()).or_insert(0) += 1;
                top_level_samples
                    .entry(top_level)
                    .or_default()
                    .push(rel_path.clone());

                let lang_key = language_bucket(ext_lower.as_str(), &file_name).to_string();
                *language_counts.entry(lang_key).or_insert(0) += 1;

                if KEY_FILE_NAMES.contains(&file_name.as_str()) && key_files.len() < 16 {
                    key_files.push(rel_path.clone());
                }

                if options.include_symbols
                    && symbol_exts.contains(ext_lower.as_str())
                    && symbol_sections.len() < options.max_symbol_files
                {
                    let symbols = extract_symbols(
                        path,
                        ext_lower.as_str(),
                        options.symbol_scan_line_limit,
                        patterns,
                    );
                    if !symbols.is_empty() {
                        symbol_sections.push((rel_path, symbols));
                    }
                }
            }
            Err(_) => continue,
        }
    }

    output.push_str("Summary\n");
    output.push_str(&format!(
        "- scanned_files: {} (limit: {})\n",
        scanned_files, options.max_files
    ));
    output.push_str(&format!("- max_depth: {}\n", options.max_depth));
    output.push_str(&format!(
        "- include_symbols: {}\n",
        if options.include_symbols {
            "true"
        } else {
            "false"
        }
    ));
    if file_limit_reached {
        output.push_str("- note: file scan limit reached; narrow path or increase `max_files` for deeper coverage.\n");
    }
    output.push('\n');

    if !language_counts.is_empty() {
        output.push_str("Languages / File Types (top)\n");
        let mut language_ranked = language_counts.iter().collect::<Vec<_>>();
        language_ranked.sort_by(|(lang_a, count_a), (lang_b, count_b)| {
            count_b.cmp(count_a).then_with(|| lang_a.cmp(lang_b))
        });
        for (lang, count) in language_ranked.into_iter().take(12) {
            output.push_str(&format!("- {}: {}\n", lang, count));
        }
        output.push('\n');
    }

    if !key_files.is_empty() {
        output.push_str("Key Files\n");
        for file in &key_files {
            output.push_str(&format!("- {}\n", file));
        }
        output.push('\n');
    }

    if !top_level_counts.is_empty() {
        output.push_str("Top-Level Layout\n");
        let mut top_level_ranked = top_level_counts.iter().collect::<Vec<_>>();
        top_level_ranked.sort_by(|(top_a, count_a), (top_b, count_b)| {
            count_b.cmp(count_a).then_with(|| top_a.cmp(top_b))
        });
        for (top, count) in top_level_ranked.into_iter().take(24) {
            output.push_str(&format!("- {}: {} files\n", top, count));
            if let Some(samples) = top_level_samples.get(top) {
                for sample in samples.iter().take(3) {
                    output.push_str(&format!("  - {}\n", sample));
                }
                if samples.len() > 3 {
                    output.push_str(&format!("  - ... ({} more)\n", samples.len() - 3));
                }
            }
        }
        output.push('\n');
    }

    if options.include_symbols {
        output.push_str("Symbols (sample)\n");
        if symbol_sections.is_empty() {
            output.push_str("- No symbols extracted in sampled files.\n");
        } else {
            for (file, symbols) in symbol_sections.iter().take(40) {
                output.push_str(&format!("- {}\n", file));
                for symbol in symbols.iter().take(8) {
                    output.push_str(&format!("  - {}\n", symbol));
                }
                if symbols.len() > 8 {
                    output.push_str(&format!("  - ... ({} more)\n", symbols.len() - 8));
                }
            }
        }
    }

    if scanned_files == 0 {
        output.push_str(
            "No source/config files found within current depth. Try increasing `max_depth`.\n",
        );
    }

    Ok(output)
}

fn extract_symbols(
    path: &Path,
    ext: &str,
    line_limit: usize,
    patterns: &HashMap<&str, Regex>,
) -> Vec<String> {
    let pattern = match patterns.get(ext) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut symbols = Vec::new();

    for line in reader.lines().take(line_limit).flatten() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            continue;
        }

        if !pattern.is_match(trimmed) {
            continue;
        }

        symbols.push(truncate_symbol(trimmed, 96));
        if symbols.len() >= 16 {
            break;
        }
    }

    symbols
}

fn truncate_symbol(line: &str, max_len: usize) -> String {
    if line.chars().count() <= max_len {
        return line.to_string();
    }
    let mut out = line.chars().take(max_len).collect::<String>();
    out.push_str("...");
    out
}

fn language_bucket(ext: &str, file_name: &str) -> String {
    // Named manifest files override the ext-based label.
    if let Some(&(_, bucket)) = KEY_FILE_BUCKETS.iter().find(|(name, _)| *name == file_name) {
        return bucket.to_string();
    }
    LANGUAGE_SPECS
        .iter()
        .find(|s| s.ext == ext)
        .map(|s| s.bucket.to_string())
        // Unknown extension: show the actual extension so the map is still useful.
        .unwrap_or_else(|| {
            if ext.is_empty() {
                "no-ext".to_string()
            } else {
                ext.to_string()
            }
        })
}

fn build_cache_key(root: &Path, options: &ProjectMapBuildOptions) -> String {
    format!(
        "{}|depth={}|symbols={}|max_files={}",
        root.to_string_lossy(),
        options.max_depth,
        options.include_symbols,
        options.max_files
    )
}

fn resolved_cache_ttl_secs() -> u64 {
    std::env::var("STAR_PROJECT_MAP_CACHE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PROJECT_MAP_CACHE_TTL_SECS)
        .clamp(30, 3600)
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn is_entry_fresh(entry: &ProjectMapCacheEntry, ttl_secs: u64) -> bool {
    let age = now_unix_seconds().saturating_sub(entry.generated_at_unix);
    age <= ttl_secs as i64
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Safely truncate a project map to `max_chars` while preserving the head and tail.
/// Uses char-based slicing to avoid UTF-8 byte-boundary panics.
fn truncate_map(map: &str, max_chars: usize) -> String {
    if map.len() <= max_chars {
        return map.to_string();
    }
    let head_len = max_chars * 3 / 4;
    let tail_len = max_chars - head_len;
    let head: String = map.chars().take(head_len).collect();
    let tail: String = map
        .chars()
        .rev()
        .take(tail_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!(
        "{}…\n(truncated {} → {} chars)\n{}",
        head,
        map.chars().count(),
        max_chars,
        tail
    )
}

fn cache_file_path(root: &Path) -> PathBuf {
    root.join(".star")
        .join("context")
        .join("project_map_cache.json")
}

fn load_disk_cache(path: &Path) -> ProjectMapDiskCache {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<ProjectMapDiskCache>(&content).unwrap_or_default(),
        Err(_) => ProjectMapDiskCache::default(),
    }
}

fn persist_disk_cache(path: &Path, cache: &ProjectMapDiskCache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(serialized) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, serialized);
    }
}

fn prune_cache_entries(entries: &mut HashMap<String, ProjectMapCacheEntry>) {
    if entries.len() <= MAX_PROJECT_MAP_CACHE_ENTRIES {
        return;
    }

    let mut ranked = entries
        .iter()
        .map(|(k, v)| (k.clone(), v.generated_at_unix))
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let keep = ranked
        .into_iter()
        .take(MAX_PROJECT_MAP_CACHE_ENTRIES)
        .map(|(key, _)| key)
        .collect::<HashSet<_>>();
    entries.retain(|k, _| keep.contains(k));
}

fn load_from_cache(cache_key: &str, root: &Path, ttl_secs: u64) -> Option<String> {
    {
        let mut cache = map_cache().lock().expect("project map cache lock poisoned");
        if let Some(entry) = cache.get(cache_key) {
            if is_entry_fresh(entry, ttl_secs) {
                return Some(entry.output.clone());
            }
        }
        cache.remove(cache_key);
    }

    if !root.is_dir() {
        return None;
    }
    let disk_path = cache_file_path(root);
    let mut disk_cache = load_disk_cache(&disk_path);
    if let Some(entry) = disk_cache.entries.get(cache_key).cloned() {
        if is_entry_fresh(&entry, ttl_secs) {
            let mut memory_cache = map_cache().lock().expect("project map cache lock poisoned");
            memory_cache.insert(cache_key.to_string(), entry.clone());
            prune_cache_entries(&mut memory_cache);
            return Some(entry.output);
        }
        disk_cache.entries.remove(cache_key);
        persist_disk_cache(&disk_path, &disk_cache);
    }

    None
}

fn store_cache_entry(cache_key: &str, root: &Path, output: String) {
    let entry = ProjectMapCacheEntry {
        generated_at_unix: now_unix_seconds(),
        output,
    };

    {
        let mut memory_cache = map_cache().lock().expect("project map cache lock poisoned");
        memory_cache.insert(cache_key.to_string(), entry.clone());
        prune_cache_entries(&mut memory_cache);
    }

    if !root.is_dir() {
        return;
    }
    let disk_path = cache_file_path(root);
    let mut disk_cache = load_disk_cache(&disk_path);
    disk_cache.entries.insert(cache_key.to_string(), entry);
    prune_cache_entries(&mut disk_cache.entries);
    persist_disk_cache(&disk_path, &disk_cache);
}
