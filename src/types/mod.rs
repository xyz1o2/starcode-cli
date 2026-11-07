/// 类型定义模块
///
/// 对标claude-code-main的src/types/
pub mod command;
pub mod message;
pub mod permissions;
pub mod tools;

pub use command::*;
pub use message::*;
pub use permissions::*;
pub use tools::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// 给模型看的正文，与 `output` 分离。
    ///
    /// `output` 是**给人看**的摘要：UI 把它渲染成那一行折叠结果，模型不该读它。
    /// `llm_content` 是**给模型看**的完整正文：工具名列表、参数 schema、使用说明。
    ///
    /// 曾经这两个通道只有 `output` 一条，于是 `ToolSearch` 这类"摘要一行、
    /// 正文另存"的工具，模型只收到 `"Found 3 tool(s) for 'exa'"` —— 工具名和
    /// schema 全丢，模型只能反复瞎猜工具名。`llm_content` 存在却没人读，
    /// 等于把发现工具的唯一途径堵死了。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorCommand {
    pub command: EditorCommandType,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_str: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_str: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum EntryStatus {
    #[default]
    Normal, // 正常状态
    Success,    // 成功
    Error,      // 错误
    Warning,    // 警告
    Cancelled,  // 已取消
    InProgress, // 进行中
    Pending,    // 等待中
    /// 软失败：工具执行报错但回合仍在继续，红 ✗ 提级按「回合终结性」门控 ——
    /// 正常收尾保持灰化折叠显示，回合出错（Error）时提级回红（见
    /// `stream::handle_error_message`）。对标 Claude Code：工具失败不阻断回合时
    /// 只折叠呈现，不抢视觉焦点。
    SoftError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditorCommandType {
    View,
    StrReplace,
    Create,
    Insert,
    UndoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum ChatEntryType {
    #[default]
    User,
    Assistant,
    ToolResult,
    ToolCall,
    ToolConfirmation, // 工具确认卡片（内联显示在对话流中）

    // 新增消息类型
    SystemMessage,   // 系统消息（通知、警告等）
    ErrorMessage,    // 错误消息
    DiffBlock,       // Diff 块
    CodeBlock,       // 代码块
    ProgressMessage, // 进度消息
    CollapsedGroup,  // 折叠组（用于折叠多个消息）
    GroupedToolUse,  // 分组工具调用（连续的工具调用合并显示）
    CompactSummary,  // 压缩摘要
    AgentTask,       // 单个 Agent 任务条目（含子消息列表）
    AgentGroup,      // 多个 Agent 并发任务组
}

/// Agent 任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AgentTaskStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Background, // 异步后台运行
    Rejected,   // 用户拒绝授权（对标 renderToolUseRejectedMessage）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    #[serde(rename = "type")]
    pub entry_type: ChatEntryType,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StarToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<StarToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_streaming: Option<bool>,
    // 工具确认信息（用于内联确认卡片）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ToolConfirmation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Frozen elapsed ms when reasoning finished — prevents timer from ticking
    /// on completed thinking blocks while new blocks are still streaming.
    #[serde(skip)]
    pub reasoning_finished_elapsed_ms: Option<u128>,
    /// 思考块**开始**的时刻，用于流式期间的实时计时（"Thinking 5s…"）。
    ///
    /// 为什么不能用 `reasoning_finished_elapsed_ms`：那是结束时刻冻结的值，
    /// 流式期间它还是 `None`，于是进行中的思考块没有耗时可显示。而回合级的
    /// `state.thinking_started_at` 也不够用 —— 一轮里可能有多个思考块
    /// （工具调用前后各一个），它们各自的开始时刻不同。
    #[serde(skip)]
    pub reasoning_started_at: Option<std::time::Instant>,
    // Transient welcome header — never persisted to session files
    #[serde(skip)]
    pub is_welcome: bool,

    // 折叠相关字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed_entries: Option<Vec<ChatEntry>>, // 折叠的子条目
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_collapsed: Option<bool>, // 是否折叠
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse_summary: Option<String>, // 折叠摘要
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>, // 分组 ID

    // Diff 相关字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_content: Option<String>, // Diff 内容
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>, // 文件路径

    // 状态相关字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<EntryStatus>, // 条目状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>, // 进度 (0.0 - 1.0)

    // 每条回复的费用（仅 assistant 条目）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    // 工具执行耗时（毫秒，仅 ToolResult 条目）
    #[serde(skip)]
    pub tool_elapsed_ms: Option<u128>,

    // ============ Agent 任务相关字段 ============
    /// Agent 任务 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_id: Option<String>,
    /// Agent 类型（"fork" | "general-purpose" | "worker" ...）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Agent 描述
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_description: Option<String>,
    /// Agent 任务状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentTaskStatus>,
    /// Agent 工具使用次数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_tool_use_count: Option<u32>,
    /// Agent Token 使用量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_tokens: Option<u32>,
    /// Agent 是否完成
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_resolved: Option<bool>,
    /// Agent 是否出错
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_error: Option<bool>,
    /// Agent 是否异步
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_async: Option<bool>,
    /// Agent 最后工具信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_last_tool_info: Option<String>,
    /// Agent 内部子消息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_sub_entries: Option<Vec<ChatEntry>>,
    /// Agent 组的任务 ID 列表（仅 AgentGroup 类型）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_ids: Option<Vec<String>>,
    /// Agent 自定义名称（teammate `@name` 显示，对标 renderGroupedAgentToolUse 的 name）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// 后台 Agent 的任务描述（backgrounded 状态下替代 "Done" 显示）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_description: Option<String>,
    // ============================================
}

impl ChatEntry {
    pub fn new(entry_type: ChatEntryType, content: String) -> Self {
        Self {
            entry_type,
            content,
            timestamp: Utc::now(),
            tool_calls: None,
            tool_call: None,
            tool_result: None,
            is_streaming: None,
            confirmation: None,
            reasoning_content: None,
            reasoning_finished_elapsed_ms: None,
            reasoning_started_at: None,
            is_welcome: false,
            // 新增字段
            collapsed_entries: None,
            is_collapsed: None,
            collapse_summary: None,
            group_id: None,
            diff_content: None,
            file_path: None,
            status: None,
            progress: None,
            cost: None,
            tool_elapsed_ms: None,
            // Agent 任务字段
            agent_task_id: None,
            agent_type: None,
            agent_description: None,
            agent_status: None,
            agent_tool_use_count: None,
            agent_tokens: None,
            agent_is_resolved: None,
            agent_is_error: None,
            agent_is_async: None,
            agent_last_tool_info: None,
            agent_sub_entries: None,
            agent_task_ids: None,
            agent_name: None,
            agent_task_description: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(ChatEntryType::User, content.into())
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(ChatEntryType::Assistant, content.into())
    }

    pub fn assistant_with_tool_calls(tool_calls: Vec<StarToolCall>) -> Self {
        let mut entry = Self::new(ChatEntryType::Assistant, String::new());
        entry.tool_calls = Some(tool_calls);
        entry
    }

    pub fn tool_call(content: impl Into<String>, tool_call: StarToolCall) -> Self {
        let mut entry = Self::new(ChatEntryType::ToolCall, content.into());
        entry.tool_call = Some(tool_call);
        entry
    }

    pub fn tool_result(
        content: impl Into<String>,
        tool_call: StarToolCall,
        tool_result: ToolResult,
    ) -> Self {
        let mut entry = Self::new(ChatEntryType::ToolResult, content.into());
        entry.tool_call = Some(tool_call);
        entry.tool_result = Some(tool_result);
        entry
    }

    /// 失败的工具结果在到达时就置为软失败，避免回合中途闪一帧红 ✗ 又在
    /// Done 后变暗 —— 提级只发生在回合走到 Error（`handle_error_message`）时。
    pub fn with_soft_error_on_failure(mut self, tool_result: &ToolResult) -> Self {
        if !tool_result.success {
            self.status = Some(EntryStatus::SoftError);
        }
        self
    }

    pub fn with_streaming(mut self, is_streaming: bool) -> Self {
        self.is_streaming = Some(is_streaming);
        self
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning_content = Some(reasoning.into());
        self
    }

    pub fn with_confirmation(mut self, confirmation: ToolConfirmation) -> Self {
        self.confirmation = Some(confirmation);
        self
    }

    pub fn with_welcome(mut self) -> Self {
        self.is_welcome = true;
        self
    }

    // 新增构造函数
    pub fn system_message(content: impl Into<String>) -> Self {
        Self::new(ChatEntryType::SystemMessage, content.into())
    }

    pub fn error_message(content: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::ErrorMessage, content.into());
        entry.status = Some(EntryStatus::Error);
        entry
    }

    pub fn diff_block(diff_content: impl Into<String>, file_path: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::DiffBlock, String::new());
        entry.diff_content = Some(diff_content.into());
        entry.file_path = Some(file_path.into());
        entry
    }

    pub fn code_block(content: impl Into<String>, language: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::CodeBlock, content.into());
        entry.file_path = Some(language.into()); // 复用 file_path 存储语言
        entry
    }

    pub fn progress_message(content: impl Into<String>, progress: f32) -> Self {
        let mut entry = Self::new(ChatEntryType::ProgressMessage, content.into());
        entry.progress = Some(progress.clamp(0.0, 1.0));
        entry.status = Some(EntryStatus::InProgress);
        entry
    }

    pub fn collapsed_group(entries: Vec<ChatEntry>, summary: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::CollapsedGroup, String::new());
        entry.collapsed_entries = Some(entries);
        entry.collapse_summary = Some(summary.into());
        entry.is_collapsed = Some(true);
        entry
    }

    pub fn grouped_tool_use(entries: Vec<ChatEntry>, group_id: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::GroupedToolUse, String::new());
        entry.collapsed_entries = Some(entries);
        entry.group_id = Some(group_id.into());
        entry
    }

    pub fn with_status(mut self, status: EntryStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_progress(mut self, progress: f32) -> Self {
        self.progress = Some(progress.clamp(0.0, 1.0));
        self
    }

    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    pub fn with_group_id(mut self, id: impl Into<String>) -> Self {
        self.group_id = Some(id.into());
        self
    }

    // ── Agent 任务构造函数 ──

    /// 创建单个 Agent 任务条目
    pub fn agent_task(task_id: impl Into<String>, agent_type: impl Into<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::AgentTask, String::new());
        entry.agent_task_id = Some(task_id.into());
        entry.agent_type = Some(agent_type.into());
        entry.agent_status = Some(AgentTaskStatus::Running);
        entry.agent_tool_use_count = Some(0);
        entry.agent_tokens = Some(0);
        entry.agent_is_resolved = Some(false);
        entry.agent_is_error = Some(false);
        entry.agent_is_async = Some(false);
        entry.agent_sub_entries = Some(Vec::new());
        entry
    }

    /// 创建 Agent 并发任务组条目
    pub fn agent_group(task_ids: Vec<String>) -> Self {
        let mut entry = Self::new(ChatEntryType::AgentGroup, String::new());
        entry.agent_task_ids = Some(task_ids);
        entry
    }

    /// 设置 Agent 描述
    pub fn with_agent_description(mut self, desc: impl Into<String>) -> Self {
        self.agent_description = Some(desc.into());
        self
    }

    /// 设置 Agent 异步状态
    pub fn with_agent_async(mut self, is_async: bool) -> Self {
        self.agent_is_async = Some(is_async);
        self
    }

    /// 设置 Agent 自定义名称（teammate `@name`）
    pub fn with_agent_name(mut self, name: Option<String>) -> Self {
        self.agent_name = name.filter(|s| !s.trim().is_empty());
        self
    }

    /// 设置后台 Agent 的任务描述
    pub fn with_agent_task_description(mut self, desc: Option<String>) -> Self {
        self.agent_task_description = desc.filter(|s| !s.trim().is_empty());
        self
    }

    /// 该 Agent 是否处于「已转入后台」状态（对标 AgentProgressLine 的 isBackgrounded）
    pub fn agent_is_backgrounded(&self) -> bool {
        self.agent_is_async.unwrap_or(false) && self.agent_is_resolved.unwrap_or(false)
    }

    // 辅助方法
    pub fn is_collapsible(&self) -> bool {
        matches!(
            self.entry_type,
            ChatEntryType::CollapsedGroup | ChatEntryType::GroupedToolUse
        )
    }

    pub fn get_status_icon(&self) -> &'static str {
        match &self.status {
            Some(EntryStatus::Success) => "✓",
            Some(EntryStatus::Error) => "✗",
            Some(EntryStatus::SoftError) => "✗",
            Some(EntryStatus::Warning) => "⚠",
            Some(EntryStatus::Cancelled) => "⊘",
            Some(EntryStatus::InProgress) => "●",
            Some(EntryStatus::Pending) => "○",
            _ => "",
        }
    }

    pub fn get_status_color(&self) -> ratatui::style::Color {
        match &self.status {
            Some(EntryStatus::Success) => ratatui::style::Color::Green,
            Some(EntryStatus::Error) => ratatui::style::Color::Red,
            Some(EntryStatus::SoftError) => ratatui::style::Color::DarkGray,
            Some(EntryStatus::Warning) => ratatui::style::Color::Yellow,
            Some(EntryStatus::Cancelled) => ratatui::style::Color::DarkGray,
            Some(EntryStatus::InProgress) => ratatui::style::Color::Blue,
            Some(EntryStatus::Pending) => ratatui::style::Color::DarkGray,
            _ => ratatui::style::Color::White,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StarToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Anthropic-style prompt cache control hint.
    /// When set to `{"type": "ephemeral"}`, this content block will be
    /// cached server-side for reuse across turns, reducing input token
    /// costs by 30-50% for repeated system prompts.
    ///
    /// 这个字段**不参与请求构造** —— 请求侧的断点由两条路径各自打：
    /// Anthropic 走 rig 的 `with_prompt_caching()`，OpenAI 兼容走
    /// `llm::prompt_cache_ext::PromptCacheExt`。所以「provider 不支持」不是
    /// 「静默忽略」，而是「根本没发出去」。
    ///
    /// 它保留下来是为了让 prompt builder 一侧能区分静态段与准静态段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<serde_json::Value>,
}

impl StarMessage {
    pub fn new(role: impl Into<String>, content: impl Into<Option<String>>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            cache_control: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", Some(content.into()))
    }

    /// Create a system message with Anthropic-style prompt cache hint.
    /// Marks this content block with `cache_control: {"type": "ephemeral"}`
    /// so it's cached server-side for reuse across turns. Use this for
    /// static system prompt parts (core identity, security policy, etc.).
    pub fn cached_system(content: impl Into<String>) -> Self {
        let mut msg = Self::new("system", Some(content.into()));
        msg.cache_control = Some(serde_json::json!({"type": "ephemeral"}));
        msg
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", Some(content.into()))
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", Some(content.into()))
    }

    pub fn assistant_with_tool_calls(tool_calls: Vec<StarToolCall>) -> Self {
        let mut msg = Self::new("assistant", None::<String>);
        msg.tool_calls = Some(sanitize_tool_call_list(tool_calls));
        msg
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning_content = Some(reasoning.into());
        self
    }

    pub fn with_cache_control(mut self, cache_control: serde_json::Value) -> Self {
        self.cache_control = Some(cache_control);
        self
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<StarToolCall>) -> Self {
        self.tool_calls = Some(sanitize_tool_call_list(tool_calls));
        self
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            cache_control: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: StarToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: StarToolParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarToolParameters {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: StarToolCallFunction,
}

impl StarToolCall {
    /// Ensure `function.arguments` is valid JSON.
    /// Attempts simple repairs (trailing commas, unbalanced braces) on invalid input.
    /// Falls back to `"{}"` if repair fails.
    pub fn sanitize_arguments(&mut self) {
        self.function.arguments = sanitize_tool_arguments(&self.function.arguments);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarToolCallFunction {
    pub name: String,
    pub arguments: String,
}

/// Validate and optionally repair a tool call's arguments JSON string.
///
/// Some providers (e.g., Minimax) reject requests when the conversation
/// history contains a previous tool call with malformed arguments JSON.
/// This function catches and repairs common LLM JSON generation errors
/// before the arguments are serialized back into the API request.
fn sanitize_tool_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }

    // Fast path: already valid JSON
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }

    // Repair attempt 1: strip trailing commas (common LLM error)
    let no_trailing = trimmed.replace(",}", "}").replace(",]", "]");
    if no_trailing != trimmed {
        if serde_json::from_str::<serde_json::Value>(&no_trailing).is_ok() {
            return no_trailing;
        }
    }

    // Repair attempt 2: balance braces — append missing closing chars
    let (open_braces, close_braces) = count_braces(&no_trailing);
    if open_braces > close_braces {
        let diff = open_braces - close_braces;
        let suffix = "}".repeat(diff);
        let balanced = format!("{}{}", no_trailing, suffix);
        if serde_json::from_str::<serde_json::Value>(&balanced).is_ok() {
            return balanced;
        }
    }

    // Fallback: empty object
    "{}".to_string()
}

fn sanitize_tool_call_list(tool_calls: Vec<StarToolCall>) -> Vec<StarToolCall> {
    tool_calls
        .into_iter()
        .map(|mut tc| {
            tc.sanitize_arguments();
            tc
        })
        .collect()
}

fn count_braces(s: &str) -> (usize, usize) {
    let mut open = 0usize;
    let mut close = 0usize;
    for ch in s.chars() {
        match ch {
            '{' => open += 1,
            '}' => close += 1,
            _ => {}
        }
    }
    (open, close)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StarResponse {
    pub choices: Vec<StarChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<StarUsage>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct StarUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Cache read tokens (prompt cache hits) — Anthropic/DeepSeek specific
    #[serde(default)]
    pub cache_read_tokens: u32,
    /// Cache creation tokens (prompt cache writes) — Anthropic/DeepSeek specific
    #[serde(default)]
    pub cache_creation_tokens: u32,
    /// Whether the provider explicitly reported cache accounting for this response.
    ///
    /// Zero counters alone cannot distinguish no cache use from an adapter that has no
    /// cache telemetry, so callers must use this provenance flag before warning users.
    #[serde(default)]
    pub cache_telemetry_reported: bool,
}

/// 计算 prompt cache 命中率（0-100）。
///
/// # 为什么不能直接相除
///
/// 两种 provider 的 `prompt_tokens` 语义相反：
///
/// - **Anthropic**：`input_tokens` **不含**缓存读，缓存读是额外的 `cache_read_input_tokens`。
/// - **OpenAI / DeepSeek / 聚合网关**：`prompt_tokens` **已含**缓存命中
///   （`prompt_tokens_details.cached_tokens` 是它的子集）。
///
/// 直接拿 `cache_read / prompt_tokens` 在 Anthropic 侧会算出大于 100% 的值，在
/// OpenAI 侧才是对的。唯一的跨 provider 分母是「输入侧总量」：
///
/// ```text
/// input_side = total_tokens - completion_tokens
/// ```
///
/// Anthropic 侧它恰等于 `input + cache_read + cache_creation`，OpenAI 侧退回
/// `prompt_tokens`。这也是状态栏 `input_context_tokens` 用的同一个表达式。
///
/// provider 没有上报缓存遥测时返回 `None` —— 零计数无法区分「没缓存」和
/// 「这个 adapter 根本不报缓存」，不该编一个命中率出来。
pub fn cache_hit_rate(usage: &StarUsage) -> Option<f64> {
    if !usage.cache_telemetry_reported {
        return None;
    }
    let input_side = usage.total_tokens.saturating_sub(usage.completion_tokens);
    let denominator = if input_side > 0 {
        input_side
    } else if usage.prompt_tokens > 0 {
        usage.prompt_tokens
    } else {
        usage.total_tokens
    };
    if denominator == 0 {
        return None;
    }
    Some((usage.cache_read_tokens as f64 / denominator as f64) * 100.0)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StarChoice {
    pub message: StarMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_parameters: Option<SearchParameters>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamingChunk {
    pub chunk_type: StreamingChunkType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StarToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<StarToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<StarUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_event: Option<TraceEvent>,
    // ============ 智能化改进 10: 进度信息 ============
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ToolProgress>, // 工具执行进度信息
    // ============================================
    // ============ UX 改进: 确认信息 ============
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ToolConfirmation>, // 工具执行确认信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    // ============================================
    // ============ Agent 任务 UI ============
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentTaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_tool_use_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_async: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_resolved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_last_tool_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_new_sub_entries: Option<Vec<ChatEntry>>,
    // ============================================
}

impl StreamingChunk {
    pub fn new(chunk_type: StreamingChunkType) -> Self {
        Self {
            chunk_type,
            ..Default::default()
        }
    }

    pub fn content(content: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::Content,
            content: Some(content.into()),
            ..Default::default()
        }
    }

    pub fn thinking(content: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::Thinking,
            content: Some(content.into()),
            ..Default::default()
        }
    }

    pub fn text_delta(content: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::TextDelta,
            content: Some(content.into()),
            ..Default::default()
        }
    }

    pub fn reasoning_delta(content: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::ReasoningDelta,
            content: Some(content.into()),
            ..Default::default()
        }
    }

    pub fn assistant_note(content: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::AssistantNote,
            content: Some(content.into()),
            ..Default::default()
        }
    }

    pub fn trace_event(event: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            chunk_type: StreamingChunkType::Trace,
            trace_event: Some(TraceEvent {
                event: event.into(),
                payload,
            }),
            ..Default::default()
        }
    }

    pub fn tool_calls(tool_calls: Vec<StarToolCall>) -> Self {
        Self {
            chunk_type: StreamingChunkType::ToolCalls,
            tool_calls: Some(tool_calls),
            ..Default::default()
        }
    }

    pub fn tool_result(tool_call: StarToolCall, tool_result: ToolResult) -> Self {
        Self {
            chunk_type: StreamingChunkType::ToolResult,
            tool_call: Some(tool_call),
            tool_result: Some(tool_result),
            ..Default::default()
        }
    }

    pub fn tool_progress(progress: ToolProgress) -> Self {
        Self {
            chunk_type: StreamingChunkType::ToolProgress,
            progress: Some(progress),
            ..Default::default()
        }
    }

    pub fn with_tool_progress(mut self, progress: ToolProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning_content = Some(reasoning.into());
        self
    }

    pub fn done() -> Self {
        Self::new(StreamingChunkType::Done)
    }

    /// 调用失败。`content` 放最终要给用户看的那段文字
    /// （通常是 `llm::error_kind::diagnose` 的输出）。
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            chunk_type: StreamingChunkType::Error,
            content: Some(message.into()),
            ..Default::default()
        }
    }

    /// Agent 任务更新
    pub fn agent_task_update(payload: AgentTaskUpdatePayload) -> Self {
        Self {
            chunk_type: StreamingChunkType::AgentTaskUpdate,
            agent_task_id: Some(payload.task_id),
            agent_type: Some(payload.agent_type),
            agent_description: Some(payload.description),
            agent_status: Some(payload.status),
            agent_tool_use_count: Some(payload.tool_use_count),
            agent_tokens: Some(payload.tokens),
            agent_is_async: Some(payload.is_async),
            agent_is_resolved: Some(payload.is_resolved),
            agent_is_error: Some(payload.is_error),
            agent_last_tool_info: payload.last_tool_info,
            agent_name: payload.name,
            agent_task_description: payload.task_description,
            agent_new_sub_entries: if payload.new_sub_entries.is_empty() {
                None
            } else {
                Some(payload.new_sub_entries)
            },
            ..Default::default()
        }
    }
}

