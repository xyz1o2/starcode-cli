use crate::core::mcp::config::load_effective_mcp_config;
use crate::core::mcp::server::MCPServer;
use crate::core::mcp::types::{
    MCPPrompt, MCPResource, MCPResourceContent, MCPServerConfig, MCPTool, McpError, TransportConfig,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const PLUGIN_MCP_SERVER_PREFIX: &str = "plugin:";

/// 「刷新结果 + 工具缓存」→ 这次调用能不能继续。
///
/// 纯函数，好让"刷新失败不许变成 Tool not found"这条规则能直接被测，
/// 不必真的连一个 MCP server。
///
/// - 刷新失败 → 原样返回刷新错误。**不能**落到"工具不存在"的结论上：
///   那会把"没问成"说成"没有"。
/// - 刷新成功但缓存里没有这个工具 → `Tool X not found in server Y`。
/// - 刷新成功且工具在 → `Ok(())`，调用继续。
fn resolve_tool_call(
    server_name: &str,
    tool_name: &str,
    refresh: Result<(), McpError>,
    cached_tools: &[MCPTool],
) -> Result<(), McpError> {
    refresh?;
    if !cached_tools.iter().any(|t| t.name == tool_name) {
        return Err(format!("Tool {} not found in server {}", tool_name, server_name).into());
    }
    Ok(())
}

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

    /// 拆 `mcp__<server>__<tool>` 工具名，**按注册表消歧**。
    ///
    /// 见 `core::mcp::split_mcp_tool_name_disambiguated`：普通 server 恰好
    /// 长成插件前缀形（真叫 `plugin_x_y`）时，前缀猜出的 `plugin:x:y` 不在
    /// 注册表，此时退回原始段。要拿"与注册键一致的 server 名"，执行路径
    /// 统统走这里，不要对 `core::mcp::split_mcp_tool_name` 的结果直接
    /// `call_tool`。
    pub async fn split_mcp_tool_name(&self, name: &str) -> Option<(String, String)> {
        let servers = self.servers.read().await;
        crate::core::mcp::split_mcp_tool_name_disambiguated(name, &|n| servers.contains_key(n))
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

        // 刷新失败必须**让这次调用失败**，不能吞掉。
        //
        // 以前是 `let _ = server.refresh_tools().await;`：刷新失败后接着用
        // **空/过期缓存**判断工具存在与否，于是"刷新失败"（连不上、协议
        // 错误、权限问题）被报成 "Tool X not found in server Y" —— 一个
        // 看起来像"这个 server 没有这个工具"的结论，实际是"我们没问成"。
        // 现在刷新错误按原样带出去，工具是否存在只在刷新成功后才判断。
        let refresh = server.refresh_tools().await;
        resolve_tool_call(server_name, tool_name, refresh, server.get_tools())?;

        // `MCPServer::call_tool` 是唯一的传输重连边界。这里不能再包一层
        // retry：业务错误已由它区分，且调用结果不确定时重发可能重复副作用。
        server.call_tool(tool_name, arguments).await
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
        // 逐 server 容错：以前一个 `register_plugin_server` 失败就 `?` 短路，
        // 一个坏 manifest 连累**所有**后续插件server 不注册（现场看不到任何
        // server，只知道"failed"）。现在失败的名字收集进错误信息，其余照常注册。
        let mut failures: Vec<String> = Vec::new();
        for server in servers {
            let name = server.qualified_name.clone();
            if let Err(error) = self
                .register_plugin_server(server.to_mcp_server_config())
                .await
            {
                failures.push(format!("{name}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; ").into())
        }
    }

    /// 装载**所有**作用域的 MCP server（user + project），后者覆盖前者。
    ///
    /// 见 `core::mcp::config::load_effective_mcp_config` 的作用域表。只读
    /// `.star/mcp.json` 的话，用户在 Claude Code 里配好的 server 永远进不了
    /// registry —— `ToolSearch` 于是搜不到任何 MCP 工具。
    pub async fn initialize_mcp_servers(&self) -> Result<(), McpError> {
        let cfg = load_effective_mcp_config().await?;
        self.register_configured_servers(cfg).await
    }

    async fn register_configured_servers(
        &self,
        cfg: crate::core::mcp::config::WindsurfMcpConfig,
    ) -> Result<(), McpError> {
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
                headers: s.headers.clone(),
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

#[cfg(test)]
mod split_disambiguation_tests {
    use super::*;
    use crate::core::mcp::types::TransportConfig;

    #[tokio::test]
    async fn project_http_headers_reach_the_registered_transport() {
        let config = serde_json::from_str(
            r#"{"mcpServers":{"fixture":{"type":"http","url":"https://example.invalid/mcp","headers":{"X-Fixture":"fixture-value"}},"disabled":{"command":"must-not-run","disabled":true}}}"#,
        )
        .expect("parse fixture config");
        let manager = MCPManager::new();
        manager
            .register_configured_servers(config)
            .await
            .expect("register without connecting");
        let server = manager
            .get_server("fixture")
            .await
            .expect("registered server");
        assert_eq!(server.config.transport.transport_type, "http");
        assert_eq!(
            server.config.transport.headers.as_ref().unwrap()["X-Fixture"],
            "fixture-value"
        );
        assert!(manager.get_server("disabled").await.is_none());
    }

    fn stdio_config(name: &str) -> MCPServerConfig {
        MCPServerConfig {
            name: name.to_string(),
            transport: TransportConfig {
                transport_type: "stdio".to_string(),
                command: Some("true".to_string()),
                args: None,
                env: None,
                url: None,
                headers: None,
            },
            command: None,
            args: None,
            env: None,
            disabled: None,
        }
    }

    /// 普通 server 恰好叫 `plugin_x_y`：注册表在册的名字必须赢过前缀猜名；
    /// 真插件 server 的还原不受影响。
    #[tokio::test]
    async fn registry_lookup_beats_plugin_prefix_guess() {
        let manager = MCPManager::new();
        manager
            .add_server(stdio_config("plugin_x_y"))
            .await
            .unwrap();

        assert_eq!(
            manager.split_mcp_tool_name("mcp__plugin_x_y__do").await,
            Some(("plugin_x_y".to_string(), "do".to_string()))
        );

        manager
            .register_plugin_server(stdio_config("plugin:exa:exa"))
            .await
            .unwrap();
        assert_eq!(
            manager.split_mcp_tool_name("mcp__plugin_exa_exa__do").await,
            Some(("plugin:exa:exa".to_string(), "do".to_string()))
        );
    }
}

#[cfg(test)]
mod call_tool_error_tests {
    use super::*;
    use crate::core::mcp::types::join_call_and_reconnect_errors;

    fn tool(name: &str) -> MCPTool {
        MCPTool {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// 刷新失败时，**不能**报成 "Tool not found" —— 那会把"没问成"说成
    /// "这个 server 没有这个工具"。
    #[test]
    fn a_failed_refresh_is_never_reported_as_a_missing_tool() {
        let err = resolve_tool_call(
            "srv",
            "do_thing",
            Err("mcp server closed transport".into()),
            &[],
        )
        .expect_err("刷新失败必须让调用失败");

        let msg = err.to_string();
        assert!(
            !msg.contains("not found in server"),
            "刷新失败不能被报成工具不存在：{msg}"
        );
        assert!(
            msg.contains("mcp server closed transport"),
            "刷新失败的原因必须保留：{msg}"
        );
    }

    /// 刷新失败 + 缓存里其实有那个工具：仍然必须失败（刷新失败是硬错误），
    /// 且原因不能被"工具存在"掩盖。
    #[test]
    fn a_failed_refresh_wins_over_a_populated_cache() {
        let err = resolve_tool_call(
            "srv",
            "do_thing",
            Err("tools/list timed out".into()),
            &[tool("do_thing")],
        )
        .expect_err("刷新失败必须让调用失败");

        let msg = err.to_string();
        assert!(
            msg.contains("tools/list timed out"),
            "刷新失败的原因必须保留：{msg}"
        );
        assert!(
            !msg.contains("not found in server"),
            "有缓存也不该绕过刷新失败：{msg}"
        );
    }

    /// 刷新成功、工具确实不在 → 才是 "Tool not found"。
    #[test]
    fn a_successful_refresh_reports_a_genuinely_missing_tool() {
        let err = resolve_tool_call("srv", "do_thing", Ok(()), &[tool("other")])
            .expect_err("工具不存在必须报错");
        assert_eq!(
            err.to_string(),
            "Tool do_thing not found in server srv",
            "只有刷新成功后才允许下'工具不存在'的结论"
        );
    }

    /// 刷新成功、工具在 → 放行。
    #[test]
    fn a_successful_refresh_with_the_tool_present_passes() {
        resolve_tool_call("srv", "do_thing", Ok(()), &[tool("do_thing")])
            .expect("工具存在时不该报错");
    }

    /// 重连失败时，两段原因都要留着。
    #[test]
    fn a_failed_reconnect_keeps_both_reasons() {
        let call_err: McpError = "tool call timed out".into();
        let reconnect_err: McpError = "Max retries (5) exhausted for server srv".into();

        let msg = join_call_and_reconnect_errors(&call_err, &reconnect_err).to_string();
        assert!(
            msg.contains("tool call timed out"),
            "首次调用失败的原因必须保留：{msg}"
        );
        assert!(
            msg.contains("Max retries (5) exhausted"),
            "重连失败的原因必须保留：{msg}"
        );
    }

    /// 通过 manager 真实路径验证：刷新失败时，错误会**原样**报出，而不是
    /// 变成 "Tool not found in server ..."。
    ///
    /// 用一个**不支持的 transport 类型** —— `ensure_connected` 会直接返回
    /// 确定性的错误，不会去起任何子进程，也不需要网络（跨平台一致）。
    #[tokio::test]
    async fn a_manager_call_reports_the_refresh_failure_verbatim() {
        let manager = MCPManager::new();
        manager
            .add_server(MCPServerConfig {
                name: "srv".to_string(),
                transport: TransportConfig {
                    transport_type: "not-a-real-transport".to_string(),
                    command: None,
                    args: None,
                    env: None,
                    url: None,
                    headers: None,
                },
                command: None,
                args: None,
                env: None,
                disabled: None,
            })
            .await
            .unwrap();

        let err = manager
            .call_tool("srv", "do_thing", serde_json::json!({}))
            .await
            .expect_err("刷新失败必须让调用失败");

        let msg = err.to_string();
        assert!(
            !msg.contains("not found in server"),
            "刷新失败不能被报成工具不存在：{msg}"
        );
        assert!(
            msg.contains("unsupported MCP transport"),
            "刷新失败的原因必须原样带出：{msg}"
        );
    }

    /// 同一个 server 上：刷新失败发生在**已有缓存**之后，也一样不许变成
    /// "Tool not found" —— 缓存里有工具不代表这次刷新成功了。
    #[tokio::test]
    async fn a_manager_call_reports_the_refresh_failure_even_with_cached_tools() {
        let manager = MCPManager::new();
        manager
            .add_server(MCPServerConfig {
                name: "srv".to_string(),
                transport: TransportConfig {
                    transport_type: "not-a-real-transport".to_string(),
                    command: None,
                    args: None,
                    env: None,
                    url: None,
                    headers: None,
                },
                command: None,
                args: None,
                env: None,
                disabled: None,
            })
            .await
            .unwrap();

        // 手工塞一个缓存，模拟"之前发现过这个工具"。
        {
            let mut servers = manager.servers.write().await;
            let srv = servers.get_mut("srv").expect("server 在册");
            srv.add_tool(tool("do_thing"));
        }
        assert_eq!(
            manager.get_cached_tools("srv").await.map(|t| t.len()),
            Some(1),
            "缓存里应有这个工具"
        );

        let err = manager
            .call_tool("srv", "do_thing", serde_json::json!({}))
            .await
            .expect_err("刷新失败必须让调用失败");

        let msg = err.to_string();
        assert!(
            !msg.contains("not found in server"),
            "有缓存也不该绕过刷新失败：{msg}"
        );
        assert!(
            msg.contains("unsupported MCP transport"),
            "刷新失败的原因必须原样带出：{msg}"
        );
    }
}
