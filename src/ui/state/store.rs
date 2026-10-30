use crate::core::feature_flags::FeatureFlags;
use crate::core::sandbox::SandboxManager;
use crate::types::ChatEntry;
use crate::ui::components::virtual_list::VirtualList;
use ratatui::{
    layout::Rect,
    widgets::{ListState, ScrollbarState},
    // text::Line,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;
use tui_textarea::TextArea;

/// Simple placeholder for preview scroll state (replaces tui-scrollview, unused in rendering)
#[derive(Default, Clone)]
pub struct PreviewScrollState {
    pub scroll: usize,
}

pub const INPUT_FOLD_MIN_LINES: usize = 8;

/// Toast notification type
#[derive(Debug, Clone)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Toast notification — temporary overlay that auto-dismisses
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub created_at: Instant,
    pub duration_secs: u64,
}

/// 已经到达、但还没轮到回答的确认请求。
///
/// 工具是并发执行的，一次可能同时发出多张确认卡。UI 一次只展示/回答一张，
/// 其余必须排队 —— 丢掉它们等于让对应的门禁永远等一个不会到来的答复，
/// 整轮就卡死在这里。
#[derive(Debug, Clone)]
pub struct QueuedConfirmation {
    pub entry_idx: usize,
    pub message_id: u64,
    /// bus 侧的 correlation id：回答时必须用它。
    pub tool_call_id: String,
    /// 模型侧的 tool call id：工具结束时靠它把卡片收掉。
    pub originating_tool_call_id: Option<String>,
    /// `AskUserQuestion` 的选择项从 0 开始，其余确认卡从 1 开始。
    pub is_ask: bool,
}

/// UI 侧用户设置文件写入的独立结果；它不能代替 worker runtime acknowledgement。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSettingsPersistence {
    Pending { ui_revision: u64 },
    Saved { ui_revision: u64 },
    Failed { ui_revision: u64, reason: String },
}

/// /fast 尚未收到 worker acknowledgement 的 session-only 转换。只有匹配 revision
/// 的成功 acknowledgement 才能提交 `fast_mode` 与恢复模型；失败则完整丢弃。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFastModeTransition {
    pub ui_revision: u64,
    pub enabled: bool,
    pub previous_model: Option<crate::runtime::messages::RuntimeModelSelection>,
    pub target_model: crate::runtime::messages::RuntimeModelSelection,
}

/// 一次**等待 y/n 确认**的模型切换。
///
/// provider 必须在这里一起存下来，不能等到按 `y` 时再回头推导 —— 那条推导路径
/// （`resolve_provider_for_model`）会去翻磁盘缓存里**所有**桶，包括已被删除的
/// provider，于是把模型发给一个不存在的 provider，切换静默失败。确认框是延迟
/// 的，延迟期间列表和缓存都可能已经变了，只有存下来的归属是可信的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingModelChange {
    pub model: String,
    /// 选项自带的归属；`None` 表示列表没给，回退到推导。
    pub provider_id: Option<String>,
}

/// 粘贴块类型
#[derive(Debug, Clone)]
pub enum PasteKind {
    Text,
    Image {
        path: String,
        width: u32,
        height: u32,
    },
    Files(Vec<String>),
}

/// 粘贴块：多行粘贴内容以缩略形式内嵌在输入框中
#[derive(Debug, Clone)]
pub struct PasteSegment {
    pub id: usize,
    pub content: String,
    pub line_count: usize,
    pub kind: PasteKind,
}

/// 生成文本粘贴占位符格式: [Pasted text #N +M lines]
pub fn format_text_paste_ref(id: usize, line_count: usize) -> String {
    if line_count == 0 {
        format!("[Pasted text #{}]", id)
    } else {
        format!("[Pasted text #{} +{} lines]", id, line_count)
    }
}

/// 生成图片粘贴占位符格式: [Image #N]
pub fn format_image_paste_ref(id: usize) -> String {
    format!("[Image #{}]", id)
}

/// 生成文件粘贴占位符格式: [Files #N]
pub fn format_files_paste_ref(id: usize) -> String {
    format!("[Files #{}]", id)
}

/// 正则匹配粘贴占位符: [Pasted text #N], [Image #N], [Files #N]
pub fn parse_paste_reference(line: &str) -> Option<usize> {
    // Match [Pasted text #N +M lines] or [Pasted text #N]
    if let Some(captures) = line.strip_prefix("[Pasted text #") {
        let id_str = captures.split(|c: char| !c.is_ascii_digit()).next()?;
        return id_str.parse::<usize>().ok();
    }
    // Match [Image #N]
    if let Some(captures) = line.strip_prefix("[Image #") {
        let id_str = captures.split(|c: char| !c.is_ascii_digit()).next()?;
        return id_str.parse::<usize>().ok();
    }
    // Match [Files #N]
    if let Some(captures) = line.strip_prefix("[Files #") {
        let id_str = captures.split(|c: char| !c.is_ascii_digit()).next()?;
        return id_str.parse::<usize>().ok();
    }
    None
}

/// 将文本中所有粘贴块占位符展开为实际内容
pub fn expand_paste_segments(text: &str, segments: &[PasteSegment]) -> String {
    if segments.is_empty() {
        return text.to_string();
    }
    let mut result = text.to_string();
    for seg in segments {
        let placeholder = match seg.kind {
            PasteKind::Text => format_text_paste_ref(seg.id, seg.line_count),
            PasteKind::Image { .. } => format_image_paste_ref(seg.id),
            PasteKind::Files(_) => format_files_paste_ref(seg.id),
        };
        result = result.replace(&placeholder, &seg.content);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickMenuKind {
    Provider,
    Model,
    Session,
}

/// 对话内搜索状态（对标 Claude Code 的 Ctrl+F）。
///
/// 与 `GlobalSearchState`（ripgrep 搜文件）**不是一回事**：那个搜仓库里的文件，
/// 这个搜已经发生过的对话 —— 长会话里"刚才那条命令是什么"只能靠它。
///
/// 命中位置用**渲染行号**记录（`rendered_cache` 按 entry 下标展开后的行下标），
/// 不是 entry 下标：滚动、跳转、高亮都以屏幕行为单位，entry 下标在这三件事上都不够用。
#[derive(Debug, Clone, Default)]
pub struct ChatSearchState {
    /// 面板是否打开
    pub active: bool,
    /// 查询串
    pub query: String,
    /// 命中的渲染行号，升序
    pub matches: Vec<usize>,
    /// 当前命中的下标（指向 `matches`）
    pub selected: usize,
    /// 输入光标在 query 里的位置
    pub cursor: usize,
    /// 面板出现前的滚动位置，关闭时还原
    pub restore_scroll: usize,
    /// 上一次搜索的结果数，用于"无匹配"提示的翻转
    pub last_result_count: usize,
}

impl ChatSearchState {
    pub fn open(&mut self, current_scroll: usize) {
        if !self.active {
            self.restore_scroll = current_scroll;
        }
        self.active = true;
    }

    pub fn close(&mut self) {
        self.active = false;
        self.query.clear();
        self.matches.clear();
        self.selected = 0;
        self.cursor = 0;
        self.last_result_count = 0;
    }

    pub fn is_empty_query(&self) -> bool {
        self.query.is_empty()
    }

    /// 当前命中的渲染行号。
    pub fn current_match(&self) -> Option<usize> {
        self.matches.get(self.selected).copied()
    }

    /// 跳到下一个/上一个命中，到头则绕回。
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.selected = if forward {
            (self.selected + 1) % self.matches.len()
        } else if self.selected == 0 {
            self.matches.len() - 1
        } else {
            self.selected - 1
        };
        self.current_match()
    }

    /// 用新的查询词重跑搜索。`haystack` 是逐条目的纯文本（与渲染行一一对应）。
    pub fn run(&mut self, haystack: &[String]) {
        self.matches.clear();
        self.selected = 0;
        let needle = self.query.to_lowercase();
        if !needle.is_empty() {
            for (row, text) in haystack.iter().enumerate() {
                if text.to_lowercase().contains(&needle) {
                    self.matches.push(row);
                }
            }
        }
        self.last_result_count = self.matches.len();
    }
}

/// 文本选择状态
#[derive(Debug, Clone)]
pub struct TextSelection {
    /// 选择起始位置 (行索引, 列索引)
    pub start: Option<(usize, usize)>,
    /// 选择结束位置 (行索引, 列索引)  
    pub end: Option<(usize, usize)>,
    /// 是否正在拖拽选择中
    pub is_selecting: bool,
    /// 选择开始的聊天项索引
    pub start_entry_idx: Option<usize>,
    /// 选择结束的聊天项索引
    pub end_entry_idx: Option<usize>,
}

impl TextSelection {
    pub fn new() -> Self {
        Self {
            start: None,
            end: None,
            is_selecting: false,
            start_entry_idx: None,
            end_entry_idx: None,
        }
    }

    /// 开始选择
    pub fn start_selection(&mut self, entry_idx: usize, row: usize, col: usize) {
        self.start = Some((row, col));
        self.end = Some((row, col));
        self.is_selecting = true;
        self.start_entry_idx = Some(entry_idx);
        self.end_entry_idx = Some(entry_idx);
    }

    /// 更新选择结束位置
    pub fn update_selection(&mut self, entry_idx: usize, row: usize, col: usize) {
        if self.is_selecting {
            self.end = Some((row, col));
            self.end_entry_idx = Some(entry_idx);
        }
    }

    /// 结束选择
    pub fn end_selection(&mut self) {
        self.is_selecting = false;
    }

    /// 清除选择
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.is_selecting = false;
        self.start_entry_idx = None;
        self.end_entry_idx = None;
    }

    /// 是否有活动的选择
    pub fn has_selection(&self) -> bool {
        self.start.is_some() && self.end.is_some()
    }

    // 起止 → 范围的换算、以及"某点是否在选区内"的判定，都搬到 `ui::selection`
    // 了（`SelectionRange::resolve` / `SelectionRange::contains`）。以前这两份
    // 逻辑在 store 与 chat_history 各写一遍，列宽口径还不一致。
}

/// 后台代理选择器最多列出的行数（不含 main 行）
pub const MAX_BG_AGENT_ROWS: usize = 8;

/// Agent 任务追踪信息
#[derive(Debug, Clone)]
pub struct AgentTaskInfo {
    /// 任务 ID
    pub task_id: String,
    /// Agent 类型（"fork" | "general-purpose" | "worker" ...）
    pub agent_type: String,
    /// Agent 描述
    pub description: String,
    /// 任务状态
    pub status: crate::types::AgentTaskStatus,
    /// 工具使用次数
    pub tool_use_count: u32,
    /// Token 使用量
    pub tokens: u32,
    /// 是否异步
    pub is_async: bool,
    /// 是否完成
    pub is_resolved: bool,
    /// 是否出错
    pub is_error: bool,
    /// 最后工具信息
    pub last_tool_info: Option<String>,
    /// teammate 自定义名称（`@name` 显示，对标 renderGroupedAgentToolUse 的 name）
    pub name: Option<String>,
    /// 后台运行时替代 "Done" 的任务描述
    pub task_description: Option<String>,
    /// 开始时间
    pub started_at: Instant,
    /// 完成时间（首次 resolved 时冻结，避免耗时随渲染继续增长）
    pub finished_at: Option<Instant>,
    /// 子消息列表
    pub sub_entries: Vec<crate::types::ChatEntry>,
    /// 在 chat_history 中的位置索引
    pub entry_idx: usize,
}

impl AgentTaskInfo {
    /// 已耗时：完成后取冻结值，未完成时实时计算
    pub fn elapsed(&self) -> std::time::Duration {
        match self.finished_at {
            Some(end) => end.saturating_duration_since(self.started_at),
            None => self.started_at.elapsed(),
        }
    }

    /// 是否「已转入后台」（对标 AgentProgressLine 的 isBackgrounded）
    pub fn is_backgrounded(&self) -> bool {
        self.is_async && self.is_resolved
    }
}

