use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult as CoreToolResult,
};
use crate::core::mcp::manager::MCPManager;
use crate::core::mcp::types::MCPTool;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct McpToolWrapper {
    manager: Arc<MCPManager>,
    server_name: String,
    tool_def: MCPTool,
    registered_name: String,
}

impl McpToolWrapper {
    pub fn new(
        manager: Arc<MCPManager>,
        server_name: String,
        tool_def: MCPTool,
        registered_name: String,
    ) -> Self {
        Self {
            manager,
            server_name,
            tool_def,
            registered_name,
        }
    }
}

pub struct McpToolInvocation {
    manager: Arc<MCPManager>,
    server_name: String,
    tool_name: String,
    params: Value,
}

impl BaseDeclarativeTool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.registered_name
    }

    fn display_name(&self) -> &str {
        &self.tool_def.name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    fn kind(&self) -> Kind {
        Kind::Execute // MCP tools are generic executions
    }

    fn parameter_schema(&self) -> Value {
        self.tool_def.input_schema.clone()
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(McpToolInvocation {
            manager: self.manager.clone(),
            server_name: self.server_name.clone(),
            tool_name: self.tool_def.name.clone(),
            params,
        }))
    }
}

impl ToolInvocation for McpToolInvocation {
    fn get_description(&self) -> String {
        format!(
            "Executing MCP tool {} on server {}",
            self.tool_name, self.server_name
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let manager = self.manager.clone();
        let server_name = self.server_name.clone();
        let tool_name = self.tool_name.clone();
        let params = self.params.clone();

        Box::pin(async move {
            match manager.call_tool(&server_name, &tool_name, params).await {
                Ok(result) => {
                    // MCP result is usually { content: [ { type: "text", text: "..." } ], isError: false }
                    // We need to parse this into a string output
                    let output =
                        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
                            content
                                .iter()
                                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            result.to_string()
                        };

                    let is_error = result
                        .get("isError")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);

                    Ok(CoreToolResult {
                        llm_content: None,
                        return_display: None,
                        output: output.clone(),
                        error: if is_error {
                            Some(crate::core::tools::tools::ToolError {
                                error_type: "mcp_error".to_string(),
                                message: output,
                            })
                        } else {
                            None
                        },
                        data: None,
                    })
                }
                Err(e) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(crate::core::tools::tools::ToolError {
                        error_type: "mcp_execution_error".to_string(),
                        message: e.to_string(),
                    }),
                    data: None,
                }),
            }
        })
    }
}
