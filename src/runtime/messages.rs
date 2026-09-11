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

/// 一条全局搜索匹配，作为 UI/worker 协议数据而非组件私有渲染状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalSearchMatch {
    pub file: String,
    pub line_number: usize,
    pub content: String,
    pub score: i32,
}

#[derive(Clone, Debug)]
pub enum PendingCheckpointAction {
    List { message_id: u64 },
    Restore { message_id: u64, id: String },
}

/// 流开始的来源。只有用户回合会重置最近一次 provider 用量，并固定该逻辑请求的计价模型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamStartKind {
    UserTurn {
        model: String,
    },
    /// 压缩、checkpoint、确认后的工具执行等非模型回合操作。
    Operation,
}

#[derive(Clone, Debug)]
pub enum StreamMessage {
    Start {
        message_id: u64,
        kind: StreamStartKind,
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
    ModelsList {
        models: Vec<crate::types::ModelInfo>,
        /// `None` = 刚从 API 拉的；`Some(n)` = 命中缓存，n 秒前拉的。
        /// UI 用它在模型面板上显示"多久之前拉的"，好让用户判断要不要刷新。
        cache_age_secs: Option<u64>,
    },
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
    PluginOpResult {
        message: Option<String>,
    },
    /// 全局搜索后台任务完成。取消任务不发送结果，避免无效 UI 刷新。
    GlobalSearchResults {
        request_id: u64,
        results: Vec<GlobalSearchMatch>,
        truncated: bool,
    },
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
    /// 请求模型列表。`force = false`（打开面板等隐式触发）走缓存优先的便宜路径；
    /// `force = true`（面板里的 `⟳`）跳过缓存并扇出到所有已配置 provider。
    /// 详见 `agent::model_list` 的模块说明。
    ListModels {
        force: bool,
    },
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
    /// `!command` 的输出：追加进会话上下文，但**不触发**模型回合。
    ///
    /// 对标 Claude Code —— 用户在 `!` 里跑的命令，模型下一轮要能看到结果，
    /// 否则「跑一下测试再修」这种最常见的用法要靠用户手动把输出粘回去。
    /// `!!command` 走本地执行、不调用这条消息。
    AppendContext {
        content: String,
    },
    ToggleYoloMode,
    SetApprovalMode(crate::types::ApprovalMode),
    /// 重新读取磁盘上的权限规则（`/permissions allow|deny|ask|remove` 改完 settings 之后）。
    /// PolicyEngine 活在 MessageBus 里，UI 侧改不到，只能靠这条消息让运行时自己重载。
    ReloadPermissions,
    /// 思考力度档位（Alt+T / `/effort` / 命令面板）。UI 侧只改显示，
    /// 真正让它作用到请求上要靠这条消息落到 `llm::thinking`。
    SetThinkingEffort(crate::types::ThinkingEffort),
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
        /// 对标 Claude Code: 用户在确认时可附加反馈文本 (Tab to provide feedback)
        feedback: Option<String>,
    },
    EmitStatus(String),
    /// 保存 coherent UI transcript 与 worker 所有的原生上下文。
    SaveSession {
        id: String,
        history: Vec<crate::types::ChatEntry>,
        last_usage: Option<crate::types::StarUsage>,
        response: tokio::sync::mpsc::Sender<Result<(), String>>,
    },
    /// 恢复已加载且验证过的原生会话上下文和待注入本地命令输出。
    RestoreSession {
        messages: Vec<crate::types::StarMessage>,
        pending_local_context: Vec<String>,
        response: tokio::sync::mpsc::Sender<Result<(), String>>,
    },
    /// 完成已排队的会话操作后结束 worker。用于 TUI 退出时等待终态快照和 session-end hooks。
    Shutdown {
        response: tokio::sync::mpsc::Sender<Result<(), String>>,
    },
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
    /// 全局搜索在 worker 后台运行，避免 ripgrep 阻塞终端键盘事件。
    RunGlobalSearch {
        request_id: u64,
        query: String,
        cwd: std::path::PathBuf,
        cancellation: tokio_util::sync::CancellationToken,
    },
}

/// 可在后台执行的插件市场操作。结果通过
/// [`StreamMessage::PluginOpResult`] 回传 UI。
#[derive(Clone, Debug)]
pub enum PluginOp {
    EnsureDefaultMarketplace,
    AddMarketplace {
        source: String,
    },
    RemoveMarketplace {
        name: String,
    },
    /// 更新 marketplace 内容（官方走 GCS 比对，其他重新 clone）
    UpdateMarketplace {
        name: String,
    },
    InstallPlugin {
        plugin: crate::core::plugins::marketplace::MarketplacePlugin,
        /// 安装范围："user" 或 "project"（对标 Claude Code）
        scope: String,
    },
}
