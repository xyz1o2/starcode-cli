use std::sync::Arc;

pub mod types;
pub use types::*;

#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    pub prompt: String,
    pub max_rounds: Option<u32>,
    /// 单次调用的模型覆盖；None 或 inherit 继承父级模型及 provider。
    pub model: Option<String>,
    /// 本次子代理调用使用的思考力度，按逻辑回合冻结。
    pub effort: Option<crate::types::ThinkingEffort>,
    /// 恢复当前项目中已结束子代理的有效原生上下文；本次执行使用新任务身份。
    pub resume: Option<String>,
    /// 子代理类型：共享流式循环，Explorer 选择专用搜索提示与工具白名单。
    pub subagent_type: SubagentType,
    /// fork 与 `/subtask` 子代理在开始前继承的父会话消息。
    ///
    /// 普通 AgentTool 调用为空；只有明确要求复制当前对话的路径才填充。
    /// 这里保留原始 `StarMessage` 而不是拼成文本，子代理才能继续看到
    /// tool_call/tool_result 等结构，而不是一段有损摘要。
    pub parent_messages: Vec<crate::types::StarMessage>,
    /// 父会话里尚未随下一条用户消息发出的本地命令输出。
    pub parent_pending_context: Vec<String>,
    /// 本子代理在消息 hub 里的身份（mailbox 键）。
    ///
    /// 异步 runner 在启动前统一写入任务 ID；可读名称仅作为路由别名。
    /// 直接同步调用可显式提供身份；未提供时 runner 生成临时身份。
    /// 设计文档：`docs/agent-messaging-design.zh-CN.md` §6 Phase 3。
    pub identity: Option<String>,
    /// **发起方**的递归深度 —— 本子代理将落在 `depth + 1`。
    ///
    /// # 为什么必须由调用方显式带上
    ///
    /// 子代理的深度不能靠"数自己被创建了几层"推导出来：创建子代理的
    /// `AgentTool` 装在**共享的** `ToolRegistry` 里（见
    /// `core/config/runtime_bootstrap/agent_runtime.rs`），它持有的是**父级**
    /// `Arc<Config>`。子代理再调 `Agent` 时，从注册表拿到的仍是那个父级
    /// runner —— 于是深度永远停在父级那一层，链式派生收不住。
    ///
    /// 显式带上 `depth` 后，每一级都从**真实的调用链**上数，派生关系与
    /// 深度不再依赖注册表里恰好装着谁的 runner。
    pub parent_depth: usize,
}

impl SubAgentRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            max_rounds: None,
            model: None,
            effort: None,
            resume: None,
            subagent_type: SubagentType::GeneralPurpose,
            parent_messages: Vec::new(),
            parent_pending_context: Vec::new(),
            identity: None,
            parent_depth: 0,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_effort(mut self, effort: crate::types::ThinkingEffort) -> Self {
        self.effort = Some(effort);
        self
    }

    pub fn with_max_rounds(mut self, max_rounds: u32) -> Self {
        self.max_rounds = Some(max_rounds);
        self
    }

    pub fn with_subagent_type(mut self, subagent_type: SubagentType) -> Self {
        self.subagent_type = subagent_type;
        self
    }

    /// 设置本子代理在消息 hub 里的身份（有 identity 才能收 SendMessage）。
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// 继承父会话的完整 provider 上下文。
    pub fn with_parent_context(
        mut self,
        messages: Vec<crate::types::StarMessage>,
        pending_context: Vec<String>,
    ) -> Self {
        self.parent_messages = messages;
        self.parent_pending_context = pending_context;
        self
    }

    /// 记录**发起方**所处的递归深度（本子代理落在 `depth + 1`）。
    pub fn with_parent_depth(mut self, depth: usize) -> Self {
        self.parent_depth = depth;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct SubAgentResult {
    pub output: String,
    /// 执行过程中产生的所有条目（包括 ToolCall、Assistant 消息等）
    pub entries: Vec<crate::types::ChatEntry>,
    /// 工具调用次数（口径同 Claude Code `calculateAgentStats`：数 tool_result）
    pub tool_use_count: u32,
    /// 累计 token 用量
    pub total_tokens: u32,
    /// 最近一次工具的语义摘要，用于完成态仍需展示时回填
    pub last_tool_info: Option<String>,
}

impl SubAgentResult {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubAgentErrorKind {
    RecursionLimitExceeded,
    InitializationFailed,
    ExecutionFailed,
}

#[derive(Debug, Clone)]
pub struct SubAgentError {
    pub kind: SubAgentErrorKind,
    pub message: String,
}

impl SubAgentError {
    pub fn new(kind: SubAgentErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SubAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SubAgentError {}

pub type SharedSubAgentRunner = Arc<crate::agent::StarAgentRunner>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StarMessage;

    #[test]
    fn parent_context_is_empty_for_ordinary_agent_calls() {
        let request = SubAgentRequest::new("review this change");
        assert!(request.parent_messages.is_empty());
        assert!(request.parent_pending_context.is_empty());
    }

    #[test]
    fn forked_subtask_carries_structured_parent_messages() {
        let parent = vec![
            StarMessage::new("user", Some("investigate login".to_string())),
            StarMessage::new("assistant", Some("I found the auth module.".to_string())),
        ];
        let request = SubAgentRequest::new("write the regression test")
            .with_parent_context(parent.clone(), vec!["cargo test output".to_string()]);

        assert_eq!(request.parent_messages.len(), 2);
        assert_eq!(request.parent_messages[0].role, "user");
        assert_eq!(request.parent_messages[1].role, "assistant");
        assert_eq!(request.parent_pending_context, vec!["cargo test output"]);
    }
}
