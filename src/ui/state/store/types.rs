//! ChatState 周边的小类型：粘贴、提示条、待确认队列、会话内搜索、选区等。
//!
//! 它们散在 store.rs 里时，要读 ChatState 的字段定义得先跳过一堆不相干的东西；
//! 拆出来后 state/store/mod.rs 只剩状态本体。

use std::collections::HashMap;
use std::time::Instant;

use crate::types::ChatEntry;

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
