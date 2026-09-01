use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::core::i18n;
use crate::ui::state::{ChatState, PasteKind};
use crate::ui::themes::theme::Theme;

const MAX_RENDER_LINES: usize = 100;
const FOLDED_PREVIEW_MAX_CHARS: usize = 72;

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    if value.is_empty() {
        return i18n::t("ui.input.empty", "(空内容)", "(empty)");
    }
    let mut out = String::new();
    let mut count = 0usize;
    for ch in value.chars() {
        if count >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
        count += 1;
    }
    if out.is_empty() {
        i18n::t("ui.input.empty", "(空内容)", "(empty)")
    } else {
        out
    }
}

/// Calculate border color based on approval mode and input prefix
pub fn resolve_border_color(state: &ChatState) -> Color {
    let theme = state.theme_manager.current();
    let first_line = state
        .textarea
        .lines()
        .first()
        .map(|s| s.trim())
        .unwrap_or("");
    let base_color = match state.approval_mode {
        crate::types::ApprovalMode::Default => theme.input_border,
        crate::types::ApprovalMode::Plan => Color::Cyan,
        crate::types::ApprovalMode::Yolo => Color::Red,
    };
    // Send animation: brief cyan flash when message is sent
    if let Some(since) = state.send_animation_since {
        if since.elapsed() < std::time::Duration::from_millis(300) {
            return Color::Cyan;
        }
    }
    // Vim mode override: green border in Normal mode
    if state.vim_enabled && state.vim_state.is_normal_mode() {
        return Color::Green;
    }
    if first_line.starts_with('!') {
        if state.approval_mode == crate::types::ApprovalMode::Plan {
            base_color // Plan mode keeps cyan even with !
        } else {
            Color::Magenta
        }
    } else if first_line.starts_with('#') {
        Color::Yellow
    } else if first_line.starts_with('/') {
        Color::Blue
    } else {
        base_color
    }
}

/// Calculate the required height for the input area
pub fn calc_input_height(state: &ChatState) -> u16 {
    let total_lines = state.input_line_count;
    let has_paste_blocks = !state.paste_segments.is_empty();
    let can_fold = total_lines >= crate::ui::state::INPUT_FOLD_MIN_LINES;
    let is_folded = can_fold && state.input_folded && !has_paste_blocks;

    let content_lines = if has_paste_blocks {
        total_lines.min(MAX_RENDER_LINES)
    } else if is_folded {
        2
    } else {
        total_lines.min(MAX_RENDER_LINES).max(1)
    };

    // Block with TOP + BOTTOM borders adds 2 lines
    (content_lines as u16) + 2
}

/// Render the input box with proper Block borders.
/// Uses textarea widget for proper cursor handling.
pub fn render_input(f: &mut Frame<'_>, state: &ChatState, area: Rect) {
    // Clear the area first, then overwrite with terminal's default background
    f.render_widget(Clear, area);
    // Use a block with no background to let terminal's own color show through
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    let border_color = resolve_border_color(state);

    let all_lines = state.textarea.lines();
    let total_lines = state.input_line_count;
    let is_truncated = total_lines > MAX_RENDER_LINES;
    let has_paste_blocks = !state.paste_segments.is_empty();
    let can_fold = total_lines >= crate::ui::state::INPUT_FOLD_MIN_LINES;
    let is_folded = can_fold && state.input_folded && !has_paste_blocks;

    // Block with cursor position in top-right corner
    let (cursor_row, cursor_col) = state.textarea.cursor();
    let line_count = state.textarea.lines().len();
    let count_label = if line_count > 1 {
        format!("{}:{}", cursor_row + 1, cursor_col + 1)
    } else {
        format!("{}:{}", cursor_row + 1, cursor_col + 1)
    };
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(border_color))
        .title_top(
            Line::from(Span::styled(
                format!(" {} ", count_label),
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(ratatui::layout::Alignment::Right),
        );

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // Render content based on state
    if has_paste_blocks {
        let lines = build_paste_lines(state);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner_area);
    } else if is_folded {
        let lines = build_folded_lines(state, total_lines);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner_area);
        if inner_area.width > 0 && inner_area.height > 0 {
            f.set_cursor_position((inner_area.x, inner_area.y));
        }
    } else if is_truncated {
        let lines = build_truncated_lines(state);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner_area);
    } else {
        // Render textarea widget directly — it handles cursor internally
        f.render_widget(&state.textarea, inner_area);
    }
}

