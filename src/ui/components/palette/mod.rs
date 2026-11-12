//! 命令面板（Ctrl+P）。
//!
//! 本文件只管**调度与渲染**：按 PaletteMode 挑条目、过滤、画框。
//! 条目本身在 [items]，条目描述在 [describe]。
//!
//! # 视觉方向：它是一个终端输入区，不是一个网页弹窗
//!
//! 面板此前照着"模态弹窗"的样式做：圆角双层边框、emoji 图标、四种角色各配
//! 一种颜色、每一行都加粗。五个角色五种颜色意味着**没有重点**；双层边框让
//! 搜索框和外框看起来像两个控件叠在一起；等粗的每一行则让层级消失。
//!
//! 现在按**终端自己的语法**重做：
//!
//! - **层级用缩进**表达，不��颜色。分组标题顶格，分组内的条目缩进 2 列 ——
//!   终端里没有别的东西用缩进表达层级，这一处独占它（见 `INDENT_GROUPED`）。
//! - **只有一处有颜色**：当前选中行（主题的 `selection_bg`）。其余全是同一支
//!   灰阶，靠字重和颜色深浅分层。
//! - **一个边框**。外框即是搜索框的边框，不再套第二层。
//! - **字重只在一处用**：选中项（`BOLD`）。普通项不加粗 —— 12 行全粗等于
//!   12 行都没粗。
//! - **没有 emoji**。`⟳` `＋` `⏳` `●` 这些符号被去掉：其中 `⏳` 是宽字符
//!   （占两列），`＋` `●` `←` 是歧义宽度，在不同终端里对齐各不相同。

pub mod describe;
pub mod items;

use crate::core::config::providers::{get_provider_by_id, ALL_PROVIDERS};
use crate::core::i18n;
use crate::ui::state::{ChatState, PaletteAction, PaletteItem, PaletteMode};
use crate::ui::utils::status::{
    approval_mode_label, current_model_id, current_provider_display, current_provider_id,
    status_summary,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::collections::HashSet;

use describe::*;
pub use items::*;

/// 命令面板（Ctrl+P）占屏比例。宽度刻意压得比常见弹窗窄。
const PALETTE_WIDTH_PERCENT: u16 = 58;
const PALETTE_HEIGHT_PERCENT: u16 = 60;

/// 分组内条目的缩进列数。
///
/// 缩进是这个面板唯一的层级手段：分组标题顶格，它下面的条目缩进两列。
/// 散项（刷新、加 provider、返回）不缩进 —— 它们不属于任何分组。
const INDENT_GROUPED: usize = 2;

/// 描述与标题之间的间隔列数。
///
/// 两列：一个空格会让描述看起来像标题的一部分（"Status Not set"），四列以上
/// 在 58% 宽的面板里会把描述挤到换行。
const DESCRIPTION_GAP: usize = 2;

pub fn palette_item_matches_query(item: &PaletteItem, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }

    if item.label.to_lowercase().contains(query_lower) {
        return true;
    }

    if item.description.to_lowercase().contains(query_lower) {
        return true;
    }

    if let Some(category) = &item.category {
        if category.to_lowercase().contains(query_lower) {
            return true;
        }
    }

    false
}

pub fn get_items(mode: &PaletteMode, state: &ChatState) -> Vec<PaletteItem> {
    match mode {
        PaletteMode::Main => get_main_palette_items_for_state(Some(state)),
        // 过滤词由 `state.palette_filter` 提供（`get_items` 没有 query 参数，
        // 所以这里只出全部条目，真正的过滤统一在 `get_search_items` /
        // `palette_item_matches_query` 里做，和其它模式完全一致）。
        PaletteMode::Command => crate::commands::system::command_palette_items("/"),
        PaletteMode::Session => get_session_palette_items(),
        PaletteMode::System => get_system_palette_items(),
        PaletteMode::Agent => get_agent_palette_items(),
        PaletteMode::Provider => {
            get_provider_palette_items(&state.configured_providers, &state.keyless_providers)
        }
        PaletteMode::ProviderPopular => get_provider_popular_items(&state.configured_providers),
        PaletteMode::ProviderOther => get_provider_other_items(&state.configured_providers),
        PaletteMode::ProviderLocal => get_provider_local_items(&state.configured_providers),
        PaletteMode::ProviderOptions(pid) => {
            get_provider_options_items(pid, &state.configured_providers)
        }
        PaletteMode::ProviderDelete(provider_id) => get_provider_delete_items(provider_id),
        PaletteMode::Memory => get_memory_palette_items(),
        PaletteMode::AgentMode => get_agent_mode_palette_items(),
        PaletteMode::ThinkingEffort => {
            get_thinking_effort_palette_items(&state.thinking_effort, &state.current_model)
        }
        PaletteMode::ContextWindow => get_context_window_palette_items(
            state
                .requested_runtime_settings
                .as_ref()
                .map(|settings| settings.context_window)
                .or_else(|| {
                    state
                        .context_window_override
                        .map(crate::core::context_policy::ContextWindowSelection::Fixed)
                })
                .unwrap_or_default(),
        ),
        PaletteMode::Theme => get_theme_palette_items(state),
        PaletteMode::Model => get_model_palette_items(
            &state.available_models,
            &state.current_model,
            // 走 `models_loading()` 而不是裸标志：worker 那边的回复一旦丢���，
            // 裸标志会永远停在 true，转圈停不下来。
            state.models_loading(),
            &state.model_provider_map,
            state.models_list_age_secs(),
        ),
        PaletteMode::Project => get_project_palette_items(),
        PaletteMode::Integrations => get_integrations_palette_items(),
        PaletteMode::McpManage => get_mcp_manage_palette_items(),
        PaletteMode::Help => get_help_palette_items(),
        PaletteMode::Language => get_language_palette_items(),
        PaletteMode::OutputStyle => get_output_style_palette_items(),
        PaletteMode::Git => get_git_palette_items(),
        _ => vec![],
    }
}

pub fn get_search_items(mode: &PaletteMode, state: &ChatState, query: &str) -> Vec<PaletteItem> {
    if query.trim().is_empty() {
        return get_items(mode, state);
    }

    match mode {
        PaletteMode::Main => get_global_search_palette_items(state),
        PaletteMode::Provider => get_items(mode, state),
        _ => get_items(mode, state),
    }
}

/// 面板这一页是否按分类分组渲染。
///
/// # 搜索时一律平铺
///
/// 全局搜索的结果来自十几个来源（模型、provider、命令、会话……），按来源分组
/// 会把一份**排名列表**切成一堆碎片 —— 搜索要的是排序，不是归类。所以只要
/// 过滤词非空，这一页就平铺：没有分组标题，也没有缩进。
pub fn palette_is_grouped(mode: &PaletteMode, query: &str) -> bool {
    let _ = mode;
    query.trim().is_empty()
}

/// 一个分类标题是不是**在重复页面标题**。
///
/// 主题页的分类叫 "Theme"、页面标题也是 "Theme"；语言页、上下文页同理。
/// 同一个词在一屏里出现两次，用户得读两遍才知道它不是两件事。
///
/// 大小写不敏感 —— 分类名是 i18n 的结果，页面标题是硬编码的英文，
/// "Context Window" 与 "Context window" 指的是同一件事。
fn palette_category_repeats_title(mode: &PaletteMode, category: &str) -> bool {
    category.eq_ignore_ascii_case(palette_scope(mode, ""))
}

/// 这个分类要不要画标题（以及要不要给它的条目缩进）。
///
/// 缩进的意义是"这一行属于上面那个组"。没有标题就没有组，也就没有缩进 ——
/// 两件事必须一起决定，否则会出现"缩进了却找不到组标题"的悬空缩进。
///
/// 唯一的例外是"分类名与页面标题相同"：页面标题已经在说"这是主题页"，
/// 再画一个 "Theme" 标题只是把同一个词念两遍。
pub fn palette_category_has_header(mode: &PaletteMode, query: &str, category: &str) -> bool {
    palette_is_grouped(mode, query) && !palette_category_repeats_title(mode, category)
}

