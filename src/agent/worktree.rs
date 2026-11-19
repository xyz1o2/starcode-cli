//! Worktree 隔离模块
//!
//! 对标 Claude Code 的 worktree-isolation.mdx：
//! - Git worktree 创建和管理
//! - EnterWorktreeTool / ExitWorktreeTool
//! - 安全防护（防止主工作区污染）

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Worktree 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub id: String,
    pub path: PathBuf,
    pub branch: String,
    pub parent_path: PathBuf,
    pub created_at: u64,
    pub status: WorktreeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeStatus {
    Active,
    Detached,
    Merged,
    Abandoned,
}

/// Worktree 管理器
pub struct WorktreeManager {
    base_dir: PathBuf,
    worktrees: Vec<WorktreeInfo>,
}

impl WorktreeManager {
    pub fn new(project_root: &Path) -> Self {
        let base_dir = project_root.join(".star").join("worktrees");
        Self {
            base_dir,
            worktrees: Vec::new(),
        }
    }

    /// 创建 worktree
    pub fn create_worktree(
        &self,
        branch_name: &str,
        from_branch: Option<&str>,
    ) -> Result<WorktreeInfo, String> {
        let worktree_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let worktree_path = self.base_dir.join(&worktree_id);

        // 确保目录存在
        std::fs::create_dir_all(&self.base_dir)
            .map_err(|e| format!("Failed to create worktree base dir: {}", e))?;

        // 创建分支（如果不存在）
        let from = from_branch.unwrap_or("HEAD");

        // git worktree add
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch_name,
                &worktree_path.to_string_lossy(),
                from,
            ])
            .output()
            .map_err(|e| format!("Failed to run git worktree: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git worktree add failed: {}", stderr));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(WorktreeInfo {
            id: worktree_id,
            path: worktree_path,
            branch: branch_name.to_string(),
            parent_path: std::env::current_dir().unwrap_or_default(),
            created_at: now,
            status: WorktreeStatus::Active,
        })
    }

    /// 移除 worktree
    pub fn remove_worktree(&self, worktree_path: &Path) -> Result<(), String> {
        let output = Command::new("git")
            .args(["worktree", "remove", &worktree_path.to_string_lossy()])
            .output()
            .map_err(|e| format!("Failed to remove worktree: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git worktree remove failed: {}", stderr));
        }

        Ok(())
    }

    /// 列出所有 worktree
    pub fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>, String> {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .output()
            .map_err(|e| format!("Failed to list worktrees: {}", e))?;

        if !output.status.success() {
            return Err("git worktree list failed".to_string());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current_path = None;
        let mut current_branch = None;

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                current_path = Some(PathBuf::from(&line[9..]));
            } else if line.starts_with("branch ") {
                current_branch = Some(line[7..].to_string());
            } else if line.is_empty() {
                if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                    worktrees.push(WorktreeInfo {
                        id: path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path,
                        branch,
                        parent_path: PathBuf::new(),
                        created_at: 0,
                        status: WorktreeStatus::Active,
                    });
                }
            }
        }

        // Handle last entry
        if let (Some(path), Some(branch)) = (current_path, current_branch) {
            worktrees.push(WorktreeInfo {
                id: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path,
                branch,
                parent_path: PathBuf::new(),
                created_at: 0,
                status: WorktreeStatus::Active,
            });
        }

        Ok(worktrees)
    }

    /// 合并 worktree 分支到主分支
    pub fn merge_worktree(&self, branch: &str, target: &str) -> Result<(), String> {
        let output = Command::new("git")
            .args(["merge", "--no-ff", branch])
            .output()
            .map_err(|e| format!("Failed to merge: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git merge failed: {}", stderr));
        }

        Ok(())
    }

    /// 获取 worktree 的 diff
    pub fn get_diff(&self, worktree_path: &Path) -> Result<String, String> {
        let output = Command::new("git")
            .args(["diff"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to get diff: {}", e))?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// 获取 worktree 的状态
    pub fn get_status(&self, worktree_path: &Path) -> Result<String, String> {
        let output = Command::new("git")
            .args(["status", "--short"])
            .current_dir(worktree_path)
            .output()
            .map_err(|e| format!("Failed to get status: {}", e))?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// 清理所有 worktree
    pub fn cleanup_all(&self) -> Result<(), String> {
        let output = Command::new("git")
            .args(["worktree", "prune"])
            .output()
            .map_err(|e| format!("Failed to prune worktrees: {}", e))?;

        if !output.status.success() {
            return Err("git worktree prune failed".to_string());
        }

        Ok(())
    }
}

/// 单次子代理的 Git 工作目录；保留有改动或提交的结果，不改变进程 cwd。
pub(crate) struct AgentWorktree {
    pub path: PathBuf,
    project_root: PathBuf,
    initial_head: String,
}

/// 清除继承自启动环境的仓库定位，确保 Git 按子命令 cwd 发现工作树。
/// 只在工作树管理命令和已绑定独立工作目录的子代理 Shell 中使用。
pub(crate) fn clear_git_repository_env(command: &mut tokio::process::Command) {
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_SHALLOW_FILE",
        "GIT_QUARANTINE_PATH",
    ] {
        command.env_remove(name);
    }
}

impl AgentWorktree {
    async fn git(root: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
        let mut command = tokio::process::Command::new("git");
        clear_git_repository_env(&mut command);
        let output = command
            .current_dir(root)
            .args(args)
            .output()
            .await
            .map_err(|error| format!("Failed to execute git in {}: {error}", root.display()))?;
        if !output.status.success() {
            return Err(format!(
                "Git failed in {}: {}",
                root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    pub(crate) async fn create(project_root: &Path) -> Result<Self, String> {
        let root = Self::git(
            project_root,
            &["rev-parse".as_ref(), "--show-toplevel".as_ref()],
        )
        .await?;
        let project_root = PathBuf::from(root);
        let initial_head =
            Self::git(&project_root, &["rev-parse".as_ref(), "HEAD".as_ref()]).await?;
        let base = crate::core::config::storage::Storage::new(project_root.clone())
            .star_dir()
            .join("worktrees");
        tokio::fs::create_dir_all(&base)
            .await
            .map_err(|error| format!("Failed to create {}: {error}", base.display()))?;
        let path = base.join(format!("agent-{}", uuid::Uuid::new_v4()));
        Self::git(
            &project_root,
            &[
                "worktree".as_ref(),
                "add".as_ref(),
                "--detach".as_ref(),
                path.as_os_str(),
                initial_head.as_ref(),
            ],
        )
        .await?;
        Ok(Self {
            path,
            project_root,
            initial_head,
        })
    }

    /// 只有 HEAD 未移动且包含未跟踪文件的状态为空，才移除本次创建的工作树。
    pub(crate) async fn finish(&self) -> Result<bool, String> {
        let head = Self::git(&self.path, &["rev-parse".as_ref(), "HEAD".as_ref()]).await?;
        let status = Self::git(
            &self.path,
            &[
                "status".as_ref(),
                "--porcelain".as_ref(),
                "--untracked-files=all".as_ref(),
            ],
        )
        .await?;
        if head != self.initial_head || !status.is_empty() {
            return Ok(false);
        }
        Self::git(
            &self.project_root,
            &[
                "worktree".as_ref(),
                "remove".as_ref(),
                self.path.as_os_str(),
            ],
        )
        .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod agent_worktree_tests {
    use super::AgentWorktree;
    use std::process::Command;

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git must be available for worktree regression");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// 在子命令上模拟污染环境，不修改测试进程的全局环境。
    #[tokio::test]
    async fn inherited_git_locations_do_not_redirect_an_isolated_shell() {
        let repository = tempfile::tempdir().expect("temporary repository");
        git(repository.path(), &["init", "-q"]);
        git(
            repository.path(),
            &["config", "user.email", "starcode-tests@example.invalid"],
        );
        git(
            repository.path(),
            &["config", "user.name", "StarCode Tests"],
        );
        std::fs::write(repository.path().join("tracked.txt"), "base").unwrap();
        git(repository.path(), &["add", "tracked.txt"]);
        git(repository.path(), &["commit", "-qm", "initial"]);
        let worktree = AgentWorktree::create(repository.path()).await.unwrap();
        let mut command = tokio::process::Command::new("git");
        command
            .current_dir(&worktree.path)
            .args(["rev-parse", "--show-toplevel"]);
        command.env("GIT_DIR", repository.path().join(".git"));
        command.env("GIT_WORK_TREE", repository.path());
        command.env("GIT_INDEX_FILE", repository.path().join(".git/index"));
        command.env("GIT_COMMON_DIR", repository.path().join(".git"));
        command.env(
            "GIT_OBJECT_DIRECTORY",
            repository.path().join(".git/objects"),
        );
        super::clear_git_repository_env(&mut command);
        let output = command.output().await.expect("git probe");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        assert_eq!(
            actual.canonicalize().unwrap(),
            worktree.path.canonicalize().unwrap()
        );
        // 走真实 Shell invocation，验证默认 cwd 及相对写入确实落在工作树。
        let mut config = crate::core::config::Config::new(crate::core::config::ConfigParameters {
            target_dir: repository.path().to_owned(),
            ..Default::default()
        });
        config.agent_working_dir = Some(worktree.path.clone());
        let shell = crate::core::tools::shell::ShellToolInvocation::new(
            std::sync::Arc::new(config),
            crate::core::tools::shell::ShellToolParams {
                command: "git rev-parse --show-toplevel && printf 'child only' > tracked.txt"
                    .into(),
                description: None,
                dir_path: None,
                dangerously_disable_sandbox: false,
                run_in_background: Some(false),
            },
            None,
            None,
            None,
        );
        let result = crate::core::tools::tools::ToolInvocation::execute(&shell, None, None)
            .await
            .expect("isolated shell should execute");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(
            result.output.contains(worktree.path.to_str().unwrap()),
            "{}",
            result.output
        );
        assert_eq!(
            std::fs::read_to_string(worktree.path.join("tracked.txt")).unwrap(),
            "child only"
        );
        assert_eq!(
            std::fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
            "base"
        );
        assert!(!worktree.finish().await.unwrap());
        git(
            repository.path(),
            &["worktree", "remove", "-f", worktree.path.to_str().unwrap()],
        );
    }

    #[tokio::test]
    async fn clean_worktree_is_removed_but_dirty_and_committed_results_are_retained() {
        let repository = tempfile::tempdir().expect("temporary repository");
        git(repository.path(), &["init", "-q"]);
        git(
            repository.path(),
            &["config", "user.email", "starcode-tests@example.invalid"],
        );
        git(
            repository.path(),
            &["config", "user.name", "StarCode Tests"],
        );
        std::fs::write(repository.path().join("tracked.txt"), "base").unwrap();
        git(repository.path(), &["add", "tracked.txt"]);
        git(repository.path(), &["commit", "-qm", "initial"]);

        let clean = AgentWorktree::create(repository.path()).await.unwrap();
        let clean_path = clean.path.clone();
        assert_eq!(clean.finish().await.unwrap(), true);
        assert!(!clean_path.exists(), "clean worktree should be removed");

        let dirty = AgentWorktree::create(repository.path()).await.unwrap();
        let dirty_path = dirty.path.clone();
        std::fs::write(dirty_path.join("new.txt"), "keep").unwrap();
        assert_eq!(dirty.finish().await.unwrap(), false);
        assert!(dirty_path.exists(), "dirty worktree must remain available");
        git(
            repository.path(),
            &["worktree", "remove", "-f", dirty_path.to_str().unwrap()],
        );

        let committed = AgentWorktree::create(repository.path()).await.unwrap();
        let committed_path = committed.path.clone();
        std::fs::write(committed_path.join("tracked.txt"), "changed").unwrap();
        git(&committed_path, &["add", "tracked.txt"]);
        git(&committed_path, &["commit", "-qm", "agent result"]);
        assert_eq!(committed.finish().await.unwrap(), false);
        assert!(
            committed_path.exists(),
            "committed result must remain available"
        );
        git(
            repository.path(),
            &["worktree", "remove", "-f", committed_path.to_str().unwrap()],
        );
    }
}

/// Worktree 安全防护
pub struct WorktreeSafety {
    /// 禁止在主 worktree 中直接修改的路径
    protected_paths: Vec<PathBuf>,
}

impl WorktreeSafety {
    pub fn new() -> Self {
        Self {
            protected_paths: vec![
                PathBuf::from(".git"),
                PathBuf::from(".star"),
                PathBuf::from("Cargo.lock"),
                PathBuf::from("package-lock.json"),
            ],
        }
    }

    /// 检查路径是否受保护
    pub fn is_protected(&self, path: &Path) -> bool {
        self.protected_paths
            .iter()
            .any(|protected| path.starts_with(protected))
    }

    /// 检查操作是否安全
    pub fn check_operation(&self, operation: &str, path: &Path) -> Result<(), String> {
        if self.is_protected(path) {
            return Err(format!(
                "Operation '{}' on protected path '{}' is not allowed in worktree context",
                operation,
                path.display()
            ));
        }
        Ok(())
    }
}
