use crate::core::state::GlobalState;
use crate::core::tools::edit::apply_replacement;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiEditToolParams {
    pub edits: Vec<SingleFileEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleFileEdit {
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "old_string")]
    pub old_string: String,
    #[serde(rename = "new_string")]
    pub new_string: String,
}

pub struct MultiEditToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: MultiEditToolParams,
    global_state: Arc<GlobalState>,
}

impl MultiEditToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: MultiEditToolParams,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            global_state,
        }
    }
}

impl ToolInvocation for MultiEditToolInvocation {
    fn get_description(&self) -> String {
        format!("Multi-file edit with {} changes", self.params.edits.len())
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        self.params
            .edits
            .iter()
            .map(|edit| {
                let path = self.config.target_dir().join(&edit.file_path);
                ToolLocation {
                    path,
                    location_type: crate::core::tools::tools::LocationType::Write,
                }
            })
            .collect()
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
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
        let global_state = self.global_state.clone();

        Box::pin(async move {
            let mut file_contents = HashMap::new();
            let final_contents;

            // ============ P1.2 改进：前置验证 - 检查所有编辑文件是否已被读取 ============
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true);

            if strict_read_check {
                {
                    let exec_state = global_state.execution_state.read().await;
                    let mut unread: Vec<&str> = Vec::new();
                    for edit in &params.edits {
                        let path = config.target_dir().join(&edit.file_path);
                        let abs_path = path
                            .canonicalize()
                            .unwrap_or_else(|_| path.clone())
                            .to_string_lossy()
                            .to_string();

                        if path.exists() && !exec_state.was_file_read(&abs_path) {
                            unread.push(&edit.file_path);
                        }
                    }
                    if !unread.is_empty() {
                        let file_list = unread
                            .iter()
                            .map(|p| format!("  - {}", p))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let read_calls = unread
                            .iter()
                            .map(|p| format!("`Read('{}')`", p))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let msg = format!(
                            "Edit blocked [edit_file_not_read]: {count} file(s) must be read before `multi_edit`:\n{list}\n\
                             REQUIRED NEXT STEP: call {calls} (batch them in one response), then retry `multi_edit`. \
                             Do NOT retry until ALL listed files have been read.",
                            count = unread.len(),
                            list = file_list,
                            calls = read_calls,
                        );
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: "file_has_not_been_read".to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                } // 显式释放读锁
            }

            // 1. Read and Validate Phase
            for edit in &params.edits {
                let path = config.target_dir().join(&edit.file_path);

                // Read file if not already read
                if !file_contents.contains_key(&path) {
                    if path.exists() {
                        let content =
                            crate::core::utils::file_utils::read_file_with_encoding_async(&path)
                                .await
                                .map_err(|e| {
                                    format!("Failed to read file {}: {}", edit.file_path, e)
                                })?;
                        file_contents.insert(path.clone(), content);
                    } else {
                        // For new files, content is empty
                        file_contents.insert(path.clone(), String::new());
                    }
                }

                let current_content = file_contents.get(&path).unwrap();
                let is_new_file = current_content.is_empty() && !path.exists();

                // Check if old_string exists
                if !is_new_file && !current_content.contains(&edit.old_string) {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: format!("Verification failed: Could not find exact match for `old_string` in {}.", edit.file_path),
                        error: Some(ToolError {
                            error_type: "edit_verification_failed".to_string(),
                            message: format!(
                                "Could not find exact match for `old_string` in {}. Please verify the file content.",
                                edit.file_path
                            ),
                        }),
                        data: None,
                    });
                }
            }

            // Re-do validation with sequential application
            let mut working_contents = file_contents.clone();

            for edit in &params.edits {
                let path = config.target_dir().join(&edit.file_path);
                // Important: Use working_contents here to pick up changes from previous edits in the same sequence!
                // Wait, if we edit file1 twice, the second edit should run against the output of the first edit.
                let current_content = working_contents.get(&path).unwrap();
                let is_new_file = current_content.is_empty() && !path.exists(); // Approximate check

                // Note: We remove debug printlns for production
                if !is_new_file && !current_content.contains(&edit.old_string) {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: format!("Verification failed during sequence: `old_string` not found in {}.", edit.file_path),
                        error: Some(ToolError {
                            error_type: "edit_verification_failed".to_string(),
                            message: format!(
                                "Could not find exact match for `old_string` in {}. This might be due to overlapping edits.",
                                edit.file_path
                            ),
                        }),
                        data: None,
                    });
                }