/// `AgentTaskUpdate` chunk 的载荷。
///
/// 原先是 11 个位置参数，极易错位；改为具名结构体 + `Default`，
/// 调用方只需填写关心的字段。
#[derive(Debug, Clone, Default)]
pub struct AgentTaskUpdatePayload {
    pub task_id: String,
    /// 用户可见的 Agent 类型标签（对标 `userFacingName`）
    pub agent_type: String,
    pub description: String,
    pub status: AgentTaskStatus,
    pub tool_use_count: u32,
    pub tokens: u32,
    pub is_async: bool,
    pub is_resolved: bool,
    pub is_error: bool,
    /// 最近工具摘要（对标 `extractLastToolInfo`）
    pub last_tool_info: Option<String>,
    /// teammate 自定义名称（`@name`）
    pub name: Option<String>,
    /// 后台运行时替代 "Done" 的描述
    pub task_description: Option<String>,
    /// 本次新增的子条目（增量）
    pub new_sub_entries: Vec<ChatEntry>,
}

impl AgentTaskUpdatePayload {
    pub fn new(task_id: impl Into<String>, agent_type: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            agent_type: agent_type.into(),
            ..Default::default()
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_status(mut self, status: AgentTaskStatus) -> Self {
        self.is_resolved = matches!(
            status,
            AgentTaskStatus::Completed
                | AgentTaskStatus::Failed
                | AgentTaskStatus::Rejected
                | AgentTaskStatus::Background
        );
        self.is_error = matches!(status, AgentTaskStatus::Failed | AgentTaskStatus::Rejected);
        self.status = status;
        self
    }

    pub fn with_stats(mut self, tool_use_count: u32, tokens: u32) -> Self {
        self.tool_use_count = tool_use_count;
        self.tokens = tokens;
        self
    }

    pub fn with_async(mut self, is_async: bool) -> Self {
        self.is_async = is_async;
        self
    }

    pub fn with_last_tool_info(mut self, info: Option<String>) -> Self {
        self.last_tool_info = info.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_name(mut self, name: Option<String>) -> Self {
        self.name = name.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_task_description(mut self, desc: Option<String>) -> Self {
        self.task_description = desc.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_sub_entries(mut self, entries: Vec<ChatEntry>) -> Self {
        self.new_sub_entries = entries;
        self
    }

    /// 覆盖 `with_status` 推导出的 resolved/error（用于 Running 但已出错等边界）
    pub fn with_resolved(mut self, is_resolved: bool) -> Self {
        self.is_resolved = is_resolved;
        self
    }

    pub fn with_error(mut self, is_error: bool) -> Self {
        self.is_error = is_error;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub event: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

// ============ 工具执行进度信息 ============
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolProgress {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub status: ToolProgressStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<u32>, // 当前进度
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>, // 总进度
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolProgressStatus {
    Starting,  // 开始执行
    Running,   // 正在执行
    Completed, // 已完成
    Failed,    // 失败
}
// ============ 进度信息结构完成 ============

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamingChunkType {
    #[default]
    Content,
    Thinking,
    TextDelta,      // 流式文本块
    ReasoningDelta, // 流式推理块
    AssistantNote,
    Trace,
    ToolCalls,
    ToolResult,
    Done,
    /// 调用失败。必须和 `Content` 分开：以前 `AgentEvent::Error` 被塞成
    /// `Content("[Error] …")`，于是它只是助手回答里的一行字 —— 不触发错误分类、
    /// 不开重试浮层、不清理 processing 状态。走这一路才能变成
    /// `StreamMessage::Error`。
    Error,
    TokenCount,
    // ============ 智能化改进 10: 工具执行进度反馈 ============
    ToolProgress, // 工具执行进度提示
    // ============ 进度反馈类型完成 ============
    // ============ UX 改进: 工具确认 ============
    ToolConfirmation, // 工具执行确认请求
    // ============================================
    // ============ Agent 任务 UI ============
    AgentTaskUpdate, // Agent 任务生命周期更新
                     // ============================================
}

// ============ UX 改进: ApprovalMode（审批模式）============
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    #[default]
    Default, // 默认模式：写操作与命令需要确认，只读工具免确认 (Build)
    AcceptEdits, // 接受编辑：文件改动与只读命令免确认，其余仍问
    Plan,        // Plan 模式：只读研究/规划，禁止非只读工具
    Yolo,        // YOLO 模式：所有操作自动执行（危险！）
    Auto,        // auto 模式：AI 分类器驱动，默认档位
}

impl ApprovalMode {
    /// 档位的**规范 slug** —— 与 `approval_mode_from_str` 的别名口径互通。
    ///
    /// 真源在 policy 侧（[`crate::core::policy::types::ApprovalMode::as_str`]），
    /// 那边带 serde、是 settings 里序列化的那个；session 侧只是转一道。
    /// 两个枚举各写一份字面量时，加档位就会漏掉一处。
    pub fn as_str(&self) -> &'static str {
        crate::core::policy::types::ApprovalMode::from(self.clone()).as_str()
    }

    /// 档位的**显示标签**（面向用户）。
    pub fn label(&self) -> &'static str {
        crate::core::policy::types::ApprovalMode::from(self.clone()).label()
    }

    /// 中文标签（中文界面用）。
    ///
    /// 英文档位名保留在括号里 —— 用户在英文文档、命令行 `--permission-mode`
    /// 里看到的就是那些名字，中文标签不该把它们藏起来。
    pub fn label_zh(&self) -> &'static str {
        crate::core::policy::types::ApprovalMode::from(self.clone()).label_zh()
    }
}

/// session 侧的审批档位 → policy 侧的同名档位。
///
/// 两个枚举是分开的（session 要给 UI 显示，policy 要给规则引擎），
/// 转换只在这里做一次 —— 三个兜底构造点各写一遍 `match` 早晚会漂。
impl From<ApprovalMode> for crate::core::policy::types::ApprovalMode {
    fn from(mode: ApprovalMode) -> Self {
        match mode {
            ApprovalMode::Default => crate::core::policy::types::ApprovalMode::Default,
            ApprovalMode::AcceptEdits => crate::core::policy::types::ApprovalMode::AcceptEdits,
            ApprovalMode::Plan => crate::core::policy::types::ApprovalMode::Plan,
            ApprovalMode::Yolo => crate::core::policy::types::ApprovalMode::Yolo,
            ApprovalMode::Auto => crate::core::policy::types::ApprovalMode::Auto,
        }
    }
}

/// policy 侧的审批档位 → session 侧的同名档位。
impl From<crate::core::policy::types::ApprovalMode> for ApprovalMode {
    fn from(mode: crate::core::policy::types::ApprovalMode) -> Self {
        match mode {
            crate::core::policy::types::ApprovalMode::Default => ApprovalMode::Default,
            crate::core::policy::types::ApprovalMode::AcceptEdits => ApprovalMode::AcceptEdits,
            crate::core::policy::types::ApprovalMode::Plan => ApprovalMode::Plan,
            crate::core::policy::types::ApprovalMode::Yolo => ApprovalMode::Yolo,
            crate::core::policy::types::ApprovalMode::Auto => ApprovalMode::Auto,
        }
    }
}

// ============ UX 改进: ThinkingEffort（思考努力级别）============
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThinkingEffort {
    Off,    // 禁用思考
    Low,    // 低努力：快速简单思考
    Medium, // 中等努力：平衡思考
    High,   // 高努力：深度思考
    Xhigh,  // 超高努力：旗舰模型扩展档（对标 OpenAI/Anthropic 的 xhigh）
}

impl Default for ThinkingEffort {
    fn default() -> Self {
        Self::Off
    }
}

impl ThinkingEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Xhigh => "Xhigh",
        }
    }

