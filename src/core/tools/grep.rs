use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::ripgrep::{
    format_grep_hits, search_with_ripgrep, try_repair_pattern, GrepHit, RipgrepConfig,
    DEFAULT_MAX_RESULTS,
};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{
    make_relative, normalize_cross_platform_path, resolve_tool_path, shorten_path,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepToolParams {
    pub pattern: String,
    #[serde(rename = "dir_path")]
    pub dir_path: Option<String>,
    pub include: Option<String>,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub regex: bool,
    /// Exclude patterns for filtering results (e.g., ["*_test.*", "*.test.*"])
    #[serde(default)]
    pub exclude_patterns: Option<Vec<String>>,
    /// Exclude comment lines from results
    #[serde(default)]
    pub exclude_comments: bool,
    /// Include context lines around matches
    #[serde(default)]
    pub context_lines: Option<usize>,
}

pub struct GrepToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: GrepToolParams,
}

impl GrepToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: GrepToolParams,
        _message_bus: Arc<MessageBus>,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
    ) -> Self {
        Self { config, params }
    }
}

impl ToolInvocation for GrepToolInvocation {
    fn get_description(&self) -> String {
        let mut description = format!("'{}'", self.params.pattern);

        if let Some(include) = &self.params.include {
            description.push_str(&format!(" in {}", include));
        }

        if let Some(dir_path) = &self.params.dir_path {
            let resolved_path = resolve_tool_path(self.config.target_dir(), dir_path);
            let relative_path = make_relative(&resolved_path, self.config.target_dir());
            description.push_str(&format!(
                " within {}",
                shorten_path(&relative_path.to_string_lossy(), 80)
            ));
        } else {
            description.push_str(" across all workspace directories");
        }

        description
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        if let Some(dir_path) = &self.params.dir_path {
            vec![ToolLocation {
                path: normalize_cross_platform_path(dir_path),
                location_type: crate::core::tools::tools::LocationType::Read,
            }]
        } else {
            vec![]
        }
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let path = if let Some(dir) = &self.params.dir_path {
            resolve_tool_path(config.target_dir(), dir)
        } else {
            config.target_dir().to_path_buf()
        };

        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let is_trusted = tf.is_path_trusted(&path).unwrap_or(false);
                    if !is_trusted {
                        let path_clone = path.clone();
                        let config_clone = config.clone();
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Folder".to_string(),
                             prompt: format!("Security: Path {:?} is not in a trusted folder. Do you want to proceed?", path),
                             on_confirm: std::sync::Arc::new(move |outcome, _feedback| {
                                 if let crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave = outcome {
                                     if let Some(tf) = config_clone.trusted_folders() {
                                         let _ = tf.set_trust_level(&path_clone, crate::core::config::trusted_folders::TrustLevel::TrustFolder);
                                     }
                                 }
                             }),
                         }));
                    }
                }
            }
            Ok(None)
        })
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let signal = signal.cloned();
        Box::pin(async move {
            if let Some(signal) = signal {
                if signal.is_cancelled() {
                    let msg = "Command was cancelled by user before it could start.".to_string();
                    return Ok(ToolResult {
                        llm_content: Some(msg.clone()),
                        return_display: Some(msg.clone()),
                        output: msg,
                        error: None,
                        data: None,
                    });
                }
            }

            let result = tokio::task::spawn_blocking(move || {
                let search_dir_abs = if let Some(dir_path) = &params.dir_path {
                    let target_path = resolve_tool_path(config.target_dir(), dir_path);
                    if !target_path.exists() {
                        return Ok::<ToolResult, Box<dyn std::error::Error + Send + Sync>>(
                            ToolResult {
                                llm_content: Some(format!(
                                    "Path does not exist: {}",
                                    target_path.display()
                                )),
                                return_display: Some(format!(
                                    "Path does not exist: {}",
                                    target_path.display()
                                )),
                                output: format!("Path does not exist: {}", target_path.display()),
                                error: Some(crate::core::tools::tools::ToolError {
                                    error_type: ToolErrorType::SearchPathNotFound.to_string(),
                                    message: format!(
                                        "Path does not exist: {}",
                                        target_path.display()
                                    ),
                                }),
                                data: None,
                            },
                        );
                    }
                    if !target_path.is_dir() {
                        return Ok(ToolResult {
                            llm_content: Some(format!(
                                "Path is not a directory: {}",
                                target_path.display()
                            )),
                            return_display: Some(format!(
                                "Path is not a directory: {}",
                                target_path.display()
                            )),
                            output: format!("Path is not a directory: {}", target_path.display()),
                            error: Some(crate::core::tools::tools::ToolError {
                                error_type: ToolErrorType::SearchPathNotADirectory.to_string(),
                                message: format!(
                                    "Path is not a directory: {}",
                                    target_path.display()
                                ),
                            }),
                            data: None,
                        });
                    }
                    Some(target_path)
                } else {
                    Some(config.target_dir().to_path_buf())
                };

                let search_dir = search_dir_abs.unwrap();

                // Configure Ripgrep
                let rg_config = RipgrepConfig {
                    case_sensitive: params.case_sensitive,
                    whole_word: params.whole_word,
                    regex: params.regex, // Default to true if not specified in old tool? No, param schema says default regex.
                    // But actually GrepTool usually implies regex.
                    // Let's assume params.regex is the flag. If param schema says "regex pattern", we should treat query as regex.
                    // The old implementation used `regex::Regex::new`, so it was always regex.
                    // We should set regex: true by default if not provided, or ensure the query is treated as regex.
                    // However, RipgrepConfig separates literal vs regex.
                    // If we want to support "grep" behavior, we should enable regex.
                    // Let's force regex = true if we want to match old behavior,
                    // or better, respect the new flags if we update the schema.
                    // For backward compatibility, let's treat the pattern as regex.
                    include_patterns: params.include.map(|s| vec![s]),
                    // 1000 条 × 每行一整行源码，一次调用就能吃掉几万 token。
                    max_results: Some(DEFAULT_MAX_RESULTS),
                    ..Default::default()
                };

                // Note: The old tool treated pattern as regex always.
                // RipgrepConfig has a `regex` field.
                let mut final_config = rg_config;
                final_config.regex = true; // Force regex to match old behavior

                // 窄修复：模型想搜字面量左括号时打成 `\(`，而 `\(` 在 JSON 里是
                // 非法转义、反斜杠会被丢掉，发过来就成了 `func Relay(c|...` ——
                // 第 12 列的 `(` 没配对，整个正则编译失败，搜索颗粒无收。
                let mut pattern_note: Option<String> = None;
                let mut effective_pattern = params.pattern.clone();
                if let Some(repaired) = try_repair_pattern(&params.pattern) {
                    pattern_note = Some(format!(
                        "Unbalanced parenthesis in pattern, escaped as a literal and retried: {}",
                        repaired
                    ));
                    effective_pattern = repaired;
                }

                let matches = search_with_ripgrep(
                    &effective_pattern,
                    &search_dir.to_string_lossy(),
                    final_config,
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

                // Apply filters
                let filtered_matches: Vec<_> = matches
                    .iter()
                    .filter(|m| {
                        // Filter by exclude patterns (test files, etc.)
                        if let Some(exclude_patterns) = &params.exclude_patterns {
                            for pattern in exclude_patterns {
                                if matches_glob_pattern(&m.file, pattern) {
                                    return false;
                                }
                            }
                        }

                        // 这里以前无条件 `if is_test_file(&m.file) { return false }`，
                        // 而 `is_test_file` 是拿**绝对路径**做子串匹配（`/test/`、
                        // `test_`、`_spec.` …）。仓库放在 `/mnt/h/test/starcode`
                        // 这种路径下时，每一条结果都被判成测试文件，Grep 只会回
                        // "No matches found (after filtering)"。要排除测试就用
                        // `exclude_patterns`，那是调用方显式表达的意图。

                        // Filter out comment lines
                        if params.exclude_comments {
                            if let Some(text) = &m.text {
                                if is_comment_line(text) {
                                    return false;
                                }
                            }
                        }

                        true
                    })
                    .collect();

                if filtered_matches.is_empty() {
                    let mut message = format!(
                        "No matches found for pattern \"{}\" (after filtering)",
                        params.pattern
                    );
                    if let Some(note) = &pattern_note {
                        message = format!("{}\n{}", note, message);
                    }
                    return Ok(ToolResult {
                        llm_content: Some(message.clone()),
                        return_display: Some(message.clone()),
                        output: message,
                        error: None,
                        data: None,
                    });
                }

                let hits: Vec<GrepHit<'_>> = filtered_matches
                    .iter()
                    .map(|m| GrepHit {
                        file: &m.file,
                        line: m.line,
                        text: m.text.as_deref(),
                    })
                    .collect();
                let mut llm_content = format_grep_hits(&hits, &search_dir, &params.pattern);
                if let Some(note) = &pattern_note {
                    llm_content = format!("{}\n{}", note, llm_content);
                }

                Ok(ToolResult {
                    llm_content: Some(llm_content.clone()),
                    return_display: Some(llm_content.clone()),
                    output: llm_content,
                    error: None,
                    data: None,
                })
            })
            .await;

            match result {
                Ok(inner_result) => inner_result.map_err(|e| e as Box<dyn std::error::Error>),
                Err(e) => Err(Box::new(e)),
            }
        })
    }
}

