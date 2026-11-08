//! AgentTool 分流决策树。
//!
//! 对标 CCB sub-agents.mdx §路由规则 + coordinator-and-swarm.mdx §AgentTool 分流。
//! 纯决策函数：根据参数和上下文返回 AgentRoute，不执行任何副作用。

use crate::core::agents::{
    AgentExecutionMode, AgentIsolation, AgentToolFullInput, SubAgentRequest, SubagentType,
};

// ── AgentRoute ───────────────────────────────────────────────────────────

/// 分流结果：决定 SubAgent 以什么方式执行
#[derive(Debug, Clone)]
pub enum AgentRoute {
    /// 同步命名 SubAgent — 阻塞等待，返回 tool_result
    SyncNamedAgent {
        /// UI 任务 ID：同步 Agent 也需要一个稳定 id，才能在 chat_history
        /// 里创建/更新对应的 AgentTask 条目
        agent_id: String,
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
                parent_messages: Vec::new(),   // 由调用方填充
                parent_tool_names: Vec::new(), // 由调用方填充
                description: input.description.clone(),
            },
        };
    }

    // 规则 2.5：Explore 默认后台（对标 CC 2.1.196「Subagents now run in the
    // background by default … and is notified when they finish」—— 检索类
    // 子代理一次性、可并行、结果自足，阻塞主线程没有收益；前台等待只在
    // 调用方显式 `background=false` 时保留）。
    if matches!(input.subagent_type.as_ref(), Some(SubagentType::Explorer))
        && input.background.unwrap_or(true)
    {
        return AgentRoute::AsyncAgent {
            agent_id: generate_agent_id(),
            request: build_request(input),
            name: input.name.clone(),
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
            agent_id: generate_agent_id(),
            subagent_type: subagent_type.clone(),
            request: build_request(input),
        };
    }

    // 规则 5：省略 subagent_type → 默认同步 general-purpose
    AgentRoute::SyncNamedAgent {
        agent_id: generate_agent_id(),
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
        subagent_type: input
            .subagent_type
            .clone()
            .unwrap_or(SubagentType::GeneralPurpose),
        parent_messages: Vec::new(),
        parent_pending_context: Vec::new(),
        identity: None,
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
        assert!(matches!(
            route,
            AgentRoute::SyncNamedAgent {
                subagent_type: SubagentType::GeneralPurpose,
                ..
            }
        ));
    }

    /// 回归：`subagent_type` 必须一路透到 `SubAgentRequest`，runner 靠它
    /// 派发 explorer 到 `run_explore`。丢在路由上 explorer 就退回通用 agent。
    #[test]
    fn request_carries_subagent_type_through_route() {
        let mut input = default_input();
        input.subagent_type = Some(SubagentType::Explorer);
        let route = route_agent_call(&input, false, false);
        match route {
            AgentRoute::SyncNamedAgent { request, .. } => {
                assert_eq!(request.subagent_type, SubagentType::Explorer);
            }
            _ => panic!("explorer input must route to SyncNamedAgent"),
        }
    }

    #[test]
    fn request_defaults_to_general_purpose_type() {
        let route = route_agent_call(&default_input(), false, false);
        match route {
            AgentRoute::SyncNamedAgent { request, .. } => {
                assert_eq!(request.subagent_type, SubagentType::GeneralPurpose);
            }
            _ => panic!(),
        }
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
        assert!(matches!(
            route,
            AgentRoute::SyncNamedAgent {
                subagent_type: SubagentType::CodeReviewer,
                ..
            }
        ));
    }

    /// 对标 CC 2.1.178「隐式 team」：Agent 的 `name` 参数即注册进隐式 team 的
    /// 可寻址名。路由侧的保证是：**带 background 的调用把 name 原样带出**，
    /// `AgentTool` 据此 `with_identity(name)` → `config.agent_identity`，因此
    /// SendMessage 的 name 寻址、worker 内部回话用的发件人、hub 注册键三者同源。
    /// 没有 name 的后台 spawn 继续匿名（identity None，回落 agent_id）。
    #[test]
    fn named_background_spawn_carries_name_for_hub_addressing() {
        let mut input = default_input();
        input.background = Some(true);
        input.name = Some("researcher".to_string());

        match route_agent_call(&input, false, false) {
            AgentRoute::AsyncAgent { request, name, .. } => {
                assert_eq!(name.as_deref(), Some("researcher"));
                // request.identity 由 AgentTool 设置（见 agent_tool.rs AsyncAgent 分支）；
                // 这里锁定路由只对 background 生效、name 不被路由层吞掉。
                assert!(
                    request.identity.is_none(),
                    "路由层不注入 identity（由 AgentTool 决定）"
                );
            }
            other => panic!("expected AsyncAgent, got {:?}", other),
        }
    }

    /// 对标 CC 2.1.196「Subagents now run in the background by default」里
    /// 检索类子代理的那一半：Explorer **默认后台**（规则 2.5），显式
    /// `background=false` 才回落同步；非 Explorer 类型同步默认不变。
    #[test]
    fn explorer_defaults_to_background_but_respects_explicit_sync() {
        // Explorer 无 background 参数 → 后台
        let mut input = default_input();
        input.subagent_type = Some(SubagentType::Explorer);
        assert!(matches!(
            route_agent_call(&input, false, false),
            AgentRoute::AsyncAgent { .. }
        ));

        // 显式 background=false → 保留同步等待
        input.background = Some(false);
        assert!(matches!(
            route_agent_call(&input, false, false),
            AgentRoute::SyncNamedAgent { .. }
        ));

        // 显式 background=true → 后台（与规则 3 一致）
        input.background = Some(true);
        assert!(matches!(
            route_agent_call(&input, false, false),
            AgentRoute::AsyncAgent { .. }
        ));

        // 非 Explorer 默认不动：general-purpose 无 background 还是同步
        let default = default_input();
        assert!(matches!(
            route_agent_call(&default, false, false),
            AgentRoute::SyncNamedAgent { .. }
        ));
    }
}
