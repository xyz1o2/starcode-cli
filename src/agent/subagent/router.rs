//! AgentTool 分流决策树。
//!
//! 对标 CCB sub-agents.mdx §路由规则 + coordinator-and-swarm.mdx §AgentTool 分流。
//! 纯决策函数：根据参数和上下文返回 AgentRoute，不执行任何副作用。

use crate::core::agents::{AgentExecutionMode, AgentIsolation, AgentToolFullInput, SubAgentRequest, SubagentType};

// ── AgentRoute ───────────────────────────────────────────────────────────

/// 分流结果：决定 SubAgent 以什么方式执行
#[derive(Debug, Clone)]
pub enum AgentRoute {
    /// 同步命名 SubAgent — 阻塞等待，返回 tool_result
    SyncNamedAgent {
        subagent_type: SubagentType,
        request: SubAgentRequest,
    },
    /// 异步 SubAgent — 后台执行，通知回流
    AsyncAgent {
        agent_id: String,
        request: SubAgentRequest,
        name: Option<String>,
    },
    /// Coordinator Worker — 强制异步 + 工具池受限
    CoordinatorWorker {
        agent_id: String,
        request: SubAgentRequest,
    },
    /// Fork Agent — 继承父上下文 + exact tools，并行探索分支
    ForkAgent {
        agent_id: String,
        request: ForkRequest,
    },
}

/// Fork Agent 请求：包含父上下文信息
#[derive(Debug, Clone)]
pub struct ForkRequest {
    /// 基础请求
    pub base: SubAgentRequest,
    /// 父 Agent 的对话历史（用于继承上下文）
    pub parent_messages: Vec<crate::types::StarMessage>,
    /// 父 Agent 的活跃工具名称列表
    pub parent_tool_names: Vec<String>,
    /// Fork 描述（用于 UI 显示）
    pub description: String,
}

// ── Fork Gate 配置 ──────────────────────────────────────────────────────

/// 检查 Fork Agent 是否启用
///
/// 环境变量 `STAR_FORK_AGENT_ENABLED=true` 启用
/// 默认禁用（需要用户主动选择）
fn is_fork_agent_enabled() -> bool {
    std::env::var("STAR_FORK_AGENT_ENABLED")
        .ok()
        .map(|v| {
            let v = v.trim().to_lowercase();
            matches!(v.as_str(), "1" | "true" | "on" | "yes")
        })
        .unwrap_or(false)
}

/// Fork 条件判断：是否应该使用 Fork 而非普通 Async
///
/// Fork 适用于：
/// - 需要继承父上下文的并行探索
/// - 多个独立的研究/验证任务
/// - 需要共享对话历史的分支实验
fn should_fork(input: &AgentToolFullInput) -> bool {
    // 检查 description 是否包含 "fork" 关键词（用户通过描述请求 fork）
    if input.description.to_lowercase().contains("fork") {
        return true;
    }
    // 检查 isolation 是否为 Worktree（fork 的一种形式）
    if input.isolation == Some(AgentIsolation::Worktree) {
        return true;
    }
    false
}

// ── 核心分流函数 ─────────────────────────────────────────────────────────

/// 根据输入参数和上下文决定执行路径
///
/// # 参数
/// - `input`: AgentTool 完整输入
/// - `is_coordinator`: 当前是否处于 Coordinator Mode
/// - `agent_def_background`: 命名 agent 定义是否标记了 `background: true`
pub fn route_agent_call(
    input: &AgentToolFullInput,
    is_coordinator: bool,
    agent_def_background: bool,
) -> AgentRoute {
    // 规则 1：Coordinator 模式下所有 Agent 调用强制走 Worker 路径
    if is_coordinator {
        return AgentRoute::CoordinatorWorker {
            agent_id: generate_agent_id(),
            request: build_request(input),
        };
    }

    // 规则 2：Fork Agent — 省略 subagent_type + fork gate 启用时走此路径
    if is_fork_agent_enabled() && should_fork(input) {
        return AgentRoute::ForkAgent {
            agent_id: generate_agent_id(),
            request: ForkRequest {
                base: build_request(input),
                parent_messages: Vec::new(), // 由调用方填充
                parent_tool_names: Vec::new(), // 由调用方填充
                description: input.description.clone(),
            },
        };
    }

    // 规则 3：显式 background 参数 或 agent 定义 background → 异步
    if input.background.unwrap_or(false) || agent_def_background {
        return AgentRoute::AsyncAgent {
            agent_id: generate_agent_id(),
            request: build_request(input),
            name: input.name.clone(),
        };
    }

    // 规则 4：subagent_type 有值 → 同步命名 SubAgent
    if let Some(subagent_type) = &input.subagent_type {
        return AgentRoute::SyncNamedAgent {
            subagent_type: subagent_type.clone(),
            request: build_request(input),
        };
    }

    // 规则 5：省略 subagent_type → 默认同步 general-purpose
    AgentRoute::SyncNamedAgent {
        subagent_type: SubagentType::GeneralPurpose,
        request: build_request(input),
    }
}

// ── 辅助函数 ─────────────────────────────────────────────────────────────

fn generate_agent_id() -> String {
    let id = uuid::Uuid::new_v4().to_string();
    format!("agent-{}", &id[..8])
}

fn build_request(input: &AgentToolFullInput) -> SubAgentRequest {
    SubAgentRequest {
        prompt: input.prompt.clone(),
        max_rounds: input.max_rounds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_input() -> AgentToolFullInput {
        AgentToolFullInput {
            description: "test agent".to_string(),
            prompt: "do something".to_string(),
            subagent_type: None,
            name: None,
            isolation: None,
            model: None,
            background: None,
            max_rounds: None,
        }
    }

    #[test]
    fn default_routes_to_sync_general() {
        let route = route_agent_call(&default_input(), false, false);
        assert!(matches!(route, AgentRoute::SyncNamedAgent {
            subagent_type: SubagentType::GeneralPurpose, ..
        }));
    }

    #[test]
    fn coordinator_forces_worker() {
        let route = route_agent_call(&default_input(), true, false);
        assert!(matches!(route, AgentRoute::CoordinatorWorker { .. }));
    }

    #[test]
    fn background_routes_to_async() {
        let mut input = default_input();
        input.background = Some(true);
        let route = route_agent_call(&input, false, false);
        assert!(matches!(route, AgentRoute::AsyncAgent { .. }));
    }

    #[test]
    fn agent_def_background_routes_to_async() {
        let route = route_agent_call(&default_input(), false, true);
        assert!(matches!(route, AgentRoute::AsyncAgent { .. }));
    }

    #[test]
    fn named_agent_routes_to_sync() {
        let mut input = default_input();
        input.subagent_type = Some(SubagentType::CodeReviewer);
        let route = route_agent_call(&input, false, false);
        assert!(matches!(route, AgentRoute::SyncNamedAgent {
            subagent_type: SubagentType::CodeReviewer, ..
        }));
    }
}
