use super::{SubAgent, SubTask, SubTaskResult};
use crate::core::prompts::skills::navigator::NAVIGATOR_SYSTEM_PROMPT;
use crate::agent::StarAgent;
use crate::core::config::Config;
use crate::core::tools::semantic_search::search_codebase;
use crate::core::utils::paths::resolve_tool_path;
use crate::llm::client::StarClient;
use async_trait::async_trait;
use serde_json::json;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub struct NavigatorAgent {
    id: String,
    client: StarClient,
    config: Arc<Config>,
}

impl NavigatorAgent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self {
            id: "navigator".to_string(),
            client,
            config,
        }
    }

    async fn run_navigation_loop(
        &self,
        task: &SubTask,
    ) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        let query = if task.objective.trim().is_empty() {
            task.target.trim().to_string()
        } else {
            task.objective.trim().to_string()
        };
        let target = task.target.trim();
        let requested_root = if target.is_empty() || target == "." {
            self.config.target_dir().clone()
        } else {
            resolve_tool_path(self.config.target_dir(), target)
        };
        let (root_path, root_note) = if requested_root.exists() && requested_root.is_dir() {
            (requested_root, None)
        } else {
            (
                self.config.target_dir().clone(),
                Some(format!(
                    "Requested navigation path '{}' is invalid; fallback to workspace root '{}'.",
                    requested_root.display(),
                    self.config.target_dir().display()
                )),
            )
        };
        let search_root_display = root_path.display().to_string();

        let max_depth = parse_usize_param(&task.params, "max_depth", 3).clamp(1, 6);
        let max_files = parse_usize_param(&task.params, "max_files", 24).clamp(4, 80);
        let max_refs_per_file =
            parse_usize_param(&task.params, "max_refs_per_file", 12).clamp(4, 30);

        crate::utils::logging::append_debug_log_line(&format!(
            "NavigatorAgent: query='{}', root='{}', max_depth={}, max_files={}",
            query, search_root_display, max_depth, max_files
        ));

        let root_for_search = root_path.clone();
        let query_for_search = query.clone();
        let mut semantic_results = match tokio::task::spawn_blocking(move || {
            search_codebase(&root_for_search, &query_for_search, None)
        })
        .await
        {
            Ok(res) => res.unwrap_or_else(|e| format!("Semantic search error: {}", e)),
            Err(e) => format!("Semantic search execution failed: {}", e),
        };
        if let Some(note) = root_note {
            semantic_results = format!("{}\n\n{}", note, semantic_results);
        }

        let seed_files = extract_seed_files_from_semantic_output(&semantic_results);
        let recursion_context = build_recursive_context(
            &root_path,
            &seed_files,
            max_depth,
            max_files,
            max_refs_per_file,
        );

        let fast_path_details = format!(
            "Search Root: {}\nMode: deterministic fast path\n\n## LAYER 0 - ACE SEMANTIC SEEDS\n{}\n\n## LAYERED CONTEXT MAP (AUTO-EXPANDED)\n{}",
            search_root_display, semantic_results, recursion_context
        );

        let deep_filter_enabled = navigator_deep_filter_enabled(task);
        if !deep_filter_enabled {
            return Ok(SubTaskResult::success(
                task.id.clone(),
                "Navigation Complete (fast path)".to_string(),
            )
            .with_details(fast_path_details)
            .with_data(json!({
                "mode": "fast_path",
                "seed_count": seed_files.len(),
                "max_depth": max_depth,
                "max_files": max_files,
                "max_refs_per_file": max_refs_per_file,
                "search_root": search_root_display,
                "deep_filter": false,
            })));
        }

        let mut agent = StarAgent::new(
            &self.client.api_key,
            Some(self.client.model.clone()),
            self.client.base_url.clone(),
            Some(12),
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        let prompt = format!(
            "{}\n\n\
            ## CURRENT TASK\n\
            Objective: {}\n\
            Search Root: {}\n\
            Params: {:?}\n\n\
            ## LAYER 0 - ACE SEMANTIC SEEDS\n\
            {}\n\n\
            ## LAYERED CONTEXT MAP (AUTO-EXPANDED)\n\
            {}\n\n\
            ## EXECUTION REQUIREMENT\n\
            Use tools to close missing context. If one file references another key file, jump to it and continue recursively.\n\
            Prefer `Grep` for quick location, then `Read` for exact verification.\n\
            ",
            NAVIGATOR_SYSTEM_PROMPT,
            query,
            search_root_display,
            task.params,
            semantic_results,
            recursion_context,
        );

        let entries = match tokio::time::timeout(
            navigator_deep_filter_timeout(),
            agent.process_user_message(&prompt),
        )
        .await
        {
            Ok(Ok(entries)) => entries,
            Ok(Err(err)) => {
                let mut details = fast_path_details.clone();
                details.push_str(&format!("\n\n[Navigator error: {}]", err));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Navigation Complete (fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_fallback",
                    "seed_count": seed_files.len(),
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "max_refs_per_file": max_refs_per_file,
                    "search_root": search_root_display,
                    "deep_filter": true,
                    "fallback_reason": format!("{}", err),
                })));
            }
            Err(_) => {
                let mut details = fast_path_details.clone();
                details.push_str(&format!(
                    "\n\n[Navigator timed out after {}ms; returning deterministic context map.]",
                    navigator_deep_filter_timeout().as_millis()
                ));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Navigation Complete (timeout fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_timeout_fallback",
                    "seed_count": seed_files.len(),
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "max_refs_per_file": max_refs_per_file,
                    "search_root": search_root_display,
                    "deep_filter": true,
                    "timeout_ms": navigator_deep_filter_timeout().as_millis(),
                })));
            }
        };

        let response = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == crate::types::ChatEntryType::Assistant)
            .map(|e| e.content.clone())
            .unwrap_or_else(|| "No response".to_string());

        let final_details = if response.trim().is_empty() || response == "No response" {
            fast_path_details
        } else {
            response
        };

        Ok(
            SubTaskResult::success(task.id.clone(), "Navigation Complete".to_string())
                .with_details(final_details)
                .with_data(json!({
                    "mode": "deep_filter",
                    "seed_count": seed_files.len(),
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "max_refs_per_file": max_refs_per_file,
                    "search_root": search_root_display,
                    "deep_filter": true,
                })),
        )
    }
}

