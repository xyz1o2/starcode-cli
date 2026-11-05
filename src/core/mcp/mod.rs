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

/// 拼出 MCP 工具的注册名：`mcp__<server>__<tool>`。
///
/// # 为什么 server 名要先「去冒号」
///
/// 插件的 MCP server 名是 `plugin:<plugin>:<server>`（见
/// `core::plugins::mcp::qualify_plugin_mcp_server_name`），直接拼出来的
/// 工具名会长成 `mcp__plugin:exa:exa__web_search_exa` —— **冒号在
/// OpenAI 兼容的 function name 里不合法**（只允许 `a-zA-Z0-9_-`）。
/// 这样的工具名发出去会被 provider 拒掉，或者被当成非法标识符丢掉。
///
/// Claude Code 的做法也是把分隔符换成下划线：
/// `mcp__claude_ai_Slack__slack_send_message`。
///
/// 所以这里把 server 名里的 `:` 换成 `_`。同名冲突的极端情况
/// （`a:b` 与 `a_b` 同时存在）不额外处理 —— 那要求插件名本身含冒号，
/// 而插件名来自 marketplace 的 `name` 字段，不允许冒号。
pub fn mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    let server = server_name.replace(':', "_");
    format!("mcp__{}__{}", server, tool_name)
}

/// 从工具注册名**还原** server 名（[`mcp_tool_name`] 的逆运算）。
///
/// 拼名时把 `:` 换成了 `_`（function name 不允许冒号），所以拆名时要换回来
/// —— 否则 `mcp__plugin_exa_exa__web_search_exa` 会被拆出 server
/// `plugin_exa_exa`，而 manager 里注册的是 `plugin:exa:exa`，调用直接
/// 「server not found」。
///
/// 只对**插件** server 做这个还原：普通 server 名里本来就没有冒号，
/// 换回去反而会找不到。
pub fn server_name_from_tool_name(server: &str) -> String {
    if !server.starts_with("plugin_") {
        return server.to_string();
    }
    // `plugin_<plugin>_<server>` → `plugin:<plugin>:<server>`。
    // 插件名本身不含冒号，也不含下划线之外的分隔符，所以切两段即可。
    let rest = &server["plugin_".len()..];
    match rest.split_once('_') {
        Some((plugin, name)) => format!("plugin:{}:{}", plugin, name),
        None => server.to_string(),
    }
}

#[cfg(test)]
mod tool_name_tests {
    use super::*;

    /// 插件 server 的工具名不能带冒号 —— function name 只允许
    /// `a-zA-Z0-9_-`。
    ///
    /// 现场：插件的 MCP server 装上后工具进不了模型的 schema，
    /// 因为拼出来的名字是 `mcp__plugin:exa:exa__web_search_exa`。
    #[test]
    fn a_plugin_tool_name_has_no_colons() {
        let name = mcp_tool_name("plugin:exa:exa", "web_search_exa");
        assert_eq!(name, "mcp__plugin_exa_exa__web_search_exa");
        assert!(
            !name.contains(':'),
            "工具名不能带冒号（function name 不允许）：{name}"
        );
    }

    /// 普通 server 名原样保留 —— 不能把 `filesystem` 变成 `filesystem` 之外的东西。
    #[test]
    fn an_ordinary_server_name_is_untouched() {
        assert_eq!(
            mcp_tool_name("filesystem", "read_file"),
            "mcp__filesystem__read_file"
        );
        assert_eq!(
            server_name_from_tool_name("filesystem"),
            "filesystem",
            "普通 server 名不该被还原"
        );
    }

    /// 拼名 → 拆名 必须回到原样（往返不变量）。
    ///
    /// 拆出来的 server 名要交给 `manager.call_tool(server, ...)`，
    /// 与注册时的名字对不上就是「server not found」。
    #[test]
    fn the_name_round_trips() {
        for server in [
            "filesystem",
            "exa",
            "plugin:exa:exa",
            "plugin:my-plugin:some-server",
            "chrome-devtools",
        ] {
            let tool_name = mcp_tool_name(server, "do_thing");
            let suffix = tool_name
                .strip_prefix("mcp__")
                .expect("prefix stripped by mcp_tool_name");
            let (raw_server, tool) = suffix.split_once("__").expect("split by __");
            assert_eq!(tool, "do_thing");
            assert_eq!(
                server_name_from_tool_name(raw_server),
                server,
                "{server} 的名字往返后对不上"
            );
        }
    }
}
