use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type McpError = Box<dyn std::error::Error + Send + Sync>;

/// MCP JSON-RPC 层返回的错误。
///
/// 这是协议错误而不是传输错误。调用方不能把它当作连接断开来重连并
/// 重发：服务端已经收到请求，重发可能重复执行非幂等工具。
#[derive(Debug)]
pub struct McpJsonRpcError {
    pub code: Option<i64>,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl std::fmt::Display for McpJsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mcp jsonrpc error: {}", self.message)?;
        if let Some(code) = self.code {
            write!(f, " (code {code})")?;
        }
        Ok(())
    }
}

impl std::error::Error for McpJsonRpcError {}

/// 保留调用失败与重连失败的两段原因，供 server 和 manager 共用。
pub(super) fn join_call_and_reconnect_errors(
    call_err: &McpError,
    reconnect_err: &McpError,
) -> McpError {
    format!(
        "MCP tool call failed: {}; reconnect failed: {}",
        call_err, reconnect_err
    )
    .into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MCPServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MCPServerConfig {
    pub name: String,
    pub transport: TransportConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TransportConfig {
    #[serde(rename = "type")]
    pub transport_type: String, // stdio, http, sse, streamable_http
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MCPConfig {
    pub servers: Vec<MCPServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPResource {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPPrompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<MCPPromptArgument>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPPromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPPromptMessage {
    pub role: String,
    pub content: MCPPromptContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPPromptContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<MCPResourceContent>,
}

pub fn load_mcp_config() -> MCPConfig {
    MCPConfig {
        servers: Vec::new(),
    }
}
