use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::ChatEntryType;
use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;

/// 渲染非工具条目（assistant / user）的文本块。
///
/// `is_sub_transcript`：agent 组展开视图里的子转录条目不走「完成后隐藏」规则，
/// thinking 折叠头始终可见（它们不参与主历史的 30 秒宽限窗口）。
pub(crate) fn render_non_tool_entry_blocks(
    state: &ChatState,
    entry: &crate::types::ChatEntry,
    entry_idx: usize,
    wrap_width: usize,
    is_sub_transcript: bool,
) -> Vec<Vec<Line<'static>>> {
    let mut blocks = Vec::new();
    let theme = state.theme_manager.current();
    let is_status_line = |s: &str| {
        let t = s.trim();
        crate::core::i18n::status_prefixes()
            .iter()
            .any(|prefix| t.starts_with(prefix))
    };

    let is_streaming_now = entry.is_streaming == Some(true);
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
                    // 对标 Claude Code：本进程内完成的 thinking 块只在 30 秒宽限窗口内
                    // 可见，之后隐藏；transcript 模式（Ctrl+O）或手动展开时重新显示。
                    // 恢复的历史条目没有完成时刻（serde skip），不适用隐藏窗口——
                    // 否则重启后所有思考块会直接消失。
                    let within_grace = entry
                        .reasoning_finished_at
                        .map(|t| t.elapsed() < std::time::Duration::from_secs(30))
                        .unwrap_or(false);
                    let is_expanded = state.is_transcript_mode
                        || state.expanded_thinking_indices.contains(&entry_idx);
                    let is_persistent_history = entry.reasoning_finished_at.is_none();
                    let is_visible = is_expanded
                        || is_sub_transcript
                        || is_streaming_now
                        || is_persistent_history
                        || within_grace;

                    // 间距规则（对标 Claude Code：只有"块的顶部 margin"一个所有者）：
                    // thinking 块前不另加空行 — entry 级 leading blank 已提供分隔，
                    // 此处再加会叠加成双空行

                    if is_visible {
                        // 标签颜色随状态变化（对标 Claude Code：思考中亮、完成后
                        // 降为弱化色）。思考中用 primary + 加粗 + 斜体：
                        // - primary 各主题都是鲜艳强调色（secondary 在 10/13 套
                        //   主题里和 inactive 同色，不能用）
                        // - 加粗是主题无关的兜底：就算某主题色值接近，粗细差异
                        //   也肉眼可见
                        // 与流式光标 ▌（primary+BOLD）同一视觉语言。
                        let label_style = if is_streaming_now || cancelling {
                            Style::default()
                                .fg(theme.primary)
                                .add_modifier(Modifier::ITALIC | Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.inactive)
                        };

                        // 头部随状态变化（对标 Claude Code）：
                        // 思考中   → `∴ Thinking…`（不带展开提示，结束时才出现）
                        // 已完成   → `∴ Thought for 14s`（折叠态补展开提示）
                        // 取消中   → `∴ Canceling…`
                        let header_text = if cancelling {
                            format!(
                                "{}…",
                                crate::core::i18n::t(
                                    "ui.thinking.cancelling",
                                    "Canceling",
                                    "Canceling"
                                )
                            )
                        } else if is_streaming_now {
                            format!(
                                "{}…",
                                crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                            )
                        } else {
                            match entry.reasoning_finished_elapsed_ms {
                                Some(ms) => format!(
                                    "{} {}",
                                    crate::core::i18n::t(
                                        "ui.thinking.thought_for",
                                        "思考了",
                                        "Thought for"
                                    ),
                                    format_thinking_duration(ms),
                                ),
                                // 历史会话恢复的条目没有冻结耗时，退回普通标签
                                None => crate::core::i18n::t(
                                    "ui.thinking.label",
                                    "Thinking",
                                    "Thinking",
                                ),
                            }
                        };

                        if is_expanded {
                            // 展开态：头部（不重复展开提示）
                            blocks.push(vec![Line::from(vec![
                                Span::styled("∴ ", label_style),
                                Span::styled(header_text, label_style),
                            ])]);

                            // 内容：dim 色 + 缩进 2 列，不走 markdown 段落排版。
                            // 思考文本（尤其中文模型）行间普遍用空行分隔，markdown
                            // 会把每行渲染成独立段落、段间空一行，整块稀疏得没法看。
                            // 这里改回逐行渲染：保留每个思考步骤一行及其前导缩进；
                            // 长行折行时续行继承源行缩进，源空行不渲染，行间不额外加空行。
                            let content_width = wrap_width.saturating_sub(2);
                            let thinking_style = if is_streaming_now || cancelling {
                                Style::default()
                                    .fg(theme.primary)
                                    .add_modifier(Modifier::ITALIC)
                            } else {
                                Style::default().fg(theme.inactive)
                            };
                            let mut thinking_lines = Vec::new();
                            for raw_line in reasoning_display.lines() {
                                let line = raw_line.trim_end();
                                if line.trim().is_empty() {
                                    continue;
                                }
                                let indent = line.len() - line.trim_start().len();
                                let indent_width =
                                    crate::ui::utils::render::display_width(&line[..indent]);
                                let first_width = content_width.saturating_sub(indent_width);
                                let rest_width = first_width.max(2);
                                let wrapped = crate::ui::utils::render::wrap_char_ranges(
                                    &line.chars().collect::<Vec<_>>(),
                                    first_width,
                                    rest_width,
                                )
                                .into_iter()
                                .map(|(start, end)| {
                                    line.chars()
                                        .skip(start)
                                        .take(end - start)
                                        .collect::<String>()
                                });
                                for (line_idx, w) in wrapped.into_iter().enumerate() {
                                    let prefix = if line_idx == 0 {
                                        "  ".to_string()
                                    } else {
                                        format!("  {}", &line[..indent])
                                    };
                                    let mut spans = vec![Span::raw(prefix)];
                                    spans.extend(thinking_line_spans(
                                        &w,
                                        thinking_style,
                                        line_idx == 0,
                                    ));
                                    // 围栏行等剥掉标记后为空：整行跳过，不留悬空行
                                    if spans.len() > 1 {
                                        thinking_lines.push(Line::from(spans));
                                    }
                                }
                            }
                            if thinking_lines.is_empty() {
                                let mut spans = vec![Span::raw("  ")];
                                spans.extend(thinking_line_spans(
                                    reasoning_trimmed,
                                    thinking_style,
                                    true,
                                ));
                                thinking_lines.push(Line::from(spans));
                            }
                            blocks.push(thinking_lines);
                        } else {
                            // 折叠态：完成的块补展开提示；思考中不显示提示
                            // （进行中的耗时由状态行 `· thinking Xs` 负责）
                            let mut header_spans = vec![
                                Span::styled("∴ ", label_style),
                                Span::styled(header_text, label_style),
                            ];
                            if !is_streaming_now && !cancelling {
                                header_spans.push(Span::styled(
                                    format!(
                                        " {}",
                                        crate::core::i18n::t(
                                            "ui.thinking.expand_hint",
                                            "(ctrl+o 展开思考)",
                                            "(ctrl+o to expand)",
                                        )
                                    ),
                                    Style::default().fg(theme.subtle),
                                ));
                            }
                            blocks.push(vec![Line::from(header_spans)]);
                        }
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
                // 流式占位：对标 Claude Code 流式期 thinking 头 `∴ Thinking…`
                let label = if cancelling {
                    crate::core::i18n::t("ui.thinking.cancelling", "Canceling", "Canceling")
                } else {
                    crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                };
                let label_style = Style::default()
                    .fg(theme.thinking_fg)
                    .add_modifier(Modifier::ITALIC);
                blocks.push(vec![Line::from(vec![
                    Span::styled("∴ ", label_style),
                    Span::styled(format!("{}…", label), label_style),
                ])]);
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
        // 兜底占位：对标 Claude Code 流式期 thinking 头 `∴ Thinking…`
        let label = crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking");
        let label_style = Style::default()
            .fg(theme.thinking_fg)
            .add_modifier(Modifier::ITALIC);
        blocks.push(vec![Line::from(vec![
            Span::styled("∴ ", label_style),
            Span::styled(format!("{}…", label), label_style),
        ])]);
    }

    blocks
}

