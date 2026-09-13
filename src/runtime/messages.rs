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
use crate::core::context_policy::{
    ContextWindowSelection, ResolvedContextPolicy, RuntimeSettingOutcome,
};
use crate::types::{AgentTaskStatus, ChatEntry, StarToolCall, ThinkingEffort, ToolResult};
use serde::{Deserialize, Serialize};

/// 一次模型切换的完整目标。provider 与 model 必须作为一个逻辑设置更新，
/// 避免 UI 只看到其中一半已经生效。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeModelSelection {
    pub model: String,
    pub provider_id: Option<String>,
}

/// 运行时变更是否应写入用户设置文件。revision 描述协议顺序，不能再兼任持久化开关：
/// `/fast` 等临时会话行为同样需要 revision acknowledgement，但绝不能改磁盘设置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSettingsPersistencePolicy {
    #[default]
    Persistent,
    SessionOnly,
}

/// UI 发给 worker 的原子运行时设置变更。`ui_revision` 由 UI 单调递增，
/// worker 据此保留流式期间的 FIFO 顺序并让 UI 丢弃过时确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSettingsMutation {
    pub ui_revision: u64,
    /// 是否在 runtime 安装成功后异步写入设置文件。
    #[serde(default)]
    pub persistence: RuntimeSettingsPersistencePolicy,
    pub model: Option<RuntimeModelSelection>,
    pub context_window: Option<ContextWindowSelection>,
    pub thinking_effort: Option<ThinkingEffort>,
}

impl RuntimeSettingsMutation {
    /// `0` 保留给尚未迁移的兼容调用点；UI 发起的请求必须从 1 开始单调递增。
    pub const LEGACY_UI_REVISION: u64 = 0;

    pub fn has_changes(&self) -> bool {
        self.model.is_some() || self.context_window.is_some() || self.thinking_effort.is_some()
    }

    pub fn legacy_model(model: String, provider_id: Option<String>) -> Self {
        Self {
            ui_revision: Self::LEGACY_UI_REVISION,
            persistence: RuntimeSettingsPersistencePolicy::Persistent,
            model: Some(RuntimeModelSelection { model, provider_id }),
            context_window: None,
            thinking_effort: None,
        }
    }

    pub fn legacy_thinking_effort(thinking_effort: ThinkingEffort) -> Self {
        Self {
            ui_revision: Self::LEGACY_UI_REVISION,
            persistence: RuntimeSettingsPersistencePolicy::Persistent,
            model: None,
            context_window: None,
            thinking_effort: Some(thinking_effort),
        }
    }
}

/// 用户最近请求的运行时设置。它在 provider 降级或拒绝某项设置时依然保留，
/// 因此 UI 不会把用户意图误显示成当前实际状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedRuntimeSettings {
    pub model: RuntimeModelSelection,
    pub context_window: ContextWindowSelection,
    pub thinking_effort: ThinkingEffort,
}

/// worker 当前确认已生效的运行时设置。上下文策略同时携带名义容量、
/// provider 安全上限后的有效容量、来源和派生阈值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveRuntimeSettings {
    pub model: RuntimeModelSelection,
    pub context_policy: ResolvedContextPolicy,
    pub thinking_effort: ThinkingEffort,
}

/// worker 所有的完整运行时设置状态。`runtime_revision` 只在 worker 成功
/// 改变可供后续逻辑回合使用的状态时单调递增。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSettingsSnapshot {
    pub runtime_revision: u64,
    pub requested: RequestedRuntimeSettings,
    pub active: ActiveRuntimeSettings,
}

/// 对一个 UI 设置变更的终态确认。失败或降级时快照仍会同时报告保留的请求
/// 和实际 active 状态；`reason` 描述 mutation 层面的失败或 supersession。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSettingsAcknowledgement {
    pub ui_revision: u64,
    pub outcome: RuntimeSettingOutcome,
    pub snapshot: RuntimeSettingsSnapshot,
    pub reason: Option<String>,
}

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
    /// worker 对一次带 UI revision 的运行时设置变更的终态确认。
    RuntimeSettingsAcknowledged(RuntimeSettingsAcknowledgement),
    /// 用户设置文件写入是独立于 runtime application 的后续结果。
    /// `None` 表示成功；worker acknowledgement 绝不依赖这条消息。
    RuntimeSettingsPersistenceCompleted {
        ui_revision: u64,
        error: Option<String>,
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
    /// 单一运行时设置协议入口。流式回合期间会按接收顺序排队，
    /// 并且只会在该逻辑回合结束后才影响下一条用户消息。
    UpdateRuntimeSettings(RuntimeSettingsMutation),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_mutation_reports_whether_it_changes_any_setting() {
        let empty = RuntimeSettingsMutation {
            ui_revision: 1,
            persistence: RuntimeSettingsPersistencePolicy::Persistent,
            model: None,
            context_window: None,
            thinking_effort: None,
        };
        assert!(!empty.has_changes());

        let context_only = RuntimeSettingsMutation {
            ui_revision: 2,
            persistence: RuntimeSettingsPersistencePolicy::Persistent,
            model: None,
            context_window: Some(ContextWindowSelection::Fixed(1_000_000)),
            thinking_effort: None,
        };
        assert!(context_only.has_changes());
    }

    #[test]
    fn runtime_snapshot_keeps_requested_and_active_context_distinct() {
        let requested = RequestedRuntimeSettings {
            model: RuntimeModelSelection {
                model: "model-a".to_string(),
                provider_id: Some("provider-a".to_string()),
            },
            context_window: ContextWindowSelection::Fixed(1_000_000),
            thinking_effort: ThinkingEffort::High,
        };
        let policy = ResolvedContextPolicy::from_evidence(
            requested.context_window,
            crate::core::context_policy::ContextWindowEvidence {
                provider_safe_cap: Some(200_000),
                ..Default::default()
            },
        );
        let snapshot = RuntimeSettingsSnapshot {
            runtime_revision: 4,
            requested,
            active: ActiveRuntimeSettings {
                model: RuntimeModelSelection {
                    model: "model-a".to_string(),
                    provider_id: Some("provider-a".to_string()),
                },
                context_policy: policy,
                thinking_effort: ThinkingEffort::High,
            },
        };

        assert_eq!(
            snapshot.requested.context_window,
            ContextWindowSelection::Fixed(1_000_000)
        );
        assert_eq!(snapshot.active.context_policy.effective_tokens, 200_000);
        assert_eq!(
            snapshot.active.context_policy.outcome,
            RuntimeSettingOutcome::Degraded
        );
    }
}
