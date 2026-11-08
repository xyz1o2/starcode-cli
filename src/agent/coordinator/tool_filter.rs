//! Coordinator Mode 工具池过滤。
//!
//! 对标 CCB coordinator-and-swarm.mdx §工具过滤状态机，并按设计文档
//! `docs/agent-messaging-design.zh-CN.md` §6 Phase 4 调整：
//! - Coordinator 主线程只能使用 Agent / SendMessage / TaskStop
//! - Worker 仍禁 TeamCreate / TeamDelete / SyntheticOutput（防嵌套编排），
//!   但**放行 SendMessage** —— worker 必须能主动汇报 / 求助 / 回话，
//!   通信不能只有 lead → worker 一个方向。

use crate::types::StarTool;

/// Coordinator 主线程允许的工具名列表
const COORDINATOR_ALLOWED_TOOLS: &[&str] =
    &["Agent", "SendMessage", "task_stop", "synthetic_output"];

/// Worker 工具池中排除的工具（防止嵌套编排；SendMessage 已放行）
const WORKER_DISALLOWED_TOOLS: &[&str] = &["team_create", "team_delete", "synthetic_output"];

/// 过滤 Coordinator 主线程工具池
pub fn filter_coordinator_tools(all_tools: &[StarTool]) -> Vec<StarTool> {
    all_tools
        .iter()
        .filter(|t| COORDINATOR_ALLOWED_TOOLS.contains(&t.function.name.as_str()))
        .cloned()
        .collect()
}

/// 过滤 Worker 工具池（排除不可用的工具）
pub fn filter_worker_tools(all_tools: &[StarTool]) -> Vec<StarTool> {
    all_tools
        .iter()
        .filter(|t| !WORKER_DISALLOWED_TOOLS.contains(&t.function.name.as_str()))
        .cloned()
        .collect()
}

/// MCP 工具不受额外限制（Worker 可使用已连接的 MCP 工具）
pub fn filter_worker_mcp_tools(all_tools: &[StarTool]) -> Vec<StarTool> {
    all_tools
        .iter()
        .filter(|t| t.function.name.starts_with("mcp__"))
        .cloned()
        .collect()
}