/// 模型列表"还在拉"的最长时间。见 [`ChatState::models_loading`]。
pub const MODELS_LOADING_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

pub struct ChatState {
    pub chat_history: Vec<ChatEntry>,
    pub input: String,
    pub textarea: TextArea<'static>, // Added for multiline input support
    pub input_line_count: usize,     // Cached line count to avoid O(n) scans
    pub scroll: usize,               // Changed from u16 to usize for line-based scrolling
    pub total_rendered_lines: usize, // Track total lines for scrolling bounds
    pub chat_list_state: ListState,
    pub chat_scrollbar_state: ScrollbarState,
    pub expanded_tool_call_ids: HashSet<String>,
    pub expanded_thinking_indices: HashSet<usize>,
    pub complete_task_message_ids: HashSet<u64>,
    pub auto_continued_message_ids: HashSet<u64>,
    pub pending_user_messages: VecDeque<String>,
    // ============ Command History ============
    pub command_history: VecDeque<String>,
    pub history_index: Option<usize>,
    pub history_input_snapshot: Option<String>,
    // ==========================================
    pub tool_started_at: HashMap<String, Instant>,
    pub tool_call_args_cache: HashMap<String, String>,
    pub tool_call_transcript_written: HashSet<String>,
    pub transcript_enabled: bool,
    pub transcript_path: Option<std::path::PathBuf>,
    pub transcript_run_id: String,
    pub transcript_seq: u64,
    pub last_chat_area: Option<Rect>,
    pub last_input_area: Option<Rect>,
    pub last_item_heights: Vec<u16>,
    pub show_command_hints: bool,
    pub command_hints: Vec<String>,
    pub selected_hint: usize,
    pub show_mention_hints: bool,
    pub mention_hints: Vec<String>,
    pub selected_mention_hint: usize,
    pub preview_visible: bool,
    pub preview_scroll_state: PreviewScrollState,
    pub preview_last_entry_idx: Option<usize>,
    // ============ UX 改进: 移除 auto_edit_enabled，统一使用 ApprovalMode ============
    // 旧的 auto_edit_enabled 与 ApprovalMode 功能重叠，造成混淆
    // 现在只保留 ApprovalMode 作为统一的审批模式控制
    // ============================================
    pub is_processing: bool,
    pub is_streaming: bool,
    pub processing_started_at: Option<Instant>,
    pub model_wait_started_at: Option<Instant>,
    /// ESC/Ctrl+C 取消流式操作的时间点，用于过渡动画
    pub cancelling_since: Option<Instant>,
    pub processing_time_secs: u64,
    pub token_count: u32,
    pub total_cost: f64,
    /// 每个逻辑请求内已收到的所有真实 provider 响应费用。
    pub response_costs: HashMap<u64, f64>,
    /// 逻辑请求开始时固定的模型，用于避免中途切换模型影响已发出的响应计价。
    pub response_models: HashMap<u64, String>,
    /// 会话创建时刻，用于状态栏展示会话耗时（对标 Claude Code running duration）
    pub session_started_at: Instant,
    /// Last time a token was received (for stall detection)
    pub last_token_time: Option<Instant>,
    /// When thinking/reasoning started (for thinking duration display)
    pub thinking_started_at: Option<Instant>,
    /// (思考总秒数, 结束时刻)：思考结束后状态行继续显示 `thought for Xs`
    /// （对标 Claude Code Spinner，每个状态至少驻留 2 秒再消失）。
    pub thinking_finished: Option<(u64, Instant)>,
    pub current_status_line: Option<String>,
    pub available_models: Vec<String>,
    /// 完整的模型信息列表（包含 supports_thinking 等字段）
    pub available_models_info: Vec<crate::types::ModelInfo>,
    pub model_provider_map: std::collections::HashMap<String, String>,
    pub current_model: String,
    /// 当前模型是否支持 thinking/reasoning 功能（从模型列表中获取）
    pub current_model_supports_thinking: Option<bool>,
    pub current_provider_id: Option<String>,
    /// 最近一个回合是否以错误收场。
    ///
    /// 为什么不靠 `content` 开头是不是 `"Error: "`：
    /// 错误条目在 `chat_history` 里就是一条普通 assistant 文本，没有可查询的标记，
    /// 于是"上一轮是不是失败了"这件事谁也答不上来 —— Esc 只能落到 no-op 分支
    /// （`input.rs`），错误浮层又只对可重试错误弹。用户换完 provider 之后唯一
    /// 合理的动作就是"重发一次"，而界面上没有任何键能触发它。
    ///
    /// 置位点只有 `handle_error_message`；正常收尾（`handle_done_message`）和真正
    /// 发起新回合（`enqueue_user_message`）会清掉，所以它精确表示"**当前**这条
    /// 对话正卡在一个失败回合上"。
    pub last_turn_failed: bool,
    pub pending_confirmation: Option<String>,
    pub pending_model_change: Option<PendingModelChange>,
    pub pending_model_provider: Option<String>,
    pub pending_provider_selected_model: Option<String>,
    /// 一次性豁免：provider 刚被配置/切换后，用户紧接着的第一次模型选择
    /// 免 (y/n) 确认 —— 切 provider 本身已是明确意图，选模型是它的延续。
    /// 在 `request_runtime_model_change` 统一消费清零。
    pub waive_next_model_confirmation: bool,
    pub pending_palette_action: Option<crate::ui::state::palette::PaletteAction>,
    pub quick_menu_origin_palette: bool,
    pub quick_menu_back: Option<QuickMenuKind>,
    pub show_help: bool, // 快捷键帮助界面
    pub auto_follow: bool,
    pub last_chat_height: u16,
    pub last_max_scroll: u16,
    pub next_message_id: u64,
    /// 每一轮流式输出落在 `chat_history` 的哪一条。
    ///
    /// **值可能是悬空的**（== `chat_history.len()`）：工具调用之后、下一段正文
    /// 到达之前，落点就指着"下一条新建的助手条目"（见 `services/stream.rs` 的
    /// ToolCalls 分支）。所以读它的人**必须**先判 `idx < chat_history.len()`，
    /// 直接 `chat_history[idx]` 会 panic（index out of bounds）。
    pub stream_targets: HashMap<u64, usize>,
    pub message_start_indices: HashMap<u64, usize>,
    pub active_message_id: Option<u64>,
    pub awaiting_models: bool,
    /// `awaiting_models` 是在**什么时候**被置位的。
    ///
    /// 存在的唯一理由是给这个标志加一个上限。`awaiting_models` 只有收到
    /// `ModelsList` / `ModelsError` 才会被清掉，而那两条消息由 worker 里一个
    /// `tokio::spawn` 发出 —— 那个 future 一旦 panic，spawn 的 JoinHandle 没人管，
    /// **一条消息都不会发出去**，标志就永远停在 true，模型面板的转圈也就永远
    /// 不停（用户截图里那个 `◎ …`）。
    ///
    /// 所以它不能是个裸 bool：得能问"等够久了吗"。见 `models_loading`。
    pub awaiting_models_since: Option<Instant>,
    /// `available_models` 里那份列表是**什么时候从 API 拉下来的**（可能是上一个进程
    /// 拉的 —— 磁盘缓存见 `agent::model_cache`）。模型面板用它显示"多久之前拉的"。
    ///
    /// 这里刻意存两个量而不是"拉取时刻那个 `Instant`"：
    ///
    /// - `models_listed_at` 是**本进程**观察到这份列表的时刻，永远用得上。
    /// - `models_list_age_at_set` 是那一刻它就已经多老了（0 = 刚拉的）。
    ///
    /// 以前是把 `Instant::now()` 往回减 `age_secs` 得到"拉取时刻"。`Instant` 的
    /// 可回溯范围有限，机器刚重启不到 `age_secs` 秒时 `checked_sub` 直接返回
    /// `None`，于是 `or(Some(now))` 把一份两小时前的缓存显示成"刚拉的"—— 正是这条
    /// 注释要防的事。缓存年龄可以远大于机器已开机时长，所以这个减法在真实使用中
    /// 会经常失效，而不是只��测试里。
    pub models_listed_at: Option<Instant>,
    /// 观察时刻那一刻，列表已经多老了（秒）。见 `models_listed_at`。
    pub models_list_age_at_set: u64,
    pub mcp_ready: bool,
    pub auto_continue_enabled: bool,
    pub auto_continue_remaining: u32,
    // ============ UX 改进: 审批模式状态 ============
    pub approval_mode: crate::types::ApprovalMode,
    /// 仅由 worker acknowledgement 更新的当前生效 thinking 档位。
    pub thinking_effort: crate::types::ThinkingEffort,
    /// UI 发出的下一个 runtime-setting mutation revision。`0` 留给兼容请求。
    pub next_runtime_settings_revision: u64,
    /// 最近一次用户明确请求的语义设置。失败或降级时仍保留 intent。
    pub requested_runtime_settings: Option<crate::runtime::messages::RequestedRuntimeSettings>,
    /// 尚未收到 worker 终态确认的 UI mutations，按发出 revision 排序。
    /// worker 在流式回合结束后按 FIFO 应用；UI 只展示最新 revision 的确认。
    pub pending_runtime_settings: VecDeque<crate::runtime::messages::RuntimeSettingsMutation>,
    /// worker 已确认的完整运行时快照，是 UI 显示 active 状态的唯一来源。
    pub confirmed_runtime_settings: Option<crate::runtime::messages::RuntimeSettingsSnapshot>,
    /// 最近一条可见 runtime acknowledgement，含终态和原因。
    pub last_runtime_settings_ack: Option<crate::runtime::messages::RuntimeSettingsAcknowledgement>,
    /// 用户设置文件写入结果，独立于 worker 是否应用该 mutation。
    pub runtime_settings_persistence: Option<RuntimeSettingsPersistence>,
    /// 欢迎抬头上次渲染时的内容指纹（模型 + 思考档位）。抬头是渲染期现算的，但结果进了
    /// `rendered_cache`，只认 dirty 标记；靠这份指纹在渲染路径上发现内容变了。
    /// 见 `ui::components::welcome_header::refresh_if_stale`。
    pub welcome_header_fingerprint: Option<String>,
    /// /add-dir 追加的工作目录（对标 Claude Code add-dir：扩展 @ 文件访问范围）
    pub extra_working_dirs: Vec<std::path::PathBuf>,
    /// /fast 快速模式（对标 Claude Code fast mode：切到轻量模型 + 状态栏指示）
    pub fast_mode: bool,
    /// /network 离线模式（对标 Claude Code network 开关：拒绝网络请求 + 状态栏指示）
    pub network_offline: bool,
    /// /break-cache 标记：下一条消息强制重建 context（打破隐式缓存延续）
    pub break_cache_next: bool,
    /// fast 开启前的完整模型/provider 选择（关闭时精确恢复）
    pub fast_mode_prev_model: Option<crate::runtime::messages::RuntimeModelSelection>,
    /// 正在等待 runtime acknowledgement 的 /fast 转换。它不能乐观改变 fast mode。
    pub pending_fast_mode_transition: Option<PendingFastModeTransition>,
    /// /poor 省电模式（对标 Claude Code poor mode：跳过记忆抽取与提示建议）
    pub poor_mode: bool,
    /// /advisor 顾问模式（每轮结束后附带简短建议）
    pub advisor_mode: bool,
    pub context_window_override: Option<u32>,
    // ============ UX 改进: 工具确认状态 ============
    // 内联确认卡片：存储待确认的工具调用，等待用户响应
    pub pending_tool_calls: Option<Vec<crate::types::StarToolCall>>,
    pub pending_message_id: Option<u64>,
    pub pending_confirmation_entry_idx: Option<usize>, // 确认卡片在 chat_history 中的索引
    pub pending_confirmation_choice: usize,            // 当前选中项：1/2/3
    pub is_awaiting_confirmation: bool,                // 新增：是否处于等待键盘确认状态
    pub pending_tool_call_id: Option<String>,          // 新增：当前等待确认的工具调用ID
    /// 当前等待确认的那次调用在**模型侧**的 tool_call id。
    ///
    /// 和上面的 `pending_tool_call_id` 不是一回事：后者是 bus 的 correlation id，
    /// 回答问题必须用它；这个用来和 `ToolResult` 里的 `tool_call.id` 比对，
    /// 两者混用会导致卡片永远等不到"我这次已经结束了"的信号。
    pub pending_confirmation_originating_id: Option<String>,
    pub pending_question_selections: Vec<usize>, // 多选问题的已选选项索引
    pub pending_other_input: String,             // "Other" 文本输入值
    pub pending_question_other_focused: bool,    // "Other" 输入框是否聚焦
    pub last_confirmation_message_id: Option<u64>,
    /// 对标 Claude Code: Tab 切换到反馈输入模式
    pub confirmation_feedback_mode: bool,
    /// 对标 Claude Code: 确认时附加的反馈文本
    pub pending_confirmation_feedback: String,
    /// 同时到达、还没轮到回答的确认请求（先进后出：刚被顶下去的那张先答）。
    pub pending_confirmation_queue: VecDeque<QueuedConfirmation>,
    // ============================================
    // ============ 文本选择和复制功能 ============
    pub text_selection: TextSelection,
    pub copy_status: Option<std::time::Instant>, // 复制状态显示时间
    // ============ 对话内搜索（Ctrl+F）============
    pub chat_search: ChatSearchState,
    // ============ Git Status ============
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    // ====================================
    // ============ Stats =================
    pub au2_compressed: bool,
    pub token_usage: Option<crate::types::StarUsage>,
    // ============ Cache Stats ============
    /// Latest provider-reported cache read tokens (not a session total).
    pub cache_read_tokens: u64,
    /// Latest provider-reported cache creation tokens (not a session total).
    pub cache_creation_tokens: u64,
    /// 已显示过缓存警告的响应和类别，避免同一 usage 被流式重复提示。
    pub(crate) cache_warning_shown: HashSet<(u64, crate::agent::cache_warning::CacheWarningType)>,
    // ============ Rendering Cache ============
    pub rendered_cache: HashMap<usize, (u16, Vec<ratatui::text::Line<'static>>)>,
    /// Virtual-scrolling list with per-entry dirty tracking (tuie-inspired).
    pub virtual_list: VirtualList,
    pub last_terminal_width: u16,
    pub last_rendered_stream_key: HashMap<usize, (usize, usize)>,
    // =========================================
    // ============ Palette State ============
    pub palette_items: Vec<crate::ui::state::palette::PaletteItem>,
    pub selected_palette_index: usize,
    pub palette_mode: crate::ui::state::palette::PaletteMode,
    pub palette_history: Vec<crate::ui::state::palette::PaletteMode>,
    pub palette_filter: String,
    // ============ Unified Modal Stack ============
    /// 单一模态栈：替代散落的 show_* bool。栈顶接收按键与渲染。
    pub modal_stack: Vec<crate::ui::state::modal::Modal>,
    // ---- MCP modal data ----
    pub mcp_modal_servers: Vec<crate::ui::state::modal::McpServerRow>,
    pub mcp_modal_index: usize,
    pub mcp_modal_menu_index: usize,
    pub mcp_modal_tools: Vec<crate::core::mcp::types::MCPTool>,
    pub mcp_modal_loading: bool,
    pub mcp_modal_error: Option<String>,
    pub mcp_modal_action_msg: Option<String>,
    // ---- Market modal data ----
    pub market_entries: Vec<crate::core::extensions::types::MarketplaceEntry>,
    pub market_index: usize,
    pub market_query: String,
    pub market_loading: bool,
    pub market_message: Option<String>,
    pub market_confirm: Option<crate::ui::state::modal::MarketConfirm>,
    /// 已安装名集合（extensions 注册表 ∪ plugins 系统），Browse tab 打 ✓ 用
    pub market_installed_names: std::collections::HashSet<String>,
    /// 已安装项的启用状态（含 plugins），Installed tab 图标与开关用
    pub market_enabled_map: std::collections::HashMap<String, bool>,
    /// 来自 plugins 系统（/plugin install）的条目名，启停/卸载走插件 API
    pub market_plugin_names: std::collections::HashSet<String>,
    // ---- Plugins modal data（/plugin，对标 Claude Code PluginSettings）----
    pub plugin_discover: Vec<crate::ui::state::modal::DiscoverRow>,
    pub plugin_installed: Vec<crate::core::plugins::ResolvedPlugin>,
    pub plugin_marketplaces: Vec<crate::core::plugins::marketplace::PluginMarketplace>,
    pub plugin_marketplace_counts: std::collections::HashMap<String, usize>,
    pub plugin_errors: Vec<(String, String)>,
    pub plugin_errors_hint: Option<String>,
    pub plugin_index: usize,
    /// Discover tab 实时搜索文本（空 = 不过滤）
    pub plugin_search: String,
    /// Discover tab 勾选待批量安装的插件名（Space 切换，i 批量安装）
    pub plugin_selected: std::collections::HashSet<String>,
    /// 插件详情页（对标 Claude Code 详情视图）：(marketplace, plugin)
    pub plugin_detail: Option<(String, crate::core::plugins::marketplace::MarketplacePlugin)>,
    /// 批量安装进度聚合（对标 Claude Code 批量安装确认页的逐项状态）
    pub plugin_batch_total: usize,
    pub plugin_batch_done: usize,
    pub plugin_loading: bool,
    /// 有插件市场后台操作（经 AgentRequest::PluginOp）在执行中
    pub plugin_op_pending: bool,
    pub plugin_message: Option<String>,
    pub plugin_confirm: Option<crate::ui::state::modal::PluginConfirm>,
    // ============ Status Modal State ============
    // 状态弹窗是否打开由 `modal_stack` 里的 `Modal::StatusModal` 表达
    // （`ChatState::open_status_modal`）。曾经还有一个平行的 `show_status_modal`
    // flag，只被赋 `false`、渲染侧又拿它当门控，于是出现过「看不见的窗口吃掉按键」。
    pub show_provider_menu: bool,
    pub selected_provider_index: usize,
    pub show_session_menu: bool,
    pub selected_session_index: usize,
    pub available_sessions: Vec<crate::utils::session_manager::SessionSummary>,
    // ============ Input Modal State ============
    // 输入框是否打开同样由 `modal_stack` 里的 `Modal::InputModal` 表达
    // （`ChatState::is_input_modal_open`）。平行的 `show_input_modal` flag 已删除：
    // 两个真相源靠 `enter_input_modal`/`exit_input_modal` 手工同步，异步 stream
    // 消息（ModelsList）一打穿就会让「面板画得出来、按键却全被隐形输入框吞掉」。
    pub input_modal_title: String,
    pub input_modal_prompt: String,
    pub input_modal_value: String,
    pub modal_textarea: TextArea<'static>,
    // Context for the input modal to determine action on completion
    pub input_context: Option<crate::ui::state::palette::InputContext>,
    // Providers that are ready to use or have meaningful saved setup.
    // 顺序有意义：新的自定义 provider 在前（`ProviderSettings::order` 降序），
    // provider 面板直接按这个顺序渲染。
    pub configured_providers: Vec<String>,
    /// `configured_providers` 里**没有可用凭证**的那些 id。
    ///
    /// 由 `StreamMessage::ConfiguredProviders` 的处理分支从 provider store 现算
    /// （那个分支本来就是 async，所以能读盘）。Provider 菜单拿它打"⚠ no API key"
    /// 标记 —— 有了凭证随时会变，所以必须是现算的，不能存成配置里的字段。
    ///
    /// 注意这与"该不该出现在菜单里"（`provider_is_configured`：有 Base URL 就算
    /// 配好了）是两回事：留空密钥的 provider 依然列出，只是被标出来。见
    /// `providers::provider_has_usable_credentials`。
    pub keyless_providers: Vec<String>,
    /// "新增 provider" 单面板表单的编辑状态。`input_context` 是
    /// `ProviderForm` 时它才有效。
    pub provider_form: crate::ui::state::palette::ProviderFormState,
    // ============ Task Panel State ============
    pub task_panel: crate::ui::components::task_panel::TaskPanel,
    // ============ 粘贴状态 ============
    pub paste_in_progress: bool,
    pub paste_end_time: Option<std::time::Instant>,
    pub input_folded: bool,
    /// 内嵌粘贴块列表（每块以占位符 [Pasted text #N +M lines] 或 [Image #N] 存于 textarea）
    pub paste_segments: Vec<PasteSegment>,
    // ============ App Control ============
    pub should_exit: bool,
    /// 上次按 Ctrl+C 的时间，用于双击退出检测
    pub last_ctrl_c: Option<Instant>,
    /// 上次按 Esc 的时间，用于双击清空输入检测
    pub last_esc_at: Option<Instant>,
    /// 权限对话框：显示"为什么询问此权限"解释区（Ctrl+E 切换）
    pub show_permission_explanation: bool,
    /// 权限对话框：显示工具入参 debug 详情（Ctrl+D 切换）
    pub show_permission_debug: bool,
    /// UI verbose 模式：工具行不截断参数、显示完整路径（palette 可切换）
    pub ui_verbose: bool,
    /// Kill ring — Ctrl+W/U/K 删除的文本循环缓冲，Ctrl+Y 取回、Alt+Y 轮换
    pub kill_ring: std::collections::VecDeque<String>,
    /// yank 时记录 kill_ring 位置（yank-pop 轮换用）
    pub kill_ring_pos: Option<usize>,
    /// 流式条目高度下限（渲染高度只增不减，避免增量 markdown 重排引起整页上下跳动）
    pub streaming_height_floor: std::collections::HashMap<usize, u16>,
    /// 上次 yank 插入的字符数（yank-pop 时先删除再替换）
    pub last_yank_len: usize,
    // ============ Sandbox State ============
    pub sandbox_enabled: bool,
    /// 沙箱可用性探测结果（进程内只算一次）。
    ///
    /// `SandboxManager::is_available()` 在 Windows 上会 spawn `wsl -- which bwrap`
    /// 子进程（`core/sandbox/bubblewrap_windows.rs`），单次 300ms~2s。状态栏每帧都要
    /// 问一次可用性，此前会把子进程塞进渲染线程 —— 实测每秒 30 次 spawn，表现为闪屏与卡顿。
    /// 该探测的语义是「本机有没有 bwrap」，不随时间变化，缓存无副作用。
    pub sandbox_available: std::cell::OnceCell<bool>,
    /// 动画帧计数器 — 每帧 +1，用于旋转指示器和闪烁效果
    pub animation_tick: u64,
    /// 鼠标滚轮检测：待处理的箭头键事件
    /// 当收到 Up/Down 时，先缓存方向，等待下一个事件判断是否为滚轮
    pub pending_scroll_direction: Option<i32>, // -1=Up, 1=Down
    pub pending_scroll_time: Option<Instant>,
    // ============ Smooth Scrolling ============
    /// Scroll velocity for momentum-based scrolling (lines per tick)
    pub scroll_velocity: f64,
    /// Last scroll input time for momentum decay
    pub last_scroll_time: Option<Instant>,
    // ============ Tool Progress ============
    /// Currently executing tool name for spinner display
    pub current_tool_name: Option<String>,
    // ============ Session Persistence ============
    /// 当前恢复的 canonical session 文件名；退出时覆盖它而非创建新的 auto snapshot。
    pub active_session_id: Option<String>,
    pub current_session_title: Option<String>,
    // ============ Permission Rules ============
    pub permission_rules: crate::core::permission_rules::PermissionRuleEngine,
    // ============ Feature Flags ============
    pub feature_flags: FeatureFlags,
    // ============ Proactive Suggestions ============
    pub proactive_suggestions: crate::core::proactive::ProactiveSuggestions,
    pub tools_used: Vec<String>,
    // ============ Remote Settings ============
    pub remote_settings: crate::core::remote_settings::RemoteSettings,
    // ============ Voice Mode ============
    pub voice_config: crate::core::voice::VoiceConfig,
    // ============ Notifications ============
    pub notifications: crate::core::notifications::NotificationManager,
    // ============ Theme System ============
    pub theme_manager: crate::ui::themes::ThemeManager,
    // ============ MDM ============
    pub mdm: crate::core::mdm::MdmManager,
    // ============ Code Structure Index ============
    pub structure_index: Option<crate::core::context::structure_index::StructureIndex>,
    // ============ Highlight/Dialog States ============
    //
    // 这些 `show_*` 曾经各配一个 bool，但窗口早已迁到 `modal_stack` 表达，
    // bool 只剩下声明与初始化、从不写入也从不读取 —— 是一层假状态：任何
    // 「if state.show_xxx 就……」的守卫都会恒为假，而窗口其实开着。
    // 可见性一律看 `modal_stack`（见 `top_modal()` / `is_modal_open()`）。
    pub global_search_state: crate::ui::components::highlight::search::GlobalSearchState,
    /// 全局搜索请求序列号，跨弹窗生命周期单调递增，防止旧结果命中新弹窗。
    next_global_search_request_id: u64,
    pub quick_open_state: crate::ui::components::highlight::quick_open::QuickOpenState,
    pub history_search_state: crate::ui::components::highlight::history::HistorySearchState,
    pub selected_theme_index: usize,
    /// 进入主题选择器前的主题名（Esc 取消时恢复）
    pub theme_picker_prev: Option<String>,
    pub usage_stats: crate::ui::components::highlight::stats::UsageStats,
    pub export_state: crate::ui::components::highlight::export::ExportState,
    pub compression_state: crate::ui::components::highlight::compression::CompressionState,
    // ============ New Feature States ============
    pub context_breakdown: crate::ui::components::highlight::context_viz::TokenBreakdown,
    pub error_overlay_state: crate::ui::components::error_overlay::ErrorOverlayState,
    pub log_selector_state: crate::ui::components::log_selector::LogSelectorState,
    pub diff_preview_scroll: usize,
    pub pending_model_confirmation: bool,
    pub last_code_block_content: Option<String>,
    pub vim_enabled: bool,
    pub vim_state: crate::ui::vim::VimState,
    /// 鼠标捕获是否开着。
    ///
    /// 开着时滚轮以真实滚轮事件到达（不再被终端转成箭头键污染输入历史），
    /// 代价是**终端在整个会话期间不再自己做选区** —— 而本应用只在聊天区实现了
    /// 左键拖选。关掉它原生选区就回来了（滚轮改由终端转成箭头键）。
    /// 启动初值来自 `STARCODE_ENABLE_MOUSE`。
    pub mouse_capture_enabled: bool,
    pub settings_selected_index: usize,
    pub show_scroll_to_bottom: bool,
    pub show_clear_confirmation: bool,
    /// Optional `/clear <name>` label. Applied only after the user confirms, so
    /// cancelling does not create a stray session snapshot.
    pub pending_clear_name: Option<String>,
    pub colorblind_mode: bool,
    pub toast_queue: VecDeque<Toast>,
    pub send_animation_since: Option<Instant>,
    pub draft_path: Option<std::path::PathBuf>,
    pub request_clear_screen: bool,
    pub show_paste_confirmation: bool,
    pub pending_paste: Option<String>,
    // ============ Agent 任务追踪 ============
    /// 活跃的 Agent 任务信息
    pub active_agent_tasks: HashMap<String, AgentTaskInfo>,
    /// 当前活跃的 Agent 组 ID
    pub agent_group_id: Option<String>,
    /// 正在聚焦查看的 Agent ID
    pub viewing_agent_task_id: Option<String>,
    /// 后台代理选择器的焦点（对标 background agent selector 的 selectedBgAgentIndex）
    ///
    /// `None` = 焦点在输入框（默认）；`Some(0)` = main 会话行；`Some(n)` = 第 n-1 个后台代理。
    /// 刻意**不从** `viewing_agent_task_id` 推导：参照实现要求两者分离 —— 「光标停在哪一行」
    /// 和「当前正在看谁的输出」是两件事，用户可以边浏览行边保持在某个 agent 的详情里。
    pub bg_agent_selection: Option<usize>,
    /// 全局 transcript 模式开关（Ctrl+O 切换）
    pub is_transcript_mode: bool,
    // ========================================
}

/// 输入框草稿（还没发出去的内容）落盘在哪里。
///
/// 优先级：`STAR_DRAFT_FILE` 显式指定 > 按当前工作目录分桶的
/// `~/.star/drafts/<cwd-hash>.txt`。按项目分桶和命令历史
/// （`history_store`，同为 `<cwd-hash>.json`）一致：在 A 项目敲到一半退出，
/// 不该在 B 项目启动时冒出来当「残留」。
///
/// 单元测试刻意指向临时目录下的独立文件。`save_draft` 在每次输入变化时同步写盘，
/// 而测试用例的输入文本（"queued"、"half typed" 之类）绝不能写进用户真实的草稿 ——
/// 否则下次启动 `restore_draft` 会把它原样填进输入框。
fn resolve_draft_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("STAR_DRAFT_FILE") {
        return Some(std::path::PathBuf::from(path));
    }

