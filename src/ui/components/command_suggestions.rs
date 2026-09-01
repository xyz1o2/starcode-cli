use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem},
    Frame,
};

use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;
use std::time::Duration;

const PASTE_HINT_GUARD_MS: u64 = 350;

fn extract_hint_primary_text(hint: &str) -> &str {
    hint.split(" - ").next().unwrap_or("").trim()
}

fn extract_mention_path(hint: &str) -> &str {
    hint.split("  ").next().unwrap_or(hint).trim()
}

/// 定位输入末尾正在输入的 @ token。
/// 返回 Some((start, quoted))：start 是 '@' 的字节位置，quoted 表示 @" 引号形式。
/// 引号形式允许 token 内含空格；普通形式以空白结尾即失效。
fn last_at_token_start(input: &str) -> Option<(usize, bool)> {
    let bytes = input.as_bytes();
    let mut at = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'@' && (i == 0 || (bytes[i - 1] as char).is_whitespace()) {
            at = Some(i);
        }
    }
    let at = at?;
    let quoted = bytes.get(at + 1) == Some(&b'"');
    if !quoted && input[at + 1..].contains(char::is_whitespace) {
        return None; // 普通 token 已被空白终结，不再激活
    }
    Some((at, quoted))
}

/// 从 @ token 提取搜索片段（已接受时替换、输入时刷新共用）
fn at_token_frag(input: &str, at: usize, quoted: bool) -> &str {
    let after = &input[at + 1..];
    if quoted {
        // 去掉开头引号；未闭合时取到末尾
        let body = after.strip_prefix('"').unwrap_or(after);
        match body.find('"') {
            Some(i) => &body[..i],
            None => body,
        }
    } else {
        // 到空白为止
        match after.find(char::is_whitespace) {
            Some(i) => &after[..i],
            None => after,
        }
    }
}

/// 把 @ token 替换为指定路径。路径含空格时使用 @"..." 引号形式（对标 Claude Code）。
/// is_dir 为真时以 `/` 结尾且不关闭提示（继续下钻），否则补空格。
fn replace_at_token_with_path(input: &mut String, path: &str, is_dir: bool) {
    let Some((at, _quoted)) = last_at_token_start(input) else {
        return;
    };
    // 计算原 token 的结束位置：引号形式到闭合引号或末尾；普通形式到空白或末尾
    let end = {
        let after = &input[at + 1..];
        if after.starts_with('"') {
            let body = &after[1..];
            body.find('"')
                .map(|i| at + 1 + 1 + i + 1)
                .unwrap_or(input.len())
        } else {
            after
                .find(char::is_whitespace)
                .map(|i| at + 1 + i)
                .unwrap_or(input.len())
        }
    };

    let suffix = input[end..].to_string();
    input.truncate(at);

    let clean = path.trim_end_matches('/');
    if clean.chars().any(char::is_whitespace) {
        // 引号形式，目录的 '/' 放在引号内
        if is_dir {
            input.push_str(&format!("@\"{}/\"", clean));
        } else {
            input.push_str(&format!("@\"{}\"", clean));
        }
    } else if is_dir {
        input.push_str(&format!("@{}/", clean));
    } else {
        input.push_str(&format!("@{}", clean));
    }

    if suffix.is_empty() {
        // 文件补空格；目录不补（等待继续输入或下钻刷新）
        if !is_dir {
            input.push(' ');
        }
    } else if !suffix.starts_with(' ') && !is_dir {
        input.push(' ');
        input.push_str(&suffix);
    } else {
        input.push_str(&suffix);
    }
}

pub fn on_input_changed(state: &mut ChatState) {
    if should_suppress_hints(state) {
        state.show_command_hints = false;
        state.command_hints.clear();
        state.show_mention_hints = false;
        state.mention_hints.clear();
        return;
    }

    refresh_mention_hints(state);

    if state.input.starts_with('/') && !state.show_mention_hints {
        state.show_command_hints = true;
        state.command_hints = crate::commands::system::get_command_hints(&state.input);
        state.selected_hint = 0;
    } else {
        state.show_command_hints = false;
        state.command_hints.clear();
    }
}

