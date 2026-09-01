/// 提示词建议系统
///
/// 对标claude-code-main的src/services/PromptSuggestion/
/// 基于上下文提供智能提示词建议
use serde::{Deserialize, Serialize};

/// 提示词建议
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSuggestion {
    /// 建议ID
    pub id: String,
    /// 建议文本
    pub text: String,
    /// 相关性分数
    pub relevance_score: f64,
    /// 建议类型
    pub suggestion_type: SuggestionType,
    /// 上下文
    pub context: Option<String>,
}

/// 建议类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestionType {
    /// 基于历史
    History,
    /// 基于上下文
    Context,
    /// 基于模板
    Template,
    /// 基于技能
    Skill,
}

/// 提示词建议管理器
pub struct PromptSuggestionManager {
    /// 历史提示词
    history: Vec<String>,
    /// 最大历史数
    max_history: usize,
    /// 模板列表
    templates: Vec<PromptTemplate>,
}

/// 提示词模板
#[derive(Debug, Clone)]
struct PromptTemplate {
    /// 模板ID
    id: String,
    /// 模板模式
    pattern: String,
    /// 模板内容
    template: String,
    /// 使用场景
    use_case: String,
}

impl PromptSuggestionManager {
    pub fn new(max_history: usize) -> Self {
        let mut manager = Self {
            history: Vec::new(),
            max_history,
            templates: Vec::new(),
        };

        manager.load_default_templates();
        manager
    }

    /// 加载默认模板
    fn load_default_templates(&mut self) {
        self.templates.push(PromptTemplate {
            id: "explain".to_string(),
            pattern: "explain".to_string(),
            template: "Explain how {} works in this codebase".to_string(),
            use_case: "Code explanation".to_string(),
        });

        self.templates.push(PromptTemplate {
            id: "fix".to_string(),
            pattern: "fix".to_string(),
            template: "Fix the error in {}: {}".to_string(),
            use_case: "Error fixing".to_string(),
        });

        self.templates.push(PromptTemplate {
            id: "test".to_string(),
            pattern: "test".to_string(),
            template: "Write tests for {}".to_string(),
            use_case: "Test writing".to_string(),
        });

        self.templates.push(PromptTemplate {
            id: "refactor".to_string(),
            pattern: "refactor".to_string(),
            template: "Refactor {} to improve {}".to_string(),
            use_case: "Code refactoring".to_string(),
        });
    }

    /// 记录提示词
    pub fn record_prompt(&mut self, prompt: &str) {
        self.history.push(prompt.to_string());
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    /// 获取建议
    pub fn get_suggestions(&self, current_input: &str) -> Vec<PromptSuggestion> {
        let mut suggestions = Vec::new();
        let input_lower = current_input.to_lowercase();

        // 1. 基于历史的建议
        for (i, history_item) in self.history.iter().rev().enumerate() {
            if history_item.to_lowercase().starts_with(&input_lower) && !input_lower.is_empty() {
                suggestions.push(PromptSuggestion {
                    id: format!("history_{}", i),
                    text: history_item.clone(),
                    relevance_score: 1.0 - (i as f64 * 0.1),
                    suggestion_type: SuggestionType::History,
                    context: None,
                });
            }
        }

        // 2. 基于模板的建议
        for template in &self.templates {
            if input_lower.contains(&template.pattern) {
                suggestions.push(PromptSuggestion {
                    id: template.id.clone(),
                    text: template.template.clone(),
                    relevance_score: 0.8,
                    suggestion_type: SuggestionType::Template,
                    context: Some(template.use_case.clone()),
                });
            }
        }

        // 按相关性排序
        suggestions.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap());
        suggestions.truncate(5);
        suggestions
    }
}