/// 面板里**真正画出来的顺序**，也是 `selected_palette_index` 唯一该解释的顺序。
///
/// # 为什么必须有这一个函数
///
/// 渲染和按键曾经各自按**不同**的顺序解释 `selected_palette_index`：
///
/// - `render_palette` 先画带 `category` 的条目、再画 `category: None` 的散项，
///   于是 `selectable_indices` 是**渲染序**；
/// - `handle_palette` 的 Enter 直接 `filtered[index]`，是**过滤序**。
///
/// 模型面板正好同时有两种：`⟳ Fetch model list from API` 是 `category: None`
/// 且被 `push_model_refresh_option` 放在**第一位**，其余模型按 provider 分组。
///
/// 于是渲染序 = `[模型…, 刷新]`，过滤序 = `[刷新, 模型…]` —— **同一个下标指向两个
/// 不同的条目**。用户把高亮移到某个模型上按 Enter，触发的是刷新模型列表；列表
/// 重新拉一遍、模型没换，表现就是"这个模型怎么都切不上去"。
///
/// 返回值按渲染序排列，且**只含可选项**（标题行不算），所以
/// `display_items[i]` 就是第 `i` 个可选项 —— 画高亮和响应 Enter 必须都用它。
///
/// # 置顶 / 置底
///
/// 面板顺序不是简单的"分组在前、散项在后"：刷新模型列表永远在最前，
/// 「Add New Provider」紧跟其后，返回项固定压到最后。这三条都是**跨分组的
/// 全局动作**，混在 provider/模型分组里只会让人先翻过一屏才找到它们。
pub fn palette_display_items(
    mode: &PaletteMode,
    state: &ChatState,
    query: &str,
) -> Vec<PaletteItem> {
    let query_lower = query.trim().to_lowercase();
    let filtered: Vec<PaletteItem> = get_search_items(mode, state, query)
        .into_iter()
        .filter(|item| palette_item_matches_query(item, &query_lower))
        .collect();

    // 先把置顶/置底项摘出来，剩下的才参与分组。用稳定排序保证同组内的原始
    // 顺序不变（自定义 provider 的 order、模型列表顺序都得原样保留）。
    let mut pinned_top: Vec<PaletteItem> = Vec::new();
    let mut pinned_bottom: Vec<PaletteItem> = Vec::new();
    let mut rest: Vec<PaletteItem> = Vec::new();
    for item in filtered {
        match palette_pin_rank(&item, mode) {
            0 | 1 => pinned_top.push(item),
            3 => pinned_bottom.push(item),
            _ => rest.push(item),
        }
    }
    pinned_top.sort_by_key(|item| palette_pin_rank(item, mode));

    let mut ordered = pinned_top;
    // 分不分组：**只有**渲染器也会画标题时才重排。
    //
    // 两边各算一套就是当初那个 bug 的温床 —— 渲染按 A 排序、Enter 按 B 解释
    // 同一个下标。这里用的 `palette_is_grouped` 与渲染器共用。
    //
    // 注意"某个分类的标题被抑制"（分类名与页面标题相同）不影响**重排**：
    // 重排只是让同分类的条目挨在一起，标题画不画是渲染时逐个分类决定的。
    // 于是主题页虽然不画 "Theme" 标题，条目仍然是连着的一整块。
    if palette_is_grouped(mode, query) {
        let mut groups: Vec<(String, Vec<PaletteItem>)> = Vec::new();
        let mut loose: Vec<PaletteItem> = Vec::new();
        for item in rest {
            match item.category.clone() {
                // `None` 不兜底成 "Other"：否则单条返回项会被提升成一个只含自己的
                // "Other" 分组标题。
                None => loose.push(item),
                Some(category) => match groups.iter_mut().find(|(name, _)| *name == category) {
                    Some((_, items)) => items.push(item),
                    None => groups.push((category, vec![item])),
                },
            }
        }

        for (_, items) in groups {
            ordered.extend(items);
        }
        ordered.extend(loose);
    } else {
        ordered.extend(rest);
    }

    ordered.extend(pinned_bottom);
    ordered
}

/// 返回模型面板打开时应聚焦的条目：当前模型优先，其次第一个可选项。
///
/// 刷新入口是置顶的全局动作，但不应该抢默认焦点；用户打开 Ctrl+P 后通常
/// 先确认/切换当前模型，而不是立刻重新请求所有 provider。
pub fn model_selection_index(state: &ChatState) -> usize {
    let items = palette_display_items(&PaletteMode::Model, state, "");
    items
        .iter()
        .position(|item| {
            matches!(
                item.action,
                PaletteAction::SetModel { ref model, .. } if model == &state.current_model
            )
        })
        .unwrap_or_else(|| {
            items
                .iter()
                .position(|item| !matches!(item.action, PaletteAction::RefreshModels))
                .unwrap_or(0)
        })
}

/// 全局动作在列表里的位置：0/1 置顶，3 置底，2 跟随原有分组。
///
/// 只在模型/Providers 两页置顶刷新项：全局搜索里它只是一个普通结果，抢到
/// 第一条反而会盖住用户真正在找的命令。
fn palette_pin_rank(item: &PaletteItem, mode: &PaletteMode) -> u8 {
    match (&item.action, mode) {
        (PaletteAction::RefreshModels, PaletteMode::Model | PaletteMode::Provider) => 0,
        (PaletteAction::OpenProviderForm, PaletteMode::Provider) => 1,
        (PaletteAction::Back, _) => 3,
        _ => 2,
    }
}

/// 面板标题。
///
/// 标题只说**这是哪一页**，不重复"按 Ctrl+P 打开"—— 面板已经开着了，
/// 把入口键写进标题既占地方，又承诺了一个不总成立的按键（Esc 只在有
/// 历史时才回退，无历史时是关闭面板）。
pub fn palette_title(mode: &PaletteMode, query: &str) -> String {
    format!(" {} ", palette_scope(mode, query))
}

/// 这一页的**名字**（标题栏里那一行）。
///
/// 单独抽出来，是因为"某个分类标题是不是在重复页面标题"这条判断需要它 ——
/// 主题页的分类叫 "Theme"、页面标题也叫 "Theme"，同一个词在一屏里出现两次
/// 就是噪音。
fn palette_scope(mode: &PaletteMode, query: &str) -> &'static str {
    if !query.trim().is_empty() && matches!(mode, PaletteMode::Main) {
        return "Search all";
    }
    match mode {
        PaletteMode::Main => "Actions",
        PaletteMode::Command => "Commands",
        PaletteMode::Provider => "Providers",
        PaletteMode::Session => "Session",
        PaletteMode::System => "Settings",
        PaletteMode::Agent => "Agent",
        PaletteMode::Memory => "Memory",
        PaletteMode::Git => "Git",
        PaletteMode::Mcp => "MCP",
        PaletteMode::Project => "Project",
        PaletteMode::Integrations => "Integrations",
        PaletteMode::McpManage => "MCP",
        PaletteMode::Help => "Help",
        PaletteMode::Model => "Models",
        PaletteMode::AgentMode => "Approval mode",
        PaletteMode::ThinkingEffort => "Thinking",
        PaletteMode::ContextWindow => "Context window",
        PaletteMode::Theme => "Theme",
        PaletteMode::ProviderPopular => "Popular providers",
        PaletteMode::ProviderOther => "Regional providers",
        PaletteMode::ProviderLocal => "Local providers",
        PaletteMode::ProviderOptions(_) => "Provider setup",
        PaletteMode::ProviderDelete(_) => "Remove provider",
        PaletteMode::Language => "Language",
        PaletteMode::OutputStyle => "Output style",
    }
}

