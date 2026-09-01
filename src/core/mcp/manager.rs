use crate::core::mcp::config::load_project_mcp_config;
use crate::core::mcp::server::MCPServer;
use crate::core::mcp::types::{
    MCPPrompt, MCPResource, MCPResourceContent, MCPServerConfig, MCPTool, McpError, TransportConfig,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const PLUGIN_MCP_SERVER_PREFIX: &str = "plugin:";

pub struct MCPManager {
    servers: Arc<RwLock<HashMap<String, MCPServer>>>,
    plugin_server_names: Arc<RwLock<Vec<String>>>,
}

impl MCPManager {
    pub fn new() -> Self {
        MCPManager {
            servers: Arc::new(RwLock::new(HashMap::new())),
            plugin_server_names: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn add_server(&self, config: MCPServerConfig) -> Result<(), McpError> {
        let server = MCPServer::new(config.clone());
        let mut servers = self.servers.write().await;
        servers.insert(config.name.clone(), server);
        Ok(())
    }

    pub async fn remove_server(&self, server_name: &str) -> Result<(), McpError> {
        let mut servers = self.servers.write().await;
        servers.remove(server_name);
        Ok(())
    }

    pub async fn get_server(&self, server_name: &str) -> Option<MCPServer> {
        let servers = self.servers.read().await;
        servers.get(server_name).cloned()
    }

    pub async fn get_cached_tools(&self, server_name: &str) -> Option<Vec<MCPTool>> {
        let servers = self.servers.read().await;
        servers.get(server_name).map(|s| s.tools.clone())
    }

    pub async fn get_cached_resources(&self, server_name: &str) -> Option<Vec<MCPResource>> {
        let servers = self.servers.read().await;
        servers.get(server_name).map(|s| s.resources.clone())
    }

    pub async fn get_cached_prompts(&self, server_name: &str) -> Option<Vec<MCPPrompt>> {
        let servers = self.servers.read().await;
        servers.get(server_name).map(|s| s.prompts.clone())
    }

    pub async fn discover_all(&self) -> HashMap<String, String> {
        let names = self.list_server_names().await;
        let mut errors: HashMap<String, String> = HashMap::new();
        for name in names {
            let mut servers = self.servers.write().await;
            let Some(srv) = servers.get_mut(&name) else {
                continue;
            };
            if let Err(e) = srv.discover().await {
                errors.insert(name.clone(), e.to_string());
            }
        }
        errors
    }

    pub async fn restart_server(&self, server_name: &str) -> Result<(), McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.disconnect().await;
        server.discover().await?;
        Ok(())
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;

        let _ = server.refresh_tools().await;

        if !server.get_tools().iter().any(|t| t.name == tool_name) {
            return Err(format!("Tool {} not found in server {}", tool_name, server_name).into());
        }

        match server.call_tool(tool_name, arguments.clone()).await {
            Ok(result) => Ok(result),
            Err(e) => {
                if server.try_reconnect().await.is_ok() {
                    server.call_tool(tool_name, arguments).await
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn get_prompt(
        &self,
        server_name: &str,
        prompt_name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.get_prompt(prompt_name, arguments).await
    }

    pub async fn read_resource(
        &self,
        server_name: &str,
        uri: &str,
    ) -> Result<Vec<MCPResourceContent>, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.read_resource(uri).await
    }

    pub async fn list_tools(&self, server_name: &str) -> Result<Vec<MCPTool>, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.refresh_tools().await?;
        Ok(server.tools.clone())
    }

    pub async fn list_resources(&self, server_name: &str) -> Result<Vec<MCPResource>, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.refresh_resources().await?;
        Ok(server.resources.clone())
    }

    pub async fn list_prompts(&self, server_name: &str) -> Result<Vec<MCPPrompt>, McpError> {
        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .ok_or_else(|| format!("Server {} not found", server_name))?;
        server.refresh_prompts().await?;
        Ok(server.prompts.clone())
    }

    pub async fn register_plugin_server(&self, config: MCPServerConfig) -> Result<(), McpError> {
        let name = config.name.clone();
        if !name.starts_with(PLUGIN_MCP_SERVER_PREFIX) {
            return Err(format!(
                "Plugin MCP server name must start with '{}', got: {}",
                PLUGIN_MCP_SERVER_PREFIX, name
            )
            .into());
        }

        let server = MCPServer::new(config);
        let mut servers = self.servers.write().await;
        servers.insert(name.clone(), server);

        let mut plugin_names = self.plugin_server_names.write().await;
        if !plugin_names.contains(&name) {
            plugin_names.push(name);
        }

        Ok(())
    }

    pub async fn unregister_plugin_server(&self, server_name: &str) -> Result<(), McpError> {
        let mut servers = self.servers.write().await;
        servers.remove(server_name);

        let mut plugin_names = self.plugin_server_names.write().await;
        plugin_names.retain(|n| n != server_name);

        Ok(())
    }

    pub async fn unregister_all_plugin_servers(&self) -> Result<(), McpError> {
        let plugin_names = self.plugin_server_names.read().await;
        let names: Vec<String> = plugin_names.clone();
        drop(plugin_names);

        let mut servers = self.servers.write().await;
        for name in &names {
            servers.remove(name);
        }

        let mut plugin_names = self.plugin_server_names.write().await;
        plugin_names.clear();

        Ok(())
    }

    pub async fn list_plugin_server_names(&self) -> Vec<String> {
        let plugin_names = self.plugin_server_names.read().await;
        plugin_names.clone()
    }

    pub async fn is_plugin_server(&self, server_name: &str) -> bool {
        server_name.starts_with(PLUGIN_MCP_SERVER_PREFIX)
    }

    pub async fn initialize_plugin_mcp_servers(
        &self,
        servers: Vec<crate::core::plugins::ResolvedPluginMcpServer>,
    ) -> Result<(), McpError> {
        for server in servers {
            let config = server.to_mcp_server_config();
            self.register_plugin_server(config).await?;
        }
        Ok(())
    }

    pub async fn initialize_mcp_servers(&self) -> Result<(), McpError> {
        let cfg = load_project_mcp_config().await?;
        for (name, s) in cfg.mcp_servers {
            if s.disabled.unwrap_or(false) {
                continue;
            }

            let transport_type = s.transport_type.clone().unwrap_or_else(|| {
                if s.url.is_some() {
                    "sse".to_string()
                } else {
                    "stdio".to_string()
                }
            });

            let transport = TransportConfig {
                transport_type,
                command: s.command.clone(),
                args: s.args.clone(),
                env: s.env.clone(),
                url: s.url.clone(),
                headers: None,
            };
            let server_cfg = MCPServerConfig {
                name: name.clone(),
                transport,
                command: None,
                args: None,
                env: s.env.clone(),
                disabled: s.disabled,
            };

            self.add_server(server_cfg).await?;
        }

        Ok(())
    }

    pub async fn list_server_names(&self) -> Vec<String> {
        let servers = self.servers.read().await;
        let mut out: Vec<String> = servers.keys().cloned().collect();
        out.sort();
        out
    }

    pub async fn discover_all_mcp_skills(&self) -> Vec<(String, String)> {
        let names = self.list_server_names().await;
        let mut all_skills: Vec<(String, String)> = Vec::new();

        for name in names {
            let mut servers = self.servers.write().await;
            let Some(srv) = servers.get_mut(&name) else {
                continue;
            };
            match srv.discover_mcp_skills().await {
                Ok(skills) => {
                    for (skill_name, content) in skills {
                        all_skills.push((format!("{}:{}", name, skill_name), content));
                    }
                }
                Err(_) => {}
            }
        }

        all_skills
    }

    pub async fn get_mcp_skills(&self) -> Vec<(String, String)> {
        let servers = self.servers.read().await;
        let mut all_skills: Vec<(String, String)> = Vec::new();

        for (server_name, server) in servers.iter() {
            for (skill_name, content) in server.get_mcp_skills() {
                all_skills.push((format!("{}:{}", server_name, skill_name), content.clone()));
            }
        }

        all_skills
    }
}

impl Clone for MCPManager {
    fn clone(&self) -> Self {
        MCPManager {
            servers: Arc::clone(&self.servers),
            plugin_server_names: Arc::clone(&self.plugin_server_names),
        }
    }
}

impl Default for MCPManager {
    fn default() -> Self {
        Self::new()
    }
}

 