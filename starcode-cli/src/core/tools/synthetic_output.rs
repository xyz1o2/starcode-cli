use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SyntheticOutputTool;

impl SyntheticOutputTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SyntheticOutputParams {
    pub output: serde_json::Value,
}

#[derive(Debug, Serialize, Clone)]
pub struct SyntheticOutputOutput {
    pub output: serde_json::Value,
}

pub struct SyntheticOutputInvocation {
    params: SyntheticOutputParams,
}

impl ToolInvocation for SyntheticOutputInvocation {
    fn get_description(&self) -> String {
        "Return structured output".to_string()
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
            let output = params.output.clone();

            // In a real implementation, this would:
            // 1. Validate the output against the schema
            // 2. Return the structured output

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some(format!("Structured output: {}", output)),
                return_display: Some("Structured output returned".to_string()),
                output: serde_json::to_string(&SyntheticOutputOutput {
                    output: output.clone(),
                })?,
                error: None,
                data: Some(output),
            })
        })
    }
}

impl BaseDeclarativeTool for SyntheticOutputTool {
    fn name(&self) -> &str {
        "synthetic_output"
    }

    fn display_name(&self) -> &str {
        "StructuredOutput"
    }

    fn description(&self) -> &str {
        "以结构化JSON格式返回最终响应（用于非交互式会话）。(Return the final response in structured JSON format, for non-interactive sessions.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "output": {
                    "description": "要返回的结构化输出 (The structured output to return)"
                }
            },
            "required": ["output"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SyntheticOutputParams = serde_json::from_value(params)?;
        Ok(Box::new(SyntheticOutputInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}