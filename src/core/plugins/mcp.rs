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

/// 插件名（marketplace `name` 字段）必须 `[a-zA-Z0-9-]` —— **不含下划线**。
///
/// 原因在 `core::mcp::server_name_from_tool_name`：工具名拼出来是
/// `mcp__plugin_<plugin>_<server>__<tool>`，拆名时按**从左第一段下划线**切
/// 插件名 / server 名。插件名若含 `_`，往返必然拆错
/// （`plugin:my_plugin:search` → 拆回 `plugin:my:plugin_search`），
/// 调用直接「server not found」。所以校验规则比 server 名严一格：
/// server 名允许 `_`（左切后它原样保留），插件名不行。
pub fn is_valid_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

pub fn qualify_plugin_mcp_server_name(plugin_name: &str, server_name: &str) -> String {
    format!("plugin:{}:{}", plugin_name, server_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 插件 manifest 里的 `mcpServers` 必须能被解析成可注册的 server 配置。
    ///
    /// 这条链路以前**整段没被调用过**：
    /// - `discover_plugin_mcp_servers()` 读 manifest、解析、出 `ResolvedPluginMcpServer`
    /// - `MCPManager::initialize_plugin_mcp_servers()` 把它注册进 manager
    ///
    /// 两半都写好了，但**都没有调用者** —— 插件带来的 MCP server 从来没接进
    /// 运行时。现场：插件装了 MCP server，`/mcp` 里看不到，模型也调不着。
    #[tokio::test]
    async fn a_plugin_manifests_mcp_server_is_discovered() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();

        // 插件目录：.star/extensions/plugins/<name>/.star-plugin/plugin.json
        let plugin_root = crate::core::plugins::plugins_dir(root).join("my-plugin");
        let manifest_dir = plugin_root.join(".star-plugin");
        std::fs::create_dir_all(&manifest_dir).expect("mkdir");
        std::fs::write(
            manifest_dir.join("plugin.json"),
            r#"{
                "name": "my-plugin",
                "version": "1.0.0",
                "mcpServers": {
                    "search": { "command": "npx", "args": ["-y", "exa-mcp-server@latest"] }
                }
            }"#,
        )
        .expect("write manifest");

        // 插件清单：.star/extensions/star-extension.json
        let manifest_path = crate::core::plugins::manifest_path(root);
        std::fs::create_dir_all(manifest_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &manifest_path,
            r#"{
                "plugins": [
                    {
                        "name": "my-plugin",
                        "source": "test",
                        "install_type": "local",
                        "installed_at": 0,
                        "enabled": true,
                        "scope": "project"
                    }
                ]
            }"#,
        )
        .expect("write plugin list");

        let servers = crate::core::plugins::discover_plugin_mcp_servers(root)
            .await
            .expect("discover plugin mcp servers");
        assert_eq!(
            servers.len(),
            1,
            "插件 manifest 里的 mcpServers 必须被发现：{servers:?}"
        );

        let resolved = &servers[0];
        assert_eq!(resolved.plugin_name, "my-plugin");
        assert_eq!(resolved.server_name, "search");
        assert!(
            resolved.qualified_name.starts_with("plugin:"),
            "插件 server 名要有 plugin: 前缀（用于与普通 server 区分）：{}",
            resolved.qualified_name
        );

        // 拼出的工具名不能带冒号（function name 不允许）。
        let tool_name = crate::core::mcp::mcp_tool_name(&resolved.qualified_name, "web_search");
        assert!(!tool_name.contains(':'), "工具名不能带冒号：{tool_name}");

        // 往返：拆回来必须与注册的 server 名一致。
        let suffix = tool_name.strip_prefix("mcp__").expect("prefix");
        let (raw_server, _) = suffix.split_once("__").expect("split");
        assert_eq!(
            crate::core::mcp::server_name_from_tool_name(raw_server),
            resolved.qualified_name,
            "工具名拆回来必须与注册的 server 名一致"
        );
    }
}
