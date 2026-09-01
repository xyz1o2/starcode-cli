use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct McpAuthTool {
    oauth_states: Arc<Mutex<HashMap<String, McpOAuthState>>>,
}

impl McpAuthTool {
    pub fn new() -> Self {
        Self {
            oauth_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_state(&self, server_name: &str) -> Option<McpOAuthState> {
        let states = self.oauth_states.lock().await;
        states.get(server_name).cloned()
    }

    pub async fn set_state(&self, server_name: &str, state: McpOAuthState) {
        let mut states = self.oauth_states.lock().await;
        states.insert(server_name.to_string(), state);
    }

    pub async fn update_token(
        &self,
        server_name: &str,
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<i64>,
    ) {
        let mut states = self.oauth_states.lock().await;
        if let Some(state) = states.get_mut(server_name) {
            state.access_token = Some(access_token);
            if let Some(rt) = refresh_token {
                state.refresh_token = Some(rt);
            }
            state.expires_at = expires_at;
        }
    }

    pub async fn get_valid_token(&self, server_name: &str) -> Option<String> {
        let states = self.oauth_states.lock().await;
        if let Some(state) = states.get(server_name) {
            if let Some(token) = &state.access_token {
                if let Some(expires_at) = state.expires_at {
                    let now = chrono::Utc::now().timestamp();
                    if now < expires_at - 30 {
                        return Some(token.clone());
                    }
                } else {
                    return Some(token.clone());
                }
            }
        }
        None
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpAuthParams {
    pub server_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct McpOAuthState {
    pub server_name: String,
    pub auth_url: Option<String>,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub scopes: Vec<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

impl McpOAuthState {
    pub fn new(server_name: String) -> Self {
        Self {
            server_name,
            ..Default::default()
        }
    }

    pub fn is_token_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now().timestamp();
            now >= expires_at - 30
        } else {
            false
        }
    }

    pub fn has_valid_token(&self) -> bool {
        self.access_token.is_some() && !self.is_token_expired()
    }
}

pub fn parse_www_authenticate(header: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(rest) = header.strip_prefix("Bearer") {
        for part in rest.split(',') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=') {
                let key = k.trim().to_string();
                let val = v.trim().trim_matches('"').to_string();
                params.insert(key, val);
            }
        }
    }
    params
}

pub struct McpAuthInvocation {
    params: McpAuthParams,
    oauth_states: Arc<Mutex<HashMap<String, McpOAuthState>>>,
}

impl ToolInvocation for McpAuthInvocation {
    fn get_description(&self) -> String {
        format!("Authenticate MCP server '{}'", self.params.server_name)
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
        let server_name = self.params.server_name.clone();
        let oauth_states = self.oauth_states.clone();
        Box::pin(async move {
            let state = {
                let states = oauth_states.lock().await;
                states.get(&server_name).cloned()
            };

            let (message, error_type, status) = match state {
                Some(ref s) if s.has_valid_token() => {
                    let msg = format!(
                        "MCP server '{}' is already authenticated. Token is valid.",
                        server_name
                    );
                    (msg, None, "authenticated")
                }
                Some(ref s) => {
                    if let Some(auth_url) = &s.auth_url {
                        let mut msg = format!(
                            "MCP server '{}' requires OAuth authentication.\n\n\
                             Auth URL: {}\n",
                            server_name, auth_url
                        );
                        if let Some(client_id) = &s.client_id {
                            msg.push_str(&format!("Client ID: {}\n", client_id));
                        }
                        if !s.scopes.is_empty() {
                            msg.push_str(&format!("Scopes: {}\n", s.scopes.join(", ")));
                        }
                        msg.push_str(
                            "\nPlease visit the auth URL to authorize, then run `mcp_auth` again.",
                        );
                        (msg, Some("auth_required"), "auth_required")
                    } else {
                        let msg = format!(
                            "MCP server '{}' requires OAuth but no auth URL was discovered.\n\
                             Check server configuration.",
                            server_name
                        );
                        (msg, Some("no_auth_url"), "error")
                    }
                }
                None => {
                    let msg = format!(
                        "No OAuth state found for MCP server '{}'.\n\
                         The server may not have been connected yet or does not require OAuth.\n\
                         Use `mcp_list_servers` to check server status.",
                        server_name
                    );
                    (msg, Some("no_state"), "not_configured")
                }
            };

            Ok(ToolResult {
                llm_content: Some(message.clone()),
                return_display: Some(format!("MCP Auth status for '{}'", server_name)),
                output: message,
                error: error_type.map(|et| ToolError {
                    error_type: et.to_string(),
                    message: format!("OAuth authentication required for '{}'", server_name),
                }),
                data: Some(serde_json::json!({
                    "server": server_name,
                    "status": status,
                    "has_token": state.as_ref().map(|s| s.access_token.is_some()).unwrap_or(false),
                    "token_expired": state.as_ref().map(|s| s.is_token_expired()).unwrap_or(false),
                    "auth_url": state.as_ref().and_then(|s| s.auth_url.clone()),
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for McpAuthTool {
    fn name(&self) -> &str {
        "mcp_auth"
    }

    fn display_name(&self) -> &str {
        "MCP Auth"
    }

    fn description(&self) -> &str {
        "管理 MCP 服务器 OAuth 认证流程。(Manage OAuth authentication for MCP servers.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "server_name": {
                    "type": "string",
                    "description": "MCP 服务器名称 (MCP server name to authenticate)"
                }
            },
            "required": ["server_name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: McpAuthParams = serde_json::from_value(params)?;
        Ok(Box::new(McpAuthInvocation {
            params,
            oauth_states: self.oauth_states.clone(),
        }))
    }
}