/// 搜索框的占位符 —— 说的是**这一页能搜什么**，不是"搜索"两个字。
///
/// 此前 Main 页写的是 "Search actions, models, providers"，但 Main 页空输入时
/// 显示的是全部动作（不是搜索结果），标题写着 "Actions"、占位符写着 "Search…"，
/// 两句话描述的是两种不同状态，读者得先猜面板现在是哪种。
pub fn palette_placeholder(mode: &PaletteMode) -> &'static str {
    match mode {
        PaletteMode::Main => "Filter actions",
        PaletteMode::Command => "Filter commands",
        PaletteMode::Provider => "Filter providers",
        PaletteMode::Model => "Filter models",
        PaletteMode::Language => "Filter languages",
        PaletteMode::System => "Filter settings",
        PaletteMode::Memory => "Filter memory actions",
        PaletteMode::Session => "Filter session actions",
        PaletteMode::Theme => "Filter themes",
        PaletteMode::AgentMode => "Filter modes",
        PaletteMode::ThinkingEffort => "Filter levels",
        PaletteMode::ContextWindow => "Filter sizes",
        PaletteMode::OutputStyle => "Filter styles",
        PaletteMode::Help => "Filter commands",
        _ => "Filter",
    }
}

fn get_global_search_palette_items(state: &ChatState) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    push_unique_actionable_items(
        &mut items,
        &mut seen,
        get_main_palette_items_for_state(Some(state))
            .into_iter()
            .filter(|item| !matches!(item.action, PaletteAction::Navigate(_)))
            .collect(),
    );
    push_unique_actionable_items(&mut items, &mut seen, get_session_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_project_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_integrations_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_memory_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_system_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_help_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_agent_mode_palette_items());
    push_unique_actionable_items(
        &mut items,
        &mut seen,
        get_all_provider_search_items(&state.configured_providers),
    );

    if !state.available_models.is_empty() {
        push_unique_actionable_items(
            &mut items,
            &mut seen,
            get_model_palette_items(
                &state.available_models,
                &state.current_model,
                state.awaiting_models,
                &state.model_provider_map,
                state.models_list_age_secs(),
            ),
        );
    }

    items
}

fn get_all_provider_search_items(configured: &[String]) -> Vec<PaletteItem> {
    let builtin_ids: HashSet<&str> = ALL_PROVIDERS.iter().map(|p| p.id).collect();
    // 变量名刻意不叫 `items`：本文件里 `items` 是 `pub mod items`，同名绑定虽然
    // 分属值/模块两个命名空间、不影响 `items::provider_row_is_hidden` 解析，但读
    // 代码时很容易看错。
    let mut provider_items: Vec<PaletteItem> = ALL_PROVIDERS
        .iter()
        .filter(|provider| !items::provider_row_is_hidden(provider.id))
        .map(|provider| build_provider_item_flat(provider, configured))
        .collect();

    // Add custom providers
    for id in configured
        .iter()
        .filter(|id| !builtin_ids.contains(id.as_str()))
    {
        provider_items.push(PaletteItem {
            id: format!("provider_{}", id),
            label: id.clone(),
            description: "Custom provider".to_string(),
            category: Some("Custom".to_string()),
            action: PaletteAction::Navigate(PaletteMode::ProviderOptions(id.clone())),
        });
    }

    provider_items
}

fn is_actionable_item(item: &PaletteItem) -> bool {
    !matches!(item.action, PaletteAction::Back)
}

fn push_unique_actionable_items(
    target: &mut Vec<PaletteItem>,
    seen: &mut HashSet<String>,
    items: Vec<PaletteItem>,
) {
    for item in items.into_iter().filter(is_actionable_item) {
        let key = format!(
            "{}|{}|{}",
            item.label.to_lowercase(),
            item.description.to_lowercase(),
            palette_action_key(&item.action)
        );
        if seen.insert(key) {
            target.push(item);
        }
    }
}

fn palette_action_key(action: &PaletteAction) -> String {
    match action {
        PaletteAction::Navigate(mode) => format!("navigate:{}", palette_mode_key(mode)),
        PaletteAction::ShowStatus => "show_status".to_string(),
        PaletteAction::ShowModelMenu => "show_model_menu".to_string(),
        PaletteAction::ShowProviderMenu => "show_provider_menu".to_string(),
        PaletteAction::ShowSessionMenu => "show_session_menu".to_string(),
        PaletteAction::ExecuteCommand(cmd) => format!("execute:{}", cmd),
        PaletteAction::TypeCommand(cmd) => format!("type:{}", cmd),
        PaletteAction::SelectProvider(provider_id) => format!("provider:{}", provider_id),
        PaletteAction::ToggleFeature(feature) => format!("toggle:{}", feature),
        PaletteAction::Back => "back".to_string(),
        // 去重键必须含 provider：两个 provider 可以合法地提供同名模型，只用
        // 模型名会把其中一条去重掉，另一个就永远选不到（`get_model_palette_items`
        // 里同理，`id` 也带上了 owner）。
        PaletteAction::SetModel { model, provider_id } => match provider_id {
            Some(owner) => format!("model:{}@{}", model, owner),
            None => format!("model:{}", model),
        },
        PaletteAction::SetAgentMode(mode) => format!("agent_mode:{}", mode),
        PaletteAction::SetThinkingEffort(effort) => {
            format!("thinking_effort:{}", effort.as_str())
        }
        PaletteAction::SetContextWindow(selection) => {
            format!("context_window:{}", selection.display())
        }
        PaletteAction::InputContextWindow => "input_context_window".to_string(),
        PaletteAction::SetTheme(theme) => format!("theme:{}", theme),
        PaletteAction::SetOutputStyle(style) => format!("output_style:{}", style),
        PaletteAction::ShowLogSelector => "show_log_selector".to_string(),
        PaletteAction::ShowContextViz => "show_context_viz".to_string(),
        PaletteAction::ToggleVimMode => "toggle_vim_mode".to_string(),
        PaletteAction::ToggleMouseCapture => "toggle_mouse_capture".to_string(),
        PaletteAction::ToggleUiVerbose => "toggle_ui_verbose".to_string(),
        PaletteAction::CreatePr => "create_pr".to_string(),
        PaletteAction::ToggleColorblindMode => "toggle_colorblind_mode".to_string(),
        PaletteAction::EditProvider(provider_id) => format!("edit_provider:{}", provider_id),
        PaletteAction::OpenProviderForm => "open_provider_form".to_string(),
        PaletteAction::DeleteProvider(provider_id) => format!("delete_provider:{}", provider_id),
        PaletteAction::ClearProviderKey(provider_id) => {
            format!("clear_provider_key:{}", provider_id)
        }
        PaletteAction::RefreshModels => "refresh_models".to_string(),
        PaletteAction::OpenMcpModal => "open_mcp_modal".to_string(),
        PaletteAction::OpenMarketModal => "open_market_modal".to_string(),
    }
}

fn palette_mode_key(mode: &PaletteMode) -> String {
    match mode {
        PaletteMode::Main => "main".to_string(),
        PaletteMode::Command => "command".to_string(),
        PaletteMode::Provider => "provider".to_string(),
        PaletteMode::Session => "session".to_string(),
        PaletteMode::System => "system".to_string(),
        PaletteMode::Agent => "agent".to_string(),
        PaletteMode::Memory => "memory".to_string(),
        PaletteMode::Git => "git".to_string(),
        PaletteMode::Mcp => "mcp".to_string(),
        PaletteMode::Project => "project".to_string(),
        PaletteMode::Integrations => "integrations".to_string(),
        PaletteMode::McpManage => "mcp_manage".to_string(),
        PaletteMode::Help => "help".to_string(),
        PaletteMode::Model => "model".to_string(),
        PaletteMode::AgentMode => "agent_mode".to_string(),
        PaletteMode::ThinkingEffort => "thinking_effort".to_string(),
        PaletteMode::ContextWindow => "context_window".to_string(),
        PaletteMode::Theme => "theme".to_string(),
        PaletteMode::ProviderPopular => "provider_popular".to_string(),
        PaletteMode::ProviderOther => "provider_other".to_string(),
        PaletteMode::ProviderLocal => "provider_local".to_string(),
        PaletteMode::ProviderOptions(provider_id) => format!("provider_options:{}", provider_id),
        PaletteMode::Language => "language".to_string(),
        PaletteMode::OutputStyle => "output_style".to_string(),
        PaletteMode::ProviderDelete(provider_id) => format!("provider_delete:{}", provider_id),
    }
}

