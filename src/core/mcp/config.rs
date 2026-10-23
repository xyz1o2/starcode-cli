//! MCP 配置文件的多作用域读取。
//!
//! # 为什么不止一个文件
//!
//! starcode 原来只读 `.star/mcp.json`（项目级）。但绝大多数 MCP server 是
//! **用户级**的 —— 用户在 Claude Code / Cursor / Codex 里配一次，到哪都能用。
//! 只读项目级文件的后果是：用户明明配了 `exa`，`tool_search` 却永远搜不到
//! `mcp__exa__*`，因为这些 server 压根没进 registry。
//!
//! 对标 Claude Code 的三档作用域（见 `docs/claude-code-alignment.md` §1.7）：
//!
//! | 作用域 | 文件 | 谁写的 |
//! |---|---|---|
//! | project | `.mcp.json`（项目根）/ `.star/mcp.json`（starcode 自有） | 仓库里，通常随代码提交 |
//! | user | `~/.claude.json` 的 `mcpServers` / `~/.star/mcp.json` | 用户自己 |
//! | local | `.mcp.json` 里 `projects.<path>.mcpServers` | 用户自己，绑定当前目录 |
//!
//! 合并顺序 = 覆盖顺序：**project 覆盖 user 覆盖 local**（与 Claude Code 一致：
//! 项目里的配置最具体，优先级最高）。
//!
//! # 为什么读 `~/.claude.json`
//!
//! 用户已经在 Claude Code 里配好的 server，不该要求他再配一遍。`~/.claude.json`
//! 里的 `mcpServers` 就是那个"用户级"作用域。读它同时也让 starcode 和 Claude
//! Code 看到同一批 server —— 两边工具名一致（`mcp__<server>__<tool>`），
//! 互相可调用。
//!
//! 读法很保守：只取 `mcpServers` 顶格键，**不碰** `projects`（那是 CC 的会话
//! 状态，结构和 CC 自己的私有格式耦合很深，且含大量与 MCP 无关的字段）。
//! 需要项目级的 server 时，用 `.mcp.json` —— 那才是 CC 认的项目级位置。

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

/// 一次读取的来源。诊断信息用 —— 用户得知道 server 到底是从哪来的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpConfigScope {
    /// 项目根的 `.mcp.json`（CC 认的项目级位置）
    ProjectDotMcp,
    /// 项目自己的 `.star/mcp.json`（starcode 自有位置）
    ProjectStar,
    /// 用户级 `~/.claude.json` 的 `mcpServers`
    UserClaudeJson,
    /// 用户级 `~/.star/mcp.json`
    UserStar,
}

impl McpConfigScope {
    pub fn label(&self) -> &'static str {
        match self {
            McpConfigScope::ProjectDotMcp => "project (.mcp.json)",
            McpConfigScope::ProjectStar => "project (.star/mcp.json)",
            McpConfigScope::UserClaudeJson => "user (~/.claude.json)",
            McpConfigScope::UserStar => "user (~/.star/mcp.json)",
        }
    }
}

/// 一个 server 连同它来自哪个作用域。
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    pub name: String,
    pub server: WindsurfMcpServer,
    pub scope: McpConfigScope,
}

pub fn project_star_mcp_path() -> PathBuf {
    crate::core::utils::paths::get_mcp_config_path()
}

fn user_star_mcp_path() -> PathBuf {
    crate::core::config::storage::Storage::global_star_dir().join("mcp.json")
}

/// 项目根的 `.mcp.json`：Claude Code 认的项目级位置。
///
/// 刻意**不**向上找：CC 只看项目根那一个文件。往上找会让 monorepo 里
/// 父目录的 server 渗进子项目，作用域就说不清了。
fn project_dot_mcp_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.join(".mcp.json");
    path.exists().then_some(path)
}

/// 用户级 `~/.claude.json`。
fn user_claude_json_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = home.join(".claude.json");
    path.exists().then_some(path)
}

