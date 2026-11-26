/// 目标持久化
use super::Goal;
use crate::core::config::storage::Storage;
use std::path::{Path, PathBuf};

/// 解析目标与循环共用的项目状态目录，不创建目录或迁移已有文件。
///
/// Storage 优先继承最近的现有项目 `.star`；全新 Git 项目还没有状态目录时，
/// 回退到最近的 Git 根，避免首次从子目录访问就把状态固定在子目录。
pub(crate) fn project_state_dir(cwd: &Path) -> PathBuf {
    let storage = Storage::new(cwd.to_path_buf());
    let state_dir = storage.star_dir();
    if state_dir.exists() {
        return state_dir;
    }

    let root = crate::core::utils::paths::find_project_file_upwards(cwd, &[".git"])
        .and_then(|marker| marker.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| cwd.to_path_buf());
    Storage::new(root).star_dir()
}

/// 解析项目共享的目标文件路径；读取不创建状态目录。
pub(crate) fn project_goals_path(cwd: &Path) -> PathBuf {
    project_state_dir(cwd).join("goals.json")
}

/// 目标持久化管理器
pub struct GoalPersistence {
    /// 存储路径
    storage_path: PathBuf,
}

impl GoalPersistence {
    /// 创建新的目标持久化管理器
    pub fn new(storage_path: Option<&str>) -> Self {
        let path = storage_path.map(|p| PathBuf::from(p)).unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".starcode")
                .join("goals.json")
        });

        Self { storage_path: path }
    }

    /// 加载目标
    pub fn load(&self) -> Result<Vec<Goal>, String> {
        if !self.storage_path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.storage_path)
            .map_err(|e| format!("Failed to read goals file: {}", e))?;

        let goals: Vec<Goal> =
            serde_json::from_str(&content).map_err(|e| format!("Failed to parse goals: {}", e))?;

        Ok(goals)
    }

    /// 保存目标
    pub fn save(&self, goals: &[&Goal]) -> Result<(), String> {
        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let content = serde_json::to_string_pretty(goals)
            .map_err(|e| format!("Failed to serialize goals: {}", e))?;

        std::fs::write(&self.storage_path, content)
            .map_err(|e| format!("Failed to write goals file: {}", e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_project_state_is_shared_with_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("src/nested");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join(".star")).unwrap();
        std::fs::create_dir_all(&child).unwrap();

        let expected = dir.path().join(".star/goals.json");
        assert_eq!(project_goals_path(dir.path()), expected);
        assert_eq!(project_goals_path(&child), expected);
        assert!(!child.join(".star").exists());
    }

    #[test]
    fn fresh_git_project_uses_its_root_without_creating_state() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("src/nested");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(&child).unwrap();

        let path = project_goals_path(&child);
        assert_eq!(path, dir.path().join(".star/goals.json"));
        assert_eq!(project_goals_path(dir.path()), path);
        let persistence = GoalPersistence::new(Some(path.to_string_lossy().as_ref()));
        assert!(persistence.load().unwrap().is_empty());
        assert!(!dir.path().join(".star").exists());
        assert!(!child.join(".star").exists());
    }

    #[test]
    fn fresh_worktree_does_not_inherit_an_outer_projects_state() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        let child = worktree.join("src/nested");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join(".star")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../.git/worktrees/test\n").unwrap();

        assert_eq!(
            project_goals_path(&child),
            worktree.join(".star/goals.json")
        );
        assert!(!worktree.join(".star").exists());
        assert!(!child.join(".star").exists());
    }

    #[test]
    fn existing_nested_state_is_not_migrated_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        let child = nested.join("src");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join(".star")).unwrap();
        std::fs::create_dir_all(nested.join(".star")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        let outer_path = dir.path().join(".star/goals.json");
        let nested_path = nested.join(".star/goals.json");
        std::fs::write(&outer_path, "outer state").unwrap();
        std::fs::write(&nested_path, "nested state").unwrap();

        assert_eq!(project_goals_path(&child), nested_path);
        assert_eq!(std::fs::read_to_string(outer_path).unwrap(), "outer state");
        assert_eq!(
            std::fs::read_to_string(nested_path).unwrap(),
            "nested state"
        );
        assert!(!child.join(".star").exists());
    }
}