    #[cfg(test)]
    {
        // 每次调用一个独立文件：并行测试互不覆盖，也绝不碰用户真实草稿。
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        return Some(std::env::temp_dir().join(format!(
            "starcode-test-draft-{}-{}.txt",
            std::process::id(),
            seq
        )));
    }

    #[allow(unreachable_code)]
    {
        use std::hash::{Hash, Hasher};
        let cwd = std::env::current_dir().ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        cwd.hash(&mut hasher);
        let key = format!("{:016x}", hasher.finish());
        Some(
            dirs::home_dir()?
                .join(".star")
                .join("drafts")
                .join(format!("{key}.txt")),
        )
    }
}

impl ChatState {
    pub fn new() -> Self {
        // 不再同步阻塞获取 git status（大仓库中 git status 可能需要数秒），
        // 由 run_ui_loop 中的 spawn_git_status_loop 后台异步更新
        Self::with_git_info(None)
    }

    /// 模型列表现在**还**在拉吗？
    ///
    /// 超过 [`MODELS_LOADING_DEADLINE`] 就算是"没在拉了"，不管标志本身是不是还挂着。
    /// 宁可提前显示一个空列表 + 一句错误，也不要让转圈永远停不下来 —— 后者让
    /// 整个面板看上去卡死，而且用户没有任何办法从中恢复。
    ///
    /// 上限取 15s：worker 侧活动 provider 的拉取上限是 8s、其余 provider 各 3s
    /// 并发扇出，都留有余量；真的超了就是 worker 那边出问题了，那本来就该显示错误。
    pub fn models_loading(&self) -> bool {
        if !self.awaiting_models {
            return false;
        }
        match self.awaiting_models_since {
            // 超时了就当作没在拉 —— 宁可显示空列表 + 错误，也不要让转圈停不下来。
            Some(since) if since.elapsed() >= MODELS_LOADING_DEADLINE => false,
            _ => true,
        }
    }

