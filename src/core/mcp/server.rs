use crate::core::mcp::client::McpClient;
use crate::core::mcp::transport::{SseTransport, StdioTransport, StreamableHttpTransport};
use crate::core::mcp::types::{
    MCPPrompt, MCPPromptArgument, MCPResource, MCPResourceContent, MCPServerConfig,
    MCPServerStatus, MCPTool, McpError,
};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct MCPServer {
    pub name: String,
    pub config: MCPServerConfig,
    pub tools: Vec<MCPTool>,
    pub resources: Vec<MCPResource>,
    pub prompts: Vec<MCPPrompt>,
    client: Option<Arc<Mutex<McpClient>>>,
    pub status: MCPServerStatus,
    pub last_error: Option<String>,
    pub last_discovered_at: Option<i64>,
    is_refreshing_tools: bool,
    pending_tool_refresh: bool,
    is_refreshing_resources: bool,
    pending_resource_refresh: bool,
    is_refreshing_prompts: bool,
    pending_prompt_refresh: bool,
    retry_count: u32,
    max_retries: u32,
    last_retry_at: Option<i64>,
    base_retry_delay_ms: u64,
    cached_mcp_skills: Vec<(String, String)>,
}

impl MCPServer {
    pub fn new(config: MCPServerConfig) -> Self {
        MCPServer {
            name: config.name.clone(),
            config,
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
            client: None,
            status: MCPServerStatus::Disconnected,
            last_error: None,
            last_discovered_at: None,
            is_refreshing_tools: false,
            pending_tool_refresh: false,
            is_refreshing_resources: false,
            pending_resource_refresh: false,
            is_refreshing_prompts: false,
            pending_prompt_refresh: false,
            retry_count: 0,
            max_retries: 5,
            last_retry_at: None,
            base_retry_delay_ms: 1000,
            cached_mcp_skills: Vec::new(),
        }
    }

    pub fn add_tool(&mut self, tool: MCPTool) {
        self.tools.push(tool);
    }

    pub fn add_resource(&mut self, resource: MCPResource) {
        self.resources.push(resource);
    }

    pub fn get_tools(&self) -> &[MCPTool] {
        &self.tools
    }

    pub fn get_resources(&self) -> &[MCPResource] {
        &self.resources
    }

    pub fn get_prompts(&self) -> &[MCPPrompt] {
        &self.prompts
    }

    async fn ensure_connected(&mut self) -> Result<(), McpError> {
        if self.client.is_some() {
            return Ok(());
        }
        let t = &self.config.transport;

        let mut client = match t.transport_type.as_str() {
            "stdio" => {
                let transport = StdioTransport::new(t)?;
                McpClient::new(Box::new(transport))
            }
            "sse" => {
                let transport = SseTransport::new(t)?;
                McpClient::new(Box::new(transport))
            }
            "streamable_http" | "http" => {
                let transport = StreamableHttpTransport::new(t)?;
                McpClient::new(Box::new(transport))
            }
            _ => {
                self.status = MCPServerStatus::Error;
                self.last_error = Some(format!("unsupported MCP transport: {}", t.transport_type));
                return Err(format!("unsupported MCP transport: {}", t.transport_type).into());
            }
        };

        self.status = MCPServerStatus::Connecting;
        client.start().await?;
        client.ensure_initialized().await?;
        self.client = Some(Arc::new(Mutex::new(client)));
        self.status = MCPServerStatus::Connected;
        self.last_error = None;
        Ok(())
    }

    pub async fn disconnect(&mut self) {
        if let Some(client) = self.client.take() {
            let mut client = client.lock().await;
            let _ = client.close().await;
        }
        self.status = MCPServerStatus::Disconnected;
    }

    pub async fn discover(&mut self) -> Result<(), McpError> {
        self.refresh_tools().await?;
        self.refresh_resources().await?;
        self.refresh_prompts().await?;
        self.last_discovered_at = Some(chrono::Utc::now().timestamp());
        Ok(())
    }

    pub async fn refresh_tools(&mut self) -> Result<(), McpError> {
        if self.is_refreshing_tools {
            self.pending_tool_refresh = true;
            return Ok(());
        }
        self.is_refreshing_tools = true;

        if let Err(e) = self.ensure_connected().await {
            self.is_refreshing_tools = false;
            return Err(e);
        }

        let mut last_err: Option<String> = None;
        loop {
            self.pending_tool_refresh = false;

            let cli_arc = if let Some(c) = self.client.as_ref() {
                c.clone()
            } else {
                last_err = Some("MCP client not connected".to_string());
                break;
            };
            let mut cli = cli_arc.lock().await;
            if let Err(e) = cli.ensure_initialized().await {
                last_err = Some(e.to_string());
                break;
            }
            cli.tool_list_changed_seen = false;
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": cli.next_id,
                "method": "tools/list"
            });
            cli.next_id += 1;

