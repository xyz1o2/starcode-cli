use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitWorktreeParams {
    pub path: String,
    pub keep_changes: Option<bool>,
}

pub struct ExitWorktreeTool;

impl ExitWorktreeTool {
    pub fn new(_config: Arc<Config>, _message_bus: Arc<MessageBus>) -> Self {
        Self
    }
}

impl BaseDeclarativeTool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "exit_worktree"
    }

    fn display_name(&self) -> &str {
        "Exit Worktree"
    }

    fn description(&self) -> &str {
        "Removes an isolated git worktree created by 'enter_worktree'. \
         By default, uncommitted changes in the worktree are discarded. \
         Set 'keep_changes' to true to merge changes back before removal. \
         After exiting, the agent returns to the main working directory."
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path to the worktree to remove (as returned by enter_worktree)."
                },
                "keep_changes": {
                    "type": "boolean",
                    "description": "If true, merge worktree changes back before removal. Default: false (discard)."
                }
            },
            "required": ["path"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ExitWorktreeParams = serde_json::from_value(params)?;
        Ok(Box::new(ExitWorktreeInvocation { params }))
    }
}

pub struct ExitWorktreeInvocation {
    params: ExitWorktreeParams,
}

impl ToolInvocation for ExitWorktreeInvocation {
    fn get_description(&self) -> String {
        let keep = if self.params.keep_changes.unwrap_or(false) {
            "keeping changes"
        } else {
            "discarding changes"
        };
        format!("Exit Worktree: {} ({})", self.params.path, keep)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
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
                > + Send,
        >,
    > {
        let path = self.params.path.clone();
        let keep = self.params.keep_changes.unwrap_or(false);
        Box::pin(async move {
            let keep_msg = if keep {
                "\n\nChanges in the worktree will be kept."
            } else {
                "\n\nAll uncommitted changes in the worktree will be DISCARDED."
            };
            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                    title: "Exit Worktree?".to_string(),
                    prompt: format!(
                        "The agent wants to remove the isolated worktree at:\n\n{}{}",
                        path, keep_msg
                    ),
                    on_confirm: std::sync::Arc::new(move |outcome| {
                        if matches!(
                            outcome,
                            crate::types::ToolConfirmationOutcome::ProceedOnce
                                | crate::types::ToolConfirmationOutcome::ProceedAlways
                                | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave
                                | crate::types::ToolConfirmationOutcome::AllowSession
                        ) {
                            crate::utils::logging::append_debug_log_line(
                                "[ExitWorktree] User confirmed worktree removal.",
                            );
                        }
                    }),
                },
            ))
        })
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
        let path = self.params.path.clone();
        let keep = self.params.keep_changes.unwrap_or(false);

        Box::pin(async move {
            let path_clone = path.clone();
            let keep_clone = keep;

            let result = tokio::task::spawn_blocking(move || -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
                let cwd = std::env::current_dir().unwrap_or_default();
                let worktree_path = std::path::Path::new(&path_clone);

                // Verify the path exists and is a git worktree
                if !worktree_path.exists() {
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Worktree path does not exist: {}. It may have already been removed.",
                            path_clone
                        )),
                        return_display: Some("Worktree not found".to_string()),
                        output: format!("Worktree path not found: {}", path_clone),
                        error: Some(ToolError {
                            error_type: "not_found".to_string(),
                            message: format!("Path not found: {}", path_clone),
                        }),
                        data: None,
                    });
                }

                // If keeping changes, try to merge them back
                if keep_clone {
                    let git_dir = worktree_path.join(".git");
                    if git_dir.exists() {
                        if let Ok(branch_out) = std::process::Command::new("git")
                            .args(["rev-parse", "--abbrev-ref", "HEAD"])
                            .current_dir(worktree_path)
                            .output()
                        {
                            let branch = String::from_utf8_lossy(&branch_out.stdout)
                                .trim()
                                .to_string();
                            if !branch.is_empty() && branch != "HEAD" {
                                let merge_result = std::process::Command::new("git")
                                    .args(["merge", &format!("worktree/{}", branch)])
                                    .current_dir(&cwd)
                                    .output();

                                if let Ok(merge) = &merge_result {
                                    if merge.status.success() {
                                        crate::utils::logging::append_debug_log_line(&format!(
                                            "[ExitWorktree] Merged changes from worktree branch '{}'",
                                            branch
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                // Remove the worktree
                let output = std::process::Command::new("git")
                    .args([
                        "worktree",
                        "remove",
                        "--force",
                        worktree_path.to_str().unwrap_or(&path_clone),
                    ])
                    .current_dir(&cwd)
                    .output();

                match output {
                    Ok(out) if out.status.success() => {
                        Ok(ToolResult {
                            llm_content: Some(
                                "Worktree removed successfully. You are now back in the main working directory. \
                                 Your original working tree is preserved.".to_string(),
                            ),
                            return_display: Some("Worktree removed".to_string()),
                            output: format!("Removed worktree at: {}", path_clone),
                            error: None,
                            data: Some(serde_json::json!({
                                "removed_path": path_clone,
                                "changes_kept": keep_clone,
                                "worktree_active": false
                            })),
                        })
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        let _ = std::process::Command::new("git")
                            .args(["worktree", "prune"])
                            .current_dir(&cwd)
                            .output();

                        Ok(ToolResult {
                            llm_content: Some(format!(
                                "Failed to remove worktree: {}\n\
                                 Attempted to prune stale worktrees. You may need to manually remove: {}",
                                stderr, path_clone
                            )),
                            return_display: Some("Worktree removal failed".to_string()),
                            output: stderr.clone(),
                            error: Some(ToolError {
                                error_type: "worktree_remove".to_string(),
                                message: stderr,
                            }),
                            data: None,
                        })
                    }
                    Err(e) => Ok(ToolResult {
                        llm_content: Some(format!("Failed to run git worktree remove: {}", e)),
                        return_display: Some("Worktree removal error".to_string()),
                        output: e.to_string(),
                        error: Some(ToolError {
                            error_type: "git_error".to_string(),
                            message: e.to_string(),
                        }),
                        data: None,
                    }),
                }
            }).await.map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

            result.map_err(|e| -> Box<dyn std::error::Error> { e })
        })
    }
}
