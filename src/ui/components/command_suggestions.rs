//! 补全建议弹窗的绘制 —— TEA 的 view 侧。
//!
//! 只读 `ChatState`，不修改任何状态。状态机与按键处理在
//! `crate::ui::services::suggestions`。

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem},
    Frame,
};

use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;
pub fn render_overlays(f: &mut Frame<'_>, state: &ChatState, input_area: Rect) {
    let theme = state.theme_manager.current();
    render_mention_hints_overlay(f, state, input_area, theme);
    render_command_hints_overlay(f, state, input_area, theme);
    render_provider_selection_overlay(f, state, input_area, theme);
    render_session_selection_overlay(f, state, input_area, theme);
}

fn render_provider_selection_overlay(
    f: &mut Frame<'_>,
    state: &ChatState,
    _input_area: Rect,
    theme: &Theme,
) {
    if !state.show_provider_menu {
        return;
    }

    let providers =
        crate::ui::components::palette::get_provider_quick_items(&state.configured_providers);
    if providers.is_empty() {
        return;
    }

    let current_provider = crate::ui::utils::status::current_provider_id(state);
    let items: Vec<ListItem> = providers
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_selected = idx == state.selected_provider_index;
            let is_current = current_provider
                .as_deref()
                .map(|provider_id| item.id == format!("provider_{}", provider_id))
                .unwrap_or(false);

            let (bg, label_fg, desc_fg) = if is_selected {
                (theme.selection_bg, theme.foreground, theme.secondary)
            } else if is_current {
                (Color::Reset, theme.success, theme.inactive)
            } else {
                (Color::Reset, theme.primary, theme.inactive)
            };

            let prefix = if is_current { "* " } else { "  " };
            let category = item.category.as_deref().unwrap_or("Provider");
            let line = Line::from(vec![
                Span::styled(
                    format!("{}{}", prefix, item.label),
                    Style::default()
                        .fg(label_fg)
                        .bg(bg)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("  [{}] {}", category, item.description),
                    Style::default().fg(desc_fg).bg(bg),
                ),
            ]);

            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    render_centered_list(f, f.area(), "Providers", &items);
}

fn render_session_selection_overlay(
    f: &mut Frame<'_>,
    state: &ChatState,
    _input_area: Rect,
    theme: &Theme,
) {
    if !state.show_session_menu {
        return;
    }

    let sessions =
        crate::ui::components::palette::get_session_quick_items(&state.available_sessions);
    let items: Vec<ListItem> = sessions
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_selected = idx == state.selected_session_index;
            let (bg, label_fg, desc_fg) = if is_selected {
                (theme.selection_bg, theme.foreground, theme.secondary)
            } else {
                (Color::Reset, theme.primary, theme.inactive)
            };

            let line = Line::from(vec![
                Span::styled(
                    item.label.clone(),
                    Style::default()
                        .fg(label_fg)
                        .bg(bg)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("  {}", item.description),
                    Style::default().fg(desc_fg).bg(bg),
                ),
            ]);

            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    render_centered_list(f, f.area(), "Sessions", &items);
}

fn render_centered_list(f: &mut Frame<'_>, area: Rect, title: &str, items: &[ListItem<'_>]) {
    let visible = items.len().clamp(1, 10);
    let popup_area = centered_rect(68, (visible as u16) + 4, area);
    let list = List::new(items.to_vec()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(Color::Cyan)),
    );

    f.render_widget(Clear, popup_area);
    f.render_widget(list, popup_area);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_height = height.min(area.height.saturating_sub(2)).max(3);
    let vertical_margin = area.height.saturating_sub(popup_height) / 2;
    let popup_width = area.width * percent_x / 100;
    let horizontal_margin = area.width.saturating_sub(popup_width) / 2;

    Rect {
        x: area.x + horizontal_margin,
        y: area.y + vertical_margin,
        width: popup_width.max(20),
        height: popup_height,
    }
}