/// 思考内容的行内 markdown 轻量解析：`**粗体**`、`*斜体*`、`` `代码` ``
/// 与行首 `#` 标题转成对应样式的 Span。思考块不走完整 markdown 排版
/// （需要逐行紧凑渲染），但模型输出的行内标记也不该把 `**` 原样亮出来。
/// 未闭合的标记按字面保留；`` ``` `` 围栏行剥掉标记后为空时返回空 Vec，
/// 由调用方跳过该行。
fn thinking_line_spans(chunk: &str, base: Style, at_line_start: bool) -> Vec<Span<'static>> {
    let mut text = chunk;
    let mut base = base;

    if at_line_start {
        let trimmed = text.trim_start();
        let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
        if (1..=6).contains(&hashes) {
            let after = &trimmed[hashes..];
            if after.is_empty() {
                return Vec::new();
            }
            if let Some(rest) = after.strip_prefix(' ') {
                // 标题：去标记、整行加粗（与正文的标题视觉语言一致）
                text = rest.trim_start_matches(' ');
                base = base.add_modifier(Modifier::BOLD);
            }
        }
        // 代码围栏行（``` 或 ```lang）：整行隐藏，围栏内的代码行按普通
        // 思考文本渲染。逐行解析无法跨行维护"代码段"状态，围栏标记
        // 留着只会是噪音
        if text.trim().starts_with("```") {
            return Vec::new();
        }
    }

    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut seg = String::new();
    let (mut bold, mut italic, mut code) = (false, false, false);

    macro_rules! flush {
        () => {
            if !seg.is_empty() {
                let mut st = base;
                if bold {
                    st = st.add_modifier(Modifier::BOLD);
                }
                if italic {
                    st = st.add_modifier(Modifier::ITALIC);
                }
                if code {
                    st = st.remove_modifier(Modifier::ITALIC);
                }
                spans.push(Span::styled(std::mem::take(&mut seg), st));
            }
        };
    }

    let seg_ends_nonspace = |seg: &str| seg.chars().last().map_or(false, |c| !c.is_whitespace());

    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            // 单反引号切换代码段；双反引号按字面保留（罕见）
            let is_double = chars.get(i + 1) == Some(&'`');
            if !is_double && !bold && !italic {
                flush!();
                code = !code;
                i += 1;
                continue;
            }
        } else if c == '*' && !code {
            if chars.get(i + 1) == Some(&'*') {
                if bold {
                    if seg_ends_nonspace(&seg) {
                        flush!();
                        bold = false;
                        i += 2;
                        continue;
                    }
                } else if chars.get(i + 2).map_or(false, |n| !n.is_whitespace()) {
                    // 开启粗体：`**` 后非空白。前面不设限——中文标点
                    // （`：**重点**`）紧跟粗体是模型输出的大头
                    flush!();
                    bold = true;
                    i += 2;
                    continue;
                }
                seg.push('*');
                seg.push('*');
                i += 2;
                continue;
            }
            if italic {
                if seg_ends_nonspace(&seg) {
                    flush!();
                    italic = false;
                    i += 1;
                    continue;
                }
            } else if seg.is_empty()
                && chars.get(i + 1).map_or(false, |n| !n.is_whitespace() && *n != '*')
            {
                // 单星斜体只在段首开启（前面是空白/标记边界），避免 `3 * 4` 误判
                flush!();
                italic = true;
                i += 1;
                continue;
            }
        }
        seg.push(c);
        i += 1;
    }
    flush!();
    spans
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

