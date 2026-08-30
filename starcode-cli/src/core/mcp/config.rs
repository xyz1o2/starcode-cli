use crate::core::mcp::types::McpError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WindsurfMcpConfig {
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: HashMap<String, WindsurfMcpServer>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WindsurfMcpServer {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub disabled: Option<bool>,
    #[serde(rename = "type", alias = "transport_type")]
    pub transport_type: Option<String>,
    pub url: Option<String>,
}

fn mcp_config_path() -> PathBuf {
    crate::core::utils::paths::get_mcp_config_path()
}

async fn load_config_from_path(path: &Path) -> Result<WindsurfMcpConfig, McpError> {
    let content = tokio::fs::read_to_string(path).await?;
    if content.trim().is_empty() {
        return Ok(WindsurfMcpConfig::default());
    }

    let cfg: WindsurfMcpConfig =
        crate::core::config::json_with_comments::parse_json_with_comments(&content)?;
    Ok(cfg)
}

pub async fn load_project_mcp_config() -> Result<WindsurfMcpConfig, McpError> {
    let path = mcp_config_path();
    if !path.exists() {
        return Ok(WindsurfMcpConfig::default());
    }
    load_config_from_path(&path).await
}

pub async fn save_project_mcp_config(config: &WindsurfMcpConfig) -> Result<(), McpError> {
    let path = mcp_config_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let content = serde_json::to_string_pretty(config)?;
    tokio::fs::write(path, content).await?;
    Ok(())
}

 