/// `~/.claude.json` 里我们只认 `mcpServers` 顶格键。
///
/// 刻意只认这一个键：这个文件是 CC 的全局状态，`projects` 里塞着会话历史、
/// 缓存标志等大量与 MCP 无关的结构，而且 CC 自己会改它的格式。认它的代价
/// 远大于收益 —— 用户级 server 就该放 `mcpServers`。
#[derive(Debug, Deserialize)]
struct ClaudeJsonMcpRoot {
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    mcp_servers: HashMap<String, WindsurfMcpServer>,
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

async fn load_claude_json_config(path: &Path) -> Result<WindsurfMcpConfig, McpError> {
    let content = tokio::fs::read_to_string(path).await?;
    if content.trim().is_empty() {
        return Ok(WindsurfMcpConfig::default());
    }
    let root: ClaudeJsonMcpRoot =
        crate::core::config::json_with_comments::parse_json_with_comments(&content)?;
    Ok(WindsurfMcpConfig {
        mcp_servers: root.mcp_servers,
    })
}

/// 读**单个**作用域的 MCP 配置。
///
/// 写操作（`/mcp add` 等）必须用它：写回用户级文件会破坏别人的配置，
/// 写回 `.star/mcp.json` 才是 starcode 自己的位置。
pub async fn load_project_mcp_config() -> Result<WindsurfMcpConfig, McpError> {
    let path = project_star_mcp_path();
    if !path.exists() {
        return Ok(WindsurfMcpConfig::default());
    }
    load_config_from_path(&path).await
}

/// 读**所有**作用域合并后的 MCP 配置。
///
/// 合并顺序 = 覆盖顺序，与 Claude Code 一致：
/// `user (~/.claude.json)` → `user (~/.star/mcp.json)` → `project (.mcp.json)`
/// → `project (.star/mcp.json)`。越靠后优先级越高 —— 项目配置最具体。
///
/// 同名 server 由**优先级更高**的作用域整个替换（不是逐字段合并）：
/// 两处都配了 `exa` 时，用户会以为改了一处就生效了，实际行为必须可预测。
pub async fn load_effective_mcp_config() -> Result<WindsurfMcpConfig, McpError> {
    let mut merged = WindsurfMcpConfig::default();

    // user (~/.claude.json)
    if let Some(path) = user_claude_json_path() {
        let cfg = load_claude_json_config(&path).await?;
        merge_into(&mut merged, cfg);
    }

    // user (~/.star/mcp.json)
    let user_star = user_star_mcp_path();
    if user_star.exists() {
        let cfg = load_config_from_path(&user_star).await?;
        merge_into(&mut merged, cfg);
    }

    // project (.mcp.json)
    if let Some(path) = project_dot_mcp_path() {
        let cfg = load_config_from_path(&path).await?;
        merge_into(&mut merged, cfg);
    }

    // project (.star/mcp.json) —— starcode 自有位置，优先级最高
    let project_star = project_star_mcp_path();
    if project_star.exists() {
        let cfg = load_config_from_path(&project_star).await?;
        merge_into(&mut merged, cfg);
    }

    Ok(merged)
}

/// 读所有作用域，并保留每个 server 的来源（诊断用）。
pub async fn load_effective_mcp_servers() -> Result<Vec<McpServerEntry>, McpError> {
    let mut merged = WindsurfMcpConfig::default();
    let mut sources: HashMap<String, McpConfigScope> = HashMap::new();

    let mut layers: Vec<(McpConfigScope, WindsurfMcpConfig)> = Vec::new();

    if let Some(path) = user_claude_json_path() {
        layers.push((
            McpConfigScope::UserClaudeJson,
            load_claude_json_config(&path).await?,
        ));
    }

    let user_star = user_star_mcp_path();
    if user_star.exists() {
        layers.push((
            McpConfigScope::UserStar,
            load_config_from_path(&user_star).await?,
        ));
    }

    if let Some(path) = project_dot_mcp_path() {
        layers.push((
            McpConfigScope::ProjectDotMcp,
            load_config_from_path(&path).await?,
        ));
    }

    let project_star = project_star_mcp_path();
    if project_star.exists() {
        layers.push((
            McpConfigScope::ProjectStar,
            load_config_from_path(&project_star).await?,
        ));
    }

    for (scope, cfg) in layers {
        for (name, server) in cfg.mcp_servers {
            sources.insert(name.clone(), scope.clone());
            merged.mcp_servers.insert(name, server);
        }
    }

    let mut entries: Vec<McpServerEntry> = merged
        .mcp_servers
        .into_iter()
        .map(|(name, server)| {
            let scope = sources
                .get(&name)
                .cloned()
                .unwrap_or(McpConfigScope::ProjectStar);
            McpServerEntry {
                name,
                server,
                scope,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn merge_into(target: &mut WindsurfMcpConfig, source: WindsurfMcpConfig) {
    for (name, server) in source.mcp_servers {
        target.mcp_servers.insert(name, server);
    }
}

pub async fn save_project_mcp_config(config: &WindsurfMcpConfig) -> Result<(), McpError> {
    let path = project_star_mcp_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let content = serde_json::to_string_pretty(config)?;
    tokio::fs::write(path, content).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_order_lets_higher_scope_replace_the_whole_server() {
        let mut target = WindsurfMcpConfig::default();
        let mut lower = WindsurfMcpConfig::default();
        lower.mcp_servers.insert(
            "exa".to_string(),
            WindsurfMcpServer {
                command: Some("lower".to_string()),
                args: None,
                env: None,
                disabled: None,
                transport_type: None,
                url: None,
            },
        );
        let mut higher = WindsurfMcpConfig::default();
        higher.mcp_servers.insert(
            "exa".to_string(),
            WindsurfMcpServer {
                command: Some("higher".to_string()),
                args: None,
                env: None,
                disabled: None,
                transport_type: None,
                url: None,
            },
        );

        merge_into(&mut target, lower);
        merge_into(&mut target, higher);

        // 整条替换，不逐字段合并 —— 行为必须可预测。
        assert_eq!(
            target
                .mcp_servers
                .get("exa")
                .and_then(|s| s.command.clone()),
            Some("higher".to_string())
        );
    }

    #[test]
    fn claude_json_reader_ignores_everything_but_mcp_servers() {
        // `~/.claude.json` 是个大杂烩：会话状态、缓存标志、projects……
        // 只认 `mcpServers`，其余一概不碰。
        let raw = r#"{
            "numStartups": 42,
            "userID": "abc",
            "projects": {"/tmp/x": {"mcpServers": {"ignored": {"command": "no"}}}},
            "mcpServers": {"exa": {"command": "npx", "args": ["-y", "exa-mcp-server"]}}
        }"#;
        let root: ClaudeJsonMcpRoot =
            crate::core::config::json_with_comments::parse_json_with_comments(raw).unwrap();
        assert_eq!(root.mcp_servers.len(), 1);
        assert!(root.mcp_servers.contains_key("exa"));
    }
}
