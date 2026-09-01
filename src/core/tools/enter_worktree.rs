use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterWorktreeParams {
    pub path: Option<String>,
}

pub struct EnterWorktreeTool;

impl EnterWorktreeTool {
    pub fn new(_config: Arc<Config>, _message_bus: Arc<MessageBus>) -> Self {
        Self
    }
}

impl BaseDeclarativeTool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "enter_worktree"
    }

    fn display_name(&self) -> &str {
        "Enter Worktree"
    }

    fn description(&self) -> &str {
        "Creates an isolated git worktree for experimental or risky changes. \
         Use this before making large-scale refactors, trying experimental approaches, \
         or working on changes that may need to be discarded. \
         The original working directory is preserved. Use 'exit_worktree' to clean up."
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
                    "description": "Optional custom path for the worktree. If omitted, a temp directory is used."
                }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: EnterWorktreeParams = serde_json::from_value(params)?;
        Ok(Box::new(EnterWorktreeInvocation { params }))
    }
}

pub struct EnterWorktreeInvocation {
    params: EnterWorktreeParams,
}

impl ToolInvocation for EnterWorktreeInvocation {
    fn get_description(&self) -> String {
        format!(
            "Enter Worktree: {}",
            self.params
                .path
                .as_deref()
                .unwrap_or("auto-generated temp dir")
        )
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
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                title: "Enter Isolated Worktree?".to_string(),
                prompt: "The agent wants to create an isolated git worktree for experimental changes. This uses `git worktree add` and will not affect your main working directory.".to_string(),
                on_confirm: std::sync::Arc::new(|outcome| {
                    if matches!(
                        outcome,
                        crate::types::ToolConfirmationOutcome::ProceedOnce
                            | crate::types::ToolConfirmationOutcome::ProceedAlways
                            | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave
                            | crate::types::ToolConfirmationOutcome::AllowSession
                    ) {
                        crate::utils::logging::append_debug_log_line(
                            "[EnterWorktree] User confirmed worktree creation.",
                        );
                    }
                }),
            }))
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
        let custom_path = self.params.path.clone();
        Box::pin(async move {
            let worktree_path = if let Some(p) = custom_path {
                std::path::PathBuf::from(&p)
            } else {
                let dir = std::env::temp_dir().join(format!(
                    "star-worktree-{}",
                    uuid::Uuid::new_v4()
                        .to_string()
                        .split('-')
                        .next()
                        .unwrap_or("0000")
                ));
                dir
            };

            // Create the worktree from current branch
            let cwd = std::env::current_dir().unwrap_or_default();
            let worktree_path_clone = worktree_path.clone();

            let result = tokio::task::spawn_blocking(
                move || -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
                    let branch = std::process::Command::new("git")
                        .args(["rev-parse", "--abbrev-ref", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "HEAD".to_string());

                    let output = std::process::Command::new("git")
                        .args([
                            "worktree",
                            "add",
                            "--detach",
                            worktree_path_clone.to_str().unwrap_or("/tmp/star-worktree"),
                            &branch,
                        ])
                        .current_dir(&cwd)
                        .output();

                    match output {
                        Ok(out) if out.status.success() => {
                            let path_str = worktree_path_clone.display().to_string();
                            Ok(ToolResult {
                                llm_content: Some(format!(
                                    "Worktree created at: {}\n\
                                 You are now working in an isolated git worktree. \
                                 Changes here will not affect the main working directory. \
                                 Use 'exit_worktree' to clean up this worktree when done.\n\
                                 Current branch: {}",
                                    path_str, branch
                                )),
                                return_display: Some(format!("Entered worktree: {}", path_str)),
                                output: format!(
                                    "Created isolated worktree at {}\nWorking on branch: {}",
                                    path_str, branch
                                ),
                                error: None,
                                data: Some(serde_json::json!({
                                    "worktree_path": path_str,
                                    "branch": branch,
                                    "worktree_active": true
                                })),
                            })
                        }
                        Ok(out) => {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            Err(format!("git worktree add failed: {}", stderr).into())
                        }
                        Err(e) => Err(format!("Failed to run git worktree add: {}", e).into()),
                    }
                },
            )
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

            result.map_err(|e| -> Box<dyn std::error::Error> { e })
        })
    }
}
