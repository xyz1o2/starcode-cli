use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::core::i18n;
use crate::ui::events::key_context::KeyContext;
use crate::ui::state::{ChatState, PasteKind};
use crate::ui::themes::theme::Theme;

const MAX_RENDER_LINES: usize = 100;
const FOLDED_PREVIEW_MAX_CHARS: usize = 72;

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    if value.is_empty() {
        return i18n::t1("ui.input.empty");
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
        i18n::t1("ui.input.empty")
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
        crate::types::ApprovalMode::AcceptEdits => Color::LightGreen,
        crate::types::ApprovalMode::Plan => Color::Cyan,
        crate::types::ApprovalMode::BypassPermissions => Color::Red,
        // 与 bypass 同属「注意：会被拒」的警示色，但深一档 —— 不是"全部放行"
        // 而是"其余一律拒"。
        crate::types::ApprovalMode::DontAsk => Color::LightRed,
        crate::types::ApprovalMode::Auto => Color::Green,
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
    if state.input_intent == crate::types::InputIntent::Literal {
        return base_color;
    }
    if first_line.starts_with('!') {
        if state.approval_mode == crate::types::ApprovalMode::Plan {
            base_color // Plan mode keeps cyan even with !
        } else {
            Color::Magenta
        }
    } else if first_line.starts_with('/') {
        Color::Blue
    } else {
        base_color
    }
}

/// 粘贴记录保留供撤销恢复；只有正文里仍存在的引用才接管输入绘制。
fn has_active_paste_blocks(state: &ChatState) -> bool {
    state.textarea.lines().iter().any(|line| {
        crate::ui::state::parse_paste_reference(line)
            .is_some_and(|id| state.paste_segments.get(id).is_some())
    })
}

/// 按当前宽度测量视觉行，软换行不修改实际提交的正文。
pub fn calc_input_height(state: &mut ChatState, width: u16) -> u16 {
    let total_lines = state.input_line_count;
    let has_paste_blocks = has_active_paste_blocks(state);
    let can_fold = total_lines >= crate::ui::state::INPUT_FOLD_MIN_LINES;
    let is_folded = can_fold && state.input_folded && !has_paste_blocks;

    let content_lines = if has_paste_blocks {
        total_lines.min(MAX_RENDER_LINES)
    } else if is_folded {
        2
    } else if total_lines > MAX_RENDER_LINES {
        MAX_RENDER_LINES
    } else {
        usize::from(state.textarea.measure(width).content_rows).clamp(1, MAX_RENDER_LINES)
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
    let has_paste_blocks = has_active_paste_blocks(state);
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
                crate::ui::themes::theme::muted_style(),
            ))
            .alignment(ratatui::layout::Alignment::Right),
        );

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // 补全和常驻任务面板仍接收正文输入，其余独占上下文不显示主输入光标。
    let context = KeyContext::of(state);
    let show_cursor = !context.is_exclusive() || context == KeyContext::SuggestionOverlay;

    // Render content based on state
    if has_paste_blocks {
        let lines = build_paste_lines(state);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner_area);
    } else if is_folded {
        let lines = build_folded_lines(state, total_lines);
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner_area);
        if show_cursor && inner_area.width > 0 && inner_area.height > 0 {
            f.set_cursor_position((inner_area.x, inner_area.y));
        }
    } else {
        f.render_widget(&state.textarea, inner_area);
        // 输入库已算好含宽字符和滚动的屏幕坐标，交给终端绘制原生光标。
        // 不向差分缓冲写反色光标块，避免快速删除时在部分终端留下色块。
        if let Some(position) = state.textarea.rendered_cursor_position() {
            if show_cursor && inner_area.contains(position) {
                f.set_cursor_position(position);
            }
        }
    }
}