    /// 从字符串解析档位（skill frontmatter 的 `effort` 字段用）。
    ///
    /// CC 的取值：`low` / `medium` / `high` / `xhigh` / `max`。
    /// `max` 映射到 `Xhigh` —— StarCode 没有 `max` 档（`Xhigh` 已是最高）。
    /// 认 `inherit`（保持当前）—— 返回 `None`。
    pub fn parse_skill_effort(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "off" | "none" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" | "default" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" | "max" => Some(Self::Xhigh),
            _ => None,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Xhigh,
            Self::Xhigh => Self::Off,
        }
    }

    /// 状态栏/抬头用的档位符号（对标 Claude Code `constants/figures.ts`：
    /// `EFFORT_LOW='○' EFFORT_MEDIUM='◐' EFFORT_HIGH='●'`）。
    ///
    /// Claude Code 没有 Off 这一档，所以空心圈在那边是 low；这里 Off 用虚线圈
    /// `◌`，把"关掉"和"低"区分开。Xhigh 是本市面通用刻度之上的旗舰扩展档，
    /// 用实心菱形 `◆` 表示比 `●` 更进一步。
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Off => "◌",
            Self::Low => "○",
            Self::Medium => "◐",
            Self::High => "●",
            Self::Xhigh => "◆",
        }
    }

    /// 档位的显示标签，例如 `◐ medium`。
    ///
    /// 状态栏、欢迎抬头、`/effort` 回执共用这一份措辞，符号和词才不会各处各样。
    pub fn label(&self) -> String {
        format!("{} {}", self.symbol(), self.as_str())
    }

    /// 一行式档位提示，例如 `◐ medium · /effort`（对标 Claude Code 的
    /// `getEffortNotificationText`）。
    ///
    /// Alt+T、`/effort`、命令面板三条路径都用它，措辞才不会各说一套。
    pub fn notification_text(&self) -> String {
        format!("{} · /effort", self.label())
    }
}