fn should_suppress_hints(state: &ChatState) -> bool {
    if state.paste_in_progress {
        return true;
    }
    if let Some(end_time) = state.paste_end_time {
        if end_time.elapsed() < Duration::from_millis(PASTE_HINT_GUARD_MS) {
            return true;
        }
    }
    // 不再因为多行输入而抑制提示，允许用户在多行输入时使用 @ 文件引用
    false
}

pub fn handle_up(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        if state.selected_session_index > 0 {
            state.selected_session_index -= 1;
        }
        return true;
    }
    if state.show_provider_menu {
        if state.selected_provider_index > 0 {
            state.selected_provider_index -= 1;
        }
        return true;
    }
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        if state.selected_mention_hint > 0 {
            state.selected_mention_hint -= 1;
        }
        return true;
    }
    if state.show_command_hints && !state.command_hints.is_empty() {
        if state.selected_hint > 0 {
            state.selected_hint -= 1;
        }
        return true;
    }
    false
}

pub fn handle_down(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        let sessions_len =
            crate::ui::components::palette::get_session_quick_items(&state.available_sessions)
                .len();
        if state.selected_session_index + 1 < sessions_len {
            state.selected_session_index += 1;
        }
        return true;
    }
    if state.show_provider_menu {
        let providers_len =
            crate::ui::components::palette::get_provider_quick_items(&state.configured_providers)
                .len();
        if state.selected_provider_index + 1 < providers_len {
            state.selected_provider_index += 1;
        }
        return true;
    }
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        if state.selected_mention_hint + 1 < state.mention_hints.len() {
            state.selected_mention_hint += 1;
        }
        return true;
    }
    if state.show_command_hints && !state.command_hints.is_empty() {
        if state.selected_hint + 1 < state.command_hints.len() {
            state.selected_hint += 1;
        }
        return true;
    }
    false
}

fn update_textarea_from_input(state: &mut ChatState) {
    let lines: Vec<String> = state.input.lines().map(|s| s.to_string()).collect();
    state.input_line_count = lines.len();
    let mut textarea = if lines.is_empty() {
        tui_textarea::TextArea::default()
    } else {
        tui_textarea::TextArea::new(lines)
    };
    textarea.set_placeholder_text("Type a message...");
    textarea.set_cursor_line_style(ratatui::style::Style::default());

    // Move cursor to end
    textarea.move_cursor(tui_textarea::CursorMove::Bottom);
    textarea.move_cursor(tui_textarea::CursorMove::End);

    // If input ends with a newline that lines() ate, or if we want to ensure cursor is after the text
    if state.input.ends_with('\n') {
        textarea.insert_newline();
        state.input_line_count += 1;
    }

    state.textarea = textarea;
}

pub fn handle_enter(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        let sessions =
            crate::ui::components::palette::get_session_quick_items(&state.available_sessions);
        if state.selected_session_index < sessions.len() {
            state.pending_palette_action =
                Some(sessions[state.selected_session_index].action.clone());
            state.show_session_menu = false;
        }
        return true;
    }
    if state.show_provider_menu {
        let providers =
            crate::ui::components::palette::get_provider_quick_items(&state.configured_providers);
        if state.selected_provider_index < providers.len() {
            state.pending_palette_action =
                Some(providers[state.selected_provider_index].action.clone());
            state.show_provider_menu = false;
        }
        return true;
    }

    if state.show_mention_hints && !state.mention_hints.is_empty() {
        accept_mention_hint(state);
        return true;
    }

    if state.show_command_hints && !state.command_hints.is_empty() {
        let selected = &state.command_hints[state.selected_hint];
        let cmd_text = extract_hint_primary_text(selected);

        // 如果提取出的命令文本已经是一个完整的命令路径（例如 /memory add），
        // 且它不是一个子命令片段（例如仅 add），则直接替换
        if cmd_text.starts_with('/') {
            state.input = cmd_text.to_string();
            state.input.push(' '); // 自动添加空格方便后续输入参数
        } else if state.input.to_lowercase().starts_with("/mcp") {
            // 特殊处理 /mcp 子命令
            state.input = format!("/mcp {} ", cmd_text);
        } else {
            // 普通文本补全
            state.input = format!("/{} ", cmd_text);
        }

        update_textarea_from_input(state);
        state.show_command_hints = false;
        state.command_hints.clear();
        return true;
    }

    false
}

