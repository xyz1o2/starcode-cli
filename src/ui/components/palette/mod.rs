//! 命令面板（Ctrl+P）。
//!
//! 本文件只管**调度与渲染**：按 PaletteMode 挑条目、过滤、画框。
//! 条目本身在 [items]，条目描述在 [describe]。

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

/// 面板是否按分组渲染（分组项在前、无标题的散项在后）。
pub fn palette_is_grouped(mode: &PaletteMode, query: &str) -> bool {
    !query.trim().is_empty() || matches!(mode, PaletteMode::Model)
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

    if !palette_is_grouped(mode, query) {
        return filtered;
    }

    let total = filtered.len();

    // 分组项在前（按首次出现顺序），`category: None` 的散项平铺在后面。
    let mut groups: Vec<(String, Vec<PaletteItem>)> = Vec::new();
    let mut loose: Vec<PaletteItem> = Vec::new();
    for item in filtered {
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

    let mut ordered = Vec::with_capacity(total);
    for (_, items) in groups {
        ordered.extend(items);
    }
    ordered.extend(loose);
    ordered
}

pub fn palette_title(mode: &PaletteMode, query: &str) -> String {
    let scope = if !query.trim().is_empty() && matches!(mode, PaletteMode::Main) {
        "Search All"
    } else {
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
            PaletteMode::AgentMode => "Agent Mode",
            PaletteMode::ThinkingEffort => "Thinking",
            PaletteMode::ContextWindow => "Context Window",
            PaletteMode::Theme => "Theme",
            PaletteMode::ProviderPopular => "Popular Providers",
            PaletteMode::ProviderOther => "Regional Providers",
            PaletteMode::ProviderLocal => "Local Providers",
            PaletteMode::ProviderOptions(_) => "Provider Setup",
            PaletteMode::ProviderDelete(_) => "Remove Provider",
            PaletteMode::Language => "Language",
            PaletteMode::OutputStyle => "Output Style",
        }
    };

    format!(" {} (Ctrl+P) ", scope)
}

