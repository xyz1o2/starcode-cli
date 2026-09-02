use std::sync::Arc;

pub mod types;
pub use types::*;

#[derive(Debug, Clone)]
pub struct SubAgentRequest {
    pub prompt: String,
    pub max_rounds: Option<u32>,
}

impl SubAgentRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            max_rounds: None,
        }
    }

    pub fn with_max_rounds(mut self, max_rounds: u32) -> Self {
        self.max_rounds = Some(max_rounds);
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