pub struct GrepTool {
    config: Arc<crate::core::config::Config>,
    message_bus: Arc<MessageBus>,
}

impl GrepTool {
    pub fn new(config: Arc<crate::core::config::Config>, message_bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            message_bus,
        }
    }

    pub fn name(&self) -> &str {
        "Grep"
    }

    pub fn display_name(&self) -> &str {
        "SearchText"
    }

    pub fn description(&self) -> &str {
        "Searches for a regular expression pattern within the content of files in a specified directory (or current working directory). Can filter files by a glob pattern. Returns the lines containing matches, along with their file paths and line numbers."
    }

    pub fn kind(&self) -> Kind {
        Kind::Search
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "properties": {
                "pattern": {
                    "description": "The regular expression (regex) pattern to search for within file contents (e.g., 'function\\s+myFunction', 'import\\s+\\{.*\\}\\s+from\\s+.*'). Unbalanced parentheses are repaired automatically and searched literally.",
                    "type": "string"
                },
                "dir_path": {
                    "description": "Optional: The absolute path to the directory to search within. If omitted, searches the current working directory.",
                    "type": "string"
                },
                "include": {
                    "description": "Optional: A glob pattern to filter which files are searched (e.g., '*.js', '*.{ts,tsx}', 'src/**'). If omitted, searches all files (respecting potential global ignores).",
                    "type": "string"
                },
                "case_sensitive": {
                    "description": "Optional: Whether the search should be case sensitive. Default is false.",
                    "type": "boolean"
                }
            },
            "required": ["pattern"],
            "type": "object"
        })
    }

    pub fn validate_tool_params(&self, params: &GrepToolParams) -> Result<(), String> {
        // Validate dir_path if provided
        if let Some(dir_path) = &params.dir_path {
            let target_path = resolve_tool_path(self.config.target_dir(), dir_path);

            if !target_path.exists() {
                return Err(format!("Path does not exist: {}", target_path.display()));
            }

            if !target_path.is_dir() {
                return Err(format!(
                    "Path is not a directory: {}",
                    target_path.display()
                ));
            }
        }

        Ok(())
    }

    pub fn build(&self, params: GrepToolParams) -> Box<dyn ToolInvocation> {
        Box::new(GrepToolInvocation::new(
            self.config.clone(),
            params,
            self.message_bus.clone(),
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
        ))
    }
}