    /// 全部仍然算数的 provider id：自定义的（`configured_providers`，由 worker 从
    /// 配置同步过来）+ 内建的。
    ///
    /// 这是"provider 还在不在"的**唯一定义**，UI 侧所有需要按这个判断过滤的地方都
    /// 必须用它 —— 切换请求（`input::resolve_provider_for_model`）和状态栏显示
    /// （`utils::status::current_provider_id`）各有各的推导链，两边判据一旦不一致，
    /// 就会出现"面板写着 Provider A、实际发给 B"，或者"发给一个已删除的 provider、
    /// 被 worker 静默拒绝"。
    ///
    /// 判据本身刻意和 worker 侧 `StarAgent::apply_model_selection` 的校验一致。
    ///
    /// `configured_providers` 尚未加载时（刚启动还没收到 `ConfiguredProviders`）
    /// 它会是空的，此时自定义 provider 全部被当成已死 —— 这比"当成全活着"安全：
    /// 前者退到 `current_provider_id`（那本来就是当前 provider，切换照样生效），
    /// 后者才会把模型发给一个可能不存在的 provider，然后静默失败。
    pub fn live_providers(&self) -> HashSet<String> {
        let mut live: HashSet<String> = self.configured_providers.iter().cloned().collect();
        live.extend(
            crate::core::config::providers::ALL_PROVIDERS
                .iter()
                .map(|p| p.id.to_string()),
        );
        live
    }

    /// 单个 provider 是否仍然算数。见 [`ChatState::live_providers`]。
    pub fn provider_is_live(&self, provider_id: &str) -> bool {
        crate::core::config::providers::get_provider_by_id(provider_id).is_some()
            || self.configured_providers.iter().any(|id| id == provider_id)
    }

    /// 模型列表的年龄（秒）。`None` = 这个会话里还没拿到过列表。
    ///
    /// 拆成"观察时刻 + 当时已多老"来算，全程单调递增，不做任何可能回退的时间减法。
    pub fn models_list_age_secs(&self) -> Option<u64> {
        let since_seen = self.models_listed_at?.elapsed().as_secs();
        Some(self.models_list_age_at_set.saturating_add(since_seen))
    }

    /// 分配一个不在同一进程中复用的全局搜索请求 ID。
    pub fn allocate_global_search_request_id(&mut self) -> u64 {
        let request_id = self.next_global_search_request_id;
        self.next_global_search_request_id = self
            .next_global_search_request_id
            .checked_add(1)
            .expect("global search request ID exhausted");
        request_id
    }

    /// 记下"这份列表是 `age_secs` 秒前拉的"。`None` 表示刚从 API 拉的。
    ///
    /// 只记录、不回推算时刻 —— 见 `models_listed_at` 的说明：`Instant` 未必能回溯
    /// 那么久，回推会把缓存年龄悄悄压成 0。
    pub fn set_models_list_age(&mut self, age_secs: Option<u64>) {
        self.models_listed_at = Some(Instant::now());
        self.models_list_age_at_set = age_secs.unwrap_or(0);
    }

    /// Push a toast notification (auto-dismisses after duration)
    pub fn push_toast(&mut self, message: &str, kind: ToastKind) {
        self.toast_queue.push_back(Toast {
            message: message.to_string(),
            kind,
            created_at: Instant::now(),
            duration_secs: 3,
        });
        // Keep max 5 toasts
        while self.toast_queue.len() > 5 {
            self.toast_queue.pop_front();
        }
    }