/// thinking 耗时展示（对标 Claude Code `Thought for 14s`）：秒向上取整、
/// 最少显示 1s，超过 1 分钟显示 `Xm Ys`。
fn format_thinking_duration(ms: u128) -> String {
    let secs = (ms / 1000).max(1);
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
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
                    render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
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

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 30, false);
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

    /// 思考文本行间普遍是空行（`\n\n`）。markdown 段落排版会把每行变成独立
    /// 段落、段间空一行，整块稀疏得没法看 —— 必须逐行渲染且行间无空行。
    #[test]
    fn thinking_lines_render_compactly_without_paragraph_gaps() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content =
            Some("先分析用户的问题\n\n拆解成三个子任务\n\n逐个检查边界情况".to_string());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
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

        // 三行各自渲染，不合并（softbreak 不吞行）也不插空行
        assert_eq!(
            rendered,
            vec![
                "  先分析用户的问题".to_string(),
                "  拆解成三个子任务".to_string(),
                "  逐个检查边界情况".to_string(),
            ],
            "rendered: {:?}",
            rendered
        );
    }

    /// thinking 里的源行前导缩进必须保留；长行软折行后，续行也要继承缩进，
    /// 否则结构化思考内容会变成同一列的“面条文本”。
    #[test]
    fn thinking_lines_preserve_indentation_across_wrapping() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("root\n  nested item\n    deeper value".to_string());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 18, false);
        let rendered: Vec<String> = blocks[1]
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        assert_eq!(
            rendered,
            vec![
                "  root".to_string(),
                "    nested item".to_string(),
                "      deeper".to_string(),
                "      value".to_string(),
            ],
            "rendered: {:?}",
            rendered
        );
    }

    /// 思考内容的行内 markdown 不得原样显示：`**粗体**` 要转成加粗样式，
    /// `##` 标题去标记后加粗，` ``` ` 围栏行整行隐藏。
    #[test]
    fn thinking_inline_markdown_renders_styled_not_literal() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some(
            "## 分析重点\n先查 **缓存失效** 的问题\n再看 `retry` 逻辑\n```bash\necho hi\n```"
                .to_string(),
        );
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
        let rendered: Vec<(String, bool)> = blocks[1]
            .iter()
            .map(|line| {
                let text: String =
                    line.spans.iter().map(|s| s.content.as_ref()).collect();
                let has_bold = line
                    .spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::BOLD));
                (text, has_bold)
            })
            .collect();
        let texts: Vec<&str> = rendered.iter().map(|(t, _)| t.as_str()).collect();

        // 无任何字面 markdown 标记残留
        for t in &texts {
            assert!(!t.contains("**"), "literal ** leaked: {:?}", texts);
            assert!(!t.contains("##"), "literal ## leaked: {:?}", texts);
            assert!(!t.contains("```"), "literal ``` leaked: {:?}", texts);
            assert!(!t.contains("`"), "literal backtick leaked: {:?}", texts);
        }

        // 标题行与粗体段都带 BOLD；围栏行被隐藏
        assert_eq!(
            texts,
            vec![
                "  分析重点",
                "  先查 缓存失效 的问题",
                "  再看 retry 逻辑",
                "  echo hi",
            ],
            "rendered: {:?}",
            texts
        );
        assert!(rendered[0].1, "heading line should be bold");
        assert!(
            rendered[1].1,
            "bold segment line should carry BOLD modifier"
        );
        assert!(!rendered[3].1, "plain code line should not be bold");
    }

    /// 未闭合的 `**` 按字面保留，不能把后半段全部误染成粗体。
    #[test]
    fn thinking_unclosed_bold_marker_stays_literal() {
        let spans = thinking_line_spans("count 2 ** 3 is six", Style::default(), true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "count 2 ** 3 is six");
        assert!(
            !spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
            "unclosed ** must not bold the rest: {:?}",
            spans
        );
    }

    /// 完成后折叠头必须从 "Thinking…" 变为 "Thought for Xs"（对标 Claude Code
    /// 完成态），并带展开提示；流式期间显示 "Thinking…" 且无提示。
    #[test]
    fn thinking_header_changes_between_streaming_and_completed_states() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("first step\n\nsecond step".to_string());
        entry.reasoning_finished_elapsed_ms = Some(14_000);
        // 30 秒宽限窗口内可见
        entry.reasoning_finished_at = Some(std::time::Instant::now());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;

        let header_of = |state: &crate::ui::state::ChatState| {
            let blocks =
                render_non_tool_entry_blocks(state, &state.chat_history[idx], idx, 70, false);
            blocks[0]
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<String>()
        };

        // 完成态：Thought for 14s + 展开提示
        let completed = header_of(&state);
        assert!(completed.contains("Thought for 14s"), "{:?}", completed);
        assert!(completed.contains("(ctrl+o to expand)"), "{:?}", completed);

        // 流式态：Thinking… 且无展开提示
        state.chat_history[idx].is_streaming = Some(true);
        state.chat_history[idx].reasoning_finished_elapsed_ms = None;
        let streaming = header_of(&state);
        assert!(streaming.contains("Thinking…"), "{:?}", streaming);
        assert!(!streaming.contains("ctrl+o"), "{:?}", streaming);
    }

    /// 恢复的历史思考块（serde skip → finished_at/elapsed 均为 None）：
    /// 必须保持折叠可见、头部是普通 "Thinking"，绝不能被冻结成
    /// "Thought for 1s"（finalize 无时钟基准时的旧 bug）也不能被 30 秒窗口隐藏。
    #[test]
    fn restored_history_thinking_blocks_stay_visible_without_bogus_duration() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("restored reasoning content here".to_string());
        // 模拟 serde 恢复：两个时间字段都是 None
        assert!(entry.reasoning_finished_at.is_none());
        assert!(entry.reasoning_finished_elapsed_ms.is_none());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
        assert!(
            !blocks.is_empty(),
            "restored thinking block must be visible"
        );

        let header: String = blocks[0]
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert!(header.contains("Thinking"), "{:?}", header);
        assert!(!header.contains("Thought for"), "{:?}", header);
        assert!(header.contains("(ctrl+o to expand)"), "{:?}", header);
    }

    /// 思考耗时格式化：秒向上取整、最少 1s、跨分钟显示 Xm Ys。
    #[test]
    fn thinking_duration_formatting() {
        assert_eq!(format_thinking_duration(0), "1s");
        assert_eq!(format_thinking_duration(500), "1s");
        assert_eq!(format_thinking_duration(14_000), "14s");
        assert_eq!(format_thinking_duration(65_000), "1m 5s");
    }
}