pub fn palette_placeholder(mode: &PaletteMode) -> &'static str {
    match mode {
        PaletteMode::Main => "Search actions, models, providers",
        PaletteMode::Command => "Search commands",
        PaletteMode::Provider => "Search providers by name or endpoint type",
        PaletteMode::Model => "Search models",
        PaletteMode::Language => "Select UI language",
        PaletteMode::System => "Search settings, language, diagnostics",
        PaletteMode::Memory => "Search memory actions",
        PaletteMode::Session => "Search session actions",
        _ => "Search commands",
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
    let mut items: Vec<PaletteItem> = ALL_PROVIDERS
        .iter()
        .map(|provider| build_provider_item_flat(provider, configured))
        .collect();

    // Add custom providers
    for id in configured
        .iter()
        .filter(|id| !builtin_ids.contains(id.as_str()))
    {
        items.push(PaletteItem {
            id: format!("provider_{}", id),
            label: format!("{} ✓", id),
            description: "Custom provider".to_string(),
            category: Some("Custom".to_string()),
            action: PaletteAction::Navigate(PaletteMode::ProviderOptions(id.clone())),
        });
    }

    items
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
    let primary_color = theme.primary;
    let inactive_color = theme.inactive;
    let foreground_color = theme.foreground;
    let selection_bg = theme.selection_bg;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(palette_title(&state.palette_mode, &state.palette_filter))
        .title_style(Style::default().fg(primary_color));

    // 70% 太宽：行内 label + description 拉得过开，视觉上压住整个会话区。
    // 收到 58% 后两侧留白足够，最长的描述行（删除 provider 的那句）在 120 列
    // 终端上正好放得下。
    let area = centered_rect(PALETTE_WIDTH_PERCENT, PALETTE_HEIGHT_PERCENT, area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
        .split(area);

    let query = state.palette_filter.trim();
    let query_lower = query.to_lowercase();
    let is_empty = query.is_empty();

    let placeholder = palette_placeholder(&state.palette_mode);
    let input_text = if is_empty {
        placeholder
    } else {
        state.palette_filter.as_str()
    };
    let input_style = if is_empty {
        Style::default().fg(inactive_color)
    } else {
        Style::default().fg(foreground_color)
    };

    let input = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(input_text.to_string(), input_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" Search ")
            .title_style(Style::default().fg(primary_color)),
    );
    f.render_widget(input, chunks[0]);

    // 顺序只在 `palette_display_items` 里定义一次（见它的文档注释）。这里和
    // `handle_palette` 用的是同一个 vec，所以高亮画在第 `selected_palette_index`
    // 项上、按 Enter 触发的也是第 `selected_palette_index` 项。
    let grouped = palette_is_grouped(&state.palette_mode, query);
    let display_items = palette_display_items(&state.palette_mode, state, query);

    let mut list_items: Vec<ListItem> = Vec::new();
    // `display_items[i]` 对应 `list_items` 里的哪一行（标题行不占位）。
    let mut row_of_item: Vec<usize> = Vec::with_capacity(display_items.len());
    // 已经发过标题的分组名；`category: None` 的散项没有标题，直接平铺。
    let mut last_header: Option<String> = None;
    let mut past_loose = false;

    if display_items.is_empty() {
        // 空列表只出一条**不可选**的提示。模型面板加载中时说明原因，而不是
        // 摆一条 action=Back 的"加载中"菜单项让人去选。
        let hint = if matches!(state.palette_mode, PaletteMode::Model) && state.awaiting_models {
            "⏳ Loading models… (Esc to close)"
        } else {
            "No results"
        };
        list_items.push(ListItem::new(Line::from(Span::styled(
            hint,
            Style::default().fg(inactive_color),
        ))));
    } else {
        for item in &display_items {
            let indented = item.category.is_some();
            if !indented {
                past_loose = true;
            } else if !past_loose {
                if let Some(category) = &item.category {
                    if last_header.as_deref() != Some(category.as_str()) {
                        list_items.push(ListItem::new(Line::from(Span::styled(
                            category.clone(),
                            Style::default()
                                .fg(primary_color)
                                .add_modifier(Modifier::BOLD),
                        ))));
                        last_header = Some(category.clone());
                    }
                }
            }

            let mut spans = Vec::new();
            if indented {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                item.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            if !item.description.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    item.description.clone(),
                    Style::default().fg(inactive_color),
                ));
            }

            list_items.push(ListItem::new(Line::from(spans)));
            row_of_item.push(list_items.len() - 1);
        }
    }

    let mut list_state = ListState::default();
    if !row_of_item.is_empty() {
        if state.selected_palette_index >= row_of_item.len() {
            state.selected_palette_index = row_of_item.len() - 1;
        }
        list_state.select(Some(row_of_item[state.selected_palette_index]));
    }

    let list = List::new(list_items)
        .highlight_style(
            Style::default()
                .bg(selection_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[1], &mut list_state);
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
    /// 模型面板正好两种都有：`⟳ Fetch model list from API` 是 `category: None`
    /// 且排在**第一位**。于是渲染序 `[模型…, 刷新]` 与过滤序 `[刷新, 模型…]`
    /// 差一条 —— 下标 0 在渲染时是第一个模型、在 Enter 时是"刷新列表"。
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

        // 刷新入口确实排在第一位且没有分组 —— 这正是错位的来源，必须锁住。
        let raw = get_model_palette_items(
            &models,
            "opencode-zen/longcat-2.5-preview-free",
            false,
            &map,
            Some(60),
        );
        assert!(
            raw.first().map(|i| i.category.is_none()) == Some(true),
            "刷新入口应当是 category: None 且排在最前，实际：{:?}",
            raw.first().map(|i| i.label.clone())
        );

        let mut state = crate::ui::state::ChatState::test_fixture();
        state.palette_mode = crate::ui::state::PaletteMode::Model;
        state.available_models = models;
        state.model_provider_map = map;
        state.current_model = "opencode-zen/longcat-2.5-preview-free".to_string();

        let display =
            palette_display_items(&state.palette_mode, &state, state.palette_filter.trim());

        // 渲染序：分组项在前（都属 magpie），刷新入口平铺在最后。
        assert_eq!(display.len(), raw.len(), "可选项数量不能因为重排而变化");
        assert!(
            display.last().map(|i| i.category.is_none()) == Some(true),
            "渲染序里刷新入口必须在最后，实际：{:?}",
            display.last().map(|i| i.label.clone())
        );

        // 关键断言：渲染序与过滤序只在"刷新入口"这一条上不同，
        // 因此每个模型的下标两者必须一致。
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

        // 下标 0：渲染时是第一个模型（magpie 分组第一行），Enter 也必须是它，
        // 不能是"刷新模型列表"。
        assert!(
            display[0].category.is_some(),
            "下标 0 应当是模型而不是刷新入口，实际：{:?}",
            display[0].label
        );
    }

    /// 模型面板没开筛选时，`palette_display_items` 的顺序就是屏幕上的顺序。
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

        // 最后一个一定是 category: None 的散项（刷新入口），中间全是分组模型。
        assert!(display.last().unwrap().category.is_none());
        assert!(
            display[..display.len() - 1]
                .iter()
                .all(|i| i.category.is_some()),
            "只有散项可以没有分组，其余都该在某个 provider 分组下"
        );
    }

    /// 模型面板不该凭空造出 "Other" / "Manual" 之类的分组层级。
    ///
    /// 回归点：`.. Back`、`Enter model name...`、`Fetch model list from API`
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
        assert!(
            flagged.label.contains("no API key"),
            "必须被标出缺密钥：{}",
            flagged.label
        );
        assert!(
            flagged.description.contains("requests will fail"),
            "说明行要说清后果：{}",
            flagged.description
        );

        let ok = items
            .iter()
            .find(|i| i.label.starts_with("has-key"))
            .expect("has-key 应在列表中");
        assert!(
            !ok.label.contains("no API key"),
            "有密钥的 provider 不该被误标：{}",
            ok.label
        );
    }

    /// 空密钥列表（还没从 worker 收到凭证状态）不能把所有人都标成缺密钥。
    #[test]
    fn an_unknown_credential_state_flags_nobody() {
        let configured = vec!["a".to_string(), "b".to_string()];
        let items = get_provider_palette_items(&configured, &[]);
        assert!(
            items.iter().all(|i| !i.label.contains("no API key")),
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
        // 刻度固定为 Off → Low → Medium → High → Xhigh，没有 “On”
        assert_eq!(
            binary_labels,
            vec!["◌ Off  (current)", "○ Low", "◐ Medium", "● High", "◆ Xhigh"]
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
    /// 回归点：这里曾平铺着 `.. Back`、`⌨ Enter model name...` 以及两条加载态
    /// 伪装的菜单项。`.. Back` 与 Esc 的实现完全相同（都走
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
        assert_eq!(labels[0], "⟳ Fetch model list from API", "{labels:?}");
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
}