    /// Save current input draft to file
    pub fn save_draft(&self) {
        if let Some(ref path) = self.draft_path {
            let text = self.textarea.lines().join("\n");
            if !text.trim().is_empty() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, &text);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Restore draft from file
    ///
    /// 草稿排在前，输入框里已有的内容（初始化等待期间敲的字）接在后面 ——
    /// 规则和 `restore_queued_messages_to_input` 一致：先敲的内容不能被后恢复的内容覆盖。
    /// 草稿文件取完即删，不会连续两次启动都恢复同一段。
    pub fn restore_draft(&mut self) {
        let Some(path) = self.draft_path.as_ref() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        if text.trim().is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let _ = std::fs::remove_file(path);

        let mut parts = vec![text];
        if !self.input.trim().is_empty() {
            parts.push(std::mem::take(&mut self.input));
        }
        self.set_input_text(&parts.join("\n"));
    }

    /// 把输入框整体重设为 `text`（重建 textarea，同步 `input` 与行数计数器）。
    ///
    /// 语义是「设」而不是「追加」：`calc_input_height` 只看 `input_line_count`，
    /// 只动 textarea 的话多行内容会被压进一行高的输入框，`state.input` 也仍是旧值。
    /// 所有需要整体重设输入框的地方都走这里，免得 textarea 和 `state.input` 走岔。
    pub fn set_input_text(&mut self, text: &str) {
        let mut textarea = TextArea::default();
        textarea.set_placeholder_text(crate::ui::utils::text::input_placeholder_text());
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_cursor_style(
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        );
        if !text.is_empty() {
            textarea.insert_str(text);
        }
        self.textarea = textarea;
        self.input = text.to_string();
        self.input_line_count = text.lines().count().max(1);
        self.input_folded = false;
        self.paste_segments.clear();
    }

    /// Clear saved draft
    pub fn clear_draft(&self) {
        if let Some(ref path) = self.draft_path {
            let _ = std::fs::remove_file(path);
        }
    }

    fn with_git_info(git_info: Option<(String, String)>) -> Self {
        let (git_branch, git_status) = git_info
            .map(|(b, s)| (Some(b), Some(s)))
            .unwrap_or((None, None));
        let transcript_enabled = crate::ui::utils::transcript::transcript_enabled_from_env();
        let transcript_path = if transcript_enabled {
            crate::ui::utils::transcript::resolve_transcript_path()
        } else {
            None
        };

        let mut textarea = TextArea::default();
        textarea.set_placeholder_text(&crate::ui::utils::text::input_placeholder_text());
        // Remove underline from cursor line
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        // 确保光标样式可见，使用反转颜色
        textarea.set_cursor_style(
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        );

        // 抬头占位条目：渲染走 ui::components::welcome_header（从 state 现算，
        // 见 message_render 里的 `is_welcome` 分支），所以 content 只是兜底文案，
        // 界面上并不显示；其余消费者一律按 is_welcome 跳过它。
        let welcome_entry = ChatEntry {
            entry_type: crate::types::ChatEntryType::Assistant,
            content: crate::core::i18n::t(
                "ui.welcome.message",
                "欢迎使用 StarCode CLI! 输入你的问题，或使用 /help 查看命令。",
                "Welcome to StarCode CLI! Type your question or use /help for commands.",
            ),
            is_welcome: true,
            ..ChatEntry::new(crate::types::ChatEntryType::Assistant, String::new())
        };

        Self {
            chat_history: vec![welcome_entry],
            input: String::new(),
            input_line_count: 1, // textarea always has at least 1 line [""]
            textarea,
            scroll: 0,
            total_rendered_lines: 0,
            chat_list_state: ListState::default(),
            chat_scrollbar_state: ScrollbarState::default(),
            expanded_tool_call_ids: HashSet::new(),
            expanded_thinking_indices: HashSet::new(),
            complete_task_message_ids: HashSet::new(),
            auto_continued_message_ids: HashSet::new(),
            pending_user_messages: VecDeque::new(),
            // ============ Command History ============
            command_history: crate::core::config::history_store::load_history(),
            history_index: None,
            history_input_snapshot: None,
            // ==========================================
            tool_started_at: HashMap::new(),
            tool_call_args_cache: HashMap::new(),
            tool_call_transcript_written: HashSet::new(),
            transcript_enabled: transcript_enabled && transcript_path.is_some(),
            transcript_path,
            transcript_run_id: format!(
                "run-{}-{}",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
                std::process::id()
            ),
            transcript_seq: 0,
            last_chat_area: None,
            last_input_area: None,
            last_item_heights: Vec::new(),
            show_command_hints: false,
            command_hints: Vec::new(),
            selected_hint: 0,
            show_mention_hints: false,
            mention_hints: Vec::new(),
            selected_mention_hint: 0,
            preview_visible: false,
            preview_scroll_state: PreviewScrollState::default(),
            preview_last_entry_idx: None,
            is_processing: false,
            is_streaming: false,
            processing_started_at: None,
            model_wait_started_at: None,
            cancelling_since: None,
            processing_time_secs: 0,
            token_count: 0,
            total_cost: 0.0,
            response_costs: HashMap::new(),
            response_models: HashMap::new(),
            session_started_at: Instant::now(),
            last_token_time: None,
            thinking_started_at: None,
            thinking_finished: None,
            current_status_line: None,
            available_models: Vec::new(),
            available_models_info: Vec::new(),
            model_provider_map: std::collections::HashMap::new(),
            current_model: String::new(),
            current_model_supports_thinking: None,
            current_provider_id: None,
            last_turn_failed: false,
            pending_confirmation: None,
            pending_model_change: None,
            pending_model_provider: None,
            pending_provider_selected_model: None,
            waive_next_model_confirmation: false,
            pending_palette_action: None,
            quick_menu_origin_palette: false,
            quick_menu_back: None,
            show_help: false,
            auto_follow: true,
            last_chat_height: 0,
            last_max_scroll: 0,
            next_message_id: 1,
            stream_targets: HashMap::new(),
            message_start_indices: HashMap::new(),
            active_message_id: None,
            awaiting_models: false,
            awaiting_models_since: None,
            models_listed_at: None,
            models_list_age_at_set: 0,
            mcp_ready: false,
            auto_continue_enabled: false,
            auto_continue_remaining: 0,
            approval_mode: crate::types::ApprovalMode::Default,
            thinking_effort: crate::types::ThinkingEffort::default(),
            next_runtime_settings_revision: 1,
            requested_runtime_settings: None,
            pending_runtime_settings: VecDeque::new(),
            confirmed_runtime_settings: None,
            last_runtime_settings_ack: None,
            runtime_settings_persistence: None,
            welcome_header_fingerprint: None,
            extra_working_dirs: Vec::new(),
            fast_mode: false,
            network_offline: false,
            break_cache_next: false,
            fast_mode_prev_model: None,
            pending_fast_mode_transition: None,
            poor_mode: false,
            advisor_mode: false,
            context_window_override: None,
            pending_tool_calls: None,
            pending_message_id: None,
            pending_confirmation_entry_idx: None,
            pending_confirmation_choice: 0,
            is_awaiting_confirmation: false,
            pending_tool_call_id: None,
            pending_confirmation_originating_id: None,
            pending_question_selections: Vec::new(),
            pending_other_input: String::new(),
            pending_question_other_focused: false,
            confirmation_feedback_mode: false,
            pending_confirmation_feedback: String::new(),
            pending_confirmation_queue: VecDeque::new(),
            last_confirmation_message_id: None,
            text_selection: TextSelection::new(),
            chat_search: ChatSearchState::default(),
            copy_status: None,
            git_branch,
            git_status,
            au2_compressed: false,
            token_usage: None,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cache_warning_shown: HashSet::new(),
            rendered_cache: HashMap::new(),
            virtual_list: VirtualList::new(),
            last_terminal_width: 0,
            last_rendered_stream_key: HashMap::new(),
            modal_stack: Vec::new(),
            mcp_modal_servers: Vec::new(),
            mcp_modal_index: 0,
            mcp_modal_menu_index: 0,
            mcp_modal_tools: Vec::new(),
            mcp_modal_loading: false,
            mcp_modal_error: None,
            mcp_modal_action_msg: None,
            market_entries: Vec::new(),
            market_index: 0,
            market_query: String::new(),
            market_loading: false,
            market_message: None,
            market_confirm: None,
            market_installed_names: std::collections::HashSet::new(),
            market_enabled_map: std::collections::HashMap::new(),
            market_plugin_names: std::collections::HashSet::new(),
            plugin_discover: Vec::new(),
            plugin_installed: Vec::new(),
            plugin_marketplaces: Vec::new(),
            plugin_marketplace_counts: std::collections::HashMap::new(),
            plugin_errors: Vec::new(),
            plugin_errors_hint: None,
            plugin_index: 0,
            plugin_search: String::new(),
            plugin_selected: std::collections::HashSet::new(),
            plugin_detail: None,
            plugin_batch_total: 0,
            plugin_batch_done: 0,
            plugin_loading: false,
            plugin_op_pending: false,
            plugin_message: None,
            plugin_confirm: None,
            palette_items: Vec::new(),
            selected_palette_index: 0,
            palette_mode: crate::ui::state::palette::PaletteMode::Main,
            palette_history: Vec::new(),
            palette_filter: String::new(),
            show_provider_menu: false,
            selected_provider_index: 0,
            show_session_menu: false,
            selected_session_index: 0,
            available_sessions: Vec::new(),
            input_modal_title: String::new(),
            input_modal_prompt: String::new(),
            input_modal_value: String::new(),
            modal_textarea: TextArea::default(),
            input_context: None,
            configured_providers: Vec::new(),
            keyless_providers: Vec::new(),
            provider_form: crate::ui::state::palette::ProviderFormState::new(),
            task_panel: crate::ui::components::task_panel::TaskPanel::new(),
            paste_in_progress: false,
            paste_end_time: None,
            input_folded: false,
            paste_segments: Vec::new(),
            should_exit: false,
            last_ctrl_c: None,
            last_esc_at: None,
            show_permission_explanation: false,
            show_permission_debug: false,
            ui_verbose: false,
            kill_ring: std::collections::VecDeque::new(),
            kill_ring_pos: None,
            streaming_height_floor: std::collections::HashMap::new(),
            last_yank_len: 0,
            sandbox_enabled: SandboxManager::is_available(),
            sandbox_available: std::cell::OnceCell::new(),
            animation_tick: 0,
            scroll_velocity: 0.0,
            last_scroll_time: None,
            current_tool_name: None,
            active_session_id: None,
            current_session_title: None,
            permission_rules: crate::core::permission_rules::PermissionRuleEngine::new(),
            feature_flags: FeatureFlags::new(),
            proactive_suggestions: crate::core::proactive::ProactiveSuggestions::new(),
            tools_used: Vec::new(),
            remote_settings: crate::core::remote_settings::RemoteSettings::new(),
            voice_config: crate::core::voice::VoiceConfig::default(),
            notifications: crate::core::notifications::NotificationManager::new(),
            theme_manager: crate::ui::themes::ThemeManager::new(),
            mdm: crate::core::mdm::MdmManager::new(),
            structure_index: None,
            // ============ Highlight/Dialog States ============
            global_search_state: crate::ui::components::highlight::search::GlobalSearchState::new(),
            next_global_search_request_id: 1,
            quick_open_state: crate::ui::components::highlight::quick_open::QuickOpenState::new(),
            history_search_state:
                crate::ui::components::highlight::history::HistorySearchState::new(),
            selected_theme_index: 0,
            theme_picker_prev: None,
            usage_stats: crate::ui::components::highlight::stats::UsageStats::default(),
            export_state: crate::ui::components::highlight::export::ExportState::new(),
            compression_state:
                crate::ui::components::highlight::compression::CompressionState::default(),
            // ============ New Feature States ============
            context_breakdown:
                crate::ui::components::highlight::context_viz::TokenBreakdown::default(),
            error_overlay_state: crate::ui::components::error_overlay::ErrorOverlayState::default(),
            log_selector_state: crate::ui::components::log_selector::LogSelectorState::default(),
            diff_preview_scroll: 0,
            pending_model_confirmation: false,
            last_code_block_content: None,
            vim_enabled: false,
            vim_state: crate::ui::vim::VimState::new(),
            mouse_capture_enabled: !crate::ui::utils::term_recovery::mouse_capture_disabled_by_env(
            ),
            settings_selected_index: 0,
            show_scroll_to_bottom: false,
            show_clear_confirmation: false,
            pending_clear_name: None,
            colorblind_mode: false,
            toast_queue: VecDeque::new(),
            send_animation_since: None,
            draft_path: resolve_draft_path(),
            request_clear_screen: false,
            show_paste_confirmation: false,
            pending_paste: None,
            // ============ Agent 任务追踪 ============
            active_agent_tasks: HashMap::new(),
            agent_group_id: None,
            viewing_agent_task_id: None,
            bg_agent_selection: None,
            is_transcript_mode: false,
            // ========================================
            pending_scroll_direction: None,
            pending_scroll_time: None,
        }
    }

    /// 分配单调 UI revision，并记录请求意图；active 值必须等 worker acknowledgement。
    pub fn begin_runtime_settings_update(
        &mut self,
        model: Option<crate::runtime::messages::RuntimeModelSelection>,
        context_window: Option<crate::core::context_policy::ContextWindowSelection>,
        thinking_effort: Option<crate::types::ThinkingEffort>,
    ) -> crate::runtime::messages::RuntimeSettingsMutation {
        self.begin_runtime_settings_update_with_persistence(
            model,
            context_window,
            thinking_effort,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent,
        )
    }

    /// 记录带明确持久化语义的 runtime mutation。会话级变更同样必须占用 revision，
    /// 以保留流式回合结束后的 FIFO 顺序，但不能让 UI 误报设置文件正在写入。
    pub fn begin_runtime_settings_update_with_persistence(
        &mut self,
        model: Option<crate::runtime::messages::RuntimeModelSelection>,
        context_window: Option<crate::core::context_policy::ContextWindowSelection>,
        thinking_effort: Option<crate::types::ThinkingEffort>,
        persistence: crate::runtime::messages::RuntimeSettingsPersistencePolicy,
    ) -> crate::runtime::messages::RuntimeSettingsMutation {
        let ui_revision = self.next_runtime_settings_revision;
        self.next_runtime_settings_revision = self.next_runtime_settings_revision.saturating_add(1);
        let mutation = crate::runtime::messages::RuntimeSettingsMutation {
            ui_revision,
            persistence,
            model,
            context_window,
            thinking_effort,
        };
        let mut requested = self
            .requested_runtime_settings
            .clone()
            .or_else(|| {
                self.confirmed_runtime_settings
                    .as_ref()
                    .map(|snapshot| snapshot.requested.clone())
            })
            .unwrap_or_else(|| crate::runtime::messages::RequestedRuntimeSettings {
                model: crate::runtime::messages::RuntimeModelSelection {
                    model: self.current_model.clone(),
                    provider_id: self.current_provider_id.clone(),
                },
                context_window: self
                    .context_window_override
                    .map(crate::core::context_policy::ContextWindowSelection::Fixed)
                    .unwrap_or_default(),
                thinking_effort: self.thinking_effort.clone(),
            });
        if let Some(model) = mutation.model.clone() {
            requested.model = model;
        }
        if let Some(context_window) = mutation.context_window {
            requested.context_window = context_window;
        }
        if let Some(thinking_effort) = mutation.thinking_effort.clone() {
            requested.thinking_effort = thinking_effort;
        }
        self.requested_runtime_settings = Some(requested);
        self.pending_runtime_settings.push_back(mutation.clone());
        mutation
    }

    /// 移除尚未送达 worker 的 mutation，并从已确认状态重建剩余请求意图。
    /// 不能只删队列：否则 channel 已关闭时 UI 仍会展示一个从未被 worker 接收的请求。
    pub fn discard_runtime_settings_update(&mut self, ui_revision: u64) {
        let previous_len = self.pending_runtime_settings.len();
        self.pending_runtime_settings
            .retain(|pending| pending.ui_revision != ui_revision);
        if self.pending_runtime_settings.len() == previous_len {
            return;
        }

        let fallback_requested = crate::runtime::messages::RequestedRuntimeSettings {
            model: crate::runtime::messages::RuntimeModelSelection {
                model: self.current_model.clone(),
                provider_id: self.current_provider_id.clone(),
            },
            context_window: self
                .context_window_override
                .map(crate::core::context_policy::ContextWindowSelection::Fixed)
                .unwrap_or_default(),
            thinking_effort: self.thinking_effort.clone(),
        };
        let mut requested = self
            .confirmed_runtime_settings
            .as_ref()
            .map(|snapshot| snapshot.requested.clone());
        for mutation in &self.pending_runtime_settings {
            let requested = requested.get_or_insert_with(|| fallback_requested.clone());
            if let Some(model) = mutation.model.clone() {
                requested.model = model;
            }
            if let Some(context_window) = mutation.context_window {
                requested.context_window = context_window;
            }
            if let Some(thinking_effort) = mutation.thinking_effort.clone() {
                requested.thinking_effort = thinking_effort;
            }
        }
        self.requested_runtime_settings = requested;
    }

    pub fn clear_cache(&mut self) {
        self.rendered_cache.clear();
        self.last_rendered_stream_key.clear();
        self.streaming_height_floor.clear();
        self.virtual_list.mark_all_dirty();
    }

    /// 推进任务面板的时间相关状态：磁盘自愈、自动显隐、完成后 5 秒收起计时。
    ///
    /// **由事件循环的周期节拍调用，不在渲染里。** 此前这段在 `render_page` 开头，
    /// 于是每帧一次 `current_dir()` + `fs::metadata`（流式时 30 次/秒），而且
    /// 显隐判定会在绘制过程中改 `is_visible` —— 「画没画过」变成了状态的一部分。
    ///
    /// 1Hz 足够：清单变更是 TodoWrite 驱动的，收起计时本身也是秒级。
    pub fn tick_task_panel(&mut self) {
        // 先自愈：磁盘清单比内存新（TodoWrite 被打断、别的进程改了文件等没人
        // 通知面板的路径）就 reload，下面的显隐判定才基于最新数据。
        self.task_panel.reload_if_stale();
        self.task_panel.auto_show_if_needed();
        // is_processing 在 agent 收到 Done 时回落 false（stream.rs）——残留
        // in_progress 的清单在 agent 停下后才能开始 5 秒收起计时
        self.task_panel.check_auto_hide(self.is_processing);
    }

    /// 清掉过期 toast。
    ///
    /// 此前这一步在 `render_toasts` 里：绘制过程中删队列元素，意味着「画没画过」
    /// 会改变状态。现在与任务面板一起挂在事件循环的 1Hz 节拍上。
    pub fn expire_toasts(&mut self) {
        let now = std::time::Instant::now();
        self.toast_queue
            .retain(|t| now.duration_since(t.created_at).as_secs() < t.duration_secs);
    }

    /// 思考阶段结束：把进行中的计时器结算成「结束后回顾」状态。
    ///
    /// 对标 Claude Code Spinner 的 `thought for Xs`：思考结束后回顾标签继续
    /// 驻留 2 秒再随 spinner 行消失，而不是思考一结束就瞬间蒸发。
    pub fn end_thinking(&mut self) {
        if let Some(started) = self.thinking_started_at.take() {
            let secs = started.elapsed().as_secs();
            self.thinking_finished = Some((secs, Instant::now()));
        }
    }

    /// 切换 transcript（verbose 输出）模式，返回切换后的状态。
    ///
    /// 对标 Claude Code 的 `app:toggleTranscript`：transcript 视图是「把这一轮的全部原始
    /// 输出摊开」，所以 thinking 块随之展开／收起，而不是再占用一个独立快捷键。
    /// Ctrl+O 与 `/tui transcript` 共用这里，保证两条入口行为一致。
    pub fn toggle_transcript_mode(&mut self) -> bool {
        self.is_transcript_mode = !self.is_transcript_mode;
        if self.is_transcript_mode {
            for (idx, entry) in self.chat_history.iter().enumerate() {
                if entry
                    .reasoning_content
                    .as_ref()
                    .is_some_and(|r| !r.is_empty())
                {
                    self.expanded_thinking_indices.insert(idx);
                }
            }
        } else {
            self.expanded_thinking_indices.clear();
        }
        self.clear_cache();
        self.is_transcript_mode
    }

    // ── 后台代理选择器（对标 background agent selector）──────────────────

    /// 选择器可列出的后台代理，单一数据源。
    ///
    /// 只收 `is_async` 的任务：同步子代理是当前 turn 的一部分，已经在聊天流里逐行展示，
    /// 再进选择器只会重复。按 `started_at` 升序，保证行序稳定（`active_agent_tasks` 是
    /// `HashMap`，直接遍历的顺序每帧都可能变）。
    ///
    /// 参照实现用 `evictAfter` 做过期驱逐；这里 `active_agent_tasks` 只插不删，所以靠
    /// [`MAX_BG_AGENT_ROWS`] 上限收敛 —— 长会话里只保留最近启动的若干个。
    pub fn background_agent_rows(&self) -> Vec<&AgentTaskInfo> {
        let mut rows: Vec<&AgentTaskInfo> = self
            .active_agent_tasks
            .values()
            .filter(|t| t.is_async)
            .collect();
        rows.sort_by_key(|t| t.started_at);
        if rows.len() > MAX_BG_AGENT_ROWS {
            rows.drain(..rows.len() - MAX_BG_AGENT_ROWS);
        }
        rows
    }

    /// 进入某个后台代理的详情视图（对标 `enterTeammateView`）。
    ///
    /// 只改 `viewing_agent_task_id`，**不动** `bg_agent_selection` —— 选择器的焦点要留在
    /// 原行上，用户才能连着 ↑/↓ 翻看下一个 agent。
    pub fn enter_teammate_view(&mut self, task_id: &str) {
        if self.viewing_agent_task_id.as_deref() == Some(task_id) {
            return;
        }
        self.viewing_agent_task_id = Some(task_id.to_string());
        self.clear_cache();
    }

    /// 退出详情视图，回到主会话（对标 `exitTeammateView`）。
    pub fn exit_teammate_view(&mut self) {
        if self.viewing_agent_task_id.take().is_some() {
            self.clear_cache();
        }
    }

    /// 选区里的纯文本。
    ///
    /// 换算逻辑在 `ui::selection`（与高亮共用一套列宽口径）；这里只负责
    /// "这个条目的渲染行从哪来"。
    pub fn get_selected_text(&self) -> Option<String> {
        let range = crate::ui::selection::SelectionRange::resolve(&self.text_selection)?;
        crate::ui::selection::selected_text(&range, |entry_idx| {
            crate::ui::selection::entry_rendered_lines(self, entry_idx)
        })
    }

    /// 某个工具块当前是展开还是折叠。
    ///
    /// 折叠状态存的是"与默认值相反的翻转位"（`expanded_tool_call_ids`）：
    /// ToolCall 默认展开；ToolResult 里只有编辑类工具默认展开。
    /// 这段判断以前在 `tool_render.rs` 里就地算，现在收在这里，读写两端同一份。
    pub fn is_tool_expanded(&self, entry: &ChatEntry) -> bool {
        let Some(tc) = entry.tool_call.as_ref() else {
            return true;
        };
        // 走统一归一，避免别名漏判（对标 constants::is_edit_tool_name）
        let is_edit_tool =
            crate::core::tools::constants::is_edit_tool_name(tc.function.name.as_str())
                || tc.function.name == "TodoWrite";

        let default_expanded = matches!(entry.entry_type, crate::types::ChatEntryType::ToolCall)
            || (matches!(entry.entry_type, crate::types::ChatEntryType::ToolResult)
                && is_edit_tool);
        default_expanded ^ self.expanded_tool_call_ids.contains(&tc.id)
    }

    /// 折叠 / 展开一个工具块，返回是否真的切换了。
    ///
    /// 以前 `mouse.rs`（单击工具头）和 `input.rs`（Tab）各写一遍增删逻辑，
    /// 而两边的收尾动作不一样：`input.rs` 会 `virtual_list.mark_dirty`，
    /// `mouse.rs` 不会 —— 靠"缓存里没有这一项"侥幸触发重渲染。
    /// 现在只此一处，切换后统一失效缓存 + 标脏。
    pub fn toggle_tool_expansion(&mut self, entry_idx: usize) -> bool {
        let Some(entry) = self.chat_history.get(entry_idx) else {
            return false;
        };
        let Some(tc) = entry.tool_call.as_ref() else {
            return false;
        };
        let tool_id = tc.id.clone();

        if self.expanded_tool_call_ids.contains(&tool_id) {
            self.expanded_tool_call_ids.remove(&tool_id);
        } else {
            self.expanded_tool_call_ids.insert(tool_id);
        }
        self.rendered_cache.remove(&entry_idx);
        self.virtual_list.mark_dirty(entry_idx);
        true
    }

    pub fn update_item_height(&mut self, index: usize, height: u16) {
        if index < self.last_item_heights.len() {
            let old_height = self.last_item_heights[index] as usize;
            let new_height = height as usize;

            if new_height != old_height {
                self.last_item_heights[index] = height;
                if new_height > old_height {
                    self.total_rendered_lines += new_height - old_height;
                } else {
                    self.total_rendered_lines -= old_height - new_height;
                }
            }
        } else if index == self.last_item_heights.len() {
            self.last_item_heights.push(height);
            self.total_rendered_lines += height as usize;
        } else {
            // Gap in cache, forced to rebuild or panic?
            // For safety, just clear cache if we get out of sync
            self.clear_cache();
        }
    }
}

pub fn scroll_chat(state: &mut ChatState, delta: i32) {
    if state.total_rendered_lines == 0 {
        state.scroll = 0;
        return;
    }

    // Viewport height is needed to calculate max scroll.
    let viewport_height = if state.last_chat_height > 0 {
        state.last_chat_height as usize
    } else {
        state
            .last_chat_area
            .map(|a| a.height as usize)
            .unwrap_or(0usize)
    };
    let max_scroll = state.total_rendered_lines.saturating_sub(viewport_height);

    // Keep scroll in valid range before applying new delta.
    if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }

    // Direct scroll application (no momentum for predictable behavior)
    if delta < 0 {
        // Scrolling up — pause auto-follow so user can read history
        let d = delta.unsigned_abs() as usize;
        state.scroll = state.scroll.saturating_sub(d);
        state.auto_follow = false;
    } else {
        // Scrolling down
        let d = delta as usize;
        state.scroll = state.scroll.saturating_add(d).min(max_scroll);
        if state.scroll >= max_scroll {
            state.auto_follow = true;
            state.show_scroll_to_bottom = false;
        }
    }
}

pub fn bump_index_map(map: &mut HashMap<u64, usize>, from: usize, delta: usize) {
    for v in map.values_mut() {
        if *v >= from {
            *v += delta;
        }
    }
}

pub fn bump_indices_after_insert(state: &mut ChatState, from: usize, delta: usize) {
    // 插入元素会导致后续索引变化，需要平移渲染缓存而不是全部清理
    let mut new_cache = HashMap::new();
    for (idx, value) in state.rendered_cache.drain() {
        if idx >= from {
            new_cache.insert(idx + delta, value);
        } else {
            new_cache.insert(idx, value);
        }
    }
    state.rendered_cache = new_cache;

    bump_index_map(&mut state.stream_targets, from, delta);
    bump_index_map(&mut state.message_start_indices, from, delta);
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx >= from {
            *idx += delta;
        }
    }
    for queued in state.pending_confirmation_queue.iter_mut() {
        if queued.entry_idx >= from {
            queued.entry_idx += delta;
        }
    }
}

