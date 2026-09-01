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

#[derive(Debug, Clone)]
pub struct SubAgentResult {
    pub output: String,
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
