use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::tools::{FileSearchResult, SearchResult, UnifiedSearchResult};
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};
use crate::types::ToolResult;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct SearchTool {
    current_directory: String,
    global_state: Arc<GlobalState>,
}

impl SearchTool {
    pub fn new(global_state: Arc<GlobalState>) -> Self {
        Self {
            current_directory: std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            global_state,
        }
    }

    fn base_dir(&self) -> String {
        let raw_base_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| self.current_directory.clone());
        normalize_existing_search_dir(&raw_base_dir)
    }

    pub async fn search(
        &self,
        query: &str,
        search_type: Option<String>,
        path: Option<String>,
        output_mode: Option<String>,
        include_pattern: Option<String>,
        exclude_pattern: Option<String>,
        case_sensitive: Option<bool>,
        whole_word: Option<bool>,
        regex: Option<bool>,
        max_results: Option<u32>,
        file_types: Option<Vec<String>>,
        exclude_files: Option<Vec<String>>,
        include_hidden: Option<bool>,
    ) -> Result<ToolResult, Box<dyn std::error::Error>> {
        let output_mode = output_mode.unwrap_or_else(|| "content".to_string());
        let search_type = search_type.unwrap_or_else(|| {
            if output_mode == "files_with_matches" {
                "text".to_string()
            } else {
                "both".to_string()
            }
        });
        let base_dir = self.resolve_search_base_dir(path.as_deref());
        // If the caller supplied a file path instead of a directory, scope the search to
        // that file's parent directory and constrain include_pattern to the filename.
        let (base_dir, include_pattern) = {
            let p = Path::new(&base_dir);
            if p.is_file() {
                let parent = p
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.base_dir());
                let filename = p
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                (parent, Some(filename))
            } else {
                (base_dir, include_pattern)
            }
        };
        if !Path::new(&base_dir).is_dir() {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Search path is not a directory: {}", base_dir)),
                data: None,
            });
        }
        let mut results = Vec::new();
        let include_hidden_default = query.trim_start().starts_with('@');
        let include_hidden_flag = include_hidden.unwrap_or_else(|| {
            std::env::var("STAR_SEARCH_INCLUDE_HIDDEN")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(include_hidden_default)
        });
        let should_search_files =
            search_type == "files" || search_type == "both" || query.starts_with('@');

        if search_type == "text" || search_type == "both" {
            let file_types_ref = file_types.as_ref();
            let exclude_files_ref = exclude_files.as_ref();
            if let Ok(text_results) = self
                .execute_ripgrep(
                    query,
                    &base_dir,
                    include_pattern.as_deref(),
                    exclude_pattern.as_deref(),
                    case_sensitive,
                    whole_word,
                    regex,
                    max_results,
                    file_types_ref.map(|v| v),
                    exclude_files_ref.map(|v| v),
                    include_hidden_flag,
                )
                .await
            {
                let text_results_formatted: Vec<UnifiedSearchResult> = text_results
                    .into_iter()
                    .map(|r| UnifiedSearchResult {
                        result_type: "text".to_string(),
                        file: r.file,
                        line: r.line,
                        column: r.column,
                        text: r.text,
                        match_content: r.match_content,
                        score: None,
                    })
                    .collect();
                results.extend(text_results_formatted);
            }
        }

        if should_search_files {
            let file_query = {
                let trimmed = query.trim_start_matches('@');
                if trimmed.is_empty() {
                    ""
                } else {
                    trimmed
                }
            };

            let file_max_depth = std::env::var("STAR_SEARCH_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(256usize);

            if let Ok(file_results) = self
                .find_files_by_pattern(
                    file_query,
                    max_results,
                    Some(include_hidden_flag),
                    exclude_pattern.as_deref(),
                    &base_dir,
                    file_max_depth,
                )
                .await
            {
                let file_results_formatted: Vec<UnifiedSearchResult> = file_results
                    .into_iter()
                    .map(|r| UnifiedSearchResult {
                        result_type: "file".to_string(),
                        file: r.path,
                        line: None,
                        column: None,
                        text: None,
                        match_content: None,
                        score: Some(r.score),
                    })
                    .collect();
                results.extend(file_results_formatted);
            }
        }

        if results.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: Some(format!("No results found for \"{}\"", query)),
                error: None,
                data: None,
            });
        }

        let formatted_output = if output_mode == "files_with_matches" {
            self.format_files_with_matches(&results, query)
        } else {
            self.format_unified_results(&results, query, &search_type)
        };

        Ok(ToolResult {
            success: true,
            output: Some(formatted_output),
            error: None,
            data: Some(serde_json::to_value(&results)?),
        })
    }

    async fn execute_ripgrep(
        &self,
        query: &str,
        base_dir: &str,
        include_pattern: Option<&str>,
        exclude_pattern: Option<&str>,
        case_sensitive: Option<bool>,
        whole_word: Option<bool>,
        regex: Option<bool>,
        max_results: Option<u32>,
        file_types: Option<&Vec<String>>,
        exclude_files: Option<&Vec<String>>,
        include_hidden: bool,
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        let mut config = crate::core::tools::ripgrep::RipgrepConfig {
            case_sensitive: case_sensitive.unwrap_or(false),
            whole_word: whole_word.unwrap_or(false),
            regex: regex.unwrap_or(false),
            max_results,
            file_types: file_types.cloned(),
            exclude_patterns: None,
            include_patterns: include_pattern.map(|p| vec![p.to_string()]),
            include_hidden,
            max_file_size: Some(2 * 1024 * 1024),
        };

        let mut excludes = Vec::new();
        if let Some(pattern) = exclude_pattern {
            excludes.push(pattern.to_string());
        }
        if let Some(files) = exclude_files {
            excludes.extend(files.iter().cloned());
        }
        if !excludes.is_empty() {
            config.exclude_patterns = Some(excludes);
        }

        let results = crate::core::tools::ripgrep::search_with_ripgrep(query, &base_dir, config)?;

        Ok(results)
    }

    fn resolve_search_base_dir(&self, path: Option<&str>) -> String {
        let Some(raw_path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
            return self.base_dir();
        };

        let candidate = normalize_input_path(raw_path);
        if candidate.is_absolute() {
            return candidate.to_string_lossy().to_string();
        }

        Path::new(&self.base_dir())
            .join(candidate)
            .to_string_lossy()
            .to_string()
    }

    fn should_skip_walk_entry(path: &std::path::Path, include_hidden: bool) -> bool {
        let ignored = [
            "node_modules",
            ".git",
            ".svn",
            ".hg",
            "dist",
            "build",
            ".next",
            ".cache",
            "target",
            ".idea",
            ".vscode",
        ];

        for component in path.components() {
            let s = component.as_os_str().to_string_lossy();
            if !include_hidden && s.starts_with('.') {
                return true;
            }
            if ignored.contains(&s.as_ref()) {
                return true;
            }
        }

        false
    }

    async fn find_files_by_pattern(
        &self,
        pattern: &str,
        max_results: Option<u32>,
        include_hidden: Option<bool>,
        exclude_pattern: Option<&str>,
        base_dir: &str,
        max_depth: usize,
    ) -> Result<Vec<FileSearchResult>, Box<dyn std::error::Error>> {
        let max_results = max_results.unwrap_or(300) as usize;
        let mut files = Vec::new();
        let search_pattern = pattern.to_lowercase();

        let include_hidden = include_hidden.unwrap_or(false);
        let walker = WalkDir::new(base_dir)
            .follow_links(false)
            .max_depth(max_depth)
            .into_iter()
            .filter_entry(|e| !Self::should_skip_walk_entry(e.path(), include_hidden))
            .filter_map(|e| e.ok());

        for entry in walker {
            let file_path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if !include_hidden && file_name.starts_with('.') {
                continue;
            }

            let relative_path = file_path
                .strip_prefix(Path::new(base_dir))
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let relative_path = if relative_path.is_empty() {
                file_path.to_string_lossy().to_string()
            } else {
                relative_path
            };

            if let Some(exclude_pat) = exclude_pattern {
                if relative_path.contains(exclude_pat) {
                    continue;
                }
            }

            if file_path.is_file() || file_path.is_dir() {
                let _score = calculate_file_score(&file_name, &relative_path, &search_pattern);
                files.push((relative_path, file_name));
            }

            if files.len() >= max_results {
                break;
            }
        }

        let mut file_results: Vec<FileSearchResult> = files
            .into_iter()
            .map(|(path, name)| {
                let score = calculate_file_score(&name, &path, &pattern.to_lowercase());
                FileSearchResult { path, name, score }
            })
            .collect();

        file_results.sort_by(|a, b| b.score.cmp(&a.score));
        file_results.truncate(max_results);

        Ok(file_results)
    }

    fn format_unified_results(
        &self,
        results: &[UnifiedSearchResult],
        query: &str,
        _search_type: &str,
    ) -> String {
        if results.is_empty() {
            return format!("No results found for \"{}\"", query);
        }

        let mut output = format!("Search results for \"{}\":\n", query);

        let text_results: Vec<&UnifiedSearchResult> =
            results.iter().filter(|r| r.result_type == "text").collect();
        let file_results: Vec<&UnifiedSearchResult> =
            results.iter().filter(|r| r.result_type == "file").collect();

        if !file_results.is_empty() {
            output.push_str("\n📂 Matched Files:\n");
            for result in file_results.iter().take(20) {
                output.push_str(&format!("- {}\n", result.file));
            }
            if file_results.len() > 20 {
                output.push_str(&format!(
                    "  ... and {} more files\n",
                    file_results.len() - 20
                ));
            }
        }

        if !text_results.is_empty() {
            output.push_str("\n📝 Text Matches:\n");

            use std::collections::HashMap;
            let mut file_matches: HashMap<String, Vec<&UnifiedSearchResult>> = HashMap::new();
            for result in &text_results {
                file_matches
                    .entry(result.file.clone())
                    .or_default()
                    .push(result);
            }

            let mut sorted_files: Vec<String> = file_matches.keys().cloned().collect();
            sorted_files.sort();

            for file in sorted_files {
                let matches = &file_matches[&file];
                output.push_str(&format!("{}\n", file));

                let mut sorted_matches = matches.clone();
                sorted_matches.sort_by_key(|m| m.line.unwrap_or(0));

                for m in sorted_matches.iter().take(10) {
                    if let Some(line_num) = m.line {
                        let content = m.match_content.as_deref().unwrap_or("").trim();
                        output.push_str(&format!("- {}: {}\n", line_num, content));
                    }
                }
                if matches.len() > 10 {
                    output.push_str(&format!("  ... +{} more matches\n", matches.len() - 10));
                }
                output.push('\n');
            }
        }

        output.trim_end().to_string()
    }

    fn format_files_with_matches(&self, results: &[UnifiedSearchResult], query: &str) -> String {
        let mut files: Vec<String> = results
            .iter()
            .filter(|r| r.result_type == "text")
            .map(|r| r.file.clone())
            .collect();
        files.sort();
        files.dedup();

        if files.is_empty() {
            return format!("No files with matches found for \"{}\"", query);
        }

        let mut output = format!("Files with matches for \"{}\":\n", query);
        for file in files {
            output.push_str(&format!("{}\n", file));
        }
        output.trim_end().to_string()
    }
}

