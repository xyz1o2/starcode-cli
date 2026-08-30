/// 记忆路径管理

use std::path::PathBuf;

/// 记忆路径管理器
pub struct MemoryPaths {
    /// 基础目录
    base_dir: PathBuf,
}

impl MemoryPaths {
    /// 创建新的记忆路径管理器
    pub fn new() -> Self {
        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".starcode")
            .join("memory");

        Self { base_dir }
    }

    /// 获取基础目录
    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    /// 获取项目记忆目录
    pub fn project_memory_dir(&self, project_path: &str) -> PathBuf {
        let project_name = std::path::Path::new(project_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("default");

        self.base_dir.join("projects").join(project_name)
    }

    /// 获取用户记忆目录
    pub fn user_memory_dir(&self) -> PathBuf {
        self.base_dir.join("user")
    }

    /// 获取团队记忆目录
    pub fn team_memory_dir(&self, team_id: &str) -> PathBuf {
        self.base_dir.join("teams").join(team_id)
    }

    /// 确保目录存在
    pub fn ensure_dir_exists(&self, path: &PathBuf) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }
}
