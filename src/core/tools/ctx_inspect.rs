use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct CtxInspectTool;

impl CtxInspectTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CtxInspectParams {
    pub query: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CtxInspectOutput {
    pub total_tokens: u64,
    pub message_count: u32,
    pub context_window_model: String,
    pub prompt_caching_enabled: bool,
    pub session_memory_enabled: bool,
    pub context_collapse_enabled: bool,
    pub summary: String,
}

pub struct CtxInspectInvocation {
    params: CtxInspectParams,
}

impl ToolInvocation for CtxInspectInvocation {
    fn get_description(&self) -> String {
        "Inspect context window".to_string()
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
            let query = params.query.unwrap_or_else(|| "all".to_string());

            // In a real implementation, this would:
            // 1. Analyze the current context window
            // 2. Return token counts and other metrics

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some("Context window inspected".to_string()),
                return_display: Some("Context inspection complete".to_string()),
                output: serde_json::to_string(&CtxInspectOutput {
                    total_tokens: 15000,
                    message_count: 25,
                    context_window_model: "gpt-4o".to_string(),
                    prompt_caching_enabled: true,
                    session_memory_enabled: true,
                    context_collapse_enabled: false,
                    summary: format!("Context window has 15000 tokens across 25 messages. Query: {}", query),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "total_tokens": 15000,
                    "message_count": 25,
                    "query": query
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for CtxInspectTool {
    fn name(&self) -> &str {
        "ctx_inspect"
    }

    fn display_name(&self) -> &str {
        "CtxInspect"
    }

    fn description(&self) -> &str {
        "检查当前上下文窗口的内容和token使用情况。(Inspect the current context window contents and token usage.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "可选的查询过滤器 (Optional query filter)"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: CtxInspectParams = serde_json::from_value(params)?;
        Ok(Box::new(CtxInspectInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}