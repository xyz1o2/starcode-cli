/// 本地技能搜索
///
/// 在本地技能索引中搜索
use super::{SkillSearchConfig, SkillSearchResult};

/// 本地技能搜索
pub struct LocalSkillSearch {
    config: SkillSearchConfig,
}

impl LocalSkillSearch {
    pub fn new(config: SkillSearchConfig) -> Self {
        Self { config }
    }

    /// 搜索技能
    pub fn search(&self, query: &str, skills: &[SkillInfo]) -> Vec<SkillSearchResult> {
        if !self.config.enabled {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let mut results: Vec<SkillSearchResult> = skills
            .iter()
            .filter_map(|skill| {
                let name_score =
                    self.calculate_similarity(&query_lower, &skill.name.to_lowercase());
                let desc_score = skill
                    .description
                    .as_ref()
                    .map(|d| self.calculate_similarity(&query_lower, &d.to_lowercase()))
                    .unwrap_or(0.0);

                let score = name_score.max(desc_score);

                if score >= self.config.min_relevance {
                    Some(SkillSearchResult {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        relevance_score: score,
                        match_reason: if name_score > desc_score {
                            "name match".to_string()
                        } else {
                            "description match".to_string()
                        },
                        description: skill.description.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());
        results.truncate(self.config.max_results);
        results
    }

    /// 计算字符串相似度
    fn calculate_similarity(&self, s1: &str, s2: &str) -> f64 {
        if s1.is_empty() || s2.is_empty() {
            return 0.0;
        }

        if s1 == s2 {
            return 1.0;
        }

        if s2.contains(s1) {
            return 0.8;
        }

        let common_words: usize = s1.split_whitespace().filter(|w| s2.contains(w)).count();

        let total_words = s1.split_whitespace().count().max(1);
        common_words as f64 / total_words as f64
    }
}

/// 技能信息
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}