pub fn handle_tab(state: &mut ChatState) -> bool {
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        accept_mention_hint(state);
        return true;
    }

    if state.show_command_hints && !state.command_hints.is_empty() {
        let selected = &state.command_hints[state.selected_hint];
        let cmd_text = extract_hint_primary_text(selected);

        if cmd_text.starts_with('/') {
            state.input = cmd_text.to_string();
        } else if state.input.to_lowercase().starts_with("/mcp") {
            state.input = format!("/mcp {}", cmd_text);
        } else {
            state.input = format!("/{}", cmd_text);
        }

        update_textarea_from_input(state);
        state.show_command_hints = false;
        state.command_hints.clear();
        return true;
    }

    false
}

/// 接受当前选中的 @ 文件提示。
/// 目录：替换为 `dir/` 并立即刷新提示（下钻）；文件：替换 + 空格并关闭提示。
fn accept_mention_hint(state: &mut ChatState) {
    let selected = state.mention_hints[state.selected_mention_hint].clone();
    let display = extract_mention_path(&selected).to_string();
    if display.is_empty() {
        return;
    }
    let is_dir = display.ends_with('/');
    let at_input = last_at_token_start(&state.input).is_some();

    if at_input {
        replace_at_token_with_path(&mut state.input, &display, is_dir);
        update_textarea_from_input(state);
        if is_dir {
            // 下钻：保持提示打开并刷新子项列表
            state.selected_mention_hint = 0;
            refresh_mention_hints(state);
        } else {
            state.show_mention_hints = false;
            state.mention_hints.clear();
        }
    } else {
        // 裸路径补全（无 @ 前缀）
        let trimmed = display.trim_end_matches('/').to_string();
        state.input = trimmed;
        state.input.push(' ');
        update_textarea_from_input(state);
        state.show_mention_hints = false;
        state.mention_hints.clear();
    }
}

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

use ratatui::widgets::Clear;

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

/// Truncate string to max length
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len - 1])
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
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(hints_height),
        width: input_area.width.min(72),
        height: hints_height,
    };

    let mut start = 0;
    if state.selected_hint >= visible {
        start = state.selected_hint - visible + 1;
    }
    if start + visible > total {
        start = total.saturating_sub(visible);
    }

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
                    format!("{:<18}", cmd_part),
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
                spans.push(Span::styled(desc_part, Style::default().fg(desc_fg).bg(bg)));
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

fn refresh_mention_hints(state: &mut ChatState) {
    // @ token（含 @" 引号形式）优先：token 可跨空格
    if let Some((at, quoted)) = last_at_token_start(&state.input) {
        let frag = at_token_frag(&state.input, at, quoted).trim();
        let hints = crate::ui::services::file_search::search_files(frag);
        if hints.is_empty() {
            // 无匹配：关闭弹窗，不提供可选中的占位行
            state.show_mention_hints = false;
            state.mention_hints.clear();
        } else {
            state.show_mention_hints = true;
            state.mention_hints = hints;
            state.selected_mention_hint = 0;
            state.show_command_hints = false;
        }
        return;
    }

    let token_start = state
        .input
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    let token = state.input.get(token_start..).unwrap_or("");
    let t = token.trim();

    // 修复：避免将以 / 开头的命令（位于输入开头）误判为文件路径
    // 如果 token 位于开头且以 / 起始，优先视为命令
    let is_command_candidate = token_start == 0 && t.starts_with('/');

    // 优化：仅在明确可能是文件路径时触发（包含路径分隔符或以.开头）
    if !t.is_empty()
        && !is_command_candidate
        && (t.contains('/') || t.contains('\\') || t.starts_with('.'))
    {
        let hints = crate::ui::services::file_search::search_files(t);
        if hints.is_empty() {
            state.show_mention_hints = false;
            state.mention_hints.clear();
        } else {
            state.show_mention_hints = true;
            state.mention_hints = hints;
            state.selected_mention_hint = 0;
            state.show_command_hints = false;
        }
        return;
    }

    state.show_mention_hints = false;
    state.mention_hints.clear();
}