// ============ UX 改进: 工具确认相关类型 ============
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfirmation {
    pub tool_name: String,
    pub operation_type: ConfirmationType,
    pub details: ConfirmationDetails,
    pub is_dangerous: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfirmationType {
    EditFile,        // 编辑文件
    CreateFile,      // 创建文件
    ShellCommand,    // Shell 命令
    DeleteFile,      // 删除文件（极度危险）
    NetworkRequest,  // 网络请求
    Generic,         // 通用请求
    AskUserQuestion, // 向用户提问
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfirmationDetails {
    EditFile {
        file_path: String,
        diff: String, // unified diff 格式
        old_lines: usize,
        new_lines: usize,
    },
    CreateFile {
        file_path: String,
        content_preview: String, // 前 20 行预览
    },
    ShellCommand {
        command: String,
        working_dir: String,
        estimated_risk: RiskLevel,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff_preview: Option<String>,
    },
    DeleteFile {
        file_path: String,
    },
    NetworkRequest {
        url: String,
        method: String,
    },
    Generic {
        title: String,
        prompt: String,
    },
    AskUserQuestion {
        question: String,
        header: Option<String>,
        options: Vec<AskUserQuestionOption>,
        multi_select: bool,
    },
}

/// 向用户提问的选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserQuestionOption {
    pub label: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Safe,     // 安全（如 ls, pwd）
    Low,      // 低风险（如 cat, grep）
    Medium,   // 中风险（如 git, npm）
    High,     // 高风险（如 rm, mv）
    Critical, // 极度危险（如 rm -rf, format）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserChoice {
    Proceed,     // 继续执行
    Skip,        // 跳过
    AlwaysAllow, // 总是允许（此类操作）
    Cancel,      // 取消
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolConfirmationOutcome {
    ProceedOnce,
    ProceedAlways,
    ProceedAlwaysAndSave,
    Cancel,
    AllowSession,
    /// User answered a question — carries the selected option labels and optional text input.
    UserAnswer {
        answers: Vec<String>,
        text_input: Option<String>,
    },
}
// ============ 确认类型定义完成 ============

/// 只读"查询类"工具：命中后可免确认直接执行（对标 Claude Code 的只读工具集）。
///
/// 名字先归一到**注册名**再比对 —— 策略引擎拿到的是模型原样发出的名字
/// （`read` / `bash` 这类全小写写法都出现过），不归一的话只读工具会因为
/// 大小写不同而被当成"要确认的工具"。
///
/// 只认注册名 —— 旧的下划线写法不是别名，是拼错的名字。
pub fn is_safe_query_tool(tool_name: &str) -> bool {
    matches!(
        crate::core::tools::constants::resolve_registered_tool_name(tool_name),
        "Read"
            | "ReadManyFiles"
            | "LS"
            | "Glob"
            | "Grep"
            | "CodebaseSearch"
            | "ToolSearch"
            | "GhPrComments"
            | "TodoWrite"
            | "McpListServers"
            | "McpListTools"
            | "McpSearchTools"
            | "McpToolInfo"
            | "McpRefresh"
            // `GetDiagnostics` 只跑 `cargo check` / `eslint` / `pylint`，纯读不写任何状态，
            // 只是把编译错误和 lint 警告收集一遍——每调用一次弹一次确认会让这个工具形同虚设。
            | "GetDiagnostics"
    )
}

/// 文件改动类工具 —— `acceptEdits` 模式下免确认（对标 CC：读 + 文件编辑）。
pub fn is_file_edit_tool(tool_name: &str) -> bool {
    crate::core::tools::constants::is_edit_tool_name(tool_name)
}

/// 只读 shell 命令 —— **所有审批模式**下都免确认。
///
/// 对标 Claude Code 的 "read-only commands"：*Claude Code recognizes a built-in
/// set of Bash commands as read-only and runs them without a permission prompt
/// in **every mode***。
///
/// 之前这条只挂在 `acceptEdits` 模式上，于是 `default` 模式里连 `ls` / `cat` /
/// `git status` 都要弹确认 —— 用户每轮都被问"就看一下目录也要批？"，正是
/// Claude Code 新版本把 auto 模式设为默认后大家抱怨的点。
///
/// 三条硬性拒绝条件，因为它们都能把"只读"变成写入：
/// - 重定向 `>` / `<`：能改文件
/// - 命令替换 `` ` `` / `$(`：真实命令藏在参数里，静态拆不出来
/// - `;` `&&` `|` 分隔的**每一段**都得是只读程序 —— `ls && rm -rf /` 不能算只读
pub fn is_readonly_fs_command(tool_name: &str, args: &serde_json::Value) -> bool {
    const READONLY_FS_COMMANDS: &[&str] = &[
        "pwd", "ls", "ll", "cat", "head", "tail", "wc", "file", "stat", "find", "fd", "grep", "rg",
        "ag", "tree", "diff", "cmp", "which", "type", "du", "df", "basename", "dirname",
        "realpath", "readlink", "sort", "uniq", "cut", "column",
    ];

    if tool_name != "Bash" {
        return false;
    }
    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    if command.contains('>')
        || command.contains('<')
        || command.contains('`')
        || command.contains("$(")
    {
        return false;
    }
    command
        .split([';', '&', '|', '\n'])
        // `&&` 被拆成两个 '&'，中间产生空段；空段不是命令，跳过。
        .filter(|segment| !segment.trim().is_empty())
        .all(|segment| {
            let program = segment.split_whitespace().next().unwrap_or_default();
            // `/usr/bin/ls`、`.\ls.exe` 都取最后一段路径
            let program = program
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(program)
                .trim_end_matches(".exe");
            if READONLY_FS_COMMANDS.contains(&program) {
                return true;
            }
            // `git` 单独放行不行 —— 它下面既有 `status` 也有 `push`/`reset --hard`。
            is_readonly_git_command(segment)
        })
}

/// `git` 的**只读**子命令。
///
/// `git status` / `git log` / `git diff` 是纯读；`git push` / `git reset --hard`
/// / `git clean -fd` 会写、会删 —— 所以 `git` 不能整条放行，只能按子命令挑。
///
/// 只放行"看"的子命令：`commit`/`push`/`reset`/`clean`/`checkout`/`merge`/
/// `rebase`/`stash`/`tag`/`branch` 一律不放行（`git branch -d` 会删分支）。
const READONLY_GIT_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "blame",
    "describe",
    "shortlog",
    "ls-files",
    "rev-parse",
    "cat-file",
    "grep",
    "whatchanged",
    "annotate",
];

/// `git` 命令是否只读（只放行上面的子命令白名单）。
fn is_readonly_git_command(segment: &str) -> bool {
    let mut words = segment.split_whitespace();
    let program = words.next().unwrap_or_default();
    let program = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .trim_end_matches(".exe");
    if program != "git" {
        return false;
    }
    match words.next() {
        Some(sub) => READONLY_GIT_SUBCOMMANDS.contains(&sub),
        None => false,
    }
}

/// Model information with provider details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 模型的上下文窗口大小（tokens），从 API /models 端点提取（如果提供商返回）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// 模型是否支持 thinking/reasoning 功能
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            display_name: None,
            context_window: None,
            supports_thinking: None,
        }
    }

    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    pub fn with_context_window(mut self, ctx: u32) -> Self {
        self.context_window = Some(ctx);
        self
    }

    pub fn with_supports_thinking(mut self, supports: bool) -> Self {
        self.supports_thinking = Some(supports);
        self
    }

    /// Get formatted display string with provider info
    pub fn display(&self) -> String {
        match &self.display_name {
            Some(name) => format!("{} ({})", name, self.provider),
            None => format!("{} ({})", self.id, self.provider),
        }
    }

    /// Get short display string (just model name)
    pub fn short_display(&self) -> String {
        self.display_name.clone().unwrap_or_else(|| self.id.clone())
    }
}

