/// 整合引擎

use super::{ConsolidationResult, DreamError};

/// 整合引擎
pub struct ConsolidationEngine {
    /// 整合计数器
    consolidation_count: u32,
}

impl ConsolidationEngine {
    /// 创建新的整合引擎
    pub fn new() -> Self {
        Self {
            consolidation_count: 0,
        }
    }

    /// 整合记忆
    pub fn consolidate(&mut self, memories: &[String]) -> Result<ConsolidationResult, DreamError> {
        if memories.is_empty() {
            return Err(DreamError::ConsolidationError("No memories to consolidate".to_string()));
        }

        self.consolidation_count += 1;

        // 提取关键主题
        let themes = self.extract_themes(memories);
        
        // 生成洞察
        let insights = self.generate_insights(memories, &themes);
        
        // 生成摘要
        let summary = self.generate_summary(memories, &themes);

        Ok(ConsolidationResult {
            id: uuid::Uuid::new_v4().to_string(),
            memories_consolidated: memories.len() as u32,
            insights_generated: insights.len() as u32,
            consolidated_at: chrono::Utc::now().timestamp(),
            summary,
            insights,
        })
    }

    /// 提取主题
    fn extract_themes(&self, memories: &[String]) -> Vec<String> {
        let mut themes = Vec::new();
        
        // 简单的主题提取：查找常见关键词
        let keywords = ["error", "fix", "feature", "bug", "test", "refactor", "performance"];
        
        for keyword in &keywords {
            let count = memories.iter()
                .filter(|m| m.to_lowercase().contains(keyword))
                .count();
            
            if count >= 2 {
                themes.push(keyword.to_string());
            }
        }

        themes
    }

    /// 生成洞察
    fn generate_insights(&self, memories: &[String], themes: &[String]) -> Vec<String> {
        let mut insights = Vec::new();

        // 基于主题生成洞察
        for theme in themes {
            let count = memories.iter()
                .filter(|m| m.to_lowercase().contains(theme))
                .count();
            
            insights.push(format!("Theme '{}' appeared {} times across memories", theme, count));
        }

        // 添加通用洞察
        if memories.len() > 10 {
            insights.push("Large number of memories collected - consider periodic review".to_string());
        }

        insights
    }

    /// 生成摘要
    fn generate_summary(&self, memories: &[String], themes: &[String]) -> String {
        format!(
            "Consolidated {} memories with themes: {}",
            memories.len(),
            if themes.is_empty() {
                "none identified".to_string()
            } else {
                themes.join(", ")
            }
        )
    }
}
