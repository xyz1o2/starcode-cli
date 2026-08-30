use crate::types::StarMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub suggestion_type: SuggestionType,
    pub message: String,
    pub action: Option<String>,
    pub priority: SuggestionPriority,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SuggestionType {
    ToolSuggestion,
    ContextSuggestion,
    ActionSuggestion,
    WarningSuggestion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuggestionPriority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveSuggestions {
    pub enabled: bool,
    pub suggestions: Vec<Suggestion>,
    pub last_check: Option<i64>,
}

impl ProactiveSuggestions {
    pub fn new() -> Self {
        Self {
            enabled: true,
            suggestions: Vec::new(),
            last_check: None,
        }
    }

    pub fn analyze_context(&mut self, messages: &[StarMessage], tools_used: &[String]) {
        if !self.enabled {
            return;
        }

        self.suggestions.clear();

        if let Some(last_msg) = messages.last() {
            if tools_used
                .iter()
                .any(|t| t.contains("edit") || t.contains("write"))
            {
                if !tools_used.iter().any(|t| t.contains("test")) {
                    self.suggestions.push(Suggestion {
                        id: "run_tests".to_string(),
                        suggestion_type: SuggestionType::ActionSuggestion,
                        message: "You've made code changes. Consider running tests.".to_string(),
                        action: Some("run_tests".to_string()),
                        priority: SuggestionPriority::Medium,
                        timestamp: chrono::Utc::now().timestamp(),
                    });
                }
            }

            if let Some(content) = &last_msg.content {
                if content.contains("import") || content.contains("use ") {
                    self.suggestions.push(Suggestion {
                        id: "read_related".to_string(),
                        suggestion_type: SuggestionType::ContextSuggestion,
                        message: "Consider reading related files for context.".to_string(),
                        action: Some("Read".to_string()),
                        priority: SuggestionPriority::Low,
                        timestamp: chrono::Utc::now().timestamp(),
                    });
                }
            }
        }

        self.last_check = Some(chrono::Utc::now().timestamp());
    }

    pub fn get_active_suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    pub fn dismiss_suggestion(&mut self, id: &str) {
        self.suggestions.retain(|s| s.id != id);
    }

    pub fn clear_all(&mut self) {
        self.suggestions.clear();
    }
}
