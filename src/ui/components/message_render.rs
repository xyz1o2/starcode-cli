use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::ChatEntryType;
use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;

/// thinking 展开态正文的最大折行宽度（显示列）。
///
/// 此前正文按 `终端宽 - 2` 折行，宽屏下一行能拉到 200+ 列：眼睛从行首扫到行尾
/// 的距离超出单行可读范围，读一段话要来回横跳，整块读不下去。封顶后宽屏上正文
/// 稳定在 ~100 列，右侧留白；终端本身比它窄时仍按终端宽折行，窄窗口行为不变。
const MAX_CONTENT_WIDTH: usize = 100;

/// 渲染非工具条目（assistant / user）的文本块。
///
/// `is_sub_transcript`：agent 组展开视图里的子转录条目。thinking 折叠头一旦
/// 有实质内容就渲染，不按"完成多久"自动隐藏。
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
                    // thinking 折叠头（含 `∴ Thought for Xs` 耗时标签）一旦出现就保留
                    // 在会话里，不再按"完成多久"自动隐藏。
                    //
                    // 旧行为靠一个 30 秒宽限窗口判断可见性，完成 30 秒后整块连带耗时
                    // 标签一起消失；可重启读回历史时又一直可见（完成时刻 serde skip，
                    // 恢复出来是 None）——同一条历史里前后表现不一致，用户没法预期它
                    // 在哪。窗口移除后统一为"始终可见"。
                    //
                    // 折叠态只占一行；想看正文用 Ctrl+O / Tab 切展开模式。
                    let is_expanded = state.is_transcript_mode
                        || state.expanded_thinking_indices.contains(&entry_idx);

                    // 间距规则（对标 Claude Code：只有"块的顶部 margin"一个所有者）：
                    // thinking 块前不另加空行 — entry 级 leading blank 已提供分隔，
                    // 此处再加会叠加成双空行

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

                    // 头部随状态变化。这里只剩两种形态：
                    // 已完成   → `∴ Thought for 14s`（折叠态补展开提示）
                    // 取消中   → `∴ Canceling…`（展开态用）
                    //
                    // "思考中 → `∴ Thinking…`" 这一支已被删掉：实时状态由紧贴输入框的
                    // spinner 行独家承担（`⠹ Spelunking… (18s · esc to interrupt)`），
                    // 而这一行此前是 primary + 粗斜体、比状态行响得多。同一个"思考中"
                    // 在屏上说了两遍、两套权重、两种措辞，而用户只需要一个"现在"。
                    // 正文该留下的，是思考**结束后**的记录。
                    let header_text = if cancelling {
                        format!(
                            "{}…",
                            crate::core::i18n::t(
                                "ui.thinking.cancelling",
                                "Canceling",
                                "Canceling"
                            )
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
                            None => {
                                crate::core::i18n::t("ui.thinking.label", "Thinking", "Thinking")
                            }
                        }
                    };

                    if is_expanded {
                        // 展开态：头部（不重复展开提示）
                        blocks.push(vec![Line::from(vec![
                            Span::styled("∴ ", label_style),
                            Span::styled(header_text, label_style),
                        ])]);

                        // 内容：dim 色 + 缩进 2 列，逐行渲染不走 markdown 段落排版。
                        //
                        // 宽度上限：正文折行宽度此前直接吃满终端宽（`wrap_width` =
                        // 终端宽 - 2），宽屏下一行能拉到 200+ 列，眼睛扫不到行尾，
                        // 整块读不下去。这里封顶 `MAX_CONTENT_WIDTH`：终端比它窄时
                        // 仍按终端宽折行，所以窄窗口行为不变，只是宽屏不再无限拉长。
                        let content_width = wrap_width.min(MAX_CONTENT_WIDTH).saturating_sub(2);

                        // 折行续行比首行多缩进 `HANGING_INDENT` 列：续行此前与源新行
                        // 前缀完全相同（都是 2 空格），长行折行后看不出哪里是断点，
                        // 读起来像一长条没有换行的面条。
                        const HANGING_INDENT: &str = "  ";
                        let thinking_style = if is_streaming_now || cancelling {
                            Style::default()
                                .fg(theme.primary)
                                .add_modifier(Modifier::ITALIC)
                        } else {
                            Style::default().fg(theme.inactive)
                        };
                        let mut thinking_lines: Vec<Line<'static>> = Vec::new();
                        let mut fence_lang: Option<String> = None;
                        let mut fence_buf: Vec<String> = Vec::new();
                        let mut pending_blank = false;
                        for raw_line in reasoning_display.lines() {
                            let line = raw_line.trim_end();

                            // ``` 围栏：开段记语言，闭段整块渲染；围栏内的行
                            // 攒进 buffer，不做行内 markdown 解析
                            if let Some(rest) = line.trim_start().strip_prefix("```") {
                                // 延迟补空行：连续源空行折叠成一个，且只在下一行
                                // 真的有内容时补 —— 块尾的空行因此不会被渲染出来
                                if pending_blank {
                                    pending_blank = false;
                                    if !thinking_lines.is_empty() {
                                        thinking_lines.push(Line::from(""));
                                    }
                                }
                                if fence_lang.is_none() {
                                    fence_lang = Some(rest.trim().to_string());
                                } else {
                                    thinking_lines.extend(
                                        crate::utils::markdown_parser::render_thinking_code_segment(
                                            &fence_buf.join("\n"),
                                            fence_lang.as_deref().unwrap_or(""),
                                            Some(content_width),
                                        ),
                                    );
                                    fence_lang = None;
                                }
                                continue;
                            }
                            if fence_lang.is_some() {
                                fence_buf.push(line.to_string());
                                continue;
                            }

                            if line.trim().is_empty() {
                                // 源空行 = 段落分隔。只置标记、不立刻落行：真正的
                                // 空行在"下一行有内容时"才补，这样尾部连续空行不会在
                                // 块尾留下悬空行。围栏内的空行由上面的
                                // `fence_lang.is_some()` 分支接管，不走这里。
                                pending_blank = true;
                                continue;
                            }
                            if pending_blank {
                                pending_blank = false;
                                if !thinking_lines.is_empty() {
                                    thinking_lines.push(Line::from(""));
                                }
                            }
                            let indent = line.len() - line.trim_start().len();
                            let indent_width =
                                crate::ui::utils::render::display_width(&line[..indent]);
                            let first_width = content_width.saturating_sub(indent_width);
                            // 续行要多让出 `HANGING_INDENT` 的宽度，否则加上缩进
                            // 后会超出 `content_width`
                            let rest_width =
                                first_width.saturating_sub(HANGING_INDENT.len()).max(2);
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
                                    format!("  {}{}", &line[..indent], HANGING_INDENT)
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
                        // 流式期间闭合 ``` 可能还没到：把攒着的半截代码段
                        // 照常渲染，随流式增长逐帧补全
                        if fence_lang.is_some() {
                            thinking_lines.extend(
                                crate::utils::markdown_parser::render_thinking_code_segment(
                                    &fence_buf.join("\n"),
                                    fence_lang.as_deref().unwrap_or(""),
                                    Some(content_width),
                                ),
                            );
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
                    } else if !is_streaming_now && !cancelling {
                        // 折叠态 + 已完成：这一块唯一的形态 —— 一条记录。
                        // `∴ Thought for 14s  (ctrl+o to expand)`
                        //
                        // 折叠 + 进行中时这里**什么都不渲染**（整块不出声）。此前的
                        // `∴ Thinking…` 与输入框上方的 spinner 行说的是同一件事，
                        // 却用两套权重两种措辞同时占着用户的注意力；而记录本来
                        // 就不该抢实时状态的活。正文只负责"事后发生了什么"。
                        let header_spans = vec![
                            Span::styled("∴ ", label_style),
                            Span::styled(header_text.clone(), label_style),
                            Span::styled(
                                format!(
                                    " {}",
                                    crate::core::i18n::t(
                                        "ui.thinking.expand_hint",
                                        "(ctrl+o 展开思考)",
                                        "(ctrl+o to expand)",
                                    )
                                ),
                                Style::default().fg(theme.subtle),
                            ),
                        ];
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

            // 此前这里在"还没有正文、模型正在思考"时先渲染一条 `∴ Thinking…` 占位。
            // 已删除：那同样是在正文里抢实时状态的活，而输入框上方的 spinner 行
            // 就在两行之下说着同一句话。此处 content 为空，正文分支自然什么都不出。
            if is_status_line(&display_content) {
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

    // 兜底占位 `∴ Thinking…`（"整条消息都还没渲染出任何东西"时补一行）同样已删除。
    // 少一行占位不是问题：spinner 行就贴在输入框上方，"现在"由它独家承担。
    // （`current_model_supports_thinking` 不再被这里消费，但仍有其他调用点。）

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
                && chars
                    .get(i + 1)
                    .map_or(false, |n| !n.is_whitespace() && *n != '*')
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

    /// 源空行 = 段落分隔，必须保留一个：模型思考里的分步结构靠它区分，丢掉就
    /// 塌成一堵没有层次的墙。但也不能照搬源空行数量 —— 连续多个空行折叠成一个，
    /// 且不在块首/块尾留下悬空行。
    #[test]
    fn thinking_lines_keep_paragraph_breaks_without_extra_padding() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content =
            Some("先分析用户的问题\n\n拆解成三个子任务\n\n\n\n逐个检查边界情况\n\n".to_string());
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

        // 三行各一行、段间恰好一个空行；连续空行折叠，尾部空行不留
        assert_eq!(
            rendered,
            vec![
                "  先分析用户的问题".to_string(),
                "".to_string(),
                "  拆解成三个子任务".to_string(),
                "".to_string(),
                "  逐个检查边界情况".to_string(),
            ],
            "rendered: {:?}",
            rendered
        );
    }

    /// thinking 里的源行前导缩进必须保留；长行软折行后，续行既要继承源行缩进，
    /// 还要比首行多缩进一档 —— 此前两者前缀完全相同（都是 2 空格），长行折行后
    /// 看不出哪里是断点，结构化思考内容会变成同一列的「面条文本」。
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
                // 续行 = 2 列块缩进 + 4 列源缩进 + 2 列悬挂缩进
                "        value".to_string(),
            ],
            "rendered: {:?}",
            rendered
        );
    }

    /// 宽终端上正文不得无限拉长：此前折行宽度直接吃满 `终端宽 - 2`，200 列宽屏
    /// 里一行能拉到 200+，眼睛扫不到行尾。折行宽度必须封顶 `MAX_CONTENT_WIDTH`，
    /// 且任何一行的显示宽度都不许超出该上限（含围栏边框）。
    #[test]
    fn thinking_body_width_is_capped_on_wide_terminals() {
        let long = "word ".repeat(80);
        let render_at = |w: usize| -> Vec<(String, usize)> {
            let mut state = crate::ui::state::ChatState::new();
            let mut entry = crate::types::ChatEntry::assistant("");
            entry.reasoning_content = Some(format!("{}\n```\n{}\n```", long.trim(), long.trim()));
            state.chat_history.push(entry);
            let idx = state.chat_history.len() - 1;
            state.expanded_thinking_indices.insert(idx);
            render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, w, false)
                .into_iter()
                .flatten()
                .map(|l| {
                    let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                    let width = crate::ui::utils::render::line_spans_display_width(&l.spans);
                    (text, width)
                })
                .collect()
        };

        let cap = MAX_CONTENT_WIDTH;
        for w in [cap + 40, cap * 3] {
            for (text, width) in render_at(w) {
                assert!(
                    width <= cap,
                    "wrap_width={w}: line exceeds cap ({width} > {cap}): {text:?}"
                );
            }
        }

        // 窄终端行为不变：仍按终端宽折行，不因为封顶而提前换行
        let narrow = render_at(40);
        assert!(
            narrow.iter().any(|(_, width)| *width > 20),
            "narrow terminal should still fill its width: {narrow:?}"
        );
    }

    /// 思考内容的行内 markdown 不得原样显示：`**粗体**` 要转成加粗样式，
    /// `##` 标题去标记后加粗，``` 围栏代码段要真正渲染（边框 + 高亮）。
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
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let has_bold = line
                    .spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::BOLD));
                (text, has_bold)
            })
            .collect();
        let texts: Vec<&str> = rendered.iter().map(|(t, _)| t.as_str()).collect();

        // 无任何字面 markdown 标记残留（围栏边框 ┌/└ 不是标记）
        for t in &texts {
            assert!(!t.contains("**"), "literal ** leaked: {:?}", texts);
            assert!(!t.contains("##"), "literal ## leaked: {:?}", texts);
            assert!(!t.contains("```"), "literal ``` leaked: {:?}", texts);
            assert!(!t.contains("`"), "literal backtick leaked: {:?}", texts);
        }

        // 围栏段渲染成边框 + 代码正文（正文 9 列宽 → 总宽 10）
        assert_eq!(
            texts,
            vec![
                "  分析重点",
                "  先查 缓存失效 的问题",
                "  再看 retry 逻辑",
                "  ┌ bash ─",
                "  echo hi",
                "  └───────",
            ],
            "rendered: {:?}",
            texts
        );
        assert!(rendered[0].1, "heading line should be bold");
        assert!(
            rendered[1].1,
            "bold segment line should carry BOLD modifier"
        );
        assert!(!rendered[2].1, "inline-code line should not be bold");
        assert!(rendered[3].1, "fence language label should be bold");
        assert!(!rendered[4].1, "code body line should not be bold");
    }

    /// 围栏未闭合（流式中途）：半截代码段也要渲染出来，不能整段消失。
    #[test]
    fn thinking_unclosed_fence_still_renders_partial_code() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("先跑一遍命令\n```bash\nnslookup example.com".to_string());
        entry.is_streaming = Some(true);
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.expanded_thinking_indices.insert(idx);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
        let texts: Vec<String> = blocks[1]
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        assert!(
            texts.iter().any(|t| t.contains("nslookup example.com")),
            "partial code body must render: {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.contains('┌')),
            "fence top border must render: {:?}",
            texts
        );
    }

    /// 未闭合的 `**` 按字面保留，不能把后半段全部误染成粗体。
    #[test]
    fn thinking_unclosed_bold_marker_stays_literal() {
        let spans = thinking_line_spans("count 2 ** 3 is six", Style::default(), true);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "count 2 ** 3 is six");
        assert!(
            !spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
            "unclosed ** must not bold the rest: {:?}",
            spans
        );
    }

    /// 思考完成态的折叠头是 `∴ Thought for 14s  (ctrl+o to expand)`；
    /// 而**思考进行中，正文整块不出声**。
    ///
    /// 此前是"流式期 `∴ Thinking…` → 完成期 `∴ Thought for Xs`"两态。这条回归现在
    /// 守住的是另一半约定：实时状态由输入框上方的 spinner 行独家承担
    /// （`⠹ Spelunking… (18s · esc to interrupt)`），正文再喊一遍"Thinking…" 就成了
    /// 同一个"思考中"在屏上说了两遍 —— 而且正文那遍还是 primary + 粗斜体，比状态行
    /// 响得多，用户第一眼读到的反而不是那行。
    #[test]
    fn thinking_header_is_the_completed_record_only() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("first step\n\nsecond step".to_string());
        entry.reasoning_finished_elapsed_ms = Some(14_000);
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;

        let header_of = |state: &crate::ui::state::ChatState| {
            let blocks =
                render_non_tool_entry_blocks(state, &state.chat_history[idx], idx, 70, false);
            blocks
                .first()
                .map(|block| {
                    block
                        .iter()
                        .map(|line| {
                            line.spans
                                .iter()
                                .map(|s| s.content.as_ref())
                                .collect::<String>()
                        })
                        .collect::<String>()
                })
                .unwrap_or_default()
        };

        // 完成态：Thought for 14s + 展开提示
        let completed = header_of(&state);
        assert!(completed.contains("Thought for 14s"), "{:?}", completed);
        assert!(completed.contains("(ctrl+o to expand)"), "{:?}", completed);

        // 流式态：折叠中的思考块不出声 —— 既没有头部，也没有正文。
        state.chat_history[idx].is_streaming = Some(true);
        state.chat_history[idx].reasoning_finished_elapsed_ms = None;
        let streaming = header_of(&state);
        assert!(
            streaming.is_empty(),
            "思考进行中正文不该出声，实际 {:?}",
            streaming
        );
        assert!(!streaming.contains("Thinking"), "{:?}", streaming);
        assert!(!streaming.contains("ctrl+o"), "{:?}", streaming);

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
        assert!(
            blocks.is_empty(),
            "折叠 + 流式时整个思考块都不该渲染，实际 {:?}",
            blocks
        );
    }

    /// 恢复的历史思考块（serde skip → finished_at/elapsed 均为 None）：
    /// 必须保持折叠可见、头部是普通 "Thinking"，绝不能被冻结成
    /// "Thought for 1s"（finalize 无时钟基准时的旧 bug）。
    #[test]
    fn restored_history_thinking_blocks_stay_visible_without_bogus_duration() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("restored reasoning content here".to_string());
        // 模拟 serde 恢复：耗时尚未冻结
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

    /// 完成的 thinking 折叠头必须始终可见，`∴ Thought for Xs` 不得因为
    /// "完成得够久"而消失。
    ///
    /// 旧行为靠 30 秒宽限窗口判断可见性，于是同一条历史里"刚完成能看到、
    /// 跑一会儿消失、重启读回后又出现"，用户无法预期。窗口已移除。
    #[test]
    fn completed_thinking_label_stays_visible_for_the_rest_of_the_session() {
        let mut state = crate::ui::state::ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("reasoning that outlives the old grace window".to_string());
        entry.reasoning_finished_elapsed_ms = Some(3_000);
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;

        let blocks = render_non_tool_entry_blocks(&state, &state.chat_history[idx], idx, 70, false);
        assert!(
            !blocks.is_empty(),
            "completed thinking block must stay visible"
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
        assert!(
            header.contains("Thought for 3s"),
            "duration label must not disappear: {:?}",
            header
        );
        assert!(header.contains("(ctrl+o to expand)"), "{:?}", header);
    }
}
