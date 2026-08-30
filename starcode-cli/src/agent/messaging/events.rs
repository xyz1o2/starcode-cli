use crate::types::{StarToolCall, ToolProgressStatus, ToolResult};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Streaming text delta from LLM
    TextDelta(String),
    /// Streaming reasoning/thinking delta from LLM (for models with reasoning_content)
    ReasoningDelta(String),
    /// Structured trace event for transcript / harness consumers
    Trace {
        event: String,
        payload: serde_json::Value,
    },
    /// Agent decided to call a tool
    ToolStarted { tool_call: StarToolCall },
    /// Tool execution finished
    ToolFinished {
        tool_call: StarToolCall,
        result: ToolResult,
    },
    /// Tool execution emitted progress output
    ToolProgress {
        tool_name: String,
        tool_call_id: Option<String>,
        status: ToolProgressStatus,
        message: String,
        current: Option<u32>,
        total: Option<u32>,
    },
    /// Agent produced a final text message for the user
    Message(String),
    /// An error occurred
    Error(String),
    /// Execution finished for the current turn
    TurnFinished,
    /// Execution completely finished (e.g. max turns reached or task done)
    Done,
    /// Stats update (Token usage, AU2 state)
    StatsUpdate {
        token_usage: Option<crate::types::StarUsage>,
    },
}