impl BaseDeclarativeTool for GrepTool {
    fn name(&self) -> &str {
        GrepTool::name(self)
    }

    fn display_name(&self) -> &str {
        GrepTool::display_name(self)
    }

    fn description(&self) -> &str {
        GrepTool::description(self)
    }

    fn kind(&self) -> Kind {
        GrepTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        GrepTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GrepToolParams = serde_json::from_value(params)?;
        self.validate_tool_params(&params)
            .map_err(|e| e.to_string())?;
        Ok(self.build(params))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

/// Check if a line is a comment based on common comment patterns
fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim();
    // Single-line comments
    if trimmed.starts_with("//")
        || trimmed.starts_with("#")
        || trimmed.starts_with("--")
        || trimmed.starts_with(";")
    {
        return true;
    }
    // Block comment starts
    if trimmed.starts_with("/*")
        || trimmed.starts_with("<!--")
        || trimmed.starts_with("'''")
        || trimmed.starts_with("\"\"\"")
    {
        return true;
    }
    // Python docstrings
    if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
        return true;
    }
    false
}

/// Simple glob pattern matching for file paths
fn matches_glob_pattern(file_path: &str, pattern: &str) -> bool {
    // Simple implementation: check if pattern matches
    // For now, just check if the file ends with the pattern (for extensions)
    if pattern.starts_with("*.") {
        let ext = &pattern[2..];
        return file_path.ends_with(ext);
    }
    // Check if pattern contains the file name
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            return file_path.starts_with(prefix) && file_path.ends_with(suffix);
        }
    }
    // Exact match
    file_path == pattern
}
