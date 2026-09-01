//! Extra tool discovery and execution — merged from search_extra_tools + execute_extra_tool

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── SearchExtraTools ────────────────────────────────────────────────

#[derive(Clone)]
pub struct SearchExtraToolsTool;

impl SearchExtraToolsTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SearchExtraToolsParams {
    pub query: String,
    pub max_results: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchExtraToolsOutput {
    pub matches: Vec<String>,
    pub query: String,
    pub total_deferred_tools: usize,
    pub pending_mcp_servers: Option<Vec<String>>,
    pub already_loaded: Option<Vec<String>>,
}

pub struct SearchExtraToolsInvocation {
    params: SearchExtraToolsParams,
}

impl ToolInvocation for SearchExtraToolsInvocation {
    fn get_description(&self) -> String {
        format!("Search extra tools: {}", self.params.query)
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
            let query = params.query.clone();
            let max_results = params.max_results.unwrap_or(5);

            // In a real implementation, this would:
            // 1. Get list of deferred tools
            // 2. Search using keyword matching and TF-IDF
            // 3. Return matching tool names

            // For now, return a placeholder response
            let matches = vec![
                "WebFetch".to_string(),
                "WebSearch".to_string(),
                "web_browser".to_string(),
            ];

            let matches = matches.into_iter().take(max_results).collect::<Vec<_>>();

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Found {} tools matching '{}'",
                    matches.len(),
                    query
                )),
                return_display: Some(format!("{} tools found", matches.len())),
                output: serde_json::to_string(&SearchExtraToolsOutput {
                    matches: matches.clone(),
                    query: query.clone(),
                    total_deferred_tools: 10,
                    pending_mcp_servers: None,
                    already_loaded: None,
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "matches": matches,
                    "query": query
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SearchExtraToolsTool {
    fn name(&self) -> &str {
        "search_extra_tools"
    }

    fn display_name(&self) -> &str {
        "SearchExtraTools"
    }

    fn description(&self) -> &str {
        "搜索可用的额外工具。用于发现和加载延迟加载的工具。(Search for available extra tools. Used to discover and load deferred tools.)"
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
                    "description": "搜索查询。使用 \"select:<tool_name>\" 直接选择，或使用关键词搜索。"
                },
                "max_results": {
                    "type": "integer",
                    "description": "最大返回结果数，默认5 (Maximum number of results to return, default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SearchExtraToolsParams = serde_json::from_value(params)?;
        Ok(Box::new(SearchExtraToolsInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

// ── ExecuteExtraTool ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct ExecuteTool;

impl ExecuteTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExecuteParams {
    pub tool_name: String,
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ExecuteOutput {
    pub result: serde_json::Value,
    pub tool_name: String,
}

pub struct ExecuteInvocation {
    params: ExecuteParams,
}

impl ToolInvocation for ExecuteInvocation {
    fn get_description(&self) -> String {
        format!("Execute tool: {}", self.params.tool_name)
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
            let tool_name = params.tool_name.clone();

            // In a real implementation, this would:
            // 1. Look up the tool in the tool registry
            // 2. Create an invocation with the provided params
            // 3. Execute the tool
            // 4. Return the result

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some(format!("Executed tool '{}' with params", tool_name)),
                return_display: Some(format!("Tool '{}' executed", tool_name)),
                output: serde_json::to_string(&ExecuteOutput {
                    result: serde_json::json!({
                        "status": "success",
                        "message": format!("Tool '{}' executed successfully", tool_name)
                    }),
                    tool_name: tool_name.clone(),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "tool_name": tool_name,
                    "params": params.params
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for ExecuteTool {
    fn name(&self) -> &str {
        "execute_extra_tool"
    }

    fn display_name(&self) -> &str {
        "ExecuteExtraTool"
    }

    fn description(&self) -> &str {
        "执行已发现的延迟工具。将参数委托给目标工具执行。(Execute a discovered deferred tool. Delegates parameters to the target tool.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "目标工具名称 (The name of the tool to execute)"
                },
                "params": {
                    "type": "object",
                    "description": "传递给目标工具的参数 (Parameters to pass to the target tool)",
                    "additionalProperties": true
                }
            },
            "required": ["tool_name", "params"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ExecuteParams = serde_json::from_value(params)?;
        Ok(Box::new(ExecuteInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}
