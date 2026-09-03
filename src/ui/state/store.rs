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

    /// 获取选择范围（确保start <= end）
    pub fn get_selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        if let (Some(start), Some(end)) = (self.start, self.end) {
            let (start_entry, end_entry) = if self.start_entry_idx <= self.end_entry_idx {
                (self.start_entry_idx.unwrap(), self.end_entry_idx.unwrap())
            } else {
                (self.end_entry_idx.unwrap(), self.start_entry_idx.unwrap())
            };

            let (start_pos, end_pos) = if start_entry < end_entry {
                (start, end)
            } else if start_entry == end_entry {
                if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                    (start, end)
                } else {
                    (end, start)
                }
            } else {
                (end, start)
            };

            Some((start_pos, end_pos))
        } else {
            None
        }
    }

    /// 检查指定位置是否在选择范围内
    pub fn is_position_selected(&self, entry_idx: usize, row: usize, col: usize) -> bool {
        if !self.has_selection() {
            return false;
        }

        let Some(((start_row, start_col), (end_row, end_col))) = self.get_selection_range() else {
            return false;
        };

        let (start_entry, end_entry) = if self.start_entry_idx <= self.end_entry_idx {
            (self.start_entry_idx.unwrap(), self.end_entry_idx.unwrap())
        } else {
            (self.end_entry_idx.unwrap(), self.start_entry_idx.unwrap())
        };

        // 检查是否在选择的聊天项范围内
        if entry_idx < start_entry || entry_idx > end_entry {
            return false;
        }

        // 在同一聊天项内检查行列位置
        if entry_idx == start_entry && entry_idx == end_entry {
            // 单个聊天项内的选择
            if row < start_row || row > end_row {
                return false;
            }
            if row == start_row && col < start_col {
                return false;
            }
            if row == end_row && col > end_col {
                return false;
            }
            true
        } else if entry_idx == start_entry {
            // 在起始聊天项内
            row > start_row || (row == start_row && col >= start_col)
        } else if entry_idx == end_entry {
            // 在结束聊天项内
            row < end_row || (row == end_row && col <= end_col)
        } else {
            // 在中间聊天项内
            true
        }
    }
}

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
    pub queued_messages_display: VecDeque<(String, Instant)>,
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
    /// Last time a token was received (for stall detection)
    pub last_token_time: Option<Instant>,
    /// When thinking/reasoning started (for thinking duration display)
    pub thinking_started_at: Option<Instant>,
    pub current_status_line: Option<String>,
    pub available_models: Vec<String>,
    /// 完整的模型信息列表（包含 supports_thinking 等字段）
    pub available_models_info: Vec<crate::types::ModelInfo>,
    pub model_provider_map: std::collections::HashMap<String, String>,
    pub current_model: String,
    /// 当前模型是否支持 thinking/reasoning 功能（从模型列表中获取）
    pub current_model_supports_thinking: Option<bool>,
    pub current_provider_id: Option<String>,
    pub pending_confirmation: Option<String>,
    pub pending_model_change: Option<String>,
    pub pending_model_provider: Option<String>,
    pub pending_provider_selected_model: Option<String>,
    pub pending_palette_action: Option<crate::ui::state::palette::PaletteAction>,
    pub quick_menu_origin_palette: bool,
    pub quick_menu_back: Option<QuickMenuKind>,
    pub show_help: bool, // 快捷键帮助界面
    pub auto_follow: bool,
    pub last_chat_height: u16,
    pub last_max_scroll: u16,
    pub next_message_id: u64,
    pub stream_targets: HashMap<u64, usize>,
    pub message_start_indices: HashMap<u64, usize>,
    pub active_message_id: Option<u64>,
    pub awaiting_models: bool,
    pub mcp_ready: bool,
    pub auto_continue_enabled: bool,
    pub auto_continue_remaining: u32,
    // ============ UX 改进: 审批模式状态 ============
    pub approval_mode: crate::types::ApprovalMode,
    pub thinking_effort: crate::types::ThinkingEffort,
    /// /add-dir 追加的工作目录（对标 Claude Code add-dir：扩展 @ 文件访问范围）
    pub extra_working_dirs: Vec<std::path::PathBuf>,
    /// /fast 快速模式（对标 Claude Code fast mode：切到轻量模型 + 状态栏指示）
    pub fast_mode: bool,
    /// fast 开启前的模型（关闭时恢复）
    pub fast_mode_prev_model: Option<String>,
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
    pub pending_question_selections: Vec<usize>,       // 多选问题的已选选项索引
    pub pending_other_input: String,                   // "Other" 文本输入值
    pub pending_question_other_focused: bool,          // "Other" 输入框是否聚焦
    pub last_confirmation_message_id: Option<u64>,
    // ============================================
    // ============ 文本选择和复制功能 ============
    pub text_selection: TextSelection,
    pub copy_status: Option<std::time::Instant>, // 复制状态显示时间
    // ============ Git Status ============
    pub git_branch: Option<String>,
    pub git_status: Option<String>,
    // ====================================
    // ============ Stats =================
    pub au2_compressed: bool,
    pub token_usage: Option<crate::types::StarUsage>,
    // ============ Cache Stats ============
    /// Cumulative cache read tokens (prompt cache hits)
    pub cache_read_tokens: u64,
    /// Cumulative cache creation tokens (prompt cache misses/writes)
    pub cache_creation_tokens: u64,
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
    pub plugin_detail: Option<(
        String,
        crate::core::plugins::marketplace::MarketplacePlugin,
    )>,
    /// 批量安装进度聚合（对标 Claude Code 批量安装确认页的逐项状态）
    pub plugin_batch_total: usize,
    pub plugin_batch_done: usize,
    pub plugin_loading: bool,
    /// 有插件市场后台操作（经 AgentRequest::PluginOp）在执行中
    pub plugin_op_pending: bool,
    pub plugin_message: Option<String>,
    pub plugin_confirm: Option<crate::ui::state::modal::PluginConfirm>,
    // ============ Status Modal State ============
    pub show_status_modal: bool,
    pub show_provider_menu: bool,
    pub selected_provider_index: usize,
    pub show_session_menu: bool,
    pub selected_session_index: usize,
    pub available_sessions: Vec<crate::utils::session_manager::SessionSummary>,
    // ============ Input Modal State ============
    pub show_input_modal: bool,
    pub input_modal_title: String,
    pub input_modal_prompt: String,
    pub input_modal_value: String,
    pub modal_textarea: TextArea<'static>,
    // Context for the input modal to determine action on completion
    pub input_context: Option<crate::ui::state::palette::InputContext>,
    // Providers that are ready to use or have meaningful saved setup
    pub configured_providers: HashSet<String>,
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
    // ============ Context Engine (integrated) ============
    pub context_engine: Option<crate::core::context::integration::ContextEngine>,
    // ============ Highlight/Dialog States ============
    pub show_global_search: bool,
    pub global_search_state: crate::ui::components::highlight::search::GlobalSearchState,
    pub show_quick_open: bool,
    pub quick_open_state: crate::ui::components::highlight::quick_open::QuickOpenState,
    pub show_history_search: bool,
    pub history_search_state: crate::ui::components::highlight::history::HistorySearchState,
    pub show_theme_picker: bool,
    pub selected_theme_index: usize,
    /// 进入主题选择器前的主题名（Esc 取消时恢复）
    pub theme_picker_prev: Option<String>,
    pub show_usage_stats: bool,
    pub usage_stats: crate::ui::components::highlight::stats::UsageStats,
    pub show_export_dialog: bool,
    pub export_state: crate::ui::components::highlight::export::ExportState,
    pub show_compression_status: bool,
    pub compression_state: crate::ui::components::highlight::compression::CompressionState,
    // ============ New Feature States ============
    pub show_context_viz: bool,
    pub context_breakdown: crate::ui::components::highlight::context_viz::TokenBreakdown,
    pub show_error_overlay: bool,
    pub error_overlay_state: crate::ui::components::error_overlay::ErrorOverlayState,
    pub show_log_selector: bool,
    pub log_selector_state: crate::ui::components::log_selector::LogSelectorState,
    pub diff_preview_scroll: usize,
    pub pending_model_confirmation: bool,
    pub last_code_block_content: Option<String>,
    pub vim_enabled: bool,
    pub vim_state: crate::ui::vim::VimState,
    pub settings_selected_index: usize,
    pub show_scroll_to_bottom: bool,
    pub show_clear_confirmation: bool,
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
    /// 全局 transcript 模式开关（Ctrl+O 切换）
    pub is_transcript_mode: bool,
    // ========================================
}

