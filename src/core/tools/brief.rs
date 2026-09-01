use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;

#[derive(Clone)]
pub struct BriefTool;

impl BriefTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BriefParams {
    pub content: String,
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

fn default_max_length() -> usize {
    200
}

pub struct BriefInvocation {
    params: BriefParams,
}

impl ToolInvocation for BriefInvocation {
    fn get_description(&self) -> String {
        format!(
            "Summarize content (max {} chars)",
            self.params.max_length
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let max_len = params.max_length.max(50);
            let content = params.content.trim();

            if content.len() <= max_len {
                return Ok(ToolResult {
                    llm_content: Some(content.to_string()),
                    return_display: Some("Brief (no truncation needed)".to_string()),
                    output: content.to_string(),
                    error: None,
                    data: None,
                });
            }

            let mut brief = String::with_capacity(max_len);
            let chars: Vec<char> = content.chars().collect();
            let mut char_count = 0;

            for ch in &chars {
                if char_count + 4 >= max_len {
                    brief.push_str("...");
                    break;
                }
                brief.push(*ch);
                char_count += 1;
            }

            let original_len = chars.len();
            let summary = format!(
                "({} chars → {} chars)\n{}",
                original_len,
                brief.len(),
                brief
            );

            Ok(ToolResult {
                llm_content: Some(summary.clone()),
                return_display: Some(format!(
                    "Brief: {} → {} chars",
                    original_len,
                    brief.len()
                )),
                output: summary,
                error: None,
                data: Some(serde_json::json!({
                    "original_length": original_len,
                    "brief_length": brief.len(),
                    "truncated": true,
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for BriefTool {
    fn name(&self) -> &str {
        "brief"
    }

    fn display_name(&self) -> &str {
        "Brief"
    }

    fn description(&self) -> &str {
        "返回内容的简短摘要，超过最大长度时截断。(Return a brief summary of content, truncating if it exceeds max_length.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "要摘要的内容 (Content to summarize)"
                },
                "max_length": {
                    "type": "integer",
                    "description": "最大长度，默认200 (Maximum length, default: 200)"
                }
            },
            "required": ["content"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: BriefParams = serde_json::from_value(params)?;
        Ok(Box::new(BriefInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
