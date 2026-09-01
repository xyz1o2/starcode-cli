use crate::core::tools::git_utils::run_git;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GitRewindTool {
    config: Arc<crate::core::config::Config>,
}

impl GitRewindTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitRewindParams {
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
}

pub struct GitRewindInvocation {
    config: Arc<crate::core::config::Config>,
    params: GitRewindParams,
}

impl ToolInvocation for GitRewindInvocation {
    fn get_description(&self) -> String {
        format!(
            "Git rewind: {} {}",
            self.params.action,
            self.params.target.as_deref().unwrap_or("")
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
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
        Box::pin(async move {
            let root = config.project_root();

            match params.action.as_str() {
                "undo_last_commit" => {
                    let msg = run_git(root, &["log", "-1", "--pretty=%s"])
                        .await
                        .unwrap_or_default();
                    let msg = msg.trim();
                    run_git(root, &["reset", "--soft", "HEAD~1"])
                        .await
                        .map_err(|e| format!("Failed to undo commit: {}", e))?;

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Undid last commit '{}'. Changes are staged.",
                            msg
                        )),
                        return_display: Some("Undid last commit".to_string()),
                        output: format!(
                            "Soft reset to HEAD~1. Commit '{}' undone. Changes remain staged.",
                            msg
                        ),
                        error: None,
                        data: None,
                    })
                }
                "reset_to" => {
                    let target = match params.target.as_ref() {
                        Some(t) => t,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "target ref is required for reset_to".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };

                    run_git(root, &["reset", "--hard", target])
                        .await
                        .map_err(|e| format!("Failed to reset to {}: {}", target, e))?;

                    Ok(ToolResult {
                        llm_content: Some(format!("Reset to {}", target)),
                        return_display: Some(format!("Reset to {}", target)),
                        output: format!("Hard reset to {}", target),
                        error: None,
                        data: None,
                    })
                }
                "stash" => {
                    let output = run_git(root, &["stash", "push", "-m", "starcode-rewind"])
                        .await
                        .map_err(|e| format!("Failed to stash: {}", e))?;

                    Ok(ToolResult {
                        llm_content: Some("Changes stashed.".to_string()),
                        return_display: Some("Stashed changes".to_string()),
                        output: output.trim().to_string(),
                        error: None,
                        data: None,
                    })
                }
                "pop" => {
                    let output = run_git(root, &["stash", "pop"])
                        .await
                        .map_err(|e| format!("Failed to pop stash: {}", e))?;

                    Ok(ToolResult {
                        llm_content: Some("Stash popped.".to_string()),
                        return_display: Some("Popped stash".to_string()),
                        output: output.trim().to_string(),
                        error: None,
                        data: None,
                    })
                }
                _ => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "invalid_action".to_string(),
                        message: format!(
                            "Unknown action '{}'. Valid: undo_last_commit, reset_to, stash, pop",
                            params.action
                        ),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for GitRewindTool {
    fn name(&self) -> &str {
        "git_rewind"
    }

    fn display_name(&self) -> &str {
        "Git Rewind"
    }

    fn description(&self) -> &str {
        "Git 撤销/回退操作：撤销上次提交、重置到指定引用、管理暂存区。(Git undo/rewind: undo last commit, reset to ref, or manage stash.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["undo_last_commit", "reset_to", "stash", "pop"],
                    "description": "操作类型 (Action type)"
                },
                "target": {
                    "type": "string",
                    "description": "目标引用，reset_to 时必需 (Target ref, required for reset_to)"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitRewindParams = serde_json::from_value(params)?;
        Ok(Box::new(GitRewindInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