fn normalize_input_path(raw_path: &str) -> PathBuf {
    crate::core::utils::paths::normalize_cross_platform_path(raw_path)
}

fn normalize_existing_search_dir(raw_path: &str) -> String {
    let normalized = normalize_input_path(raw_path);
    if normalized.is_dir() {
        return normalized.to_string_lossy().to_string();
    }

    raw_path.to_string()
}

fn calculate_file_score(file_name: &str, file_path: &str, pattern: &str) -> u32 {
    let lower_file_name = file_name.to_lowercase();
    let lower_file_path = file_path.to_lowercase();
    let lower_pattern = pattern.to_lowercase();

    if lower_file_name == lower_pattern {
        return 100;
    }
    if lower_file_name.contains(&lower_pattern) {
        return 80;
    }

    if lower_file_path.contains(&lower_pattern) {
        return 60;
    }

    let mut pattern_index = 0;
    for i in 0..lower_file_name.len() {
        if pattern_index >= lower_pattern.len() {
            break;
        }
        if lower_file_name.chars().nth(i) == lower_pattern.chars().nth(pattern_index) {
            pattern_index += 1;
        }
    }

    if pattern_index == lower_pattern.len() {
        let diff = (file_name.len() as i32 - pattern.len() as i32).max(0) as u32;
        return std::cmp::max(10, 40u32.saturating_sub(diff));
    }

    1
}

