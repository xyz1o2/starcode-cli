/// 学习策略
/// 
/// 定义技能学习的策略

use serde::{Deserialize, Serialize};

/// 学习策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningPolicy {
    /// 最小观察次数
    pub min_observations: u32,
    /// 最小成功率
    pub min_success_rate: f64,
    /// 最大技能数
    pub max_skills: u32,
    /// 技能过期时间（秒）
    pub skill_expiry_secs: i64,
    /// 是否启用自动学习
    pub auto_learning_enabled: bool,
    /// 是否启用技能进化
    pub evolution_enabled: bool,
}

impl Default for LearningPolicy {
    fn default() -> Self {
        Self {
            min_observations: 3,
            min_success_rate: 0.7,
            max_skills: 100,
            skill_expiry_secs: 30 * 24 * 3600, // 30天
            auto_learning_enabled: true,
            evolution_enabled: true,
        }
    }
}

impl LearningPolicy {
    /// 从环境变量加载策略
    pub fn from_env() -> Self {
        let min_observations = std::env::var("STAR_SKILL_MIN_OBSERVATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let min_success_rate = std::env::var("STAR_SKILL_MIN_SUCCESS_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7);

        let max_skills = std::env::var("STAR_SKILL_MAX_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let skill_expiry_secs = std::env::var("STAR_SKILL_EXPIRY_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .map(|days| days * 24 * 3600)
            .unwrap_or(30 * 24 * 3600);

        let auto_learning_enabled = std::env::var("STAR_SKILL_AUTO_LEARNING")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let evolution_enabled = std::env::var("STAR_SKILL_EVOLUTION")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            min_observations,
            min_success_rate,
            max_skills,
            skill_expiry_secs,
            auto_learning_enabled,
            evolution_enabled,
        }
    }
}
