use std::sync::Arc;

pub mod types;
pub use types::*;

#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    pub prompt: String,
    pub max_rounds: Option<u32>,
    /// 命名子代理类型。runner 按它派发：`Explorer` 走确定性检索
    /// （`ExploreAgent`），其余走通用 LLM 循环。
    pub subagent_type: SubagentType,
    /// `/subtask` 的 forked subagent 在开始前继承的父会话消息。
    ///
    /// 普通 AgentTool 调用为空；只有明确要求复制当前对话的路径才填充。
    /// 这里保留原始 `StarMessage` 而不是拼成文本，子代理才能继续看到
    /// tool_call/tool_result 等结构，而不是一段有损摘要。
    pub parent_messages: Vec<crate::types::StarMessage>,
    /// 父会话里尚未随下一条用户消息发出的本地命令输出。
    pub parent_pending_context: Vec<String>,
}

impl SubAgentRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            max_rounds: None,
            subagent_type: SubagentType::GeneralPurpose,
            parent_messages: Vec::new(),
            parent_pending_context: Vec::new(),
        }
    }

    pub fn with_max_rounds(mut self, max_rounds: u32) -> Self {
        self.max_rounds = Some(max_rounds);
        self
    }

    pub fn with_subagent_type(mut self, subagent_type: SubagentType) -> Self {
        self.subagent_type = subagent_type;
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
