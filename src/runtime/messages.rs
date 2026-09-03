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
        /// teammate 自定义名称（`@name` 显示）
        name: Option<String>,
        /// 后台运行时替代 "Done" 的描述
        task_description: Option<String>,
        /// 新增的子消息（追加到现有列表）
        new_sub_entries: Vec<ChatEntry>,
    },
    /// 插件市场后台操作完成：`None` 表示无需提示的成功（如已注册过）
    PluginOpResult { message: Option<String> },
    /// /summary、/recap 旁路生成完成（不进入主对话上下文）
    NoteGenerated {
        message_id: u64,
        kind: NoteKind,
        content: String,
    },
}

/// 旁路笔记类型（/summary 全文摘要、/recap 一句话回顾、/btw 旁路问答）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Summary,
    Recap,
    /// /btw：不进入主上下文的一次性提问
    Aside,
}

impl NoteKind {
    pub fn label(&self) -> &'static str {
        match self {
            NoteKind::Summary => "Session Summary",
            NoteKind::Recap => "Session Recap",
            NoteKind::Aside => "Aside",
        }
    }
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
    /// /summary、/recap、/btw：让 agent 做一次旁路 LLM 生成，
    /// 结果经 [`StreamMessage::NoteGenerated`] 回 UI，不污染主上下文。
    GenerateNote {
        kind: NoteKind,
        message_id: u64,
        /// /btw 的问题；/summary、/recap 为 `None`
        question: Option<String>,
    },
    /// 插件市场后台操作（git clone / 删除仓库目录等耗时操作，避免阻塞 UI 事件循环）
    PluginOp {
        project_root: std::path::PathBuf,
        op: PluginOp,
    },
}

/// 可在后台执行的插件市场操作。结果通过
/// [`StreamMessage::PluginOpResult`] 回传 UI。
#[derive(Clone, Debug)]
pub enum PluginOp {
    EnsureDefaultMarketplace,
    AddMarketplace { source: String },
    RemoveMarketplace { name: String },
    /// 更新 marketplace 内容（官方走 GCS 比对，其他重新 clone）
    UpdateMarketplace { name: String },
    InstallPlugin {
        plugin: crate::core::plugins::marketplace::MarketplacePlugin,
        /// 安装范围："user" 或 "project"（对标 Claude Code）
        scope: String,
    },
}