fn build_paste_lines(state: &ChatState) -> Vec<Line<'static>> {
    let lines = state.textarea.lines();
    let mut rendered: Vec<Line> = Vec::new();
    for line in lines.iter().take(MAX_RENDER_LINES) {
        if let Some(id) = crate::ui::state::parse_paste_reference(line) {
            if let Some(seg) = state.paste_segments.get(id) {
                let prefix = Span::styled(
                    "› ".to_string(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                );
                let line = match &seg.kind {
                    PasteKind::Text => {
                        let char_count = seg.content.len();
                        Line::from(vec![
                            prefix,
                            Span::styled(
                                "[Pasted · ~".to_string(),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled(
                                format!("{} lines", seg.line_count),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(" · ".to_string(), Style::default().fg(Color::Cyan)),
                            Span::styled(
                                format!("{} chars", char_count),
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("]".to_string(), Style::default().fg(Color::Cyan)),
                        ])
                    }
                    PasteKind::Image {
                        path,
                        width,
                        height,
                    } => {
                        let fname = std::path::Path::new(path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(path.as_str())
                            .to_string();
                        Line::from(vec![
                            prefix,
                            Span::styled(
                                format!("[📷 {}×{} ", width, height),
                                Style::default().fg(Color::Magenta),
                            ),
                            Span::styled(
                                fname,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("]".to_string(), Style::default().fg(Color::Magenta)),
                        ])
                    }
                    PasteKind::Files(paths) => {
                        let names: Vec<String> = paths
                            .iter()
                            .take(3)
                            .map(|p| {
                                std::path::Path::new(p.as_str())
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(p.as_str())
                                    .to_string()
                            })
                            .collect();
                        let extra = if paths.len() > 3 {
                            format!(" +{}", paths.len() - 3)
                        } else {
                            String::new()
                        };
                        Line::from(vec![
                            prefix,
                            Span::styled(
                                format!(
                                    "[📁 {} file{} · ",
                                    paths.len(),
                                    if paths.len() == 1 { "" } else { "s" }
                                ),
                                Style::default().fg(Color::Blue),
                            ),
                            Span::styled(
                                format!("{}{}", names.join(", "), extra),
                                Style::default()
                                    .fg(Color::LightBlue)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("]".to_string(), Style::default().fg(Color::Blue)),
                        ])
                    }
                };
                rendered.push(line);
            } else {
                rendered.push(Line::raw(line.to_string()));
            }
        } else {
            rendered.push(Line::raw(line.to_string()));
        }
    }
    rendered
}

fn build_folded_lines(state: &ChatState, total_lines: usize) -> Vec<Line<'static>> {
    let all_lines = state.textarea.lines();
    let first_non_empty = all_lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.as_str())
        .unwrap_or("");
    let preview = truncate_for_preview(first_non_empty, FOLDED_PREVIEW_MAX_CHARS);
    vec![
        Line::from(vec![
            Span::styled(
                i18n::t(
                    "ui.input.paste_folded",
                    "[已粘贴 +{lines} 行]",
                    "[Pasted +{lines} lines]",
                )
                .replace("{lines}", &total_lines.saturating_sub(1).to_string()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                " (Alt+P 展开)".to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("预览: ".to_string(), Style::default().fg(Color::DarkGray)),
            Span::raw(preview),
        ]),
    ]
}

fn build_truncated_lines(state: &ChatState) -> Vec<Line<'static>> {
    state
        .textarea
        .lines()
        .iter()
        .take(MAX_RENDER_LINES)
        .map(|line| Line::raw(line.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApprovalMode;
    use crate::ui::state::ChatState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_state(mode: ApprovalMode, input: &str) -> ChatState {
        let mut state = ChatState::new();
        state.approval_mode = mode;
        state.textarea.insert_str(input);
        state.input_line_count = state.textarea.lines().len();
        state
    }

    #[test]
    fn test_height_empty() {
        let state = make_state(ApprovalMode::Default, "");
        assert_eq!(calc_input_height(&state), 3);
    }

    #[test]
    fn test_height_single() {
        let state = make_state(ApprovalMode::Default, "hello");
        assert_eq!(calc_input_height(&state), 3);
    }

    #[test]
    fn test_height_multi() {
        let state = make_state(ApprovalMode::Default, "a\nb\nc");
        assert_eq!(calc_input_height(&state), 5);
    }

    #[test]
    fn test_border_color_default() {
        let state = make_state(ApprovalMode::Default, "");
        assert_eq!(resolve_border_color(&state), Color::DarkGray);
    }

    #[test]
    fn test_border_color_plan() {
        let state = make_state(ApprovalMode::Plan, "");
        assert_eq!(resolve_border_color(&state), Color::Cyan);
    }

    #[test]
    fn test_border_color_yolo() {
        let state = make_state(ApprovalMode::Yolo, "");
        assert_eq!(resolve_border_color(&state), Color::Red);
    }

    #[test]
    fn test_border_color_bash() {
        let state = make_state(ApprovalMode::Default, "!ls");
        assert_eq!(resolve_border_color(&state), Color::Magenta);
    }

    #[test]
    fn test_border_color_command() {
        let state = make_state(ApprovalMode::Default, "/help");
        assert_eq!(resolve_border_color(&state), Color::Blue);
    }

    #[test]
    fn test_render_shows_content() {
        let state = make_state(ApprovalMode::Default, "hello world");
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 60, 5);
                render_input(f, &state, area);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        // Check that "hello" text appears in the buffer
        let mut found_hello = false;
        for y in 0..5 {
            let mut line = String::new();
            for x in 0..60 {
                line.push_str(buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
            }
            if line.contains("hello") {
                found_hello = true;
                break;
            }
        }
        assert!(found_hello, "Text 'hello' should appear in rendered output");
    }
}
