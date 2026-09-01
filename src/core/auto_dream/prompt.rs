/// 整合提示词

/// 整合提示词
pub struct ConsolidationPrompt {
    /// 提示词模板
    template: String,
}

impl ConsolidationPrompt {
    /// 创建新的整合提示词
    pub fn new() -> Self {
        Self {
            template: Self::default_template(),
        }
    }

    /// 默认模板
    fn default_template() -> String {
        r#"You are a memory consolidation assistant. Your task is to analyze and consolidate the following memories from multiple coding sessions.

Please:
1. Identify common themes and patterns
2. Extract key insights and learnings
3. Create a concise summary that captures the most important information
4. Highlight any recurring issues or solutions

Memories to consolidate:
{memories}

Please provide:
1. A brief summary of the consolidated memories
2. Key themes identified
3. Important insights or patterns
4. Recommendations for future reference"#.to_string()
    }

    /// 生成提示词
    pub fn generate(&self, memories: &[String]) -> String {
        let memories_text = memories.join("\n---\n");
        self.template.replace("{memories}", &memories_text)
    }

    /// 设置自定义模板
    pub fn set_template(&mut self, template: String) {
        self.template = template;
    }
}
