use crate::core::confirmation_bus::MessageBus;
use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{
    make_relative, normalize_cross_platform_path, resolve_tool_path, shorten_path,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobToolParams {
    pub pattern: String,
    #[serde(rename = "dir_path")]
    pub dir_path: Option<String>,
    #[serde(rename = "case_sensitive")]
    pub case_sensitive: Option<bool>,
    #[serde(rename = "respect_git_ignore")]
    pub respect_git_ignore: Option<bool>,
    #[serde(rename = "respect_star_ignore")]
    pub respect_star_ignore: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct GlobPath {
    pub fullpath: PathBuf,
    pub mtime_ms: Option<u64>,
}

pub fn sort_file_entries(entries: &mut [GlobPath], now_timestamp: u64, recency_threshold_ms: u64) {
    entries.sort_by(|a, b| {
        let mtime_a = a.mtime_ms.unwrap_or(0);
        let mtime_b = b.mtime_ms.unwrap_or(0);
        let a_is_recent = now_timestamp.saturating_sub(mtime_a) < recency_threshold_ms;
        let b_is_recent = now_timestamp.saturating_sub(mtime_b) < recency_threshold_ms;

        if a_is_recent && b_is_recent {
            mtime_b.cmp(&mtime_a)
        } else if a_is_recent {
            std::cmp::Ordering::Less
        } else if b_is_recent {
            std::cmp::Ordering::Greater
        } else {
            a.fullpath.cmp(&b.fullpath)
        }
    });
}

pub struct GlobToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: GlobToolParams,
    global_state: Arc<GlobalState>,
}

impl GlobToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: GlobToolParams,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            global_state,
        }
    }
}

