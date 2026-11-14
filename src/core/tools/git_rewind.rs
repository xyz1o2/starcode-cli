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
                "reset_to" => Ok(ToolResult {
                    llm_content: Some("GitRewind reset_to is disabled: a general tool grant, including bypass permissions, cannot authorize discarding working-tree changes for a specific target. No Git command was run. Ask the user to perform a separately reviewed recovery operation if needed.".to_string()),
                    return_display: Some("Hard reset disabled".to_string()),
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "unsafe_reset_disabled".to_string(),
                        message: "GitRewind reset_to is disabled; working-tree changes were not discarded".to_string(),
                    }),
                    data: None,
                }),
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
        "GitRewind"
    }

    fn display_name(&self) -> &str {
        "Git Rewind"
    }

    fn description(&self) -> &str {
        "Git undo operations: soft-reset the last commit or manage stashes. Destructive reset_to is disabled and never runs git reset --hard."
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
                    "enum": ["undo_last_commit", "stash", "pop"],
                    "description": "Git undo action. Destructive reset_to is disabled."
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ConfigParameters;
    use std::process::Command;

    #[tokio::test]
    async fn reset_to_never_discards_files_even_when_invocation_is_executed_directly() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked"), "original").unwrap();
        git(&["add", "tracked"]);
        git(&["commit", "-qm", "initial"]);
        std::fs::write(root.join("tracked"), "important uncommitted work").unwrap();
        let config = Arc::new(crate::core::config::Config::new(ConfigParameters {
            target_dir: root.clone(),
            ..ConfigParameters::default()
        }));
        let invocation = GitRewindInvocation {
            config,
            params: GitRewindParams {
                action: "reset_to".to_string(),
                target: Some("HEAD".to_string()),
            },
        };
        let result = invocation.execute(None, None).await.unwrap();
        assert_eq!(result.error.unwrap().error_type, "unsafe_reset_disabled");
        assert_eq!(
            std::fs::read_to_string(root.join("tracked")).unwrap(),
            "important uncommitted work"
        );
    }
}
