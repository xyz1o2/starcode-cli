use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::mcp::types::{MCPServerConfig, TransportConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginMcpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub disabled: Option<bool>,
    #[serde(default, rename = "type", alias = "transport_type")]
    pub transport_type: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPluginMcpServer {
    pub plugin_name: String,
    pub server_name: String,
    pub qualified_name: String,
    pub config: PluginMcpServerConfig,
    pub working_dir: PathBuf,
    pub project_root: PathBuf,
}

impl ResolvedPluginMcpServer {
    pub fn to_mcp_server_config(&self) -> MCPServerConfig {
        let transport_type = self.config.transport_type.clone().unwrap_or_else(|| {
            if self.config.url.is_some() {
                "sse".to_string()
            } else {
                "stdio".to_string()
            }
        });

        MCPServerConfig {
            name: self.qualified_name.clone(),
            transport: TransportConfig {
                transport_type,
                command: Some(self.config.command.clone()),
                args: Some(self.config.args.clone()),
                env: Some(self.config.env.clone()),
                url: self.config.url.clone(),
                headers: None,
            },
            command: None,
            args: None,
            env: Some(self.config.env.clone()),
            disabled: self.config.disabled,
        }
    }
}

pub fn is_valid_plugin_mcp_server_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub fn qualify_plugin_mcp_server_name(plugin_name: &str, server_name: &str) -> String {
    format!("plugin:{}:{}", plugin_name, server_name)
}
