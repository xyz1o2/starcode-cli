use crate::core::mcp::transport::Transport;
use crate::core::mcp::types::{MCPResourceContent, McpError};
use serde_json::Value;
use tokio::time::Duration;

pub struct McpClient {
    transport: Box<dyn Transport>,
    pub next_id: u64,
    initialized: bool,
    pub tool_list_changed_seen: bool,
    pub resource_list_changed_seen: bool,
    pub prompt_list_changed_seen: bool,
}

impl McpClient {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            next_id: 1,
            initialized: false,
            tool_list_changed_seen: false,
            resource_list_changed_seen: false,
            prompt_list_changed_seen: false,
        }
    }

    pub async fn start(&mut self) -> Result<(), McpError> {
        self.transport.start().await
    }

    pub async fn ensure_initialized(&mut self) -> Result<(), McpError> {
        if self.initialized {
            return Ok(());
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "clientInfo": {"name": "starcode-cli", "version": "0.1.0"},
                "capabilities": {}
            }
        });
        self.next_id += 1;
        let _ = self.send_and_wait(req).await?;

        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        self.send_notification(notif).await?;

        self.initialized = true;
        Ok(())
    }

    pub async fn send_notification(&mut self, msg: Value) -> Result<(), McpError> {
        self.transport.send(msg).await
    }

    pub async fn ping(&mut self) -> Result<(), McpError> {
        self.ensure_initialized().await?;
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "ping"
        });
        self.next_id += 1;
        self.send_and_wait(req).await.map(|_| ())
    }

    pub async fn send_and_wait(&mut self, msg: Value) -> Result<Value, McpError> {
        let id = msg
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or("jsonrpc request missing id")?;

        self.transport.send(msg).await?;

        let result = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let resp = self.transport.receive().await?;
                let v = match resp {
                    Some(v) => v,
                    None => return Err::<Value, McpError>("mcp server closed transport".into()),
                };

                if v.is_null() {
                    continue;
                }

                if v.get("id").is_none() {
                    if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                        let m = method.to_lowercase();
                        if m.contains("tools") && m.contains("list") && m.contains("changed") {
                            self.tool_list_changed_seen = true;
                        }
                        if m.contains("resources") && m.contains("list") && m.contains("changed") {
                            self.resource_list_changed_seen = true;
                        }
                        if m.contains("prompts") && m.contains("list") && m.contains("changed") {
                            self.prompt_list_changed_seen = true;
                        }
                        if m == "notifications/message" {
                            if let Some(params) = v.get("params") {
                                let level = params
                                    .get("level")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("info");
                                let data = params.get("data");
                                let logger = params
                                    .get("logger")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("mcp");
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[MCP:{}:{}] {:?}",
                                    logger, level, data
                                ));
                            }
                        }
                    }
                    continue;
                }

                if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                    continue;
                }

                if let Some(err) = v.get("error") {
                    return Err(format!("mcp jsonrpc error: {}", err).into());
                }

                return Ok(v
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})));
            }
        })
        .await
        .map_err(|_| -> McpError { "MCP call timed out after 60s".into() })??;
        Ok(result)
    }

    pub async fn close(&mut self) -> Result<(), McpError> {
        self.transport.close().await
    }

    pub async fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        self.ensure_initialized().await?;

        let args_obj = match arguments {
            serde_json::Value::Object(m) => serde_json::Value::Object(m),
            _ => serde_json::json!({}),
        };

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args_obj
            }
        });
        self.next_id += 1;
        self.send_and_wait(req).await
    }

    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, McpError> {
        self.ensure_initialized().await?;

        let mut params = serde_json::json!({ "name": name });
        if let Some(args) = arguments {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("arguments".to_string(), args);
            }
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "prompts/get",
            "params": params
        });
        self.next_id += 1;
        self.send_and_wait(req).await
    }

    pub async fn read_resource(&mut self, uri: &str) -> Result<Vec<MCPResourceContent>, McpError> {
        self.ensure_initialized().await?;

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "resources/read",
            "params": {
                "uri": uri
            }
        });
        self.next_id += 1;

        let result = self.send_and_wait(req).await?;

        let contents = result
            .get("contents")
            .and_then(|v| v.as_array())
            .cloned()
            .ok_or("resources/read missing contents")?;

        let mut out = Vec::new();
        for c in contents {
            let uri = c
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or("resource content missing uri")?
                .to_string();
            let mime_type = c
                .get("mimeType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let text = c
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let blob = c
                .get("blob")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(MCPResourceContent {
                uri,
                mime_type,
                text,
                blob,
            });
        }
        Ok(out)
    }
}