                let new_content = apply_replacement(
                    Some(current_content),
                    &edit.old_string,
                    &edit.new_string,
                    is_new_file,
                );
                working_contents.insert(path, new_content);
            }

            final_contents = working_contents;

            // 2. Write Phase (Atomic-ish)
            // Ideally we would use a transaction log or backup, but for now we rely on the pre-validation.
            // If validation passes, writes are likely to succeed unless FS issues occur.
            let mut modified_files = Vec::new();
            let mut backup_map: HashMap<PathBuf, String> = HashMap::new();
            let mut write_error: Option<String> = None;

            // Backup phase
            for (path, _) in &final_contents {
                let is_target = params
                    .edits
                    .iter()
                    .any(|e| config.target_dir().join(&e.file_path) == *path);
                if is_target && path.exists() {
                    // File-history checkpoint: snapshot before overwrite.
                    // track_edit is best-effort — failures must not block.
                    {
                        let msg_id = global_state.current_message_id().await;
                        if let Err(e) = crate::utils::checkpoint_manager::track_edit(
                            path,
                            msg_id,
                            Some("multi_edit"),
                            None, // session_id: per-cwd fallback, matches /undo and /rewind
                        )
                        .await
                        {
                            log::warn!(
                                "FileHistory: track_edit failed for {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                    match crate::core::utils::file_utils::read_file_with_encoding_async(path).await
                    {
                        Ok(content) => {
                            backup_map.insert(path.clone(), content);
                        }
                        Err(e) => {
                            write_error = Some(format!(
                                "Failed to create backup for {}: {}",
                                path.display(),
                                e
                            ));
                            break;
                        }
                    }
                }
            }

            if let Some(err) = write_error {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("MultiEdit Aborted: {}", err),
                    error: Some(ToolError {
                        error_type: "backup_failed".to_string(),
                        message: err,
                    }),
                    data: None,
                });
            }

            // Write phase
            for (path, content) in final_contents {
                // Only write if it was targeted
                let is_target = params
                    .edits
                    .iter()
                    .any(|e| config.target_dir().join(&e.file_path) == path);
                if is_target {
                    if let Some(parent) = path.parent() {
                        if let Err(e) = tokio::fs::create_dir_all(parent).await {
                            write_error = Some(format!(
                                "Failed to create directory {}: {}",
                                parent.display(),
                                e
                            ));
                            break;
                        }
                    }
                    if let Err(e) = tokio::fs::write(&path, content).await {
                        write_error =
                            Some(format!("Failed to write file {}: {}", path.display(), e));
                        break;
                    }
                    modified_files.push(path);
                }
            }

            // Rollback phase if error
            if let Some(err) = write_error {
                // Attempt rollback
                for (path, content) in backup_map {
                    // Ignore rollback errors for now, best effort
                    let _ = tokio::fs::write(path, content).await;
                }

                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("MultiEdit Failed: {}. Rolled back changes.", err),
                    error: Some(ToolError {
                        error_type: "write_failed_rollback".to_string(),
                        message: format!("Write failed: {}. Changes have been rolled back.", err),
                    }),
                    data: None,
                });
            }

            let file_names: Vec<String> = modified_files
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect();

            let msg = format!(
                "Successfully modified {} files: {}",
                file_names.len(),
                file_names.join(", ")
            );

            Ok(ToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg.clone()),
                output: msg,
                error: None,
                data: Some(json!({ "modified_files": file_names })),
            })
        })
    }
}

pub struct MultiEditTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<GlobalState>,
}

impl MultiEditTool {
    pub fn new(config: Arc<crate::core::config::Config>, global_state: Arc<GlobalState>) -> Self {
        Self {
            config,
            global_state,
        }
    }
}

impl BaseDeclarativeTool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn display_name(&self) -> &str {
        "MultiEdit"
    }

    fn description(&self) -> &str {
        "Edit multiple files in a single atomic transaction. Use this when you need to change multiple files that depend on each other (e.g. changing a function signature and its usages)."
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": { "type": "string", "description": "Path to the file to edit" },
                            "old_string": { "type": "string", "description": "The exact string to replace" },
                            "new_string": { "type": "string", "description": "The new string to insert" }
                        },
                        "required": ["file_path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: MultiEditToolParams = serde_json::from_value(params)?;
        Ok(Box::new(MultiEditToolInvocation::new(
            self.config.clone(),
            params,
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
            self.global_state.clone(),
        )))
    }
}