impl BaseDeclarativeTool for SearchTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn display_name(&self) -> &str {
        "Search Tool"
    }

    fn description(&self) -> &str {
        "Search for files or text content in files"
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query (text or file pattern). Use either query or pattern." },
                "pattern": { "type": "string", "description": "Alias for query, accepted for grep-compatible calls." },
                "path": { "type": "string", "description": "Directory to search. Supports relative paths, absolute paths, and Windows drive paths such as G:\\\\project when running under WSL." },
                "dir_path": { "type": "string", "description": "Alias for path, accepted for compatibility with older grep tools." },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches"], "description": "Output mode. 'content' returns matching lines; 'files_with_matches' returns unique file paths only." },
                "search_type": { "type": "string", "enum": ["text", "files", "both"], "description": "Type of search: 'text' for content, 'files' for filenames, 'both' for combined" },
                "include_pattern": { "type": "string", "description": "Glob pattern to include files" },
                "exclude_pattern": { "type": "string", "description": "Glob pattern to exclude files" },
                "case_sensitive": { "type": "boolean", "description": "Case sensitive search" },
                "whole_word": { "type": "boolean", "description": "Match whole words only" },
                "regex": { "type": "boolean", "description": "Treat query as regex" },
                "max_results": { "type": "integer", "description": "Maximum number of results" },
                "file_types": { "type": "array", "items": { "type": "string" }, "description": "Filter by file types (e.g., ['rs', 'toml'])" },
                "exclude_files": { "type": "array", "items": { "type": "string" }, "description": "List of files to exclude" },
                "include_hidden": { "type": "boolean", "description": "Include hidden files" }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let query = params
            .get("query")
            .or_else(|| params.get("pattern"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| Box::<dyn std::error::Error + Send + Sync>::from("Missing query"))?
            .to_string();
        Ok(Box::new(SearchToolInvocation {
            tool: self.clone(),
            query,
            params,
            global_state: self.global_state.clone(),
        }))
    }
}