#[async_trait]
impl SubAgent for NavigatorAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Navigator Agent (Recursive Context Navigation)"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "navigate".to_string(),
            "trace".to_string(),
            "dependency".to_string(),
            "call chain".to_string(),
            "recursive".to_string(),
            "reference".to_string(),
        ]
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        self.run_navigation_loop(&task).await
    }
}

fn parse_usize_param(
    params: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    default: usize,
) -> usize {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
}

fn parse_bool_like(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(v) => Some(*v),
        serde_json::Value::String(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn task_bool_param(task: &SubTask, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| task.params.get(*key).and_then(parse_bool_like))
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn navigator_deep_filter_enabled(task: &SubTask) -> bool {
    task_bool_param(task, &["deep_filter", "deep_navigation", "agentic"])
        .unwrap_or_else(|| env_bool("STAR_NAVIGATOR_ENABLE_DEEP_FILTER", false))
}

fn navigator_deep_filter_timeout() -> Duration {
    let timeout_ms = std::env::var("STAR_NAVIGATOR_DEEP_FILTER_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(18_000);
    Duration::from_millis(timeout_ms.max(1_000))
}

fn extract_seed_files_from_semantic_output(output: &str) -> Vec<PathBuf> {
    // Match file paths from semantic search output in multiple formats:
    // "File: /path/to/file (Score: 0.95)"
    // "/path/to/file (Score: 0.95)"
    // Just "/path/to/file"
    use regex::Regex;
    let re = Regex::new(
        r"(?m)^[\s•*-]*\s*(?:File:\s*)?(?P<path>(?:/[^\s()]+)+)(?:\s*\(Score:\s*[0-9.]+\s*\))?"
    ).unwrap_or_else(|_| Regex::new(r"(?m)^\s*(?:File:\s*)?(?P<path>/[^\s]+)").unwrap());

    re.captures_iter(output)
        .filter_map(|cap| {
            let path = cap.name("path")?.as_str().trim();
            if path.is_empty() { None } else { Some(PathBuf::from(path)) }
        })
        .collect()
}

fn build_recursive_context(
    root_path: &Path,
    seed_files: &[PathBuf],
    max_depth: usize,
    max_files: usize,
    max_refs_per_file: usize,
) -> String {
    let mut queue: VecDeque<(PathBuf, usize, String)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut records: Vec<(usize, String, String)> = Vec::new();
    let mut truncated = false;

    for seed in seed_files {
        if let Some(path) = resolve_seed_path(root_path, seed) {
            queue.push_back((path, 0, "semantic_seed".to_string()));
        }
    }

    if queue.is_empty() {
        for fallback in [
            "src/main.rs",
            "src/lib.rs",
            "main.rs",
            "main.py",
            "index.ts",
            "index.js",
        ] {
            let path = root_path.join(fallback);
            if path.is_file() {
                queue.push_back((path, 0, "fallback_entrypoint".to_string()));
            }
        }
    }

    while let Some((path, depth, via)) = queue.pop_front() {
        if records.len() >= max_files {
            truncated = true;
            break;
        }

        if !path.is_file() || !is_under_root(root_path, &path) {
            continue;
        }

        let normalized = normalize_path(&path);
        if !visited.insert(normalized) {
            continue;
        }

        let display = display_path(root_path, &path);
        records.push((depth, display.clone(), via));

        if depth >= max_depth {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let refs = discover_reference_paths(root_path, &path, &content, max_refs_per_file);
        for (next_path, reason) in refs {
            if !next_path.is_file() || !is_under_root(root_path, &next_path) {
                continue;
            }
            let next_normalized = normalize_path(&next_path);
            if visited.contains(&next_normalized) {
                continue;
            }
            queue.push_back((next_path, depth + 1, format!("{} => {}", display, reason)));
        }
    }

    if records.is_empty() {
        return "No recursive context files discovered. Use Grep/Read to build context from scratch.".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Recursive context expansion (depth <= {}, files <= {}):\n",
        max_depth, max_files
    ));
    for (depth, path, via) in records {
        out.push_str(&format!("- [D{}] {}  (via: {})\n", depth, path, via));
    }
    if truncated {
        out.push_str("- [TRUNCATED] Expansion reached file budget. Continue on-demand with Grep/Read.\n");
    }
    out
}
 
fn discover_reference_paths(
    root_path: &Path,
    current_file: &Path,
    content: &str,
    max_refs_per_file: usize,
) -> Vec<(PathBuf, String)> {
    let base_dir = current_file.parent().unwrap_or(root_path);
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in content.lines().take(2_000) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(module_name) = parse_rust_mod(trimmed) {
            for candidate in resolve_rust_mod_paths(base_dir, &module_name) {
                let key = normalize_path(&candidate);
                if seen.insert(key) {
                    out.push((candidate, format!("mod {}", module_name)));
                    if out.len() >= max_refs_per_file {
                        return out;
                    }
                }
            }
        }

        if let Some(use_path) = parse_rust_use_path(trimmed) {
            for candidate in resolve_rust_use_paths(root_path, current_file, &use_path) {
                let key = normalize_path(&candidate);
                if seen.insert(key) {
                    out.push((candidate, format!("use {}", use_path)));
                    if out.len() >= max_refs_per_file {
                        return out;
                    }
                }
            }
        }

        if let Some(module_path) = parse_python_from(trimmed) {
            for candidate in resolve_python_module_paths(root_path, &module_path) {
                let key = normalize_path(&candidate);
                if seen.insert(key) {
                    out.push((candidate, format!("from {}", module_path)));
                    if out.len() >= max_refs_per_file {
                        return out;
                    }
                }
            }
        }

        for module_path in parse_python_imports(trimmed) {
            for candidate in resolve_python_module_paths(root_path, &module_path) {
                let key = normalize_path(&candidate);
                if seen.insert(key) {
                    out.push((candidate, format!("import {}", module_path)));
                    if out.len() >= max_refs_per_file {
                        return out;
                    }
                }
            }
        }

        if looks_like_reference_line(trimmed) {
            for token in extract_quoted_literals(trimmed) {
                for candidate in resolve_token_to_paths(root_path, base_dir, current_file, &token) {
                    let key = normalize_path(&candidate);
                    if seen.insert(key) {
                        out.push((candidate, format!("quote {}", token)));
                        if out.len() >= max_refs_per_file {
                            return out;
                        }
                    }
                }
            }
        }
    }

    out
}

fn resolve_seed_path(root_path: &Path, seed: &Path) -> Option<PathBuf> {
    if seed.is_absolute() {
        if seed.is_file() {
            Some(seed.to_path_buf())
        } else {
            None
        }
    } else {
        let joined = root_path.join(seed);
        if joined.is_file() {
            Some(joined)
        } else {
            None
        }
    }
}

fn parse_rust_mod(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = if let Some(rest) = trimmed.strip_prefix("pub mod ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("mod ") {
        rest
    } else {
        return None;
    };

    let module = rest
        .split(';')
        .next()
        .unwrap_or(rest)
        .trim()
        .trim_end_matches('{')
        .trim();
    if module.is_empty() {
        None
    } else {
        Some(module.to_string())
    }
}

fn parse_rust_use_path(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = if let Some(rest) = trimmed.strip_prefix("pub use ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("use ") {
        rest
    } else {
        return None;
    };

    let mut path = rest.split(';').next().unwrap_or(rest).trim();
    path = path.split(" as ").next().unwrap_or(path).trim();
    path = path.split('{').next().unwrap_or(path).trim();
    path = path.trim_end_matches("::").trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn parse_python_from(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("from ")?;
    let module = rest.split_whitespace().next()?.trim();
    if module.is_empty() || module == "." {
        None
    } else {
        Some(module.to_string())
    }
}

fn parse_python_imports(line: &str) -> Vec<String> {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("import ") else {
        return Vec::new();
    };

    rest.split(',')
        .filter_map(|part| {
            let module = part.split_whitespace().next()?.trim();
            if module.is_empty() {
                None
            } else {
                Some(module.to_string())
            }
        })
        .collect()
}

fn resolve_rust_mod_paths(base_dir: &Path, module_name: &str) -> Vec<PathBuf> {
    expand_path_candidates(base_dir.join(module_name))
}

fn resolve_rust_use_paths(root_path: &Path, current_file: &Path, use_path: &str) -> Vec<PathBuf> {
    let base_dir = current_file.parent().unwrap_or(root_path);
    let mut roots = Vec::new();
    let mut module_path = use_path.trim();

    if let Some(rest) = module_path.strip_prefix("crate::") {
        module_path = rest;
        roots.push(root_path.join("src"));
    } else if let Some(rest) = module_path.strip_prefix("super::") {
        module_path = rest;
        if let Some(parent) = base_dir.parent() {
            roots.push(parent.to_path_buf());
        }
        roots.push(root_path.join("src"));
    } else if let Some(rest) = module_path.strip_prefix("self::") {
        module_path = rest;
        roots.push(base_dir.to_path_buf());
    } else {
        roots.push(base_dir.to_path_buf());
        roots.push(root_path.join("src"));
    }

    let module_rel = module_path.replace("::", "/");
    let mut out = Vec::new();
    for base in roots {
        out.extend(expand_path_candidates(base.join(&module_rel)));
    }
    out
}

fn resolve_python_module_paths(root_path: &Path, module: &str) -> Vec<PathBuf> {
    let module_rel = module.replace('.', "/");
    let mut out = Vec::new();
    out.extend(expand_path_candidates(root_path.join(&module_rel)));
    out.extend(expand_path_candidates(
        root_path.join("src").join(&module_rel),
    ));
    out
}

fn looks_like_reference_line(line: &str) -> bool {
    line.contains(" from ")
        || line.starts_with("import ")
        || line.starts_with("use ")
        || line.starts_with("pub use ")
        || line.starts_with("mod ")
        || line.starts_with("pub mod ")
        || line.contains("require(")
        || line.contains("include!(")
        || line.contains("include_str!(")
        || line.contains("#[path")
}

fn extract_quoted_literals(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current_quote: Option<char> = None;
    let mut buf = String::new();
    let mut escaped = false;

    for ch in line.chars() {
        match current_quote {
            Some(quote) => {
                if escaped {
                    buf.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == quote {
                    if !buf.is_empty() {
                        out.push(buf.clone());
                    }
                    buf.clear();
                    current_quote = None;
                } else {
                    buf.push(ch);
                }
            }
            None => {
                if ch == '\'' || ch == '"' {
                    current_quote = Some(ch);
                }
            }
        }
    }

    out
}

fn resolve_token_to_paths(
    root_path: &Path,
    base_dir: &Path,
    current_file: &Path,
    token: &str,
) -> Vec<PathBuf> {
    let t = token.trim().trim_matches('`');
    if t.is_empty() || t.starts_with("http://") || t.starts_with("https://") || t.starts_with("@") {
        return Vec::new();
    }

    if t.contains("::")
        || t.starts_with("crate::")
        || t.starts_with("super::")
        || t.starts_with("self::")
    {
        return resolve_rust_use_paths(root_path, current_file, t);
    }

    let mut raw = Vec::new();
    if t.starts_with("./") || t.starts_with("../") {
        raw.push(base_dir.join(t));
    } else if t.starts_with('/') {
        raw.push(root_path.join(t.trim_start_matches('/')));
    } else if t.contains('/') {
        raw.push(base_dir.join(t));
        raw.push(root_path.join(t));
    } else if t.contains('.') {
        raw.push(root_path.join(t.replace('.', "/")));
        raw.push(root_path.join("src").join(t.replace('.', "/")));
    }

    let mut out = Vec::new();
    for p in raw {
        out.extend(expand_path_candidates(p));
    }
    out
}

fn expand_path_candidates(base: PathBuf) -> Vec<PathBuf> {
    const EXTENSIONS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "json", "toml", "yaml",
        "yml", "md",
    ];

    let mut out = Vec::new();
    if base.is_file() {
        out.push(base);
        return out;
    }
    if base.is_dir() {
        for entry in [
            "mod.rs",
            "index.ts",
            "index.tsx",
            "index.js",
            "index.jsx",
            "__init__.py",
        ] {
            let p = base.join(entry);
            if p.is_file() {
                out.push(p);
            }
        }
        return out;
    }

    if base.extension().is_none() {
        for ext in EXTENSIONS {
            let p = base.with_extension(ext);
            if p.is_file() {
                out.push(p);
            }
        }
        for entry in [
            "mod.rs",
            "index.ts",
            "index.tsx",
            "index.js",
            "index.jsx",
            "__init__.py",
        ] {
            let p = base.join(entry);
            if p.is_file() {
                out.push(p);
            }
        }
    }

    out
}

fn is_under_root(root_path: &Path, path: &Path) -> bool {
    let root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    candidate.starts_with(root)
}

fn display_path(root_path: &Path, path: &Path) -> String {
    path.strip_prefix(root_path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

fn normalize_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}