impl ChatState {
    pub fn new() -> Self {
        // 不再同步阻塞获取 git status（大仓库中 git status 可能需要数秒），
        // 由 run_ui_loop 中的 spawn_git_status_loop 后台异步更新
        Self::with_git_info(None)
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
                let _ = std::fs::write(path, &text);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Restore draft from file
    pub fn restore_draft(&mut self) {
        if let Some(ref path) = self.draft_path {
            if let Ok(text) = std::fs::read_to_string(path) {
                if !text.trim().is_empty() {
                    self.textarea.insert_str(&text);
                    let _ = std::fs::remove_file(path);
                }
            }
        }
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
            queued_messages_display: VecDeque::new(),
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
            last_token_time: None,
            thinking_started_at: None,
            current_status_line: None,
            available_models: Vec::new(),
            available_models_info: Vec::new(),
            model_provider_map: std::collections::HashMap::new(),
            current_model: String::new(),
            current_model_supports_thinking: None,
            current_provider_id: None,
            pending_confirmation: None,
            pending_model_change: None,
            pending_model_provider: None,
            pending_provider_selected_model: None,
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
            mcp_ready: false,
            auto_continue_enabled: false,
            auto_continue_remaining: 0,
            approval_mode: crate::types::ApprovalMode::Default,
            thinking_effort: crate::types::ThinkingEffort::default(),
            extra_working_dirs: Vec::new(),
            fast_mode: false,
            fast_mode_prev_model: None,
            poor_mode: false,
            advisor_mode: false,
            context_window_override: None,
            pending_tool_calls: None,
            pending_message_id: None,
            pending_confirmation_entry_idx: None,
            pending_confirmation_choice: 0,
            is_awaiting_confirmation: false,
            pending_tool_call_id: None,
            pending_question_selections: Vec::new(),
            pending_other_input: String::new(),
            pending_question_other_focused: false,
            last_confirmation_message_id: None,
            text_selection: TextSelection::new(),
            copy_status: None,
            git_branch,
            git_status,
            au2_compressed: false,
            token_usage: None,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
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
            show_status_modal: false,
            show_provider_menu: false,
            selected_provider_index: 0,
            show_session_menu: false,
            selected_session_index: 0,
            available_sessions: Vec::new(),
            show_input_modal: false,
            input_modal_title: String::new(),
            input_modal_prompt: String::new(),
            input_modal_value: String::new(),
            modal_textarea: TextArea::default(),
            input_context: None,
            configured_providers: HashSet::new(),
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
            animation_tick: 0,
            scroll_velocity: 0.0,
            last_scroll_time: None,
            current_tool_name: None,
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
            context_engine: None,
            // ============ Highlight/Dialog States ============
            show_global_search: false,
            global_search_state: crate::ui::components::highlight::search::GlobalSearchState::new(),
            show_quick_open: false,
            quick_open_state: crate::ui::components::highlight::quick_open::QuickOpenState::new(),
            show_history_search: false,
            history_search_state:
                crate::ui::components::highlight::history::HistorySearchState::new(),
            show_theme_picker: false,
            selected_theme_index: 0,
            theme_picker_prev: None,
            show_usage_stats: false,
            usage_stats: crate::ui::components::highlight::stats::UsageStats::default(),
            show_export_dialog: false,
            export_state: crate::ui::components::highlight::export::ExportState::new(),
            show_compression_status: false,
            compression_state:
                crate::ui::components::highlight::compression::CompressionState::default(),
            // ============ New Feature States ============
            show_context_viz: false,
            context_breakdown:
                crate::ui::components::highlight::context_viz::TokenBreakdown::default(),
            show_error_overlay: false,
            error_overlay_state: crate::ui::components::error_overlay::ErrorOverlayState::default(),
            show_log_selector: false,
            log_selector_state: crate::ui::components::log_selector::LogSelectorState::default(),
            diff_preview_scroll: 0,
            pending_model_confirmation: false,
            last_code_block_content: None,
            vim_enabled: false,
            vim_state: crate::ui::vim::VimState::new(),
            settings_selected_index: 0,
            show_scroll_to_bottom: false,
            show_clear_confirmation: false,
            colorblind_mode: false,
            toast_queue: VecDeque::new(),
            send_animation_since: None,
            draft_path: {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                let star_dir = std::path::PathBuf::from(home).join(".star");
                let _ = std::fs::create_dir_all(&star_dir);
                Some(star_dir.join("draft.txt"))
            },
            request_clear_screen: false,
            show_paste_confirmation: false,
            pending_paste: None,
            // ============ Agent 任务追踪 ============
            active_agent_tasks: HashMap::new(),
            agent_group_id: None,
            viewing_agent_task_id: None,
            is_transcript_mode: false,
            // ========================================
            pending_scroll_direction: None,
            pending_scroll_time: None,
        }
    }

