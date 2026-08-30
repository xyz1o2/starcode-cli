/// Auto Dream配置

use serde::{Deserialize, Serialize};

/// Auto Dream配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoDreamConfig {
    /// 是否启用
    pub enabled: bool,
    /// 整合间隔（秒）
    pub consolidation_interval_secs: u64,
    /// 最小记忆数
    pub min_memories: u32,
    /// 最大记忆数
    pub max_memories: u32,
    /// 是否启用自动整合
    pub auto_consolidation: bool,
    /// 整合提示词模板
    pub prompt_template: Option<String>,
}

impl Default for AutoDreamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            consolidation_interval_secs: 3600, // 1小时
            min_memories: 5,
            max_memories: 100,
            auto_consolidation: false,
            prompt_template: None,
        }
    }
}

impl AutoDreamConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_AUTO_DREAM_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let consolidation_interval_secs = std::env::var("STAR_AUTO_DREAM_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        let min_memories = std::env::var("STAR_AUTO_DREAM_MIN_MEMORIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let max_memories = std::env::var("STAR_AUTO_DREAM_MAX_MEMORIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let auto_consolidation = std::env::var("STAR_AUTO_DREAM_AUTO")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            enabled,
            consolidation_interval_secs,
            min_memories,
            max_memories,
            auto_consolidation,
            prompt_template: None,
        }
    }
}
