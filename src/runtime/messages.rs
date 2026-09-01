/// Message types for UI ↔ Agent communication.
///
/// # Communication Protocol
///
/// The UI and Agent communicate through two channels:
/// - `AgentRequest` (UI → Agent): Commands like SendMessage, Abort, SetModel
/// - `StreamMessage` (Agent → UI): Events like Content, ToolCalls, Done, Error
///
/// # Channel Capacity
/// Both channels have capacity 100. When the receiver is slow:
/// - `AgentRequest.send().await` blocks (backpressure)
/// - `StreamMessage` uses `try_send` in hot paths to avoid blocking
///
use crate::types::{AgentTaskStatus, ChatEntry, StarToolCall, ToolResult};

#[derive(Clone, Debug)]
pub enum PendingCheckpointAction {
    List { message_id: u64 },
    Restore { message_id: u64, id: String },
}

#[derive(Clone, Debug)]
pub enum StreamMessage {
    Start {
        message_id: u64,
    },
    Content {
        message_id: u64,
        content: String,
    },
    TextDelta {
        message_id: u64,
        content: String,
    },
    ReasoningDelta {
        message_id: u64,
        content: String,
    },
    Thinking {
        message_id: u64,
        content: String,
    },
    AssistantNote {
        message_id: u64,
        content: String,
    },
    Trace {
        message_id: u64,
        event: String,
        payload: serde_json::Value,
    },
    ToolCalls {
        message_id: u64,
        tool_calls: Vec<StarToolCall>,
    },
    ToolResult {
        message_id: u64,
        tool_call: StarToolCall,
        tool_result: ToolResult,
    },
    ToolOutput {
        message_id: u64,
        tool_call_id: String,
        output: String,
    },
    TokenCount {
        message_id: u64,
        tokens: u32,
        usage: Option<crate::types::StarUsage>,
    },
    Done {
        message_id: u64,
    },
    Error {
        message_id: u64,
        error: String,
    },
    RestoreCheckpointApplied {
        message_id: u64,
        checkpoint_id: String,
        summary: String,
        chat_history: Vec<ChatEntry>,
    },
    ToolConfirmationRequest {
        message_id: u64,
        tool_call_id: String,
        confirmation: crate::types::ToolConfirmation,
    },
    ModelsList(Vec<crate::types::ModelInfo>),
    ModelsError(String),
    McpStatus {
        ready: bool,
        error: Option<String>,
    },
    McpServers(Vec<String>),
    McpTools {
        server: String,
        tools: Vec<String>,
    },
    ApprovalModeChanged {
        mode: crate::types::ApprovalMode,
    },
    ConfiguredProviders(Vec<String>),
    CurrentModelChanged {
        model: String,
        provider_id: Option<String>,
    },
    ReloadTasks,
    StatsUpdate {
        au2_compressed: bool,
        token_usage: Option<crate::types::StarUsage>,
    },
    UpdateGitStatus(String),
    StatusUpdate {
        message_id: u64,
        status: String,
    },
    /// Agent 任务生命周期更新（启动/进度/完成）
    AgentTaskUpdate {
        message_id: u64,
        task_id: String,
        agent_type: String,
        description: String,
        status: AgentTaskStatus,
        tool_use_count: u32,
        tokens: u32,
        is_async: bool,
        is_resolved: bool,
        is_error: bool,
        last_tool_info: Option<String>,
        /// 新增的子消息（追加到现有列表）
        new_sub_entries: Vec<ChatEntry>,
    },
}

#[derive(Clone, Debug)]
pub enum AgentRequest {
    SendMessage {
        message_id: u64,
        message: String,
    },
    ListModels,
    SetModel {
        model: String,
        provider_id: Option<String>,
    },
    UpdateModel {
        model: String,
        provider_id: Option<String>,
    },
    Abort,
    ListCheckpoints {
        message_id: u64,
    },
    RestoreCheckpoint {
        message_id: u64,
        id: String,
    },
    PluginToolsRefresh,
    McpRefresh,
    McpListServers,
    McpListTools {
        server: String,
    },
    UpdateProviderConfig {
        provider_id: Option<String>,
        api_key: Option<String>,
        base_url: Option<String>,
        is_openai_compatible: Option<bool>,
        model: Option<String>,
    },
    MarkFilesAsRead(Vec<String>),
    ToggleYoloMode,
    SetApprovalMode(crate::types::ApprovalMode),
    LoadConfiguredProviders,
    Compress {
        message_id: u64,
    },
    ResetSession,
    UpdateGitStatus(String),
    ToolConfirmationResponse {
        tool_calls: Vec<StarToolCall>,
        message_id: u64,
        approved: bool,
        always_allow: bool,
    },
    ConfirmTool {
        tool_call_id: String,
        outcome: crate::types::ToolConfirmationOutcome,
    },
    EmitStatus(String),
    ResumeSession(String),
}