fn render_mention_hints_overlay(
    f: &mut Frame<'_>,
    state: &ChatState,
    input_area: Rect,
    theme: &Theme,
) {
    if !state.show_mention_hints || state.mention_hints.is_empty() {
        return;
    }

    let max_items: usize = 8;
    let total = state.mention_hints.len();
    let visible = total.min(max_items);
    let hints_height = (visible as u16) + 2;
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(hints_height),
        width: input_area.width.min(80),
        height: hints_height,
    };

    let mut start = 0;
    if state.selected_mention_hint >= visible {
        start = state.selected_mention_hint - visible + 1;
    }
    if start + visible > total {
        start = total.saturating_sub(visible);
    }

    let cwd = std::env::current_dir().unwrap_or_default();

    // 对标 Claude Code：每行只有选择指示符 + 路径本身。
    // 目录带尾 `/` 且用 dim 色；无 emoji 图标、无大小/类型描述、无列 padding。
    let hint_items: Vec<ListItem> = state
        .mention_hints
        .iter()
        .skip(start)
        .take(visible)
        .enumerate()
        .map(|(idx, hint)| {
            let actual_idx = start + idx;
            let is_selected = actual_idx == state.selected_mention_hint;
            let is_dir = hint.ends_with('/');

            let display_path = simplify_path(hint, &cwd);

            let (bg, fg) = if is_selected {
                (theme.selection_bg, theme.foreground)
            } else if is_dir {
                (Color::Reset, theme.inactive)
            } else {
                (Color::Reset, theme.foreground)
            };

            let selection_indicator = if is_selected { "▶" } else { " " };

            let spans = vec![
                Span::styled(
                    format!(" {} ", selection_indicator),
                    Style::default().fg(theme.primary).bg(bg),
                ),
                Span::styled(
                    truncate_str(&display_path, 60),
                    Style::default().fg(fg).bg(bg).add_modifier(if is_selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
                ),
            ];

            ListItem::new(Line::from(spans)).style(Style::default().bg(bg))
        })
        .collect();

    let hints_list = List::new(hint_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" Files ")
            .title_style(Style::default().fg(theme.primary)),
    );
    f.render_widget(Clear, popup_area);
    f.render_widget(hints_list, popup_area);
}

/// Simplify path for display
fn simplify_path(path: &str, cwd: &std::path::Path) -> String {
    let cwd_str = cwd.to_string_lossy();
    if path.starts_with(cwd_str.as_ref()) {
        let relative = &path[cwd_str.len()..];
        return relative.strip_prefix('/').unwrap_or(relative).to_string();
    }

    // Try home directory
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path.starts_with(home_str.as_ref()) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }

    path.to_string()
}

/// Truncate string to max length（按字符数截断，中文安全）
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn render_command_hints_overlay(
    f: &mut Frame<'_>,
    state: &ChatState,
    input_area: Rect,
    theme: &Theme,
) {
    if !state.show_command_hints || state.command_hints.is_empty() {
        return;
    }

    let max_items: usize = 8;
    let total = state.command_hints.len();
    let visible = total.min(max_items);
    let hints_height = (visible as u16) + 2;
    // 与 "@" 文件弹窗统一宽度
    let popup_width = input_area.width.min(80);
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(hints_height),
        width: popup_width,
        height: hints_height,
    };

    let mut start = 0;
    if state.selected_hint >= visible {
        start = state.selected_hint - visible + 1;
    }
    if start + visible > total {
        start = total.saturating_sub(visible);
    }

    // 动态命令列宽（对标 Claude Code：可见项最长命令 + 2，设上下限），
    // 替代原固定 18 padding——短命令描述贴近、长命令不粘连
    let name_w = {
        let max_cmd = state
            .command_hints
            .iter()
            .map(|h| {
                h.split_once(" - ")
                    .map(|(c, _)| c.chars().count())
                    .unwrap_or(h.chars().count())
            })
            .max()
            .unwrap_or(10);
        (max_cmd + 2).clamp(10, 24) as usize
    };
    // 描述可用宽度：弹窗宽 - 图标(3) - 命令列 - 边框(2) - 余量(2)
    let desc_w = popup_width.saturating_sub(name_w as u16 + 7).max(10) as usize;

    let hint_items: Vec<ListItem> = state
        .command_hints
        .iter()
        .skip(start)
        .take(visible)
        .enumerate()
        .map(|(idx, hint)| {
            let actual_idx = start + idx;
            let is_selected = actual_idx == state.selected_hint;

            let (cmd_part, desc_part) = if let Some((c, d)) = hint.split_once(" - ") {
                (c, d)
            } else {
                (hint.as_str(), "")
            };

            let (bg, label_fg, desc_fg, icon) = if is_selected {
                (theme.selection_bg, theme.foreground, theme.secondary, "▶")
            } else {
                (Color::Reset, theme.primary, theme.inactive, " ")
            };

            let mut spans = vec![
                Span::styled(
                    format!(" {} ", icon),
                    Style::default()
                        .fg(if is_selected {
                            theme.primary
                        } else {
                            theme.inactive
                        })
                        .bg(bg),
                ),
                Span::styled(
                    format!("{:<width$}", cmd_part, width = name_w),
                    Style::default()
                        .fg(label_fg)
                        .bg(bg)
                        .add_modifier(if is_selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            ];

            if !desc_part.is_empty() {
                spans.push(Span::styled(
                    truncate_str(desc_part.replace("  ", " ").trim(), desc_w),
                    Style::default().fg(desc_fg).bg(bg),
                ));
            }

            ListItem::new(Line::from(spans)).style(Style::default().bg(bg))
        })
        .collect();

    let hints_list = List::new(hint_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" Commands ")
            .title_style(Style::default().fg(theme.primary)),
    );
    f.render_widget(Clear, popup_area);
    f.render_widget(hints_list, popup_area);
}