#[cfg(test)]
mod thinking_effort_tests {
    use super::ApprovalMode;
    use super::ThinkingEffort;

    #[test]
    fn every_gear_has_its_own_symbol() {
        // 四档的符号必须互不相同，否则状态栏上看不出按 Alt+T 到底动没动
        let gears = [
            ThinkingEffort::Off,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
        ];
        let mut symbols: Vec<&str> = gears.iter().map(|g| g.symbol()).collect();
        symbols.sort_unstable();
        symbols.dedup();
        assert_eq!(symbols.len(), gears.len(), "档位符号有重复: {:?}", symbols);
    }

    #[test]
    fn the_notification_points_at_the_command_that_changes_it() {
        // 通知里必须带上 /effort —— 用户看见档位还得知道去哪调
        assert_eq!(ThinkingEffort::Medium.label(), "◐ medium");
        assert_eq!(
            ThinkingEffort::Medium.notification_text(),
            "◐ medium · /effort"
        );
    }

    #[test]
    fn cycling_all_gears_returns_to_the_start() {
        let mut gear = ThinkingEffort::Off;
        for _ in 0..5 {
            gear = gear.next();
        }
        assert!(matches!(gear, ThinkingEffort::Off));
        // High 的下一档必须是新增的 Xhigh，不能跳回 Off
        assert!(matches!(ThinkingEffort::High.next(), ThinkingEffort::Xhigh));
    }

