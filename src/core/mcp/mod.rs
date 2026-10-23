pub mod client;
pub mod config;
pub mod context_server;
pub mod manager;
pub mod oauth_provider;
pub mod oauth_utils;
pub mod server;
pub mod transport;
pub mod types;

pub use config::{
    load_effective_mcp_config, load_effective_mcp_servers, load_project_mcp_config,
    project_star_mcp_path, save_project_mcp_config, McpConfigScope, McpServerEntry,
    WindsurfMcpConfig, WindsurfMcpServer,
};
pub use manager::MCPManager;
pub use types::{load_mcp_config, MCPConfig, MCPServerConfig, McpError, TransportConfig};

pub fn get_mcp_manager() -> MCPManager {
    MCPManager::new()
}

pub async fn initialize_mcp_servers() -> Result<(), McpError> {
    let manager = get_mcp_manager();
    manager.initialize_mcp_servers().await
}
