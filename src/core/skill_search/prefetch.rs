/// 技能预取管理器
///
/// 预测并预取可能需要的技能
use super::SkillSearchConfig;

/// 技能预取管理器
pub struct SkillPrefetchManager {
    config: SkillSearchConfig,
    /// 已预取的技能ID
    prefetched: std::collections::HashSet<String>,
}

impl SkillPrefetchManager {
    pub fn new(config: SkillSearchConfig) -> Self {
        Self {
            config,
            prefetched: std::collections::HashSet::new(),
        }
    }

    /// 分析对话并预测需要的技能
    pub fn predict_skills(&mut self, messages: &[crate::types::StarMessage]) -> Vec<String> {
        if !self.config.prefetch_enabled {
            return Vec::new();
        }

        let mut predictions = Vec::new();

        // 分析最近的用户消息
        for msg in messages.iter().rev().take(3) {
            if msg.role == "user" {
                if let Some(content) = &msg.content {
                    let content_lower = content.to_lowercase();

                    // 基于关键词预测
                    if content_lower.contains("test") || content_lower.contains("run") {
                        predictions.push("run_tests".to_string());
                    }

                    if content_lower.contains("git") || content_lower.contains("commit") {
                        predictions.push("git_commit".to_string());
                    }

                    if content_lower.contains("search") || content_lower.contains("find") {
                        predictions.push("semantic_search".to_string());
                    }
                }
            }
        }

        // 去重并过滤已预取的
        predictions.sort();
        predictions.dedup();
        predictions.retain(|id| !self.prefetched.contains(id));

        // 记录已预取
        for id in &predictions {
            self.prefetched.insert(id.clone());
        }

        predictions
    }

    /// 重置预取状态
    pub fn reset(&mut self) {
        self.prefetched.clear();
    }
}
