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

/// 取 `mcp__<server>__<tool>` 里的**原始 server 段**（不做冒号还原）。
///
/// 只给消歧用：见 [`split_mcp_tool_name_disambiguated`]。其余场景一律走
/// [`split_mcp_tool_name`]，不要拿原始段当 server 名。
pub fn raw_mcp_server_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("mcp__")?;
    let (raw_server, _tool) = rest.split_once("__")?;
    if raw_server.is_empty() {
        return None;
    }
    Some(raw_server.to_string())
}

/// 拆 `mcp__<server>__<tool>`：返回**还原后**的 server 名与工具名的**唯一入口**。
///
/// 历史教训：拆 `mcp__` 名字的调用点曾散在全库十几处，各自 `split_once("__")`
/// 却只有一处做冒号还原 —— 插件 server 拆出来的是 `plugin_exa_exa`，
/// 与 manager 注册键 `plugin:exa:exa` 对不上，权限判定 / 统计 / 展示全错位。
/// 现在任何需要"从工具注册名拿 server 和 tool"的地方都必须走这里，
/// 不要再手写 `strip_prefix("mcp__")` + `split_once("__")`。
///
/// 极端情况：普通 server 恰好长成插件前缀形（真叫 `plugin_x_y`）会被
/// 还原成 `plugin:x:y` —— 纯函数里没法分辨两边的可能性，谁在册用谁的
/// 消歧见 [`split_mcp_tool_name_disambiguated`]。
pub fn split_mcp_tool_name(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("mcp__")?;
    let (raw_server, tool) = rest.split_once("__")?;
    if raw_server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server_name_from_tool_name(raw_server), tool.to_string()))
}

/// 带**注册表消歧**的拆名：手头有 server 注册表的调用方必须走这个。
///
/// 前缀碰撞（`plugin_x_y` 既可能是插件 server 的还原名来源，又是普通
/// server 的真名）两可时，以注册表为准：
/// 还原名 `plugin:x:y` 在册 → 用还原名；否则原始段 `plugin_x_y` 在册 →
/// 原始段；都不在册 → 保持还原名（按它报错，行为与 [`split_mcp_tool_name`]
/// 一致）。还原名与原始段同时在册时优先还原名 —— 匹配 `plugin:` 注册键
/// 的是显式注册的插件 server，证据更强。
///
/// 没有注册表可查的纯展示调用点（统计 / 进度显示）仍走 [`split_mcp_tool_name`]。
pub fn split_mcp_tool_name_disambiguated(
    name: &str,
    is_registered: &dyn Fn(&str) -> bool,
) -> Option<(String, String)> {
    let raw = raw_mcp_server_name(name)?;
    let (candidate, tool) = split_mcp_tool_name(name)?;
    if is_registered(&candidate) || !is_registered(&raw) {
        return Some((candidate, tool));
    }
    Some((raw, tool))
}

#[cfg(test)]
mod tool_name_tests {
    use super::*;

    /// [`split_mcp_tool_name`] 是拆名的唯一入口，且必须带冒号还原：
    /// 插件 server 拆出来要能直接喂 `manager.call_tool(server, ...)`。
    #[test]
    fn split_mcp_tool_name_restores_the_plugin_server_name() {
        assert_eq!(
            split_mcp_tool_name("mcp__plugin_exa_exa__web_search_exa"),
            Some(("plugin:exa:exa".to_string(), "web_search_exa".to_string()))
        );
        assert_eq!(
            split_mcp_tool_name("mcp__filesystem__read_file"),
            Some(("filesystem".to_string(), "read_file".to_string()))
        );
        // 残缺名字一律 None —— 调用方不必再判空段。
        assert_eq!(split_mcp_tool_name("mcp__exa"), None);
        assert_eq!(split_mcp_tool_name("mcp____tool"), None);
        assert_eq!(split_mcp_tool_name("Bash"), None);
    }

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

    /// 前缀碰撞按注册表消歧：普通 server 真叫 `plugin_x_y` 时不能猜成
    /// `plugin:x:y`；注册的真是插件 server 时也不能被原始段骗走。
    #[test]
    fn the_registry_disambiguates_plugin_prefix_collisions() {
        let name = "mcp__plugin_x_y__do";
        let only_raw = |n: &str| n == "plugin_x_y";
        assert_eq!(
            split_mcp_tool_name_disambiguated(name, &only_raw),
            Some(("plugin_x_y".to_string(), "do".to_string()))
        );
        let only_candidate = |n: &str| n == "plugin:x:y";
        assert_eq!(
            split_mcp_tool_name_disambiguated(name, &only_candidate),
            Some(("plugin:x:y".to_string(), "do".to_string()))
        );
        // 两边都在册：还原名优先 —— 显式注册的插件 server 证据更强。
        let both = |n: &str| n == "plugin_x_y" || n == "plugin:x:y";
        assert_eq!(
            split_mcp_tool_name_disambiguated(name, &both),
            Some(("plugin:x:y".to_string(), "do".to_string()))
        );
        // 都不在册：保持还原名 —— 与无注册表时一致，报错按它报。
        let none = |_n: &str| false;
        assert_eq!(
            split_mcp_tool_name_disambiguated(name, &none),
            Some(("plugin:x:y".to_string(), "do".to_string()))
        );
        // 普通名不受消歧影响。
        assert_eq!(
            split_mcp_tool_name_disambiguated("mcp__filesystem__read_file", &none),
            Some(("filesystem".to_string(), "read_file".to_string()))
        );
        // 残缺名照旧 None。
        assert_eq!(
            split_mcp_tool_name_disambiguated("mcp__exa", &only_raw),
            None
        );
    }
}
