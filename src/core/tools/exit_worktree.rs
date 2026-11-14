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
        "ExitWorktree"
    }

    fn display_name(&self) -> &str {
        "Exit Worktree"
    }

    fn description(&self) -> &str {
        "Removes a clean, registered secondary Git worktree only after its HEAD is contained in the main worktree's HEAD. Uncommitted, untracked and ignored files are never discarded. keep_changes does not merge changes automatically."
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
                    "description": "Compatibility option. No automatic merge or discard in either mode: removal requires a clean worktree and a commit contained in the main worktree HEAD."
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

fn git_output(dir: &std::path::Path, args: &[&str]) -> Result<std::process::Output, String> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|err| format!("git {}: {err}", args.join(" ")))
}

/// 只移除同仓库、已合并且干净的附属 worktree；校验失败时绝不删除。
fn validate_worktree_removal(
    cwd: &std::path::Path,
    target: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let target = target
        .canonicalize()
        .map_err(|err| format!("Cannot resolve worktree path: {err}"))?;
    let cwd_worktree = git_output(cwd, &["rev-parse", "--show-toplevel"])?;
    if !cwd_worktree.status.success() {
        return Err("Current directory is not inside a Git worktree".to_string());
    }
    let cwd_worktree = std::path::Path::new(
        std::str::from_utf8(&cwd_worktree.stdout)
            .map_err(|err| err.to_string())?
            .trim(),
    )
    .canonicalize()
    .map_err(|err| err.to_string())?;
    let registered = git_output(&cwd_worktree, &["worktree", "list", "--porcelain"])?;
    if !registered.status.success() {
        return Err("Cannot verify registered Git worktrees".to_string());
    }
    let listing = std::str::from_utf8(&registered.stdout).map_err(|err| err.to_string())?;
    let mut paths = listing
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "));
    let main = paths
        .next()
        .and_then(|path| std::path::Path::new(path).canonicalize().ok())
        .ok_or("Cannot resolve the main Git worktree")?;
    if target == main || target == cwd_worktree {
        return Err("Refusing to remove the main or current worktree".to_string());
    }
    if !paths.any(|path| {
        std::path::Path::new(path).canonicalize().ok().as_deref() == Some(target.as_path())
    }) {
        return Err(
            "The path is not a registered secondary worktree of this repository".to_string(),
        );
    }
    let status = git_output(
        &target,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    if !status.status.success() || !status.stdout.is_empty() {
        return Err(
            "Worktree has uncommitted or untracked changes; save them before removal".to_string(),
        );
    }
    let ignored = git_output(&target, &["clean", "-ndx"])?;
    if !ignored.status.success() || !ignored.stdout.is_empty() {
        return Err("Worktree contains ignored files or its contents cannot be checked; remove them explicitly before retrying".to_string());
    }
    let target_head = git_output(&target, &["rev-parse", "HEAD"])?;
    if !target_head.status.success() {
        return Err("Cannot resolve worktree HEAD".to_string());
    }
    let oid = std::str::from_utf8(&target_head.stdout).map_err(|err| err.to_string())?;
    let merged = git_output(&main, &["merge-base", "--is-ancestor", oid.trim(), "HEAD"])?;
    if !merged.status.success() {
        return Err("Worktree has commits not contained in the main worktree HEAD; integrate them before removal".to_string());
    }
    Ok(target)
}

impl ToolInvocation for ExitWorktreeInvocation {
    fn get_description(&self) -> String {
        format!(
            "Exit Worktree: {} (only if already integrated and clean)",
            self.params.path
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
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let path = self.params.path.clone();
        Box::pin(async move {
            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                    title: "Exit Worktree?".to_string(),
                    prompt: format!(
                        "The agent wants to remove the isolated worktree at:\n\n{}\n\nRemoval will be refused if it has uncommitted, untracked or ignored files, or commits not present in the main worktree. No changes are merged or discarded automatically.",
                        path
                    ),
                    on_confirm: std::sync::Arc::new(move |outcome, _feedback| {
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

        Box::pin(async move {
            let path_clone = path.clone();

            let result = tokio::task::spawn_blocking(move || -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
                let cwd = std::env::current_dir().unwrap_or_default();
                let worktree_path = std::path::Path::new(&path_clone);

                // 目标不存在时直接失败，不触碰 worktree 登记信息。
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

                // 校验失败不删除，也不以 --force 重试。
                let validated = match validate_worktree_removal(&cwd, worktree_path) {
                    Ok(validated) => validated,
                    Err(reason) => {
                        return Ok(ToolResult {
                            llm_content: Some(reason.clone()),
                            return_display: Some("Worktree retained".to_string()),
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "worktree_not_safe_to_remove".to_string(),
                                message: reason,
                            }),
                            data: None,
                        });
                    }
                };

                let output = std::process::Command::new("git")
                    .args(["worktree", "remove", "--"])
                    .arg(&validated)
                    .current_dir(&cwd)
                    .output();

                match output {
                    Ok(out) if out.status.success() => {
                        Ok(ToolResult {
                            llm_content: Some(
                                "Worktree removed safely. No uncommitted files were discarded.".to_string(),
                            ),
                            return_display: Some("Worktree removed".to_string()),
                            output: format!("Removed worktree at: {}", path_clone),
                            error: None,
                            data: Some(serde_json::json!({
                                "removed_path": path_clone,
                                "changes_kept": true,
                                "worktree_active": false
                            })),
                        })
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        Ok(ToolResult {
                            llm_content: Some(format!(
                                "Failed to remove worktree at {}: {}. The worktree was not force-removed.",
                                path_clone, stderr
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) {
        let result = git_output(dir, args).expect("git must start");
        assert!(
            result.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn worktree_removal_preserves_uncommitted_ignored_and_unmerged_changes() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q"]);
        git(&main, &["config", "user.email", "test@example.invalid"]);
        git(&main, &["config", "user.name", "Test"]);
        std::fs::write(main.join("tracked"), "first").unwrap();
        std::fs::write(main.join(".gitignore"), "ignored\n").unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "-qm", "initial"]);
        let secondary = temp.path().join("secondary");
        git(
            &main,
            &["worktree", "add", "--detach", secondary.to_str().unwrap()],
        );

        assert!(validate_worktree_removal(&main, &main).is_err());
        assert_eq!(
            validate_worktree_removal(&main, &secondary).unwrap(),
            secondary
        );
        std::fs::write(secondary.join("tracked"), "changed").unwrap();
        assert!(validate_worktree_removal(&main, &secondary).is_err());
        std::fs::write(secondary.join("tracked"), "first").unwrap();
        std::fs::write(secondary.join("untracked"), "important").unwrap();
        assert!(validate_worktree_removal(&main, &secondary).is_err());
        std::fs::remove_file(secondary.join("untracked")).unwrap();
        std::fs::write(secondary.join("ignored"), "important").unwrap();
        assert!(validate_worktree_removal(&main, &secondary).is_err());
        std::fs::remove_file(secondary.join("ignored")).unwrap();
        std::fs::write(secondary.join("tracked"), "second").unwrap();
        git(&secondary, &["add", "tracked"]);
        git(&secondary, &["commit", "-qm", "new worktree commit"]);
        assert!(validate_worktree_removal(&main, &secondary).is_err());
        let stranger = temp.path().join("stranger");
        git(
            &main,
            &["worktree", "add", "--detach", stranger.to_str().unwrap()],
        );
        assert!(validate_worktree_removal(&stranger, &stranger).is_err());
        let oid = git_output(&secondary, &["rev-parse", "HEAD"]).unwrap();
        git(
            &main,
            &[
                "merge",
                "--ff-only",
                std::str::from_utf8(&oid.stdout).unwrap().trim(),
            ],
        );
        assert!(validate_worktree_removal(&main, &secondary).is_ok());
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        assert!(validate_worktree_removal(&main, &outside).is_err());
        let unrelated = temp.path().join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        git(&unrelated, &["init", "-q"]);
        assert!(validate_worktree_removal(&main, &unrelated).is_err());
        let validated = validate_worktree_removal(&main, &secondary).unwrap();
        let removed = std::process::Command::new("git")
            .args(["worktree", "remove", "--"])
            .arg(validated)
            .current_dir(&main)
            .output()
            .unwrap();
        assert!(
            removed.status.success(),
            "worktree removal failed: {}",
            String::from_utf8_lossy(&removed.stderr)
        );
        assert!(!secondary.exists());
    }
}