impl ChatState {
    /// 把排队中的下一张确认卡设为当前等待回答的卡。
    ///
    /// 返回 `false` 表示队列已空。调用方通常是 [`Self::clear_pending_confirmation`]：
    /// 一张卡刚被回答/拒绝/取消，下一条并发请求要接上，否则它的门禁会一直等。
    pub fn activate_next_queued_confirmation(&mut self) -> bool {
        let Some(queued) = self.pending_confirmation_queue.pop_front() else {
            return false;
        };
        if queued.entry_idx >= self.chat_history.len() {
            // 卡片所属的回合已经被清掉了，继续找下一张。
            return self.activate_next_queued_confirmation();
        }

        // 排队时把 outcome 写成占位文案，现在要恢复成可回答的选项列表。
        if let Some(confirmation) = self.chat_history[queued.entry_idx].confirmation.as_mut() {
            confirmation.outcome = None;
        }
        self.chat_history[queued.entry_idx].is_streaming = Some(false);
        self.rendered_cache.remove(&queued.entry_idx);
        self.virtual_list.mark_dirty(queued.entry_idx);

        self.pending_confirmation_entry_idx = Some(queued.entry_idx);
        self.pending_message_id = Some(queued.message_id);
        self.pending_tool_call_id = Some(queued.tool_call_id);
        self.pending_confirmation_originating_id = queued.originating_tool_call_id;
        self.is_awaiting_confirmation = true;
        self.confirmation_feedback_mode = false;
        self.pending_confirmation_feedback.clear();
        self.pending_confirmation_choice = if queued.is_ask { 0 } else { 1 };
        self.pending_question_selections.clear();
        self.pending_other_input.clear();
        self.pending_question_other_focused = false;
        self.chat_list_state.select(Some(queued.entry_idx));
        true
    }

    /// 整个回合结束/中断：队列里的请求不再有意义，把卡片标成已取消。
    ///
    /// 注意顺序：先清队列再 [`Self::clear_pending_confirmation`]，否则清当前卡时
    /// 会把一张已经失效的请求重新变成等待态。
    pub fn discard_pending_confirmation_queue(&mut self, reason: &str) {
        let queued: Vec<QueuedConfirmation> = self.pending_confirmation_queue.drain(..).collect();
        for item in queued {
            if let Some(entry) = self.chat_history.get_mut(item.entry_idx) {
                if let Some(confirmation) = entry.confirmation.as_mut() {
                    if confirmation.outcome.is_none() {
                        confirmation.outcome = Some(reason.to_string());
                    }
                }
            }
            self.rendered_cache.remove(&item.entry_idx);
            self.virtual_list.mark_dirty(item.entry_idx);
        }
    }

    /// 把一张排队中的确认卡从队列里摘掉（对应工具已经有结果/已取消）。
    pub fn remove_queued_confirmation_by_originating_id(&mut self, originating_id: &str) {
        let mut removed = Vec::new();
        self.pending_confirmation_queue.retain(|item| {
            if item.originating_tool_call_id.as_deref() == Some(originating_id) {
                removed.push(item.entry_idx);
                false
            } else {
                true
            }
        });
        for idx in removed {
            if let Some(entry) = self.chat_history.get_mut(idx) {
                if let Some(confirmation) = entry.confirmation.as_mut() {
                    if confirmation.outcome.is_none() {
                        confirmation.outcome = Some(crate::core::i18n::t(
                            "ui.confirm.cancelled_turn",
                            "已取消 · 对应工具已有结果",
                            "Cancelled — the tool already finished",
                        ));
                    }
                }
            }
            self.rendered_cache.remove(&idx);
            self.virtual_list.mark_dirty(idx);
        }
    }

    /// 当前这张卡处理完了，接上下一张排队的卡。
    pub fn clear_pending_confirmation(&mut self) {
        self.is_awaiting_confirmation = false;
        self.pending_tool_call_id = None;
        self.pending_confirmation_originating_id = None;
        self.pending_confirmation_entry_idx = None;
        self.pending_confirmation_feedback.clear();
        self.confirmation_feedback_mode = false;
        self.pending_question_other_focused = false;
        self.pending_other_input.clear();
        self.pending_question_selections.clear();
        self.activate_next_queued_confirmation();
    }
}

#[cfg(test)]
mod tests {
    /// 回归：模型列表的"正在拉"状态**不能**永远挂着。
    ///
    /// 真实事故（用户截图里那个转不掉的 `◎ …`）：日志里只留下一条
    /// `[ListModels] Client state: …` 就再没有后续，worker 那个 spawn 什么消息都
    /// 没发出来，于是 `awaiting_models` 永远停在 true，模型面板的转圈也就永远
    /// 停不下来 —— 用户没有任何办法从中恢复。
    ///
    /// UI 侧现在靠 `models_loading()` 的上限兜底：宁可显示空列表 + 错误。
    #[test]
    fn a_lost_reply_cannot_latch_the_loading_state_forever() {
        let mut state = ChatState::new();
        assert!(!state.models_loading());

        state.awaiting_models = true;
        state.awaiting_models_since = Some(std::time::Instant::now());
        assert!(state.models_loading(), "刚置位时当然还在拉");

        // 模拟"回复丢了 + 时间过去了"：把起点往回拨，绕过真实等待。
        state.awaiting_models_since = Some(std::time::Instant::now() - MODELS_LOADING_DEADLINE);
        assert!(
            !state.models_loading(),
            "超过上限后必须判定为不再等待，否则转圈停不下来"
        );
    }

    /// 没记时刻时按"仍在拉"处理 —— 否则旧代码路径里直接置位的地方会误判。
    #[test]
    fn a_loading_flag_without_a_timestamp_still_counts_as_loading() {
        let mut state = ChatState::new();
        state.awaiting_models = true;
        state.awaiting_models_since = None;
        assert!(state.models_loading());
    }

    /// 收到回复后必须立刻不再转圈 —— 上限只是兜底，不是延后。
    #[test]
    fn a_reply_clears_the_loading_state_immediately() {
        let mut state = ChatState::new();
        state.awaiting_models = true;
        state.awaiting_models_since = Some(std::time::Instant::now());
        state.awaiting_models = false;
        state.awaiting_models_since = None;
        assert!(!state.models_loading());
    }

    /// 标志没置位时，无论时刻是什么都不该转圈。
    #[test]
    fn no_flag_means_not_loading_even_with_a_stale_timestamp() {
        let mut state = ChatState::new();
        state.awaiting_models = false;
        state.awaiting_models_since = Some(std::time::Instant::now() - MODELS_LOADING_DEADLINE);
        assert!(!state.models_loading());
    }
    use super::*;
    use crate::types::{ChatEntry, ChatEntryType};

    /// 测试用例的输入文本绝不能落进用户真实的草稿文件：`save_draft` 每次输入变化都写盘，
    /// 一旦写到 `~/.star/draft.txt`，下次启动 `restore_draft` 就会把测试字符串
    /// （"queued\nhalf typed" 之类）原样填进输入框。
    #[test]
    fn draft_path_in_tests_isolates_the_users_real_draft() {
        let state = ChatState::new();
        let path = state
            .draft_path
            .as_ref()
            .expect("draft path should always be resolved");

        assert!(
            path.starts_with(std::env::temp_dir()),
            "test draft must live in temp dir, got {}",
            path.display()
        );
        assert!(
            path.to_string_lossy().contains("starcode-test-draft"),
            "unexpected draft path: {}",
            path.display()
        );
    }

