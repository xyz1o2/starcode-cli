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

pub struct EnterWorktreeTool {
    project_root: std::path::PathBuf,
}

impl EnterWorktreeTool {
    pub fn new(config: Arc<Config>, _message_bus: Arc<MessageBus>) -> Self {
        Self {
            project_root: config.target_dir().to_owned(),
        }
    }
}

impl BaseDeclarativeTool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }

    fn display_name(&self) -> &str {
        "Enter Worktree"
    }

    fn description(&self) -> &str {
        "Creates an isolated git worktree for experimental or risky changes. \
         Use this before making large-scale refactors, trying experimental approaches, \
         or working on changes that may need to be discarded. \
         Creates the worktree without changing this session's working directory. \
         Pass the returned path explicitly to later tools. Use ExitWorktree to clean up."
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
        Ok(Box::new(EnterWorktreeInvocation {
            params,
            project_root: self.project_root.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[tokio::test]
    async fn creation_uses_the_configured_project_without_claiming_a_cwd_switch() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Local Test",
                "-c",
                "user.email=local@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
        );
        let head = git(&root, &["rev-parse", "HEAD"]);
        let cwd = std::env::current_dir().unwrap();
        let invocation = EnterWorktreeInvocation {
            params: EnterWorktreeParams {
                path: Some("relative-worktree".into()),
            },
            project_root: root.clone(),
        };
        let result = invocation.execute(None, None).await.unwrap();
        let path = root.join("relative-worktree");
        assert!(path.join(".git").is_file());
        assert_eq!(git(&path, &["rev-parse", "HEAD"]), head);
        let data = result.data.unwrap();
        assert_eq!(data["worktree_path"], path.to_string_lossy().as_ref());
        assert_eq!(data["session_cwd_changed"], false);
        assert_eq!(data["worktree_active"], false);
        assert!(result
            .llm_content
            .unwrap()
            .contains("working directory has not changed"));
        assert_eq!(std::env::current_dir().unwrap(), cwd);
        git(&root, &["worktree", "remove", "relative-worktree"]);
    }

    #[tokio::test]
    async fn creation_fails_clearly_when_project_has_no_head() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        let invocation = EnterWorktreeInvocation {
            params: EnterWorktreeParams {
                path: Some("must-not-exist".into()),
            },
            project_root: root.path().to_owned(),
        };
        let error = invocation.execute(None, None).await.unwrap_err();
        assert!(error.to_string().contains("Cannot resolve project HEAD"));
        assert!(!root.path().join("must-not-exist").exists());
    }
}

pub struct EnterWorktreeInvocation {
    params: EnterWorktreeParams,
    project_root: std::path::PathBuf,
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
                on_confirm: std::sync::Arc::new(|outcome, _feedback| {
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
        let cwd = self.project_root.clone();
        Box::pin(async move {
            let worktree_path = if let Some(p) = custom_path {
                let path = std::path::PathBuf::from(p);
                if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                }
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

            // 从所属项目的 HEAD 创建；不读取或改变进程 cwd。
            let worktree_path_clone = worktree_path.clone();

            let result = tokio::task::spawn_blocking(
                move || -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
                    let head = std::process::Command::new("git")
                        .args(["rev-parse", "--verify", "HEAD"])
                        .current_dir(&cwd)
                        .output()?;
                    if !head.status.success() {
                        return Err(format!("Cannot resolve project HEAD: {}", String::from_utf8_lossy(&head.stderr).trim()).into());
                    }
                    let head = String::from_utf8(head.stdout)?.trim().to_owned();

                    let output = std::process::Command::new("git")
                        .args(["worktree", "add", "--detach", "--"])
                        .arg(&worktree_path_clone)
                        .arg(&head)
                        .current_dir(&cwd)
                        .output();

                    match output {
                        Ok(out) if out.status.success() => {
                            let path_str = worktree_path_clone.display().to_string();
                            Ok(ToolResult {
                                llm_content: Some(format!(
                                    "Worktree created at: {}\n\
                                 This session's working directory has not changed. \
                                 Pass this path explicitly to subsequent tools. \
                                 Use ExitWorktree to clean up this worktree when done.\n\
                                 Detached HEAD: {}",
                                    path_str, head
                                )),
                                return_display: Some(format!("Created worktree: {}", path_str)),
                                output: format!(
                                    "Created worktree at {}\nDetached HEAD: {}\nSession working directory unchanged.",
                                    path_str, head
                                ),
                                error: None,
                                data: Some(serde_json::json!({
                                    "worktree_path": path_str,
                                    "head": head,
                                    "worktree_active": false,
                                    "session_cwd_changed": false
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