fn build_paste_lines(state: &ChatState) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
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
                    PasteKind::Image { width, height, .. } => Line::from(vec![
                        prefix,
                        Span::styled(
                            format!(
                                "[{} #{} · {}×{}]",
                                crate::core::i18n::t1("ui.image.label"),
                                id + 1,
                                width,
                                height
                            ),
                            Style::default().fg(theme.primary),
                        ),
                    ]),
                    PasteKind::AttachedImage(image) => Line::from(vec![
                        prefix,
                        Span::styled(
                            format!(
                                "[{} #{} · {}×{}]",
                                crate::core::i18n::t1("ui.image.label"),
                                id + 1,
                                image.width,
                                image.height
                            ),
                            Style::default().fg(Color::Magenta),
                        ),
                    ]),
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
                i18n::t1("ui.input.paste_folded")
                    .replace("{lines}", &total_lines.saturating_sub(1).to_string()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                " (Alt+P 展开)".to_string(),
                crate::ui::themes::theme::muted_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "预览: ".to_string(),
                crate::ui::themes::theme::muted_style(),
            ),
            Span::raw(preview),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApprovalMode;
    use crate::ui::state::ChatState;
    use crate::ui::test_support::assert_render;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_state(mode: ApprovalMode, input: &str) -> ChatState {
        // test_fixture 而非 new()：new() 会读 ~/.star/history.json 和环境变量，
        // 同一份断言在不同机器上结果可能不同。
        let mut state = ChatState::test_fixture();
        state.approval_mode = mode;
        state.textarea.insert_str(input);
        state.input_line_count = state.textarea.lines().len();
        state
    }

    #[test]
    fn test_height_empty() {
        let mut state = make_state(ApprovalMode::Default, "");
        assert_eq!(calc_input_height(&mut state, 60), 3);
    }

    #[test]
    fn test_height_single() {
        let mut state = make_state(ApprovalMode::Default, "hello");
        assert_eq!(calc_input_height(&mut state, 60), 3);
    }

    #[test]
    fn test_height_multi() {
        let mut state = make_state(ApprovalMode::Default, "a\nb\nc");
        assert_eq!(calc_input_height(&mut state, 60), 5);
    }

    /// 测量、绘制和光标共用输入库的软换行布局；缩放不能往正文插入换行。
    #[test]
    fn long_input_wraps_and_resizes_without_changing_text() {
        use ratatui::backend::Backend;

        for text in [
            "abcdefghijklmnopqrstuvwx",
            "中文输入中文输入中文输入",
            "🙂Ａ🙂Ａ🙂Ａ🙂Ａ🙂Ａ",
        ] {
            let mut state = make_state(ApprovalMode::Default, text);
            let narrow_height = calc_input_height(&mut state, 12);
            assert!(narrow_height > 3);
            let mut terminal = Terminal::new(TestBackend::new(12, narrow_height)).unwrap();
            terminal
                .draw(|f| render_input(f, &state, f.area()))
                .unwrap();
            let cursor = terminal.backend_mut().get_cursor_position().unwrap();
            assert!(cursor.y > 1 && cursor.y < narrow_height - 1);
            assert!(terminal.backend().cursor_visible());
            let drawn: String = (1..narrow_height - 1)
                .flat_map(|y| (0..12).map(move |x| (x, y)))
                .filter_map(|pos| terminal.backend().buffer().cell(pos))
                .map(|cell| cell.symbol())
                .collect::<String>()
                .replace(' ', "");
            assert_eq!(drawn, text);
            assert_eq!(state.textarea.lines(), [text]);
            assert_eq!(calc_input_height(&mut state, 100), 3);
            assert_eq!(calc_input_height(&mut state, 12), narrow_height);
            state.set_input_text("");
            assert_eq!(calc_input_height(&mut state, 12), 3);
        }
    }

    #[test]
    fn expanded_input_over_one_hundred_lines_keeps_the_cursor_visible() {
        use ratatui::backend::Backend;

        let text = (0..120)
            .map(|i| format!("line-{i:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = make_state(ApprovalMode::Default, &text);
        state.input_folded = false;
        assert_eq!(calc_input_height(&mut state, 40), 102);
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal
            .draw(|f| render_input(f, &state, f.area()))
            .unwrap();
        let drawn = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(drawn.contains("line-119"));
        assert!(!drawn.contains("line-000"));
        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert!(cursor.y >= 1 && cursor.y < 7);
        assert!(terminal.backend().cursor_visible());
        assert_eq!(state.textarea.lines().join("\n"), text);
    }

    #[test]
    fn test_border_color_default() {
        let state = make_state(ApprovalMode::Default, "");
        // Default 模式的边框色来自主题（theme.input_border），硬编码具体色值
        // 会在换主题时再次失效，所以直接对当前主题断言。
        assert_eq!(
            resolve_border_color(&state),
            state.theme_manager.current().input_border
        );
    }

    #[test]
    fn test_border_color_plan() {
        let state = make_state(ApprovalMode::Plan, "");
        assert_eq!(resolve_border_color(&state), Color::Cyan);
    }

    #[test]
    fn test_border_color_bypass_permissions() {
        let state = make_state(ApprovalMode::BypassPermissions, "");
        assert_eq!(resolve_border_color(&state), Color::Red);
    }

    #[test]
    fn test_border_color_bash() {
        let state = make_state(ApprovalMode::Default, "!ls");
        assert_eq!(resolve_border_color(&state), Color::Magenta);
    }

    #[test]
    fn markdown_hash_keeps_the_normal_input_border() {
        for mode in [ApprovalMode::Default, ApprovalMode::Plan] {
            let normal = make_state(mode.clone(), "ordinary message");
            let markdown = make_state(mode, "# Markdown heading");
            assert_eq!(
                resolve_border_color(&markdown),
                resolve_border_color(&normal)
            );
        }
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

    /// 连续删除必须清掉旧字符与光标样式，不能只验证一张空输入快照。
    #[test]
    fn deleting_wide_input_clears_previous_frames() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut state = make_state(
            ApprovalMode::Default,
            "中文删除🙂 abc 全角ＡＢＣ ".repeat(12).as_str(),
        );
        let mut terminal = Terminal::new(TestBackend::new(180, 5)).unwrap();
        loop {
            state.input_line_count = state.textarea.lines().len();
            terminal
                .draw(|f| render_input(f, &state, f.area()))
                .unwrap();
            assert!(terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| { !cell.modifier.contains(Modifier::REVERSED) }));
            assert!(terminal.backend().cursor_visible());
            use ratatui::backend::Backend;
            assert_eq!(
                terminal.backend_mut().get_cursor_position().unwrap(),
                state.textarea.rendered_cursor_position().unwrap(),
            );
            if state.textarea.lines() == [""] {
                break;
            }
            assert!(state
                .textarea
                .input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)));
        }
        let expected = make_state(ApprovalMode::Default, "");
        let mut clean_terminal = Terminal::new(TestBackend::new(180, 5)).unwrap();
        clean_terminal
            .draw(|f| render_input(f, &expected, f.area()))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer(),
            clean_terminal.backend().buffer()
        );
    }

    /// 独占确认或弹窗接管键盘时，普通与折叠输入都必须交还原生光标。
    #[test]
    fn native_cursor_follows_input_focus() {
        use crate::ui::state::modal::Modal;

        let cases: &[(fn(&mut ChatState), bool)] = &[
            (|_| {}, true),
            (|s| s.show_command_hints = true, true),
            (|s| s.task_panel.is_visible = true, true),
            (|s| s.vim_enabled = true, true),
            (|s| s.is_awaiting_confirmation = true, false),
            (|s| s.show_clear_confirmation = true, false),
            (|s| s.pending_model_confirmation = true, false),
            (|s| s.bg_agent_selection = Some(0), false),
            (|s| s.show_help = true, false),
            (|s| s.push_modal(Modal::Palette), false),
            (|s| s.push_modal(Modal::InputModal), false),
        ];
        for folded in [false, true] {
            let mut terminal = Terminal::new(TestBackend::new(60, 5)).unwrap();
            for (configure, visible) in cases {
                let mut focused_state = make_state(
                    ApprovalMode::Default,
                    if folded {
                        "a\nb\nc\nd\ne\nf\ng\nh"
                    } else {
                        "中文🙂"
                    },
                );
                focused_state.input_folded = folded;
                configure(&mut focused_state);
                terminal
                    .draw(|f| render_input(f, &focused_state, f.area()))
                    .unwrap();
                assert_eq!(
                    terminal.backend().cursor_visible(),
                    *visible,
                    "context={:?}, folded={folded}",
                    KeyContext::of(&focused_state),
                );
            }
        }
    }

    // ── 渲染快照 ────────────────────────────────────────────────────────────
    //
    // `resolve_border_color` 随 approval mode 给三种边框色，底下 4 个
    // `test_border_color_*` 只能断言"取到了某个 Color"，看不出这个颜色实际
    // 落在哪几格。快照把 cell 位置和样式都钉进基线：边框色改动、多行输入的
    // 框高、placeholder 的落位，任何一处偏移都会变成一条 diff。

    /// 空输入 —— 框架与 placeholder。
    #[test]
    fn snapshot_default_placeholder() {
        let state = make_state(ApprovalMode::Default, "");
        assert_render!("chat_input_default_empty", 44, 5, |f| {
            render_input(f, &state, f.area())
        });
    }

    /// 单行输入 —— Plan 模式边框为 cyan。
    #[test]
    fn snapshot_plan_single_line() {
        let state = make_state(ApprovalMode::Plan, "hello world");
        assert_render!("chat_input_plan_single", 44, 5, |f| {
            render_input(f, &state, f.area())
        });
    }

    /// 多行输入 —— 框高随行数增长（`calc_input_height`），bypassPermissions 模式边框为 red。
    #[test]
    fn snapshot_bypass_multiline() {
        let state = make_state(
            ApprovalMode::BypassPermissions,
            "first line\nsecond line\nthird",
        );
        assert_render!("chat_input_bypass_multiline", 44, 9, |f| {
            render_input(f, &state, f.area())
        });
    }

    /// `!` 前缀触发 shell 配色（magenta 边框）。
    #[test]
    fn snapshot_bash_prefix() {
        let state = make_state(ApprovalMode::Default, "!ls -la");
        assert_render!("chat_input_prefix_bash", 44, 5, |f| {
            render_input(f, &state, f.area())
        });
    }

    /// `/` 前缀触发命令配色（blue 边框）。
    ///
    /// 和 `snapshot_bash_prefix` 拆成两个测试而不是一个循环：基线缺失时断言会先失败
    /// 一次（好让人先看过内容再接受），一个测试管两份基线就得跑三轮才转绿。
    #[test]
    fn snapshot_command_prefix() {
        let state = make_state(ApprovalMode::Default, "/help");
        assert_render!("chat_input_prefix_command", 44, 5, |f| {
            render_input(f, &state, f.area())
        });
    }
}