    /// `GetDiagnostics` 是只读工具（跑 `cargo check`/`eslint`/`pylint`，不写文件），
    /// Default 模式不该对它有确认弹窗——它只是把编译错误和 lint 警告收集一遍。
    #[test]
    fn get_diagnostics_is_a_safe_query_tool() {
        assert!(crate::types::is_safe_query_tool("GetDiagnostics"));
        assert!(crate::core::tools::constants::is_read_only_tool_name(
            "GetDiagnostics"
        ));
    }

    /// 审批档位的 slug 必须**唯一** —— 加上 `Auto` 档位之后，
    /// "auto" 这个名字曾经同时指 `Default`（旧显示名）和新的 auto 档位。
    ///
    /// 断言的是"两两不同"，而不是逐个写死期望值：将来再加档位时，
    /// 忘了给新档位一个 slug，这��测试会立刻指出重复的那个。
    #[test]
    fn approval_mode_slugs_are_unique() {
        let modes = [
            ApprovalMode::Default,
            ApprovalMode::AcceptEdits,
            ApprovalMode::Plan,
            ApprovalMode::Yolo,
            ApprovalMode::Auto,
        ];

        let mut slugs: Vec<&str> = modes.iter().map(|m| m.as_str()).collect();
        let total = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), total, "审批档位的 slug 必须互不相同");
    }

    /// 显示标签同样必须唯一 —— 同一个词不能指两个档位。
    #[test]
    fn approval_mode_labels_are_unique() {
        let modes = [
            ApprovalMode::Default,
            ApprovalMode::AcceptEdits,
            ApprovalMode::Plan,
            ApprovalMode::Yolo,
            ApprovalMode::Auto,
        ];

        let mut labels: Vec<&str> = modes.iter().map(|m| m.label()).collect();
        let total = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total, "审批档位的显示标签必须互不相同");
    }

    /// slug 必须能被 `approval_mode_from_str` 解析回来 —— 那是设置项、
    /// 命令行与 `/permissions` 共用的别名口径，两边对不上就有一边失效。
    #[test]
    fn approval_mode_slugs_round_trip_through_the_alias_parser() {
        for mode in [
            ApprovalMode::Default,
            ApprovalMode::AcceptEdits,
            ApprovalMode::Plan,
            ApprovalMode::Yolo,
            ApprovalMode::Auto,
        ] {
            let policy_mode: crate::core::policy::types::ApprovalMode = mode.clone().into();
            assert_eq!(
                crate::core::policy::approval_mode_from_str(mode.as_str()),
                Some(policy_mode),
                "{} 的 slug 解析不回来",
                mode.as_str()
            );
        }
    }
}