pub fn render_palette(f: &mut Frame, area: Rect, state: &mut ChatState) {
    if !state.is_palette_open() {
        return;
    }

    // 与 `/` 浮层（`command_suggestions::render_command_hints_overlay`）取同一套
    // 主题色。之前这里写死 Blue/Gray/Yellow/DarkGray，而 `/` 那边走 theme，
    // 于是同一个应用里两套列表长得不一样，换主题时命令浮层会跟着变、面板不会。
    //
    // `current()` 是借用 `state` 的，而本函数后面还要写 `state.selected_palette_index`，
    // 所以颜色在这里逐个拷出来（`Color` 是 Copy），别把借用带过可变访问。
    let theme = state.theme_manager.current();
    let border_color = theme.border;
    let inactive_color = theme.inactive;
    let foreground_color = theme.foreground;
    let subtle_color = theme.subtle;
    let selection_bg = theme.selection_bg;

    // 70% 太宽：行内 label + description 拉得过开，视觉上压住整个会话区。
    // 收到 58% 后两侧留白足够，最长的描述行（删除 provider 的那句）在 120 列
    // 终端上正好放得下。
    let area = centered_rect(PALETTE_WIDTH_PERCENT, PALETTE_HEIGHT_PERCENT, area);
    f.render_widget(Clear, area);

    // **一个边框**。此前外框套着内框（内框是"Search"），两条同色 hairline 相距
    // 一列，看起来像两个控件叠在一起 —— 而实际只有一个输入框。
    // 现在外框同时承担两件事：它的上边框就是搜索行的位置。
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(palette_title(&state.palette_mode, &state.palette_filter))
        .title_style(Style::default().fg(inactive_color));
    f.render_widget(block, area);

    // 外框之内：标题行（搜索）1 行 + 列表。列表至少留 1 行。
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
        .split(area);

    let query = state.palette_filter.trim();
    let is_empty = query.is_empty();

    // 搜索行：空输入时显示这一页能搜什么，有输入时显示输入本身。
    // 末尾画一个**真实的光标**（反显块）—— 此前搜索框是一个纯 `Paragraph`，
    // 输完字看不见插入点在哪，得靠数字符猜。
    let input_text = if is_empty {
        palette_placeholder(&state.palette_mode).to_string()
    } else {
        state.palette_filter.clone()
    };
    let input_style = if is_empty {
        Style::default().fg(subtle_color)
    } else {
        Style::default().fg(foreground_color)
    };

    let mut input_spans = vec![Span::styled(input_text, input_style)];
    // 光标画在文字末尾。空输入时光标落在占位符之后（占位符是提示，不是内容，
    // 但光标得有地方待着，否则这一行看起来不像能输入）。
    input_spans.push(Span::styled(
        " ",
        Style::default()
            .fg(foreground_color)
            .add_modifier(Modifier::REVERSED),
    ));
    f.render_widget(Paragraph::new(Line::from(input_spans)), chunks[0]);

    // 顺序只在 `palette_display_items` 里定义一次（见它的文档注释）。这里和
    // `handle_palette` 用的是同一个 vec，所以高亮画在第 `selected_palette_index`
    // 项上、按 Enter 触发的也是第 `selected_palette_index` 项。
    let display_items = palette_display_items(&state.palette_mode, state, query);

    let mut list_items: Vec<ListItem> = Vec::new();
    // `display_items[i]` 对应 `list_items` 里的哪一行（标题行不占位）。
    let mut row_of_item: Vec<usize> = Vec::with_capacity(display_items.len());
    // 反向映射：列表第几行对应第几个条目。鼠标点选用 —— 分组标题行不占位，
    // 所以「第 3 行」不等于「第 3 个条目」。
    let mut item_of_row: Vec<usize> = Vec::with_capacity(display_items.len());
    // 已经发过标题的分组名。置顶/置底的散项可能夹在分组之间，所以不能再用
    // "见过散项就再也不画标题"的老逻辑 —— 只按分类是否变化来决定。
    let mut last_header: Option<String> = None;

    if display_items.is_empty() {
        // 空列表只出一条**不可选**的提示。模型面板加载中时说明原因，而不是
        // 摆一条 action=Back 的"加载中"菜单项让人去选。
        let hint = if matches!(state.palette_mode, PaletteMode::Model) && state.awaiting_models {
            "Loading models..."
        } else {
            "No matches"
        };
        list_items.push(ListItem::new(Line::from(Span::styled(
            hint,
            Style::default().fg(subtle_color),
        ))));
    } else {
        for (item_index, item) in display_items.iter().enumerate() {
            // 每个分类**各自**判定要不要画标题。整页一个开关是不行的：主题页
            // 的分类叫 "Theme"、页面标题也叫 "Theme"，同一个词画两遍就是噪音；
            // 而模型页的 "Anthropic" 是有信息量的，必须画。
            //
            // 缩进跟着**分类**走，不是跟着"这次是不是首次出现"走。
            //
            // 这两者必须分开：同一个分类里的第二条（比如 "AI" 组下的 Model 和
            // Providers）不会再画一次标题，但**必须**和第一条一样缩进 ——
            // 否则分组里的第一行缩进、后面几行顶格，读起来像是它们不属于
            // 那个组。
            let mut indented = false;
            if let Some(category) = &item.category {
                // 这个分类**要不要**标题，是分类的属性，与"是不是第一条"无关。
                let wants_header =
                    palette_category_has_header(&state.palette_mode, query, category.as_str());
                indented = wants_header;
                if last_header.as_deref() != Some(category.as_str()) {
                    if wants_header {
                        // 分组标题：顶格、灰、不加粗。
                        //
                        // 此前这里用的是主题的 `primary`（青色）——那让面板上出现了
                        // **两种**强调色：青色标题 + 底色高亮。两个都亮 = 都不亮，
                        // 选中行反而被青色标题压过去。
                        //
                        // 现在标题与描述同属一支灰，标题靠**顶格**和条目靠**缩进**
                        // 区分：颜色只标"次要信息"，唯一的强调色留给选中行。
                        list_items.push(ListItem::new(Line::from(Span::styled(
                            category.clone(),
                            Style::default().fg(inactive_color),
                        ))));
                    }
                    last_header = Some(category.clone());
                }
            }
            // 散项（刷新模型列表、Add New Provider、返回）：顶格 —— 它们不属于
            // 任何分组。

            // 标签不加粗：层级靠缩进和颜色，不靠字重。此前每一行都是 BOLD，
            // 12 行全粗等于 12 行都没粗，反而没有重点。
            let label_style = Style::default().fg(foreground_color);

            let mut spans = Vec::new();
            if indented {
                spans.push(Span::raw(" ".repeat(INDENT_GROUPED)));
            }
            spans.push(Span::styled(item.label.clone(), label_style));
            if !item.description.is_empty() {
                spans.push(Span::raw(" ".repeat(DESCRIPTION_GAP)));
                spans.push(Span::styled(
                    item.description.clone(),
                    Style::default().fg(inactive_color),
                ));
            }
            // 返回项的按键提示：Esc 与选中它完全等价，写出来才"功能性"。
            // 画成一枚 keycap（反显的小块）而不是普通文字 —— 它是键，不是标签。
            if matches!(item.action, PaletteAction::Back) {
                spans.push(Span::raw(" ".repeat(DESCRIPTION_GAP)));
                spans.push(Span::styled(
                    " Esc ",
                    Style::default()
                        .fg(foreground_color)
                        .bg(subtle_color)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            list_items.push(ListItem::new(Line::from(spans)));
            row_of_item.push(list_items.len() - 1);
            item_of_row.push(item_index);
        }
    }

    let mut list_state = ListState::default();
    if !row_of_item.is_empty() {
        if state.selected_palette_index >= row_of_item.len() {
            state.selected_palette_index = row_of_item.len() - 1;
        }
        list_state.select(Some(row_of_item[state.selected_palette_index]));
    }

    // 面板里**唯一**有底色的一处：当前选中行。其余全是同一支灰阶。
    let list = List::new(list_items)
        .highlight_style(Style::default().bg(selection_bg).add_modifier(Modifier::BOLD))
        // 高亮符号缩进两列，与分组内的条目对齐 —— 缩进是这个面板的层级语言，
        // 选中标记也得说同一种。
        .highlight_symbol("  ");

    f.render_stateful_widget(list, chunks[1], &mut list_state);

    // 列表可视行数写回状态，供 PageUp/PageDown 按**实际行数**翻页。
    // 此前固定跳 10 行：34 行终端上跳过头，60 行终端上跳不满。
    state.palette_visible_rows = chunks[1].height as usize;
    // 命中测试用的几何与行号映射。鼠标滚轮与点选靠它们落在面板而不是
    // 穿过去落到聊天区（见 `events::mouse`）。
    state.palette_area = Some(chunks[1]);
    state.palette_row_to_item = item_of_row;
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1]);

    layout[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[PaletteItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    /// 回归：高亮画在第几行，Enter 必须触发**同一个**条目。
    ///
    /// 真实事故：用户在模型面板把高亮移到 `opencode-zen/space-bunny-free` 上按
    /// Enter，模型没换 —— 反而把模型列表重新拉了一遍。原因是同一个
    /// `selected_palette_index` 被两边按**不同顺序**解释：
    ///
    /// - `render_palette` 先画带 `category` 的、再画 `category: None` 的散项，
    ///   所以高亮下标是**渲染序**；
    /// - `handle_palette` 的 Enter 直接 `filtered[index]`，是**过滤序**。
    ///
    /// 刷新入口是 `category: None` 的散项，分组逻辑会把它甩到最后。用户要求它
    /// **显示在第一项**，所以 `palette_display_items` 现在显式把它置顶 ——
    /// 不再是"原始 vec 里第一、屏幕上最后"。
    #[test]
    fn the_model_panels_highlight_order_is_the_order_enter_uses() {
        let models = vec![
            "opencode-zen/longcat-2.5-preview-free".to_string(),
            "opencode-zen/space-bunny-free".to_string(),
        ];
        let mut map = std::collections::HashMap::new();
        map.insert(
            "opencode-zen/longcat-2.5-preview-free".to_string(),
            "magpie".to_string(),
        );
        map.insert(
            "opencode-zen/space-bunny-free".to_string(),
            "magpie".to_string(),
        );

        // 刷新入口在原始 vec 里就是第一项。
        let raw = get_model_palette_items(
            &models,
            "opencode-zen/longcat-2.5-preview-free",
            false,
            &map,
            Some(60),
        );
        assert_eq!(
            raw.first().map(|i| i.id.as_str()),
            Some("refresh_models"),
            "刷新入口应当排在最前，实际：{:?}",
            raw.first().map(|i| i.label.clone())
        );

        let mut state = crate::ui::state::ChatState::test_fixture();
        state.palette_mode = crate::ui::state::PaletteMode::Model;
        state.available_models = models;
        state.model_provider_map = map;
        state.current_model = "opencode-zen/longcat-2.5-preview-free".to_string();

        let display =
            palette_display_items(&state.palette_mode, &state, state.palette_filter.trim());

        // 渲染序：刷新入口置顶，其余按 provider 分组。
        assert_eq!(display.len(), raw.len(), "可选项数量不能因为重排而变化");
        assert_eq!(
            display.first().map(|i| i.id.as_str()),
            Some("refresh_models"),
            "渲染序里刷新入口必须在第一项，实际：{:?}",
            display.first().map(|i| i.label.clone())
        );
        assert!(
            display.first().map(|i| i.category.is_none()) == Some(true),
            "刷新入口不该被提升成一级分组标题"
        );

        // 关键断言：下标 0 在渲染时是刷新入口，Enter 也必须是它；模型从下标 1 起。
        assert!(
            display[1..].iter().all(|i| i.category.is_some()),
            "刷新入口之后应当全是分组模型"
        );
        let raw_models: Vec<&str> = raw
            .iter()
            .filter(|i| i.category.is_some())
            .map(|i| i.label.as_str())
            .collect();
        let display_models: Vec<&str> = display
            .iter()
            .filter(|i| i.category.is_some())
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(
            raw_models, display_models,
            "分组模型之间的相对顺序不能被重排改变"
        );
    }

    /// 模型面板没开筛选时，`palette_display_items` 的顺序就是屏幕上的顺序：
    /// 刷新置顶，随后是带 provider 标题的模型。
    #[test]
    fn display_items_are_grouped_then_loose_in_every_palette() {
        let models = vec!["m-a".to_string(), "m-b".to_string()];
        let mut map = std::collections::HashMap::new();
        map.insert("m-a".to_string(), "alpha".to_string());
        map.insert("m-b".to_string(), "beta".to_string());

        let mut state = crate::ui::state::ChatState::test_fixture();
        state.palette_mode = crate::ui::state::PaletteMode::Model;
        state.available_models = models;
        state.model_provider_map = map;

        let display = palette_display_items(&state.palette_mode, &state, "");
        assert!(palette_is_grouped(&state.palette_mode, ""));

        // 第一项一定是 category: None 的刷新入口，后面全是分组模型。
        assert_eq!(
            display.first().map(|i| i.id.as_str()),
            Some("refresh_models")
        );
        assert!(display.first().unwrap().category.is_none());
        assert!(
            display[1..].iter().all(|i| i.category.is_some()),
            "只有刷新入口可以没有分组，其余都该在某个 provider 分组下"
        );
    }

    /// Main 页的分类标题不得在同一屏里出现两次。
    ///
    /// 回归点：Main 页的条目是**按声明顺序**排的，不是按分类聚拢的 ——
    /// `output_style`（Settings）排在列表末尾，`log_selector`（Workspace）也
    /// 排在后面。于是渲染器若"分类变了就画一次标题"，同屏就会出现第二个
    /// "Settings" 和第二个 "Workspace"。用户会以为面板里有两组同名设置。
    ///
    /// 现在 Main 页的条目先按分类聚拢（`palette_display_items` 重排），每个
    /// 分类只画一次标题。
    #[test]
    fn main_page_never_repeats_a_category_header() {
        let state = crate::ui::state::ChatState::test_fixture();
        let display = palette_display_items(&PaletteMode::Main, &state, "");

        // 每个分类的条目在列表里必须是**连续**的一段 —— 不连续就意味着渲染时
        // 会出现第二个同名标题。
        let mut seen: Vec<&str> = Vec::new();
        for item in &display {
            let Some(category) = item.category.as_deref() else {
                continue;
            };
            if seen.last() != Some(&category) {
                assert!(
                    !seen.contains(&category),
                    "分类 {:?} 在列表里断成了两段 —— 渲染时会出现两个同名标题",
                    category
                );
                seen.push(category);
            }
        }
    }

    /// 分类名与页面标题相同时，那个标题不画。
    ///
    /// 主题页的分类叫 "Theme"、页面标题也叫 "Theme"。同一个词在一屏里出现
    /// 两次，用户得读两遍才知道它不是两件事。
    #[test]
    fn a_category_that_repeats_the_page_title_gets_no_header() {
        assert!(
            !palette_category_has_header(&PaletteMode::Theme, "", "Theme"),
            "主题页不该再画一个 Theme 标题"
        );
        assert!(
            palette_category_has_header(&PaletteMode::Theme, "", "Dark"),
            "别的分类照常画标题"
        );
        assert!(
            !palette_category_has_header(&PaletteMode::Model, "", "Anthropic"),
            "模型页的 Anthropic 是有信息量的，必须画"
        );
    }

    /// 搜索时一律平铺：不分组、不缩进。
    ///
    /// 全局搜索的结果来自十几个来源，按来源分组会把一份排名列表切成碎片 ——
    /// 搜索要的是排序，不是归类。
    #[test]
    fn a_filtered_palette_is_flat() {
        assert!(!palette_is_grouped(&PaletteMode::Main, "claude"));
        assert!(palette_is_grouped(&PaletteMode::Main, ""));
    }

    /// Providers 页的顺序：刷新第一、Add New Provider 第二、返回压底。
    #[test]
    fn provider_panel_puts_refresh_and_add_provider_first_and_back_last() {
        let state = crate::ui::state::ChatState::test_fixture();
        let display = palette_display_items(&PaletteMode::Provider, &state, "");

        assert_eq!(
            display.first().map(|i| i.id.as_str()),
            Some("refresh_models"),
            "刷新模型列表必须是 Providers 页的第一项"
        );
        assert_eq!(
            display.get(1).map(|i| i.id.as_str()),
            Some("add_provider"),
            "Add New Provider 必须紧跟在刷新之后"
        );
        assert_eq!(
            display.last().map(|i| i.id.as_str()),
            Some("back"),
            "返回项应当固定压到最后"
        );
        assert!(matches!(
            display.last().map(|i| i.action.clone()),
            Some(PaletteAction::Back)
        ));
        assert!(
            !display.last().unwrap().label.contains(".."),
            "返回项不再带 `..`：{}",
            display.last().unwrap().label
        );
    }

    /// `claude-code` 是兼容层合成的实现细节，Providers 页只留 Anthropic。
    #[test]
    fn providers_panel_does_not_list_a_separate_claude_code_row() {
        let items = get_provider_palette_items(&[], &[]);
        assert!(
            items.iter().any(|i| i.id == "provider_anthropic"),
            "Anthropic 必须保留"
        );
        assert!(
            !items.iter().any(|i| i.id == "provider_claude-code"),
            "Claude Code 不该再单独占一行：{:?}",
            ids(&items)
        );
    }

    /// Claude Code 加载到的模型在模型面板里归到 Anthropic 分组，但归属不变 ——
    /// 选中时仍然带着 `claude-code`，端点与凭证不会被换掉。
    #[test]
    fn claude_code_models_are_grouped_under_anthropic() {
        let models = vec!["claude-opus-5".to_string(), "claude-sonnet-5".to_string()];
        let mut map = std::collections::HashMap::new();
        map.insert("claude-opus-5".to_string(), "claude-code".to_string());
        map.insert("claude-sonnet-5".to_string(), "claude-code".to_string());

        let items = get_model_palette_items(&models, "", false, &map, Some(60));
        let grouped: Vec<&PaletteItem> = items
            .iter()
            .filter(|i| i.id.starts_with("model_"))
            .collect();
        assert_eq!(grouped.len(), 2);
        for item in &grouped {
            assert_eq!(
                item.category.as_deref(),
                Some("Anthropic"),
                "Claude Code 的模型必须并到 Anthropic 分组：{}",
                item.label
            );
        }

        match &grouped[0].action {
            PaletteAction::SetModel { provider_id, .. } => assert_eq!(
                provider_id.as_deref(),
                Some("claude-code"),
                "分组标题变了，真实归属不能跟着变"
            ),
            other => panic!("预期 SetModel，实际 {:?}", other),
        }
    }

    /// 模型面板不该凭空造出 "Other" / "Manual" 之类的分组层级。
    ///
    /// 回归点：`Back`、`Enter model name...`、`Fetch model list from API`
    /// 三项原本要么被兜底进 "Other" 组、要么被塞进 "Manual" 组，于是用户找一个
    /// 模型时得先穿过两个与模型无关的分组标题。现在只剩刷新入口且它 `category: None`，
    /// 渲染时平铺、不带标题，模型分组直接顶上来。
    #[test]
    fn model_panel_keeps_navigation_and_actions_out_of_any_group() {
        let models = vec!["claude-opus-5".to_string(), "deepseek-v4-pro".to_string()];
        let mut map = std::collections::HashMap::new();
        map.insert("claude-opus-5".to_string(), "opencode".to_string());
        map.insert("deepseek-v4-pro".to_string(), "opencode".to_string());

        let items = get_model_palette_items(&models, "claude-opus-5", false, &map, Some(3600));

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

        assert!(
            !labels.iter().any(|l| l.contains("Back")),
            "Back 与 Esc 等价，不该占一行：{labels:?}"
        );

        // 关键：不能再出现任何 "Other" / "Manual" 分组。
        for item in &items {
            let cat = item.category.clone().unwrap_or_default();
            assert!(
                cat != "Other" && cat != "Manual",
                "unexpected group {:?} on item {:?}",
                cat,
                item.label
            );
        }

        // 模型仍然带 provider 分组（这是真正有用的标题）。
        assert!(
            items
                .iter()
                .any(|i| i.category.as_deref() == Some("opencode")),
            "models must stay grouped by provider"
        );
    }

    /// 回归：模型选项必须**自带**它的 provider。
    ///
    /// 真实事故：面板里明明显示着 "Provider: yunjunet"，按下 Enter 却切不动 ——
    /// 因为 action 里只带了模型名，provider 得靠事后重新推导，而那条推导会去翻
    /// 磁盘缓存里的残留桶，推到一个已删除的 provider 身上，于是 worker 静默拒绝。
    ///
    /// 选项自带归属之后，这条推导路径对面板选中项**根本不会被调用**。
    #[test]
    fn model_palette_items_carry_their_own_provider() {
        let models = vec!["deepseek-v4.1-flash".to_string()];
        let mut map = std::collections::HashMap::new();
        map.insert("deepseek-v4.1-flash".to_string(), "yunjunet".to_string());

        let items = get_model_palette_items(&models, "", false, &map, Some(60));
        let model_item = items
            .iter()
            .find(|i| i.label == "deepseek-v4.1-flash")
            .expect("应当有这个模型项");

        match &model_item.action {
            PaletteAction::SetModel { model, provider_id } => {
                assert_eq!(model, "deepseek-v4.1-flash");
                assert_eq!(
                    provider_id.as_deref(),
                    Some("yunjunet"),
                    "选项必须带着列表给出的归属，而不是让下游去猜"
                );
            }
            other => panic!("预期 SetModel，实际 {:?}", other),
        }
    }

    /// 分组用的名字猜测（`infer_provider_from_model`，认不出来一律返回
    /// "other"）**绝不能**当成 provider_id 传下去 —— 那会让 worker 以
    /// `Provider 'other' is not configured.` 拒掉一次本来能用的切换。
    #[test]
    fn an_inferred_group_is_never_sent_as_a_provider_id() {
        // 故意不给 map：面板只能靠名字前缀猜分组。
        let models = vec!["deepseek-v4.1-flash".to_string()];
        let items = get_model_palette_items(&models, "", false, &Default::default(), Some(60));

        let model_item = items
            .iter()
            .find(|i| i.label == "deepseek-v4.1-flash")
            .expect("应当有这个模型项");
        match &model_item.action {
            PaletteAction::SetModel { provider_id, .. } => assert_eq!(
                provider_id, &None,
                "按名字猜出来的分组只是标题，不能当成归属发出去"
            ),
            other => panic!("预期 SetModel，实际 {:?}", other),
        }
        // 但分组标题仍然要显示出来（按名字猜的 DeepSeek 组）。
        assert_eq!(model_item.category.as_deref(), Some("DeepSeek"));
    }

    /// 回归：去重键必须是 (provider, model) 这一对。
    ///
    /// 两个 provider 可以合法地提供同名模型（同一个模型名挂在不同 base_url 下，
    /// OpenClaude 为此专门把 profileId 编进了选项 value）。只用模型名做键，两条
    /// 会被去重成一个，另一个永远选不到 —— 于是"我明明看到它却选不上"。
    /// 回归：没填密钥的 provider 必须**留在列表里并被标出来**，不能消失。
    ///
    /// 隐藏它，用户明明配了 Base URL 却在面板上凭空少一个分组，只会以为自己没配好。
    /// 而模型列表扇出那边已经跳过它了，这枚标记就是它在界面上唯一的持续可见线索。
    #[test]
    fn a_keyless_provider_is_listed_and_flagged_rather_than_hidden() {
        let configured = vec!["has-key".to_string(), "no-key".to_string()];
        let keyless = vec!["no-key".to_string()];

        let items = get_provider_palette_items(&configured, &keyless);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

        assert!(
            labels.iter().any(|l| l.starts_with("no-key")),
            "没密钥的 provider 必须仍然出现在列表里：{labels:?}"
        );
        let flagged = items
            .iter()
            .find(|i| i.label.starts_with("no-key"))
            .expect("no-key 应在列表中");
        // 标记写在**描述**里，不在 label 里：label 是名字，混进状态符号
        // （`✓` / `⚠`）之后就没法直接拿它去 grep 配置了。
        assert!(
            flagged.description.contains("no API key"),
            "必须被标出缺密钥：{}",
            flagged.description
        );
        assert!(
            flagged.description.contains("requests will fail"),
            "说明行要说清后果：{}",
            flagged.description
        );
        assert!(
            !flagged.label.contains("no API key"),
            "label 只放名字，不放状态：{}",
            flagged.label
        );

        let ok = items
            .iter()
            .find(|i| i.label.starts_with("has-key"))
            .expect("has-key 应在列表中");
        assert!(
            !ok.description.contains("no API key"),
            "有密钥的 provider 不该被误标：{}",
            ok.description
        );
    }

    /// 空密钥列表（还没从 worker 收到凭证状态）不能把所有人都标成缺密钥。
    #[test]
    fn an_unknown_credential_state_flags_nobody() {
        let configured = vec!["a".to_string(), "b".to_string()];
        let items = get_provider_palette_items(&configured, &[]);
        assert!(
            items.iter().all(|i| !i.description.contains("no API key")),
            "凭证状态未知时不能瞎标"
        );
    }

    /// 回归：内置 provider 必须有"清空 key"的入口。
    ///
    /// 它没有 Delete（"删掉整项配置"对内置没有意义，它的配置只是 key 和 URL），
    /// 而编辑表单里留空 = "保持原 key"（密钥不预填、不上屏）。所以在这个组合下
    /// **没有任何输入能清掉 key** —— 配错了、过期了、想切回环境变量，都只能去手改
    /// `user-settings.json`。
    #[test]
    fn a_builtin_provider_still_offers_clearing_its_key() {
        let builtin = "openai";
        assert!(
            get_provider_by_id(builtin).is_some(),
            "测试前提：{builtin} 必须是内建 provider"
        );
        // 内建 provider 一般不在 configured 列表里；给一个 configured 的场景，
        // 断言"有 Delete 却没有 Clear"这种不对称已经消失。
        let configured = vec![builtin.to_string()];
        let items = get_provider_options_items(builtin, &configured);
        let has_clear = items
            .iter()
            .any(|i| matches!(i.action, PaletteAction::ClearProviderKey(ref id) if id == builtin));
        assert!(
            has_clear,
            "内建 provider 必须能清空已保存的 key：{:?}",
            items.iter().map(|i| &i.label).collect::<Vec<_>>()
        );
    }

    /// 没配置过的 provider 不必给这一项（没什么可清）。
    #[test]
    fn clearing_the_key_is_offered_only_when_the_provider_is_configured() {
        let items = get_provider_options_items("some-unconfigured-gateway", &[]);
        assert!(
            !items
                .iter()
                .any(|i| matches!(i.action, PaletteAction::ClearProviderKey(_))),
            "未配置的 provider 不该显示清空 key"
        );
    }

    /// 自定义 provider 同样要有（它虽然能整体删掉，但"只想换 key"不必删）。
    #[test]
    fn a_custom_configured_provider_also_offers_clearing_its_key() {
        let id = "my-gateway";
        let items = get_provider_options_items(id, &[id.to_string()]);
        assert!(items
            .iter()
            .any(|i| matches!(i.action, PaletteAction::ClearProviderKey(ref x) if x == id)));
    }

    /// 清空 key 不能和"删除 provider"撞成同一个去重键。
    #[test]
    fn clearing_and_deleting_are_distinct_actions() {
        let id = "my-gateway";
        assert_ne!(
            palette_action_key(&PaletteAction::ClearProviderKey(id.to_string())),
            palette_action_key(&PaletteAction::DeleteProvider(id.to_string())),
        );
    }

    #[test]
    fn same_model_name_under_two_providers_gets_distinct_action_keys() {
        let a = PaletteAction::SetModel {
            model: "shared-model".to_string(),
            provider_id: Some("magpie".to_string()),
        };
        let b = PaletteAction::SetModel {
            model: "shared-model".to_string(),
            provider_id: Some("yunjunet".to_string()),
        };
        assert_ne!(
            palette_action_key(&a),
            palette_action_key(&b),
            "同名模型在不同 provider 下必须是两个可区分的选项"
        );
    }

    /// 名字判定为“仅开/关”的模型也必须展示与精细档位模型完全一致的
    /// Off/Low/Medium/High 刻度：UI 不区分厂家差异（请求层负责映射），
    /// 更不能混入独立的 “On” 项打乱档位顺序。
    #[test]
    fn binary_thinking_models_use_the_same_uniform_scale() {
        let binary =
            get_thinking_effort_palette_items(&crate::types::ThinkingEffort::Off, "deepseek-r1");
        let granular =
            get_thinking_effort_palette_items(&crate::types::ThinkingEffort::Off, "claude-opus-5");

        let binary_labels: Vec<String> = binary
            .iter()
            .filter(|i| i.id.starts_with("thinking_"))
            .map(|i| i.label.clone())
            .collect();
        let granular_labels: Vec<String> = granular
            .iter()
            .filter(|i| i.id.starts_with("thinking_"))
            .map(|i| i.label.clone())
            .collect();

        // 两个能力等级的面板完全一致
        assert_eq!(binary_labels, granular_labels);
        // 刻度固定为 Off → Low → Medium → High → Xhigh → Max，没有 “On”
        // label 是「符号 + 档位名」；"当前"不在 label 里（写进描述）。
        assert_eq!(
            binary_labels,
            vec![
                "◌ Off",
                "○ Low",
                "◐ Medium",
                "● High",
                "◆ Xhigh",
                "◇ Max"
            ]
        );
    }

    /// 不支持思考的模型保持提示项，不展示档位。
    #[test]
    fn non_thinking_models_keep_the_unavailable_hint() {
        let items =
            get_thinking_effort_palette_items(&crate::types::ThinkingEffort::Off, "gpt-4o-mini");
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert!(ids.contains(&"thinking_unavailable"), "{ids:?}");
        assert!(!ids.contains(&"thinking_high"), "{ids:?}");
    }

    /// 模型面板里只有模型（外加空闲时的刷新入口）。
    ///
    /// 回归点：这里曾平铺着 `← Back`、`⌨ Enter model name...` 以及两条加载态
    /// 伪装的菜单项。`← Back` 与 Esc 的实现完全相同（都走
    /// `palette_history.pop()`，空了才关面板），其余三条与"选一个模型"无关 ——
    /// 其中两条状态项甚至能被选中，回车却只是后退。
    #[test]
    fn model_panel_lists_models_and_nothing_else() {
        let models = vec!["claude-opus-5".to_string(), "deepseek-v4-pro".to_string()];
        let mut map = std::collections::HashMap::new();
        map.insert("claude-opus-5".to_string(), "opencode".to_string());
        map.insert("deepseek-v4-pro".to_string(), "opencode".to_string());

        let items = get_model_palette_items(&models, "claude-opus-5", false, &map, Some(3600));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();

        assert!(
            !labels.iter().any(|l| l.contains("Back")),
            "Back 与 Esc 等价，不该占一行：{labels:?}"
        );
        assert!(
            !labels.iter().any(|l| l.contains("Enter model name")),
            "手动输入模型名入口已移除：{labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|l| !l.contains("Loading") && !l.contains("Fetching")),
            "加载状态不该是可选条目：{labels:?}"
        );

        // 空闲时保留刷新入口，且它不参与 provider 分组（分组标题只服务于模型本身）
        assert_eq!(labels[0], "Fetch model list from API", "{labels:?}");
        assert!(
            items[0].category.is_none(),
            "刷新入口不应被提升成一级分组标题"
        );

        // 模型仍带 provider 分组（这是真正有用的标题）
        assert!(
            items
                .iter()
                .any(|i| i.category.as_deref() == Some("opencode")),
            "models must stay grouped by provider"
        );
    }

    /// 加载中：既不产出刷新项、也不产出占位项，返回空列表让渲染层画不可选提示。
    #[test]
    fn loading_yields_no_items_at_all() {
        let items = get_model_palette_items(
            &[],
            "claude-opus-5",
            true,
            &std::collections::HashMap::new(),
            None,
        );
        assert!(
            items.is_empty(),
            "加载中不该有任何可选项（此前是「正在加载」的菜单项）：{:?}",
            ids(&items)
        );
    }

    /// 空列表（已加载完但一个模型都没拿到）仍要给出当前模型与刷新出口。
    #[test]
    fn an_empty_list_still_offers_a_way_forward() {
        let items = get_model_palette_items(
            &[],
            "claude-opus-5",
            false,
            &std::collections::HashMap::new(),
            None,
        );

        let labels = ids(&items);
        assert!(
            labels.iter().any(|l| l.contains("Fetch model list")),
            "{labels:?}"
        );
        // 当前模型即使不在列表里也要能选回来
        assert!(
            labels.iter().any(|l| l.starts_with("claude-opus-5")),
            "{labels:?}"
        );
    }

    /// 加载中不再有刷新项 —— 状态不该伪装成可点的东西。
    ///
    /// 回归点：此前这条会把刷新项的 label 换成 "⟳ Fetching model list..."，用户
    /// 能选中一条"正在拉取"的菜单项，回车只是再触发一次请求。
    #[test]
    fn loading_removes_the_refresh_item_instead_of_relabelling_it() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            true,
            &std::collections::HashMap::new(),
            None,
        );
        assert!(
            !items.iter().any(|i| i.id == "refresh_models"),
            "加载中不该有可选的刷新项：{:?}",
            ids(&items)
        );
        assert!(
            items.iter().all(|i| !i.label.contains("Fetching")),
            "没有任何条目该显示加载文案：{:?}",
            ids(&items)
        );
    }

    #[test]
    fn cache_age_shows_up_in_the_refresh_hint() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            false,
            &std::collections::HashMap::new(),
            Some(7_200),
        );
        let refresh = items.iter().find(|i| i.id == "refresh_models").unwrap();
        assert!(
            refresh.description.contains("2h ago"),
            "{}",
            refresh.description
        );
    }

    #[test]
    fn cache_age_is_written_in_the_biggest_fitting_unit() {
        assert_eq!(format_cache_age(None), None);
        assert_eq!(format_cache_age(Some(0)).unwrap(), "just now");
        assert_eq!(format_cache_age(Some(59)).unwrap(), "just now");
        assert_eq!(format_cache_age(Some(60)).unwrap(), "1m ago");
        assert_eq!(format_cache_age(Some(3_599)).unwrap(), "59m ago");
        assert_eq!(format_cache_age(Some(3_600)).unwrap(), "1h ago");
        assert_eq!(format_cache_age(Some(86_400)).unwrap(), "1d ago");
    }

    /// 主面板搜索列表里模型面板的条目 key 不能撞（撞了会被去重掉一个）。
    #[test]
    fn model_entries_have_distinct_dedup_keys() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            false,
            &std::collections::HashMap::new(),
            None,
        );
        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        push_unique_actionable_items(&mut merged, &mut seen, items);

        assert!(merged.iter().any(|i| i.id == "refresh_models"));
        assert_eq!(
            seen.len(),
            merged.len(),
            "去重后条目数应当不变：{:?}",
            merged.iter().map(|i| &i.id).collect::<Vec<_>>()
        );
    }

    /// provider 选项页的"改"只有 Edit Provider 一个入口（旧的 Set API Key +
    /// Set Base URL 两个串行弹窗已下线），删入口只给自定义 provider。
    #[test]
    fn provider_options_page_has_one_edit_entry() {
        // 已配置的内置 provider：主行是 Use This Provider，另有 Edit Provider
        let items = get_provider_options_items("openai", &["openai".to_string()]);
        assert!(items.iter().any(
            |i| i.id == "use_provider" && matches!(i.action, PaletteAction::SelectProvider(_))
        ));
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i.action, PaletteAction::EditProvider(_)))
                .count(),
            1
        );
        // 内置 provider 不能删
        assert!(!items.iter().any(|i| i.id == "remove_provider"));
    }

    /// 没配置过的 provider：主行就是 Configure This Provider（开表单），
    /// 不再重复列一个作用相同的 Edit Provider 行。
    #[test]
    fn unconfigured_provider_does_not_duplicate_the_edit_entry() {
        let items = get_provider_options_items("openai", &[]);
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i.action, PaletteAction::EditProvider(_)))
                .count(),
            1,
            "只有主行开表单，不该列两行同一个动作"
        );
        assert!(items.iter().any(|i| i.id == "use_provider"));
        assert!(!items.iter().any(|i| i.id == "edit_provider"));
    }

    /// 列表里没配置的 provider 直接开表单，不再走只收 API Key 的旧弹窗。
    #[test]
    fn unconfigured_list_item_opens_the_form() {
        let item = build_provider_item_flat(
            get_provider_by_id("openai").expect("openai 是内置 provider"),
            &[],
        );
        assert!(matches!(item.action, PaletteAction::EditProvider(_)));
    }

    /// 面板的**样子**基线。
    ///
    /// 逻辑断言管不了"长什么样"：层级靠缩进、颜色只有一处强调、字重只用在
    /// 选中行 —— 这些都是渲染出来才看得见的。而官方的
    /// `assert_snapshot!(terminal.backend())` 只序列化字符，颜色不进基线
    /// （ratatui#1402），一次"把分组标题改成青色"的重构会静默通过。
    /// 本仓库的基线把每个 cell 的 fg/bg/modifier 都编码进去（见
    /// `ui::test_support::snapshot`），所以改颜色即改基线。
    #[test]
    fn palette_main_page_appearance() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        state.palette_mode = PaletteMode::Main;
        state.modal_stack.push(crate::ui::state::Modal::Palette);

        crate::ui::test_support::assert_render!("palette_main", 100, 28, |f| {
            // 区域先取出来：`f` 已经是可变借用，再调 `f.area()` 会撞上借用检查。
            let area = f.area();
            render_palette(f, area, &mut state);
        });
    }
}