    pub fn clear_cache(&mut self) {
        self.rendered_cache.clear();
        self.last_rendered_stream_key.clear();
        self.streaming_height_floor.clear();
        self.virtual_list.mark_all_dirty();
    }

    /// 获取选中的文本内容（从渲染后的行中提取）
    pub fn get_selected_text(&self) -> Option<String> {
        if !self.text_selection.has_selection() {
            return None;
        }

        let (start_entry, end_entry) = match (
            self.text_selection.start_entry_idx,
            self.text_selection.end_entry_idx,
        ) {
            (Some(s), Some(e)) => {
                if s <= e {
                    (s, e)
                } else {
                    (e, s)
                }
            }
            _ => return None,
        };

        let ((start_row, start_col), (end_row, end_col)) =
            self.text_selection.get_selection_range()?;

        let mut result = String::new();

        for entry_idx in start_entry..=end_entry {
            // 获取渲染后的行
            let rendered_lines = if let Some((_, lines)) = self.rendered_cache.get(&entry_idx) {
                lines
            } else {
                continue;
            };

            let line_start = if entry_idx == start_entry {
                start_row
            } else {
                0
            };
            let line_end = if entry_idx == end_entry {
                end_row.min(rendered_lines.len().saturating_sub(1))
            } else {
                rendered_lines.len().saturating_sub(1)
            };

            for (i, line) in rendered_lines
                .iter()
                .enumerate()
                .take(line_end + 1)
                .skip(line_start)
            {
                // 将 Line 转换为纯文本
                let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();

                let col_start = if i == start_row && entry_idx == start_entry {
                    start_col.min(line_text.len())
                } else {
                    0
                };
                let col_end = if i == end_row && entry_idx == end_entry {
                    end_col.min(line_text.len())
                } else {
                    line_text.len()
                };

                // Use char-based slicing to avoid byte-index panic with
                // multi-byte characters (e.g., CJK at 3 bytes/char).
                let selected: String = line_text
                    .chars()
                    .skip(col_start)
                    .take(col_end.saturating_sub(col_start))
                    .collect();
                if !selected.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&selected);
                }
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
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
}
