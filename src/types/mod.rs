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
    /// Providers that don't support cache_control will silently ignore it.
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
    Default, // 默认模式：高风险操作需要确认 (Build)
    Plan, // Plan 模式：只读研究/规划，禁止非只读工具
    Yolo, // YOLO 模式：所有操作自动执行（危险！）
}

// ============ UX 改进: ThinkingEffort（思考努力级别）============
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThinkingEffort {
    Off,    // 禁用思考
    Low,    // 低努力：快速简单思考
    Medium, // 中等努力：平衡思考
    High,   // 高努力：深度思考
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
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
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

pub fn is_safe_query_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "view_file"
            | "Read"
            | "read_many_files"
            | "ListDir"
            | "list_directory"
            | "Glob"
            | "Grep"
            | "search_file_content"
            | "SemanticSearch"
            | "gh_pr_comments"
            | "task_search"
            | "Todo"
            | "mcp_list_servers"
            | "mcp_list_tools"
            | "mcp_search_tools"
            | "mcp_tool_info"
            | "mcp_refresh"
    )
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