            let result = match cli.send_and_wait(req).await {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(e.to_string());
                    break;
                }
            };
            if cli.tool_list_changed_seen {
                self.pending_tool_refresh = true;
            }
            let tools = match result.get("tools").and_then(|v| v.as_array()).cloned() {
                Some(v) => v,
                None => {
                    last_err = Some("mcp tools/list response missing tools field".to_string());
                    break;
                }
            };

            let mut out: Vec<MCPTool> = Vec::new();
            let mut first_raw: Option<serde_json::Value> = None;
            for t in tools {
                if first_raw.is_none() {
                    first_raw = Some(t.clone());
                }

                let name = if let Some(s) = t.as_str() {
                    s.to_string()
                } else {
                    t.get("name")
                        .and_then(|v| v.as_str())
                        .or_else(|| t.get("tool").and_then(|v| v.as_str()))
                        .or_else(|| t.get("id").and_then(|v| v.as_str()))
                        .or_else(|| {
                            t.get("function")
                                .and_then(|v| v.get("name"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("")
                        .to_string()
                };
                if name.trim().is_empty() {
                    continue;
                }

                let description = t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_schema = t
                    .get("inputSchema")
                    .or_else(|| t.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type":"object"}));
                out.push(MCPTool {
                    name,
                    description,
                    input_schema,
                });
            }

            self.tools = out;

            if !self.pending_tool_refresh {
                break;
            }
        }

        self.is_refreshing_tools = false;
        if let Some(e) = last_err {
            self.status = MCPServerStatus::Error;
            self.last_error = Some(e.clone());
            return Err(e.into());
        }
        self.status = MCPServerStatus::Connected;
        self.last_error = None;
        Ok(())
    }

    pub async fn refresh_resources(&mut self) -> Result<(), McpError> {
        if self.is_refreshing_resources {
            self.pending_resource_refresh = true;
            return Ok(());
        }
        self.is_refreshing_resources = true;

        if let Err(e) = self.ensure_connected().await {
            self.is_refreshing_resources = false;
            return Err(e);
        }

        let mut last_err: Option<String> = None;
        loop {
            self.pending_resource_refresh = false;

            let cli_arc = if let Some(c) = self.client.as_ref() {
                c.clone()
            } else {
                last_err = Some("MCP client not connected".to_string());
                break;
            };
            let mut cli = cli_arc.lock().await;
            if let Err(e) = cli.ensure_initialized().await {
                last_err = Some(e.to_string());
                break;
            }
            cli.resource_list_changed_seen = false;
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": cli.next_id,
                "method": "resources/list"
            });
            cli.next_id += 1;
            let result = match cli.send_and_wait(req).await {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(e.to_string());
                    break;
                }
            };
            if cli.resource_list_changed_seen {
                self.pending_resource_refresh = true;
            }
            let resources = result
                .get("resources")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out: Vec<MCPResource> = Vec::new();
            for r in resources {
                let uri = r
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if uri.is_empty() {
                    continue;
                }
                let name = r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mime_type = r
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let description = r
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                out.push(MCPResource {
                    uri,
                    name,
                    mime_type,
                    description,
                });
            }
            self.resources = out;

            if !self.pending_resource_refresh {
                break;
            }
        }

        self.is_refreshing_resources = false;
        if let Some(e) = last_err {
            self.status = MCPServerStatus::Error;
            self.last_error = Some(e.clone());
            return Err(e.into());
        }
        self.status = MCPServerStatus::Connected;
        self.last_error = None;
        Ok(())
    }

    pub async fn refresh_prompts(&mut self) -> Result<(), McpError> {
        if self.is_refreshing_prompts {
            self.pending_prompt_refresh = true;
            return Ok(());
        }

        self.ensure_connected().await?;
        self.is_refreshing_prompts = true;
        let mut last_err: Option<String> = None;

        loop {
            self.pending_prompt_refresh = false;

            let cli_arc = if let Some(c) = self.client.as_ref() {
                c.clone()
            } else {
                last_err = Some("MCP client not connected".to_string());
                break;
            };
            let mut cli = cli_arc.lock().await;
            if let Err(e) = cli.ensure_initialized().await {
                last_err = Some(e.to_string());
                break;
            }
            cli.prompt_list_changed_seen = false;
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": cli.next_id,
                "method": "prompts/list"
            });
            cli.next_id += 1;

            let result = match cli.send_and_wait(req).await {
                Ok(v) => v,
                Err(e) => {
                    // Ignore errors if prompts/list is not supported
                    if e.to_string().contains("Method not found") {
                        self.prompts = Vec::new();
                        break;
                    }
                    last_err = Some(e.to_string());
                    break;
                }
            };
            if cli.prompt_list_changed_seen {
                self.pending_prompt_refresh = true;
            }
            let prompts = result
                .get("prompts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut out: Vec<MCPPrompt> = Vec::new();
            for p in prompts {
                let name = p
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let description = p
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let arguments = p.get("arguments").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .map(|a| MCPPromptArgument {
                            name: a
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            description: a
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            required: a.get("required").and_then(|v| v.as_bool()),
                        })
                        .collect()
                });

                out.push(MCPPrompt {
                    name,
                    description,
                    arguments,
                });
            }
            self.prompts = out;

            if !self.pending_prompt_refresh {
                break;
            }
        }

        if let Some(e) = last_err {
            self.last_error = Some(e);
        }
        Ok(())
    }

    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        self.ensure_connected().await?;
        let cli_arc = self
            .client
            .as_ref()
            .ok_or("MCP client not connected")?
            .clone();
        let mut cli = cli_arc.lock().await;
        match cli.call_tool(tool_name, arguments.clone()).await {
            Ok(result) => {
                drop(cli);
                self.reset_reconnect();
                Ok(result)
            }
            Err(e) => {
                drop(cli);
                if self.try_reconnect().await.is_ok() {
                    let cli_arc2 = self
                        .client
                        .as_ref()
                        .ok_or("MCP client not connected after reconnect")?
                        .clone();
                    let mut cli2 = cli_arc2.lock().await;
                    cli2.call_tool(tool_name, arguments).await
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, McpError> {
        self.ensure_connected().await?;
        let cli_arc = self
            .client
            .as_ref()
            .ok_or("MCP client not connected")?
            .clone();
        let mut cli = cli_arc.lock().await;
        cli.get_prompt(name, arguments).await
    }

    pub async fn read_resource(&mut self, uri: &str) -> Result<Vec<MCPResourceContent>, McpError> {
        self.ensure_connected().await?;
        let cli_arc = self
            .client
            .as_ref()
            .ok_or("MCP client not connected")?
            .clone();
        let mut cli = cli_arc.lock().await;
        cli.read_resource(uri).await
    }

    pub fn reset_reconnect(&mut self) {
        self.retry_count = 0;
        self.last_retry_at = None;
    }

    pub async fn try_reconnect(&mut self) -> Result<(), McpError> {
        if self.retry_count >= self.max_retries {
            return Err(format!(
                "Max retries ({}) exhausted for server {}",
                self.max_retries, self.name
            )
            .into());
        }

        let delay_ms = std::cmp::min(
            self.base_retry_delay_ms * (1u64 << self.retry_count),
            30000,
        );

        if let Some(last) = self.last_retry_at {
            let now = chrono::Utc::now().timestamp_millis();
            if (now - last) < delay_ms as i64 {
                return Err("Retry too soon".into());
            }
        }

        self.disconnect().await;
        match self.ensure_connected().await {
            Ok(()) => {
                self.retry_count = 0;
                self.last_retry_at = None;
                Ok(())
            }
            Err(e) => {
                self.retry_count += 1;
                self.last_retry_at = Some(chrono::Utc::now().timestamp_millis());
                Err(e)
            }
        }
    }

    pub async fn discover_mcp_skills(&mut self) -> Result<Vec<(String, String)>, McpError> {
        self.refresh_resources().await?;

        let skill_resources: Vec<MCPResource> = self
            .resources
            .iter()
            .filter(|r| r.uri.starts_with("skill://"))
            .cloned()
            .collect();

        let mut skills: Vec<(String, String)> = Vec::new();

        for res in skill_resources {
            let uri = &res.uri;
            let name = res.name.clone();

            match self.read_resource(uri).await {
                Ok(contents) => {
                    for content in contents {
                        if let Some(text) = content.text {
                            skills.push((name.clone(), text));
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to read skill resource {}: {}",
                        uri, e
                    );
                }
            }
        }

        self.cached_mcp_skills = skills.clone();
        Ok(skills)
    }

    pub fn get_mcp_skills(&self) -> &[(String, String)] {
        &self.cached_mcp_skills
    }
}
