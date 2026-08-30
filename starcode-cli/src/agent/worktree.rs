//! Worktree 隔离模块
//!
//! 对标 Claude Code 的 worktree-isolation.mdx：
//! - Git worktree 创建和管理
//! - EnterWorktreeTool / ExitWorktreeTool
//! - 安全防护（防止主工作区污染）

use serde::{Serialize, Deserialize};
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
