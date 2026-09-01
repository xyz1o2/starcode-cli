/// 技能搜索系统
/// 
/// 对标claude-code-main的src/services/skillSearch/
/// 提供技能发现、意图归一化和预取功能

pub mod intent_normalize;
pub mod local_search;
pub mod prefetch;

pub use intent_normalize::IntentNormalizer;
pub use local_search::LocalSkillSearch;
pub use prefetch::SkillPrefetchManager;

use serde::{Deserialize, Serialize};

/// 技能搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSearchResult {
    /// 技能ID
    pub skill_id: String,
    /// 技能名称
    pub name: String,
    /// 相关性分数
    pub relevance_score: f64,
    /// 匹配原因
    pub match_reason: String,
    /// 技能描述
    pub description: Option<String>,
}

/// 技能搜索配置
#[derive(Debug, Clone)]
pub struct SkillSearchConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大结果数
    pub max_results: usize,
    /// 最小相关性分数
    pub min_relevance: f64,
    /// 是否启用预取
    pub prefetch_enabled: bool,
}

impl Default for SkillSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_results: 10,
            min_relevance: 0.3,
            prefetch_enabled: true,
        }
    }
}

impl SkillSearchConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_SKILL_SEARCH_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_results = std::env::var("STAR_SKILL_SEARCH_MAX_RESULTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let min_relevance = std::env::var("STAR_SKILL_SEARCH_MIN_RELEVANCE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.3);

        let prefetch_enabled = std::env::var("STAR_SKILL_PREFETCH_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            enabled,
            max_results,
            min_relevance,
            prefetch_enabled,
        }
    }
}