    /// 草稿恢复必须同步 `input` 和行数计数器：只动 textarea 的话多行草稿会被
    /// 压进按 0 行算的输入框，`state.input` 也仍是空的。
    #[test]
    fn restore_draft_syncs_input_and_line_count() {
        let dir = std::env::temp_dir().join(format!("starcode-draft-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("draft.txt");
        std::fs::write(&path, "line one\nline two\nline three").unwrap();

        let mut state = ChatState::new();
        state.draft_path = Some(path.clone());
        state.restore_draft();

        assert_eq!(state.input, "line one\nline two\nline three");
        assert_eq!(state.input_line_count, 3);
        assert_eq!(
            state.textarea.lines().join("\n"),
            "line one\nline two\nline three"
        );
        assert!(!path.exists(), "草稿取完应当即删除，避免重复恢复");
    }

    /// 草稿恢复发生在初始化完成之后，此时输入框可能已有等待期间敲的内容：
    /// 草稿在前、已敲的内容接在后面，不能互相覆盖。
    #[test]
    fn restore_draft_appends_after_what_was_already_typed() {
        let dir = std::env::temp_dir().join(format!("starcode-draft-{}-2", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("draft.txt");
        std::fs::write(&path, "the draft").unwrap();

        let mut state = ChatState::new();
        state.draft_path = Some(path);
        state.set_input_text("typed during loading");
        state.restore_draft();

        assert_eq!(state.input, "the draft\ntyped during loading");
    }

    /// 列表年龄要按"拉取时刻"算，而不是"收到消息的时刻"：会话开着两小时后再看，
    /// 一份两小时前从磁盘缓存读出来的列表不能显示成"刚拉的"。
    #[test]
    fn model_list_age_counts_from_the_fetch_not_from_receipt() {
        let mut state = ChatState::new();
        assert_eq!(state.models_list_age_secs(), None);

        // 刚从 API 拉的 -> 年龄约 0
        state.set_models_list_age(None);
        assert!(state.models_list_age_secs().unwrap() < 5);

        // 缓存里读出来的，两小时前拉的 -> 年龄从那时算起
        state.set_models_list_age(Some(7_200));
        let age = state.models_list_age_secs().unwrap();
        assert!((7_200..7_205).contains(&age), "age was {}", age);
    }

    /// 荒谬的缓存年龄既不能 panic 也不能被悄悄压成"刚拉的"。
    ///
    /// 以前这个位置会退化成"现在"（`checked_sub` 失败 → `or(Some(now))`），等于把
    /// 一份明显很旧的列表报成刚拉的。现在是饱和加法：保留原值，`u64::MAX` 加 elapsed
    /// 停在 `u64::MAX`，既不溢出也不回退。
    #[test]
    fn an_absurd_cache_age_is_kept_rather_than_reset_to_now() {
        let mut state = ChatState::new();
        state.set_models_list_age(Some(u64::MAX));
        assert_eq!(
            state.models_list_age_secs(),
            Some(u64::MAX),
            "荒谬年龄应原样保留，而不是被悄悄改成 0（显示成刚拉的）"
        );

        // 已经观察了一段时间时也必须饱和，而不是绕回。
        state.set_models_list_age(Some(u64::MAX));
        assert!(state.models_list_age_secs().is_some());
    }

    /// 缓存年龄大于机器已开机时长时必须照报 —— 这正是旧实现静默归零的场景。
    ///
    /// 回归：`Instant::now().checked_sub(Duration::from_secs(age))` 在 `Instant`
    /// 可回溯范围不足时返回 `None`（机器刚重启时必然如此），于是任何"比开机时长更
    /// 旧"的缓存都被显示成刚拉的。`model_list_age_counts_from_the_fetch_not_from_
    /// receipt` 用 7200 秒覆盖了这条路径；这里把上限再抬高一档并钉住语义。
    #[test]
    fn a_cache_older_than_uptime_is_still_reported_honestly() {
        let mut state = ChatState::new();
        let absurd_but_plausible = 30 * 24 * 60 * 60; // 30 天
        state.set_models_list_age(Some(absurd_but_plausible));
        assert!(
            state.models_list_age_secs().unwrap() >= absurd_but_plausible,
            "比进程/机器寿命长得多的缓存年龄必须照报，不能塌成 0"
        );
    }

    /// Ctrl+O / `/tui transcript` 共用的开关：开启时展开所有 thinking 块，关闭时收起，
    /// 对标 Claude Code transcript 视图「摊开全部原始输出」的语义。
    #[test]
    fn toggle_transcript_mode_drives_thinking_expansion() {
        let mut state = ChatState::new();
        state.chat_history.clear();
        state
            .chat_history
            .push(ChatEntry::new(ChatEntryType::User, "hi".to_string()));
        let mut with_reasoning = ChatEntry::assistant("answer");
        with_reasoning.reasoning_content = Some("thought".to_string());
        state.chat_history.push(with_reasoning);
        state
            .chat_history
            .push(ChatEntry::assistant("no reasoning here"));

        assert!(!state.is_transcript_mode);
        assert!(state.expanded_thinking_indices.is_empty());

        assert!(state.toggle_transcript_mode());
        // 只有带 reasoning_content 的条目会被展开
        let expanded: Vec<usize> = state.expanded_thinking_indices.iter().copied().collect();
        assert_eq!(expanded, vec![1]);

        assert!(!state.toggle_transcript_mode());
        assert!(state.expanded_thinking_indices.is_empty());
    }

    /// 造一个后台代理任务：`started_at` 用「now - offset_ms」拉开先后顺序
    fn bg_task(id: &str, is_async: bool, offset_ms: u64) -> AgentTaskInfo {
        AgentTaskInfo {
            task_id: id.to_string(),
            agent_type: "Explore".to_string(),
            description: format!("desc {}", id),
            status: crate::types::AgentTaskStatus::Running,
            tool_use_count: 0,
            tokens: 0,
            is_async,
            is_resolved: false,
            is_error: false,
            last_tool_info: None,
            name: None,
            task_description: None,
            started_at: Instant::now() - std::time::Duration::from_millis(offset_ms),
            finished_at: None,
            sub_entries: Vec::new(),
            entry_idx: 0,
        }
    }

    /// 选择器的数据源：只收后台（is_async）任务，按启动时间升序，超出上限丢最老的。
    #[test]
    fn background_agent_rows_filters_sorts_and_caps() {
        let mut state = ChatState::new();
        // 同步子代理不进选择器
        state
            .active_agent_tasks
            .insert("sync".to_string(), bg_task("sync", false, 500));
        // 故意乱序插入（HashMap 遍历顺序本身也不稳定）
        state
            .active_agent_tasks
            .insert("b".to_string(), bg_task("b", true, 200));
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a", true, 300));
        state
            .active_agent_tasks
            .insert("c".to_string(), bg_task("c", true, 100));

        let ids: Vec<&str> = state
            .background_agent_rows()
            .iter()
            .map(|i| i.task_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"], "先启动的排在前面，同步任务被过滤");

        // 超过上限时保留最近启动的 MAX_BG_AGENT_ROWS 个
        for n in 0..MAX_BG_AGENT_ROWS + 3 {
            let id = format!("x{}", n);
            // offset 越大越老 → x0 最老
            let task = bg_task(&id, true, (MAX_BG_AGENT_ROWS + 3 - n) as u64 * 1000);
            state.active_agent_tasks.insert(id, task);
        }
        let rows = state.background_agent_rows();
        assert_eq!(rows.len(), MAX_BG_AGENT_ROWS);
        // 上限内仍然保持升序
        for pair in rows.windows(2) {
            assert!(pair[0].started_at <= pair[1].started_at);
        }
        assert!(
            !rows.iter().any(|i| i.task_id == "x0"),
            "最老的一个应当被上限挤掉"
        );
    }

    /// 进入/退出详情视图只动 `viewing_agent_task_id`，不碰选择器焦点。
    #[test]
    fn teammate_view_toggles_without_moving_selection() {
        let mut state = ChatState::new();
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a", true, 10));
        state.bg_agent_selection = Some(1);

        state.enter_teammate_view("a");
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
        assert_eq!(state.bg_agent_selection, Some(1), "焦点必须留在原行");

        // 重复进入同一个 task 是幂等的
        state.enter_teammate_view("a");
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));

        state.exit_teammate_view();
        assert!(state.viewing_agent_task_id.is_none());
        assert_eq!(state.bg_agent_selection, Some(1));
    }

    /// 左键拖选松开发生在下一次渲染之后。此时流式条目的缓存必须仍然可取，
    /// 否则 `get_selected_text()` 拿不到选中文本，无法复制。
    #[test]
    fn selected_text_survives_render_after_drag() {
        let mut state = ChatState::new();
        state
            .chat_history
            .push(ChatEntry::assistant("hello streaming world"));

        let line = ratatui::text::Line::from(vec![
            ratatui::text::Span::raw("  "),
            ratatui::text::Span::raw("hello streaming world"),
        ]);
        state.rendered_cache.insert(0, (1, vec![line]));
        state.text_selection.start_selection(0, 0, 2);
        state.text_selection.update_selection(0, 0, 23);

        assert_eq!(
            state.get_selected_text().as_deref(),
            Some("hello streaming world")
        );
    }

    /// 可视列锚点在 CJK 文本上必须先换算成字符下标再切片：
    /// 鼠标列是可视列（中文一格占 2 列），直接当字符数用会只复制到一半。
    #[test]
    fn selected_text_handles_cjk_visual_columns() {
        let mut state = ChatState::new();
        state.chat_history.push(ChatEntry::assistant(""));

        let line = ratatui::text::Line::from(vec![
            ratatui::text::Span::raw("  "),
            ratatui::text::Span::raw("分析问题"),
        ]);
        state.rendered_cache.insert(0, (1, vec![line]));

        // 选中可视列 2..=9：覆盖全部 4 个中文字符
        state.text_selection.start_selection(0, 0, 2);
        state.text_selection.update_selection(0, 0, 9);
        assert_eq!(state.get_selected_text().as_deref(), Some("分析问题"));

        // 选中可视列 2..=5：只覆盖前两个字符
        state.text_selection.start_selection(0, 0, 2);
        state.text_selection.update_selection(0, 0, 5);
        assert_eq!(state.get_selected_text().as_deref(), Some("分析"));
    }

    /// 折叠切换：读写两端必须是同一份逻辑。
    ///
    /// 以前 `tool_render.rs`（读）、`mouse.rs`（单击写）、`input.rs`（Tab 写）
    /// 各写一遍，读的地方算 `default_expanded ^ toggled`，写的地方直接增删
    /// `expanded_tool_call_ids` —— 收尾动作还不一致（Tab 会标脏 virtual_list，
    /// 单击不会）。
    #[test]
    fn toggling_a_tool_block_is_observed_by_the_render_path() {
        let mut state = ChatState::new();
        let tool_call = crate::types::StarToolCall {
            id: "call-1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Read".to_string(),
                arguments: "{}".to_string(),
            },
        };
        // ToolResult + Read（非编辑类工具）→ 默认折叠
        state.chat_history.push(ChatEntry::tool_result(
            "",
            tool_call,
            crate::types::ToolResult {
                success: true,
                output: Some("ok".to_string()),
                error: None,
                data: None,
                llm_content: None,
            },
        ));
        // `ChatState::new()` 里已经塞了一条欢迎语，占 index 0。
        let idx = state.chat_history.len() - 1;

        let entry = &state.chat_history[idx];
        assert!(
            !state.is_tool_expanded(entry),
            "非编辑类工具的 ToolResult 默认折叠"
        );

        assert!(state.toggle_tool_expansion(idx));
        assert!(state.is_tool_expanded(&state.chat_history[idx]));

        assert!(state.toggle_tool_expansion(idx));
        assert!(!state.is_tool_expanded(&state.chat_history[idx]));
    }

    /// 折叠状态翻转后必须让渲染缓存失效，否则画面上还是旧的展开状态。
    #[test]
    fn toggling_invalidates_the_render_cache() {
        let mut state = ChatState::new();
        let tool_call = crate::types::StarToolCall {
            id: "call-2".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Grep".to_string(),
                arguments: "{}".to_string(),
            },
        };
        state.chat_history.push(ChatEntry::tool_call("", tool_call));
        let idx = state.chat_history.len() - 1;
        state
            .rendered_cache
            .insert(idx, (1, vec![ratatui::text::Line::from("x")]));

        assert!(state.toggle_tool_expansion(idx));
        assert!(
            !state.rendered_cache.contains_key(&idx),
            "切换后渲染缓存必须失效"
        );
    }

    /// 没有工具调用的条目不是可折叠对象。
    #[test]
    fn an_entry_without_a_tool_call_cannot_be_toggled() {
        let mut state = ChatState::new();
        state.chat_history.push(ChatEntry::assistant("hello"));
        assert!(!state.toggle_tool_expansion(0));
        assert!(!state.toggle_tool_expansion(99));
    }

    #[test]
    fn discarding_a_runtime_mutation_rebuilds_requested_intent_from_remaining_queue() {
        let mut state = ChatState::new();
        state.current_model = "active-model".to_string();
        state.current_provider_id = Some("active-provider".to_string());
        let first = state.begin_runtime_settings_update_with_persistence(
            None,
            Some(crate::core::context_policy::ContextWindowSelection::Fixed(
                1_000_000,
            )),
            None,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
        );
        let second = state.begin_runtime_settings_update_with_persistence(
            None,
            None,
            Some(crate::types::ThinkingEffort::High),
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
        );

        state.discard_runtime_settings_update(second.ui_revision);

        assert_eq!(state.pending_runtime_settings.len(), 1);
        assert_eq!(
            state.pending_runtime_settings[0].ui_revision,
            first.ui_revision
        );
        let requested = state.requested_runtime_settings.as_ref().unwrap();
        assert_eq!(
            requested.context_window,
            crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000)
        );
        assert_eq!(requested.thinking_effort, crate::types::ThinkingEffort::Off);
    }
}