struct SearchToolInvocation {
    tool: SearchTool,
    query: String,
    params: serde_json::Value,
    global_state: Arc<GlobalState>,
}

impl ToolInvocation for SearchToolInvocation {
    fn get_description(&self) -> String {
        format!("Search: {}", self.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: std::path::PathBuf::from("."),
            location_type: crate::core::tools::tools::LocationType::Read,
        }]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let tool = self.tool.clone();
        let query = self.query.clone();
        let params = self.params.clone();
        let global_state = self.global_state.clone();

        Box::pin(async move {
            let search_type = params["search_type"].as_str().map(|s| s.to_string());
            let path = params
                .get("path")
                .or_else(|| params.get("dir_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let output_mode = params["output_mode"].as_str().map(|s| s.to_string());
            let include_pattern = params["include_pattern"].as_str().map(|s| s.to_string());
            let exclude_pattern = params["exclude_pattern"].as_str().map(|s| s.to_string());
            let case_sensitive = params["case_sensitive"].as_bool();
            let whole_word = params["whole_word"].as_bool();
            let regex = params["regex"].as_bool();
            let max_results = params["max_results"].as_u64().map(|n| n as u32);
            let file_types = params["file_types"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
            let exclude_files = params["exclude_files"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
            let include_hidden = params["include_hidden"].as_bool();

            let result = tool
                .search(
                    &query,
                    search_type,
                    path,
                    output_mode,
                    include_pattern,
                    exclude_pattern,
                    case_sensitive,
                    whole_word,
                    regex,
                    max_results,
                    file_types,
                    exclude_files,
                    include_hidden,
                )
                .await?;

            // ── 更新 read_file_state：从搜索结果中提取文件路径 ──
            if let Some(ref data) = result.data {
                if let Ok(results) =
                    serde_json::from_value::<Vec<UnifiedSearchResult>>(data.clone())
                {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let mut state = global_state.read_file_state.write().await;
                    for r in &results {
                        state.entry(r.file.clone()).or_insert(ReadFileState {
                            content: String::new(),
                            timestamp: now,
                            file_system_timestamp: now,
                        });
                    }
                }
            }

            Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: result.output.unwrap_or_default(),
                error: result.error.map(|e| ToolError {
                    error_type: "execution_error".to_string(),
                    message: e,
                }),
                data: None,
            })
        })
    }
}
