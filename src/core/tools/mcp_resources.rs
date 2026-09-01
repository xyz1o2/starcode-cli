//! MCP resource tools — merged from mcp_list_resources + mcp_read_resource

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;

// ── McpListResources ─────────────────────────────────────────────────

#[derive(Clone)]
pub struct McpListResourcesTool;

impl McpListResourcesTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpListResourcesParams {
    #[serde(default)]
    pub server: Option<String>,
}

pub struct McpListResourcesInvocation {
    params: McpListResourcesParams,
}

impl ToolInvocation for McpListResourcesInvocation {
    fn get_description(&self) -> String {
        match &self.params.server {
            Some(s) => format!("List MCP resources from server '{}'", s),
            None => "List all MCP resources".to_string(),
        }
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
            if let Some(ref server) = params.server {
                let _ = server;
            }

            let listing = serde_json::json!({
                "tool": "mcp_list_resources",
                "note": "MCP resource listing is handled by the MCP manager at runtime.",
                "server": params.server,
                "status": "delegated_to_mcp_manager"
            });

            let output_text = serde_json::to_string_pretty(&listing)
                .unwrap_or_else(|_| "MCP resource listing requested".to_string());

            Ok(ToolResult {
                llm_content: Some(output_text.clone()),
                return_display: Some("MCP resources listed".to_string()),
                output: output_text,
                error: None,
                data: Some(listing),
            })
        })
    }
}

impl BaseDeclarativeTool for McpListResourcesTool {
    fn name(&self) -> &str {
        "mcp_list_resources"
    }
    fn display_name(&self) -> &str {
        "MCP List Resources"
    }
    fn description(&self) -> &str {
        "列出 MCP 服务器提供的可用资源。(List available resources from MCP servers.)"
    }
    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP 服务器名称，可选 (MCP server name, optional)"
                }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: McpListResourcesParams = serde_json::from_value(params)?;
        Ok(Box::new(McpListResourcesInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

// ── McpReadResource ──────────────────────────────────────────────────

#[derive(Clone)]
pub struct McpReadResourceTool;

impl McpReadResourceTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpReadResourceParams {
    pub server: String,
    pub uri: String,
}

pub struct McpReadResourceInvocation {
    params: McpReadResourceParams,
}

impl ToolInvocation for McpReadResourceInvocation {
    fn get_description(&self) -> String {
        format!(
            "Read MCP resource '{}' from server '{}'",
            self.params.uri, self.params.server
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
            let resource_info = serde_json::json!({
                "tool": "mcp_read_resource",
                "server": params.server,
                "uri": params.uri,
                "status": "delegated_to_mcp_manager"
            });

            let output_text = serde_json::to_string_pretty(&resource_info)
                .unwrap_or_else(|_| "MCP resource read requested".to_string());

            Ok(ToolResult {
                llm_content: Some(output_text.clone()),
                return_display: Some(format!("Read resource {}", params.uri)),
                output: output_text,
                error: None,
                data: Some(resource_info),
            })
        })
    }
}

impl BaseDeclarativeTool for McpReadResourceTool {
    fn name(&self) -> &str {
        "mcp_read_resource"
    }
    fn display_name(&self) -> &str {
        "MCP Read Resource"
    }
    fn description(&self) -> &str {
        "读取 MCP 服务器上指定资源的内容。(Read the content of a specific resource from an MCP server.)"
    }
    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP 服务器名称 (MCP server name)"
                },
                "uri": {
                    "type": "string",
                    "description": "资源 URI (Resource URI)"
                }
            },
            "required": ["server", "uri"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: McpReadResourceParams = serde_json::from_value(params)?;
        Ok(Box::new(McpReadResourceInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
