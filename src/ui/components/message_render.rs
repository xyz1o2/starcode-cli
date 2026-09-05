use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::ChatEntryType;
use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;

pub(crate) fn render_non_tool_entry_blocks(
    state: &ChatState,
    entry: &crate::types::ChatEntry,
    entry_idx: usize,
    wrap_width: usize,
) -> Vec<Vec<Line<'static>>> {
    let mut blocks = Vec::new();
    let theme = state.theme_manager.current();
    let is_status_line = |s: &str| {
        let t = s.trim();
        crate::core::i18n::status_prefixes()
            .iter()
            .any(|prefix| t.starts_with(prefix))
    };

    let (global_elapsed_ms, _dot_frame) = super::chat_history::animation_state(state);
    let is_streaming_now = entry.is_streaming == Some(true);
    // Use frozen completion time for finished blocks so their timers
    // don't keep ticking when a new thinking block starts streaming.
    let elapsed_ms = if is_streaming_now {
        global_elapsed_ms
    } else {
        entry.reasoning_finished_elapsed_ms.unwrap_or(0)
    };

    let cancelling = state.cancelling_since.is_some();

    match entry.entry_type {
        ChatEntryType::Assistant if entry.is_welcome => {
            // 欢迎抬头走专用渲染：左侧标记 + 右侧信息，从 state 现算，
            // 不能走 markdown（markdown 排不出"左标记右三行"的并排布局）
            blocks.push(super::welcome_header::welcome_header_lines(state));
        }
        ChatEntryType::Assistant => {
            let display_content = crate::ui::utils::text::sanitize_for_tui(&entry.content);
            if let Some(reasoning) = &entry.reasoning_content {
                let reasoning_display = crate::ui::utils::text::sanitize_for_tui(reasoning);
                // 过滤掉空的或无意义的 thinking 内容
                let reasoning_trimmed = reasoning_display.trim();
                let is_meaningful = !reasoning_trimmed.is_empty()
                    && reasoning_trimmed != "empty"
                    && reasoning_trimmed.len() > 5; // 至少 5 个字符才算有意义

                if is_meaningful {
                    let is_expanded = state.expanded_thinking_indices.contains(&entry_idx);

                    // 间距规则（对标 Claude Code：只有"块的顶部 margin"一个所有者）：
                    // thinking 块前不另加空行 — entry 级 leading blank 已提供分隔，
                    // 此处再加会叠加成双空行

                    if is_expanded {
                        let thinking_label = if cancelling {
                            crate::core::i18n::t("ui.thinking.cancelling", "Canceling", "Canceling")
                        } else {
                            crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                        };
                        let dots = "...";
                        let token_unit =
                            crate::core::i18n::t("ui.thinking.token_unit", "tokens", "tokens");
                        let token_count = state.token_count.max(reasoning_display.len() as u32 / 4);

                        let mut header_spans = vec![
                            Span::styled(
                                "✻ ",
                                Style::default()
                                    .fg(theme.thinking_fg)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                            Span::styled(
                                format!("{}{}", thinking_label, dots),
                                Style::default()
                                    .fg(theme.thinking_fg)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                        ];

                        if elapsed_ms > 0 {
                            header_spans.push(Span::styled(
                                format!(" {}", super::chat_history::format_elapsed(elapsed_ms)),
                                Style::default().fg(theme.thinking_fg),
                            ));
                        }
                        if token_count > 0 {
                            header_spans.push(Span::styled(
                                format!(
                                    " · {} {}",
                                    super::chat_history::format_token_count(token_count),
                                    token_unit
                                ),
                                Style::default().fg(theme.thinking_fg),
                            ));
                        }

                        blocks.push(vec![Line::from(header_spans)]);

                        let mut thinking_lines = Vec::new();
                        for line in reasoning_display.lines() {
                            if line.trim().is_empty() {
                                thinking_lines.push(Line::from(""));
                            } else {
                                // 对 thinking 内容进行换行处理
                                let wrapped_lines = crate::ui::utils::render::wrap_text_to_width(
                                    line,
                                    wrap_width.saturating_sub(2), // 减去 "│ " 前缀宽度
                                );
                                for (i, wrapped) in wrapped_lines.iter().enumerate() {
                                    let mut spans = Vec::new();
                                    if i == 0 {
                                        // 第一行带 │ 前缀
                                        spans.push(Span::styled(
                                            "│ ",
                                            Style::default().fg(theme.thinking_fg),
                                        ));
                                    } else {
                                        // 续行缩进对齐
                                        spans.push(Span::styled("  ", Style::default()));
                                    }
                                    spans.push(Span::styled(
                                        wrapped.to_string(),
                                        Style::default()
                                            .fg(theme.thinking_fg)
                                            .add_modifier(Modifier::ITALIC),
                                    ));
                                    thinking_lines.push(Line::from(spans));
                                }
                            }
                        }
                        if thinking_lines.is_empty() && (is_streaming_now || cancelling) {
                            let placeholder_label = if cancelling {
                                crate::core::i18n::t(
                                    "ui.thinking.cancelling",
                                    "Canceling",
                                    "Canceling",
                                )
                            } else {
                                crate::core::i18n::t(
                                    "ui.thinking.placeholder",
                                    "Thinking",
                                    "Thinking",
                                )
                            };
                            thinking_lines.push(Line::from(vec![
                                Span::styled("│ ", Style::default().fg(theme.thinking_fg)),
                                Span::styled(
                                    format!("{}{}", placeholder_label, dots),
                                    Style::default()
                                        .fg(theme.subtle)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                        blocks.push(thinking_lines);
                    } else {
                        // Collapsed view: only show header line with thinking label and elapsed time
                        // No preview lines — user clicks to expand and see content
                        let thinking_label = if cancelling {
                            crate::core::i18n::t("ui.thinking.cancelling", "Canceling", "Canceling")
                        } else {
                            crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                        };
                        let dots = "...";

                        let mut header_spans = vec![
                            Span::styled(
                                "✻ ",
                                Style::default()
                                    .fg(theme.thinking_fg)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                            Span::styled(
                                format!("{}{}", thinking_label, dots),
                                Style::default()
                                    .fg(theme.thinking_fg)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                        ];
                        if elapsed_ms > 0 {
                            header_spans.push(Span::styled(
                                format!(" {}", super::chat_history::format_elapsed(elapsed_ms)),
                                Style::default().fg(theme.thinking_fg),
                            ));
                        }

                        blocks.push(vec![Line::from(header_spans)]);
                    }
                    // 顶部 margin 规则：正文块的"顶部空行"是 thinking 与正文之间
                    // 唯一的间隔来源；只在正文确实会渲染时才加，避免条目尾部悬空行
                    // 与下一个条目的 leading blank 叠加成双空行
                    if !entry.content.trim().is_empty() {
                        blocks.push(vec![Line::from("")]);
                    }
                }
            }

            // Only show "Thinking..." placeholder when the model is actually
            // producing thinking content (thinking_started_at is set by the
            // Thinking message handler). This prevents non-thinking models
            // from briefly flashing "Thinking..." before text arrives.
            if entry.content.trim().is_empty()
                && (is_streaming_now || cancelling)
                && entry.reasoning_content.is_none()
                && state.thinking_started_at.is_some()
            {
                let label = if cancelling {
                    crate::core::i18n::t("ui.thinking.cancelling", "Canceling", "Canceling")
                } else {
                    crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                };
                let dots = "...";
                let mut stream_spans = vec![
                    Span::styled(
                        "✻ ",
                        Style::default()
                            .fg(theme.thinking_fg)
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::styled(
                        format!("{}{}", label, dots),
                        Style::default()
                            .fg(theme.thinking_fg)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ];
                if elapsed_ms > 0 {
                    stream_spans.push(Span::styled(
                        format!(" {}", super::chat_history::format_elapsed(elapsed_ms)),
                        Style::default().fg(theme.thinking_fg),
                    ));
                }
                blocks.push(vec![Line::from(stream_spans)]);
            } else if is_status_line(&display_content) {
                blocks.push(vec![Line::from(Span::styled(
                    display_content.trim().to_string(),
                    Style::default().fg(theme.secondary),
                ))]);
            } else if !display_content.trim().is_empty() {
                // 流式光标 "▌" 是在折行之后追加的，必须先给它留出 1 列，
                // 否则正好排满的那一行会超出可用宽度、末字被右边界截掉，
                // 每来一个 token 就重新截一次，看起来像整行在抖动
                let body_width = if is_streaming_now {
                    wrap_width.saturating_sub(1)
                } else {
                    wrap_width
                };
                let mut lines = crate::ui::utils::render::build_assistant_body_block(
                    &display_content,
                    is_streaming_now,
                    body_width,
                );
                // If markdown parsing returned empty lines but content exists,
                // render as plain text (handles edge cases like thinking-only responses)
                if lines.is_empty() && !display_content.trim().is_empty() {
                    lines.push(Line::from(Span::styled(
                        display_content.trim().to_string(),
                        Style::default().fg(theme.foreground),
                    )));
                }
                // No prefix for assistant messages — clean layout like Claude Code
                if is_streaming_now {
                    if let Some(last) = lines.last_mut() {
                        last.spans.push(Span::styled(
                            "▌",
                            Style::default()
                                .fg(theme.primary)
                                .add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        lines.push(Line::from(Span::styled(
                            "▌",
                            Style::default()
                                .fg(theme.primary)
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                }
                // 每条回复的成本显示已移除（不再追加 "$x.xxxx"）
                blocks.push(lines);
            }
        }
        ChatEntryType::User => {
            const USER_PREFIX: &str = "> ";
            let user_prefix = Span::styled(USER_PREFIX, Style::default().fg(theme.user_fg));
            let display_content = crate::ui::utils::text::sanitize_for_tui(&entry.content);
            // "> " 是折行之后才加到首行上的，所以正文必须按 wrap_width - 2 折行，
            // 否则首行固定超出 2 列被右边界截掉；续行同样缩进 2 列，
            // 多行输入才会整块对齐在提示符右侧
            let indent_w = USER_PREFIX.len(); // ASCII，2 列
            let mut user_lines = crate::ui::utils::render::build_user_body_block(
                &display_content,
                wrap_width.saturating_sub(indent_w),
            );
            for (idx, line) in user_lines.iter_mut().enumerate() {
                let lead = if idx == 0 {
                    user_prefix.clone()
                } else {
                    Span::raw(" ".repeat(indent_w))
                };
                let mut new_spans = vec![lead];
                new_spans.extend(std::mem::take(&mut line.spans));
                *line = Line::from(new_spans);
            }
            blocks.push(user_lines);
        }
        _ => {}
    }

    // Only show "Thinking..." when the model actually supports thinking
    // (detected from model list or model name). For non-thinking models, don't show any placeholder.
    let is_thinking = state
        .current_model_supports_thinking
        .unwrap_or_else(|| crate::core::config::models::is_thinking_model(&state.current_model));
    if blocks.is_empty() && is_streaming_now && is_thinking {
        let label = crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking");
        let dots = "...";
        let mut thinking_spans = vec![
            Span::styled(
                "✻ ",
                Style::default()
                    .fg(theme.thinking_fg)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                format!("{}{}", label, dots),
                Style::default()
                    .fg(theme.thinking_fg)
                    .add_modifier(Modifier::ITALIC),
            ),
        ];
        if elapsed_ms > 0 {
            thinking_spans.push(Span::styled(
                format!(" {}", super::chat_history::format_elapsed(elapsed_ms)),
                Style::default().fg(theme.thinking_fg),
            ));
        }
        blocks.push(vec![Line::from(thinking_spans)]);
    }

    blocks
}

fn recent_thinking_preview_lines(
    reasoning: &str,
    max_width: usize,
    max_lines: usize,
) -> Vec<String> {
    if reasoning.is_empty() || max_lines == 0 {
        return Vec::new();
    }

    let mut visual_lines = Vec::new();
    for raw_line in reasoning.lines() {
        push_wrapped_preview_line(&mut visual_lines, raw_line.trim(), max_width);
    }

    if visual_lines.is_empty() {
        return Vec::new();
    }

    let omitted = visual_lines.len().saturating_sub(max_lines);
    let mut preview: Vec<String> = visual_lines.into_iter().skip(omitted).collect();
    if omitted > 0 {
        if let Some(first) = preview.first_mut() {
            *first = format!("...{}", first);
        }
    }
    preview
}

fn push_wrapped_preview_line(out: &mut Vec<String>, line: &str, max_width: usize) {
    if line.is_empty() {
        return;
    }

    let max_width = max_width.max(1);
    let mut current = String::new();
    let mut width = 0usize;

    for ch in line.chars() {
        // 与 ratatui 的缓冲区度量保持一致（见 render::display_width）
        let ch_width = crate::ui::utils::render::char_display_width(ch).max(1);
        if width > 0 && width + ch_width > max_width {
            out.push(current);
            current = String::new();
            width = 0;
        }
        current.push(ch);
        width += ch_width;
    }

    if !current.is_empty() {
        out.push(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn thinking_label_has_no_black_bg() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("hello thinking content here".to_string());
        entry.is_streaming = Some(true);
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let backend = TestBackend::new(80, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let blocks =
                    render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70);
                let mut lines: Vec<Line> = Vec::new();
                for b in &blocks {
                    for l in b {
                        lines.push(l.clone());
                    }
                }
                f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let mut black_bg = 0usize;
        for y in 0..12 {
            for x in 0..80 {
                if let Some(cell) = buf.cell((x, y)) {
                    if cell.style().bg == Some(Color::Black) {
                        black_bg += 1;
                    }
                }
            }
        }
        assert_eq!(
            black_bg, 0,
            "found {} cells with black background",
            black_bg
        );
    }

    /// 折行不得吞掉词间空格：思考块渲染后的词序列必须与原文一致。
    #[test]
    fn thinking_content_keeps_word_spacing_across_wraps() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content =
            Some("The user wants the words spaced out properly here".to_string());
        state.chat_history.push(entry);
        // ChatState::new() 自带欢迎条目，思考条目在其后
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 30);
        // blocks[0] 是 header，blocks[1] 是思考正文
        let rendered: Vec<String> = blocks[1]
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert!(rendered.len() > 1, "expected wrapping: {:?}", rendered);

        let words: Vec<&str> = rendered
            .iter()
            .flat_map(|l| l.split_whitespace())
            .filter(|w| *w != "│")
            .collect();
        assert_eq!(
            words,
            vec!["The", "user", "wants", "the", "words", "spaced", "out", "properly", "here"],
            "rendered: {:?}",
            rendered
        );
    }
}