impl ToolInvocation for GlobToolInvocation {
    fn get_description(&self) -> String {
        let mut description = format!("'{}'", self.params.pattern);

        if let Some(dir_path) = &self.params.dir_path {
            let search_dir = resolve_tool_path(self.config.target_dir(), dir_path);
            let relative_path = make_relative(&search_dir, self.config.target_dir());
            description.push_str(&format!(
                " within {}",
                shorten_path(&relative_path.to_string_lossy(), 80)
            ));
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
        let pattern = &self.params.pattern;
        let search_dir = if let Some(dir) = &self.params.dir_path {
            resolve_tool_path(config.target_dir(), dir)
        } else {
            config.target_dir().to_path_buf()
        };

        let path_to_check = search_dir.join(pattern);

        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let is_trusted = tf.is_path_trusted(&path_to_check).unwrap_or(false);
                    if !is_trusted {
                        let path_clone = path_to_check.clone();
                        let config_clone = config.clone();
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Folder".to_string(),
                             prompt: format!("Security: Path {:?} is not in a trusted folder. Do you want to proceed?", path_to_check),
                             on_confirm: std::sync::Arc::new(move |outcome| {
                                 if let crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave = outcome {
                                     if let Some(tf) = config_clone.trusted_folders() {
                                         let folder_to_trust = if path_clone.is_dir() {
                                              path_clone.clone()
                                          } else {
                                              path_clone.parent().unwrap_or(&path_clone).to_path_buf()
                                          };
                                         let _ = tf.set_trust_level(&folder_to_trust, crate::core::config::trusted_folders::TrustLevel::TrustFolder);
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
        let global_state = self.global_state.clone();
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
                let search_dir = if let Some(dir_path) = &params.dir_path {
                    resolve_tool_path(config.target_dir(), dir_path)
                } else {
                    config.target_dir().clone()
                };

                let pattern = &params.pattern;

                // Use glob crate to find files
                let glob_pattern = if search_dir == *config.target_dir() {
                    pattern.clone()
                } else {
                    format!("{}/**/{}", search_dir.display(), pattern)
                };

                let mut entries = Vec::new();

                let paths = glob::glob(&glob_pattern)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                for entry in paths {
                    if let Ok(path) = entry {
                        if path.is_file() {
                            // Use std::fs::metadata (blocking) instead of tokio::fs::metadata
                            match std::fs::metadata(&path) {
                                Ok(metadata) => {
                                    if let Ok(modified) = metadata.modified() {
                                        if let Ok(duration) =
                                            modified.duration_since(std::time::UNIX_EPOCH)
                                        {
                                            entries.push(GlobPath {
                                                fullpath: path,
                                                mtime_ms: Some(duration.as_millis() as u64),
                                            });
                                        }
                                    }
                                }
                                Err(_) => continue, // Skip files we can't stat
                            }
                        }
                    }
                }

                if entries.is_empty() {
                    let message = format!(
                        "No files found matching pattern \"{}\" within {}",
                        pattern,
                        search_dir.display()
                    );
                    return Ok::<ToolResult, Box<dyn std::error::Error + Send + Sync>>(
                        ToolResult {
                            llm_content: Some(message.clone()),
                            return_display: Some("No files found".to_string()),
                            output: message,
                            error: None,
                            data: None,
                        },
                    );
                }

                // Sort by modification time
                let one_day_in_ms = 24 * 60 * 60 * 1000;
                let now_timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                    .as_millis() as u64;

                sort_file_entries(&mut entries, now_timestamp, one_day_in_ms);

                let sorted_paths: Vec<String> = entries
                    .iter()
                    .map(|e| e.fullpath.to_string_lossy().to_string())
                    .collect();

                // ── 收集找到的文件路径，用于更新 read_file_state ──
                let found_file_paths: Vec<String> = sorted_paths.clone();

                let file_count = sorted_paths.len();

                // LLM: full detail with paths
                let llm_message = format!(
                    "Found {} file(s) matching \"{}\":\n{}",
                    file_count,
                    pattern,
                    sorted_paths.join("\n")
                );

                // UI display: grouped by directory for compact scanning
                let display = format_glob_display(&search_dir, &sorted_paths, file_count);

                Ok::<ToolResult, Box<dyn std::error::Error + Send + Sync>>(ToolResult {
                    llm_content: Some(llm_message.clone()),
                    return_display: Some(display),
                    output: llm_message,
                    error: None,
                    data: Some(serde_json::to_value(found_file_paths).unwrap_or_default()),
                })
            })
            .await;

            match result {
                Ok(Ok(tool_result)) => {
                    // ── 更新 read_file_state：glob 找到的文件视为"已浏览" ──
                    if let Some(ref data) = tool_result.data {
                        if let Ok(paths) = serde_json::from_value::<Vec<String>>(data.clone()) {
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis();
                            let mut state = global_state.read_file_state.write().await;
                            for path in &paths {
                                state.entry(path.clone()).or_insert(ReadFileState {
                                    content: String::new(),
                                    timestamp: now,
                                    file_system_timestamp: now,
                                });
                            }
                        }
                    }
                    Ok(tool_result)
                }
                Ok(Err(e)) => Err(e as Box<dyn std::error::Error>),
                Err(e) => Err(Box::new(e)),
            }
        })
    }
}

pub struct GlobMatchTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<GlobalState>,
}

impl GlobMatchTool {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        _message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            global_state,
        }
    }

    pub fn name(&self) -> &str {
        "Glob"
    }

    pub fn display_name(&self) -> &str {
        "FindFiles"
    }

    pub fn description(&self) -> &str {
        "Efficiently finds files matching specific glob patterns (e.g., `src/**/*.ts`, `**/*.md`), returning absolute paths sorted by modification time (newest first). Ideal for quickly locating files based on their name or path structure, especially in large codebases."
    }

    pub fn kind(&self) -> Kind {
        Kind::Search
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "properties": {
                "pattern": {
                    "description": "The glob pattern to match against (e.g., '**/*.py', 'docs/*.md').",
                    "type": "string"
                },
                "dir_path": {
                    "description": "Optional: The absolute path to the directory to search within. If omitted, searches the root directory.",
                    "type": "string"
                },
                "case_sensitive": {
                    "description": "Optional: Whether the search should be case-sensitive. Defaults to false.",
                    "type": "boolean"
                },
                "respect_git_ignore": {
                    "description": "Optional: Whether to respect .gitignore patterns when finding files. Only available in git repositories. Defaults to true.",
                    "type": "boolean"
                },
                "respect_star_ignore": {
                    "description": "Optional: Whether to respect .starignore patterns when finding files. Defaults to true.",
                    "type": "boolean"
                }
            },
            "required": ["pattern"],
            "type": "object"
        })
    }

    pub fn validate_tool_params(&self, params: &GlobToolParams) -> Result<(), String> {
        if params.pattern.trim().is_empty() {
            return Err("The 'pattern' parameter cannot be empty.".to_string());
        }

        let search_dir = if let Some(dir_path) = &params.dir_path {
            resolve_tool_path(self.config.target_dir(), dir_path)
        } else {
            self.config.target_dir().clone()
        };

        if !search_dir.exists() {
            return Err(format!(
                "Search path does not exist {}",
                search_dir.display()
            ));
        }

        if !search_dir.is_dir() {
            return Err(format!(
                "Search path is not a directory: {}",
                search_dir.display()
            ));
        }

        Ok(())
    }

    pub fn build(&self, params: GlobToolParams) -> Box<dyn ToolInvocation> {
        Box::new(GlobToolInvocation::new(
            self.config.clone(),
            params,
            self.global_state.clone(),
        ))
    }
}

impl BaseDeclarativeTool for GlobMatchTool {
    fn name(&self) -> &str {
        GlobMatchTool::name(self)
    }

    fn display_name(&self) -> &str {
        GlobMatchTool::display_name(self)
    }

    fn description(&self) -> &str {
        GlobMatchTool::description(self)
    }

    fn kind(&self) -> Kind {
        GlobMatchTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        GlobMatchTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GlobToolParams = serde_json::from_value(params)?;
        self.validate_tool_params(&params)
            .map_err(|e| e.to_string())?;
        Ok(self.build(params))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

/// Group file paths by parent directory and format as a compact tree for display.
fn format_glob_display(base_dir: &Path, paths: &[String], count: usize) -> String {
    use std::collections::BTreeMap;

    if paths.is_empty() {
        return format!("No files found in {}", base_dir.display());
    }

    let mut by_dir: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for p in paths {
        let full = PathBuf::from(p);
        // Try to strip base_dir prefix for relative display
        let rel = full
            .strip_prefix(base_dir)
            .ok()
            .map(|r| r.to_path_buf())
            .unwrap_or_else(|| {
                full.file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| full.clone())
            });
        let dir = rel
            .parent()
            .map(|d| d.display().to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let fname = rel
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| rel.display().to_string());
        by_dir.entry(dir).or_default().push(fname);
    }

    let mut out = format!("{} files", count);
    for (dir, files) in &by_dir {
        if by_dir.len() > 1 || dir != "." {
            out.push_str(&format!("\n{}/", dir));
        }
        for f in files {
            out.push_str(&format!("\n  {}", f));
        }
    }
    out
}
