/// 目标持久化
use super::Goal;
use std::path::PathBuf;

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
