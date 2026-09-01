use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, PartialEq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    Thinking(String),
}

/// Parse Markdown content into structured blocks.
/// When `extract_thinking` is true, `<thinking>` blocks are extracted and
/// rendered with dimmed styling. For non-thinking models this should be false
/// so that `<thinking>` tags in the output are treated as plain text.
pub fn parse_markdown_content_ext(content: &str, extract_thinking: bool) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    if extract_thinking {
        // Extract thinking blocks into separate ContentBlock::Thinking
        let mut remaining = content.to_string();
        let thinking_tags = [
            ("<thinking>", "</thinking>"),
            ("<think>", "</think>"),
            ("<思考>", "</思考>"),
            ("<thought>", "</thought>"),
            ("<plan>", "</plan>"),
            ("<thinking_process>", "</thinking_process>"),
        ];

        // Find and extract thinking blocks
        for (open, close) in &thinking_tags {
            while let Some(start) = remaining.find(open) {
                // Add text before thinking block
                if start > 0 {
                    let before = remaining[..start].trim().to_string();
                    if !before.is_empty() {
                        blocks.push(ContentBlock::Text(before));
                    }
                }

                let content_start = start + open.len();
                if let Some(end) = remaining[content_start..].find(close) {
                    let inner = remaining[content_start..content_start + end].to_string();
                    if !inner.trim().is_empty() {
                        blocks.push(ContentBlock::Thinking(inner));
                    }
                    remaining = remaining[content_start + end + close.len()..].to_string();
                } else {
                    break;
                }
            }
        }

        // Add remaining text
        if !remaining.trim().is_empty() {
            blocks.push(ContentBlock::Text(remaining));
        }
    } else {
        // Strip all thinking tags
        let mut remaining = content.to_string();
        for tag in [
            "<thinking>",
            "<think>",
            "<思考>",
            "<thought>",
            "<plan>",
            "<thinking_process>",
        ] {
            let close = tag.replacen("<", "</", 1);
            remaining = remaining.replace(tag, "").replace(&close, "");
        }

        // If stripping thinking tags left nothing, extract content from inside the tags
        // This handles models that wrap entire response in thinking tags
        if remaining.trim().is_empty() && !content.trim().is_empty() {
            for (open, close) in [
                ("<thinking>", "</thinking>"),
                ("<think>", "</think>"),
                ("<思考>", "</思考>"),
                ("<thought>", "</thought>"),
                ("<plan>", "</plan>"),
                ("<thinking_process>", "</thinking_process>"),
            ] {
                if let Some(start) = content.find(open) {
                    let content_start = start + open.len();
                    if let Some(end) = content[content_start..].find(close) {
                        let inner = &content[content_start..content_start + end];
                        if !inner.trim().is_empty() {
                            remaining = inner.to_string();
                            break;
                        }
                    }
                }
            }
        }

        // If still empty after extraction, check if original content has any real text
        // outside of thinking tags. If so, use it. Otherwise leave empty.
        if remaining.trim().is_empty() && !content.trim().is_empty() {
            // Strip tags and check if any non-whitespace remains
            let mut stripped = content.to_string();
            for tag in [
                "<thinking>",
                "<think>",
                "<思考>",
                "<thought>",
                "<plan>",
                "<thinking_process>",
            ] {
                let close = tag.replacen("<", "</", 1);
                stripped = stripped.replace(tag, "").replace(&close, "");
            }
            if !stripped.trim().is_empty() {
                remaining = stripped;
            }
            // If stripped is also empty, the content was just thinking tags with no real text
            // In this case, remaining stays empty and no block is added
        }

        if !remaining.trim().is_empty() {
            blocks.push(ContentBlock::Text(remaining));
        }
    }

    if blocks.is_empty() {
        blocks.push(ContentBlock::Text(String::new()));
    }

    blocks
}

/// Backward-compatible wrapper: always extracts `<thinking>` blocks.
pub fn parse_markdown_content(content: &str) -> Vec<ContentBlock> {
    parse_markdown_content_ext(content, true)
}

/// Render a single thinking block with dimmed styling and vertical bar prefix.
fn render_thinking_lines(text: &str, wrap_width: Option<usize>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "Thinking Process:",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )]));

    let process_lines = |text: &str| {
        if let Some(width) = wrap_width {
            let effective_width = width.saturating_sub(2);
            let mut out_lines = Vec::new();
            for line in text.lines() {
                let wrapped = crate::ui::utils::render::wrap_text_to_width(line, effective_width);
                for w in wrapped {
                    out_lines.push(w.to_string());
                }
            }
            out_lines
        } else {
            text.lines().map(|s| s.to_string()).collect()
        }
    };

    for line in process_lines(text) {
        if line.trim().is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    line,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(ratatui::style::Modifier::ITALIC),
                ),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines
}

/// Syntax-highlight a code line using syntect (100+ languages, TextMate grammars).
/// Falls back to plain text for unknown languages.
pub fn highlight_code_line(line: &str, language: &str) -> Line<'static> {
    crate::utils::syntax_highlight::highlight_line(line, language)
}

/// Render markdown text using pulldown-cmark, producing ratatui `Line`s.
fn render_markdown(text: &str, wrap_width: Option<usize>) -> Vec<Line<'static>> {
    let parser = Parser::new_ext(text, Options::all());
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];

    // Block-level state
    let mut in_code_block = false;
    let mut in_table = false;
    let mut in_table_head = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    let mut list_depth: usize = 0;
    let mut list_is_ordered: Vec<bool> = Vec::new();
    let mut list_counters: Vec<u64> = Vec::new();
    let mut language = String::new();
    let mut code_line_number: usize = 0;
    let mut blockquote_depth: usize = 0;
    let mut link_url_stack: Vec<String> = Vec::new();
    let mut table_alignments: Vec<pulldown_cmark::Alignment> = Vec::new();

    let current_style = |stack: &[Style]| stack.last().copied().unwrap_or_default();

    let flush_line = |spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>| {
        if !spans.is_empty() {
            lines.push(Line::from(spans.clone()));
            spans.clear();
        } else {
            lines.push(Line::from(""));
        }
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { level, .. } => {
                    let heading_style = match level {
                        HeadingLevel::H1 => Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        HeadingLevel::H2 => Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                        HeadingLevel::H3 => Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                        _ => Style::default().fg(Color::Blue),
                    };
                    style_stack.push(heading_style);
                }
                Tag::BlockQuote(_) => {
                    blockquote_depth += 1;
                    let prefix = "│ ".repeat(blockquote_depth);
                    current_spans.push(Span::styled(prefix, Style::default().fg(Color::DarkGray)));
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_line_number = 0;
                    language = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    flush_line(&mut current_spans, &mut lines);
                    // 语言标签 + 分隔线（对标 Claude Code：不显示快捷键提示，边框保持干净。
                    // 顶边总宽 = 40.min(w)，与底边 "  └" + dash 对齐）
                    if !language.is_empty() {
                        let label = format!("  ┌ {} ", language);
                        let dash_count = 40
                            .min(wrap_width.unwrap_or(80))
                            .saturating_sub(label.chars().count());
                        lines.push(Line::from(vec![
                            Span::styled(
                                label,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                "─".repeat(dash_count),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));
                    } else {
                        let dash_count = 40.min(wrap_width.unwrap_or(80)).saturating_sub(4);
                        lines.push(Line::from(vec![
                            Span::styled("  ┌ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                "─".repeat(dash_count),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));
                    }
                }
                Tag::List(order) => {
                    list_depth += 1;
                    list_is_ordered.push(order.is_some());
                    list_counters.push(0);
                    if list_depth > 1 {
                        flush_line(&mut current_spans, &mut lines);
                    }
                }
                Tag::Item => {
                    if list_depth > 0 {
                        let indent = "  ".repeat(list_depth.saturating_sub(1));
                        if list_is_ordered.last().copied().unwrap_or(false) {
                            if let Some(c) = list_counters.last_mut() {
                                *c += 1;
                                current_spans.push(Span::styled(
                                    format!("{}{}. ", indent, c),
                                    Style::default().fg(Color::Yellow),
                                ));
                            }
                        } else {
                            current_spans.push(Span::styled(
                                format!("{}• ", indent),
                                Style::default().fg(Color::Yellow),
                            ));
                        }
                    }
                }
                Tag::FootnoteDefinition(_) => {}
                Tag::Table(alignments) => {
                    in_table = true;
                    table_rows.clear();
                    table_alignments = alignments;
                }
                Tag::TableHead => {
                    in_table_head = true;
                }
                Tag::TableRow => {
                    current_row.clear();
                }
                Tag::TableCell => {
                    current_cell.clear();
                }
                Tag::Emphasis => {
                    let mut s = current_style(&style_stack);
                    s = s.add_modifier(Modifier::ITALIC);
                    style_stack.push(s);
                }
                Tag::Strong => {
                    let mut s = current_style(&style_stack);
                    s = s.add_modifier(Modifier::BOLD);
                    style_stack.push(s);
                }
                Tag::Strikethrough => {
                    let mut s = current_style(&style_stack);
                    s = s.add_modifier(Modifier::CROSSED_OUT);
                    style_stack.push(s);
                }
                Tag::Link { dest_url, .. } => {
                    let mut s = current_style(&style_stack);
                    s = s.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
                    style_stack.push(s);
                    link_url_stack.push(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => {
                    current_spans
                        .push(Span::styled("[img: ", Style::default().fg(Color::DarkGray)));
                    let mut s = current_style(&style_stack);
                    s = s.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
                    style_stack.push(s);
                    link_url_stack.push(dest_url.to_string());
                }
                Tag::MetadataBlock(_) => {}
                Tag::HtmlBlock => {}
                Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    flush_line(&mut current_spans, &mut lines);
                    lines.push(Line::from(""));
                }
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    flush_line(&mut current_spans, &mut lines);
                    lines.push(Line::from(""));
                }
                TagEnd::BlockQuote(_) => {
                    blockquote_depth = blockquote_depth.saturating_sub(1);
                    flush_line(&mut current_spans, &mut lines);
                }
                TagEnd::CodeBlock => {
                    // Closing separator line — 宽度与顶边对齐:
                    // 顶边总宽 = 40.min(w)（含语言标签与 copy hint），底边 "  └" 占 3 列，
                    // 因此 dash 数 = 40.min(w) - 3，保证左右角与顶边对齐
                    lines.push(Line::from(vec![
                        Span::styled("  └", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            "─".repeat(40.min(wrap_width.unwrap_or(80)).saturating_sub(3)),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    // 与段落/标题一致：块后留一个空行
                    lines.push(Line::from(""));
                    in_code_block = false;
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_is_ordered.pop();
                    list_counters.pop();
                    flush_line(&mut current_spans, &mut lines);
                    if list_depth == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut current_spans, &mut lines);
                }
                TagEnd::Table => {
                    if !table_rows.is_empty() {
                        render_table(&mut lines, &table_rows, &table_alignments, wrap_width);
                    }
                    in_table = false;
                    table_rows.clear();
                    table_alignments.clear();
                    lines.push(Line::from(""));
                }
                TagEnd::TableHead => {
                    in_table_head = false;
                    // 表头行收集完立即入表：pulldown 的表头不触发 TableRow 结束事件，
                    // 不在这里 push 的话会被下一个 TableRow 的 clear() 清掉，表头丢失
                    if !current_row.is_empty() {
                        table_rows.push(current_row.clone());
                        current_row.clear();
                    }
                }
                TagEnd::TableRow => {
                    if !current_row.is_empty() {
                        table_rows.push(current_row.clone());
                    }
                }
                TagEnd::TableCell => {
                    current_row.push(current_cell.clone());
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => {
                    style_stack.pop();
                    if let Some(url) = link_url_stack.pop() {
                        current_spans.push(Span::styled(
                            format!(" ({})", url),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
                TagEnd::Image => {
                    style_stack.pop();
                    if let Some(url) = link_url_stack.pop() {
                        current_spans.push(Span::styled(
                            format!("]({})", url),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
                TagEnd::FootnoteDefinition | TagEnd::MetadataBlock(_) => {}
                TagEnd::HtmlBlock => {}
                TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition => {}
            },
            Event::Code(text) => {
                if in_code_block {
                    for line_str in text.lines() {
                        code_line_number += 1;
                        if let Some(max_w) = wrap_width {
                            let code_w = max_w;
                            if line_str.is_empty() {
                                lines.push(Line::from(""));
                            } else {
                                let mut is_first = true;
                                for wrapped in
                                    crate::ui::utils::render::wrap_text_to_width(line_str, code_w)
                                {
                                    let highlighted = highlight_code_line(&wrapped, &language);
                                    let mut line_spans: Vec<Span> = Vec::new();
                                    is_first = false;
                                    line_spans.extend(highlighted.spans.iter().cloned());
                                    lines.push(Line::from(line_spans));
                                }
                            }
                        } else {
                            let highlighted = highlight_code_line(line_str, &language);
                            let mut line_spans: Vec<Span> = Vec::new();
                            line_spans.extend(highlighted.spans.iter().cloned());
                            lines.push(Line::from(line_spans));
                        }
                    }
                } else if in_table_head || in_table {
                    current_cell.push_str(text.as_ref());
                } else {
                    current_spans.push(Span::styled(
                        text.to_string(),
                        Style::default().fg(Color::White).bg(Color::DarkGray),
                    ));
                }
            }
            Event::Text(text) => {
                let text = text.as_ref();
                if in_code_block {
                    for line_str in text.lines() {
                        code_line_number += 1;
                        if let Some(max_w) = wrap_width {
                            let code_w = max_w;
                            if line_str.is_empty() {
                                lines.push(Line::from(""));
                            } else {
                                for wrapped in
                                    crate::ui::utils::render::wrap_text_to_width(line_str, code_w)
                                {
                                    let highlighted = highlight_code_line(&wrapped, &language);
                                    let mut line_spans: Vec<Span> = Vec::new();
                                    line_spans.extend(highlighted.spans.iter().cloned());
                                    lines.push(Line::from(line_spans));
                                }
                            }
                        } else {
                            let highlighted = highlight_code_line(line_str, &language);
                            let mut line_spans: Vec<Span> = Vec::new();
                            line_spans.extend(highlighted.spans.iter().cloned());
                            lines.push(Line::from(line_spans));
                        }
                    }
                } else if in_table_head || in_table {
                    current_cell.push_str(text);
                } else {
                    let style = current_style(&style_stack);
                    // Apply text wrapping to regular text
                    if let Some(max_w) = wrap_width {
                        let wrapped_lines =
                            crate::ui::utils::render::wrap_text_to_width(text, max_w);
                        for (i, wrapped_line) in wrapped_lines.iter().enumerate() {
                            if i > 0 {
                                flush_line(&mut current_spans, &mut lines);
                            }
                            current_spans.push(Span::styled(wrapped_line.to_string(), style));
                        }
                    } else {
                        current_spans.push(Span::styled(text.to_string(), style));
                    }
                }
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if !in_table {
                    current_spans.push(Span::styled(
                        html.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
            Event::FootnoteReference(_) => {}
            Event::SoftBreak => {
                if !in_code_block && !in_table {
                    current_spans.push(Span::from(" "));
                }
            }
            Event::HardBreak => {
                flush_line(&mut current_spans, &mut lines);
                if blockquote_depth > 0 {
                    let prefix = "│ ".repeat(blockquote_depth);
                    current_spans.push(Span::styled(prefix, Style::default().fg(Color::DarkGray)));
                }
            }
            Event::Rule => {
                flush_line(&mut current_spans, &mut lines);
                let rule = "─".repeat(60);
                lines.push(Line::from(Span::styled(
                    rule,
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                current_spans.push(Span::styled(marker, Style::default().fg(Color::Yellow)));
            }
        }
    }

    flush_line(&mut current_spans, &mut lines);

    // 连续空行归一为单行 — 保证块与块之间固定一个空行，
    // 避免 stable/unstable 拼接或源文本多空行造成"突然多换几行"的观感
    let mut collapsed: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    let mut prev_blank = false;
    for line in lines {
        let is_blank = line.spans.is_empty() || line.spans.iter().all(|s| s.content.is_empty());
        if is_blank && prev_blank {
            continue;
        }
        prev_blank = is_blank;
        collapsed.push(line);
    }
    let mut lines = collapsed;

    // Trim trailing empty lines to avoid double-blank at end of content
    while lines
        .last()
        .is_some_and(|l| l.spans.is_empty() || l.spans.iter().all(|s| s.content.is_empty()))
    {
        lines.pop();
    }

    lines
}

/// Render a collected table into lines with box-drawing borders.
/// Handles terminal width constraints by truncating cells if needed.
fn render_table(
    lines: &mut Vec<Line<'static>>,
    rows: &[Vec<String>],
    alignments: &[pulldown_cmark::Alignment],
    max_width: Option<usize>,
) {
    if rows.is_empty() {
        return;
    }

    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return;
    }

    let mut col_widths = vec![0usize; col_count];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                col_widths[i] = col_widths[i].max(UnicodeWidthStr::width_cjk(cell.as_str()));
            }
        }
    }
    // Minimum width of 3 per column
    for w in col_widths.iter_mut() {
        *w = (*w).max(3);
    }

    // Calculate total table width
    // Format: │ + (content + padding) + │ for each column
    let border_chars = col_count + 1; // │ separators
    let padding_chars = col_count * 2; // 2 spaces padding per cell
    let total_content_width: usize = col_widths.iter().sum();
    let total_table_width = border_chars + padding_chars + total_content_width;

    // If table is too wide, adjust column widths to fit
    let adjusted_widths = if let Some(max_w) = max_width {
        if total_table_width > max_w {
            // Reduce column widths proportionally
            let available_width = max_w.saturating_sub(border_chars + padding_chars);
            let scale_factor = available_width as f64 / total_content_width as f64;
            col_widths
                .iter()
                .map(|&w| {
                    let scaled = (w as f64 * scale_factor) as usize;
                    scaled.max(3) // Minimum 3 chars per column
                })
                .collect::<Vec<_>>()
        } else {
            col_widths.clone()
        }
    } else {
        col_widths.clone()
    };

    let header_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let body_style = Style::default();
    let border_style = Style::default().fg(Color::DarkGray);

    // Convert pulldown_cmark Alignment to local Alignment for padding logic
    let col_alignments: Vec<Alignment> = alignments
        .iter()
        .map(|a| match a {
            pulldown_cmark::Alignment::None | pulldown_cmark::Alignment::Left => Alignment::Left,
            pulldown_cmark::Alignment::Center => Alignment::Center,
            pulldown_cmark::Alignment::Right => Alignment::Right,
        })
        .collect();

    for (row_idx, row) in rows.iter().enumerate() {
        let is_header = row_idx == 0;
        let cell_style = if is_header { header_style } else { body_style };

        let mut spans: Vec<Span<'static>> = Vec::new();

        // Top border for first row
        if row_idx == 0 {
            let mut border_spans = Vec::new();
            border_spans.push(Span::styled("┌", border_style));
            for (ci, w) in adjusted_widths.iter().enumerate() {
                border_spans.push(Span::styled("─".repeat(w + 2), border_style));
                if ci < col_count - 1 {
                    border_spans.push(Span::styled("┬", border_style));
                }
            }
            border_spans.push(Span::styled("┐", border_style));
            lines.push(Line::from(border_spans));
        }

        // Row content — 按 col_count 补齐缺失单元格，保证每行右边界完整对齐
        spans.push(Span::styled("│ ", border_style));
        for ci in 0..col_count {
            let cell = row.get(ci).map(|s| s.as_str()).unwrap_or("");
            let target_width = adjusted_widths.get(ci).copied().unwrap_or(3);
            let cw = UnicodeWidthStr::width_cjk(cell);

            // Truncate cell if it's too wide
            let display_cell = if cw > target_width {
                truncate_to_width(cell, target_width.saturating_sub(1))
            } else {
                cell.to_string()
            };
            let display_width = UnicodeWidthStr::width_cjk(display_cell.as_str());

            let pad = target_width.saturating_sub(display_width);
            let alignment = col_alignments.get(ci).unwrap_or(&Alignment::Left);
            let padded = match alignment {
                Alignment::Left => format!("{}{} ", display_cell, " ".repeat(pad)),
                Alignment::Center => {
                    let left_pad = pad / 2;
                    let right_pad = pad - left_pad;
                    format!(
                        "{}{}{} ",
                        " ".repeat(left_pad),
                        display_cell,
                        " ".repeat(right_pad)
                    )
                }
                Alignment::Right => format!("{}{} ", " ".repeat(pad), display_cell),
            };
            spans.push(Span::styled(padded, cell_style));
            spans.push(Span::styled("│ ", border_style));
        }
        lines.push(Line::from(spans));

        // Header-body separator
        if is_header && rows.len() > 1 {
            let mut sep_spans = Vec::new();
            sep_spans.push(Span::styled("├", border_style));
            for (ci, w) in adjusted_widths.iter().enumerate() {
                sep_spans.push(Span::styled("─".repeat(w + 2), border_style));
                if ci < col_count - 1 {
                    sep_spans.push(Span::styled("┼", border_style));
                }
            }
            sep_spans.push(Span::styled("┤", border_style));
            lines.push(Line::from(sep_spans));
        }
    }

    // Bottom border
    let mut bottom_spans = Vec::new();
    bottom_spans.push(Span::styled("└", border_style));
    for (ci, w) in adjusted_widths.iter().enumerate() {
        bottom_spans.push(Span::styled("─".repeat(w + 2), border_style));
        if ci < col_count - 1 {
            bottom_spans.push(Span::styled("┴", border_style));
        }
    }
    bottom_spans.push(Span::styled("┘", border_style));
    lines.push(Line::from(bottom_spans));
}

/// Truncate a string to fit within a certain display width
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let ch_width = UnicodeWidthChar::width_cjk(ch).unwrap_or(0);
        if width + ch_width > max_width {
            result.push('…');
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result
}

/// Convert content blocks to Ratatui Lines for display.
/// get custom dimmed rendering.
pub fn render_content_blocks(
    blocks: &[ContentBlock],
    wrap_width: Option<usize>,
) -> Vec<Line<'static>> {
    const MAX_MD_LINES: usize = 3000;
    let mut lines = Vec::new();

    for block in blocks {
        if lines.len() >= MAX_MD_LINES {
            lines.push(Line::from(Span::styled(
                "... (content too long, truncated) ...",
                Style::default().fg(Color::Yellow),
            )));
            break;
        }

        match block {
            ContentBlock::Thinking(text) => {
                lines.extend(render_thinking_lines(text, wrap_width));
            }
            ContentBlock::Text(text) => {
                let safe_text = text.replace('\t', "    ");
                lines.extend(render_markdown(&safe_text, wrap_width));
            }
        }
    }

    lines
}

/// Find the byte offset of the last paragraph boundary in the content.
/// A paragraph boundary is a double newline (`\n\n`) OUTSIDE a fenced code block.
/// Returns 0 if no safe boundary is found (entire content is unstable).
fn last_paragraph_boundary(content: &str) -> usize {
    let mut candidates: Vec<usize> = Vec::new();
    let mut in_fence = false;

    // 逐行扫描，跟踪代码围栏状态；围栏内的空行是代码内容，不能作为切分点，
    // 否则 stable 部分会包含未闭合围栏，流式期间反复重排导致页面跳动
    let bytes = content.as_bytes();
    let mut line_start = 0usize;
    for i in 0..=bytes.len() {
        if i == bytes.len() || bytes[i] == b'\n' {
            let line = &content[line_start..i];
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
            } else if !in_fence && line.trim().is_empty() && line_start > 0 && i < bytes.len() {
                // 边界取换行符之后；i == len 是文件末尾虚拟行，不是有效边界
                candidates.push(i + 1);
            }
            line_start = i + 1;
        }
    }

    // 从后往前取第一个"安全"边界：边界之后的下一个非空行不能是列表项，
    // 否则会把列表从中间切开（unstable 段重新编号、间距翻倍）
    for &boundary in candidates.iter().rev() {
        let rest = content[boundary..].trim_start();
        if !starts_list_item(rest) {
            return boundary;
        }
    }

    0
}

/// 判断文本开头是否是 markdown 列表项标记
fn starts_list_item(s: &str) -> bool {
    let first_line = s.lines().next().unwrap_or("").trim_start();
    for marker in ["- ", "* ", "+ ", "• "] {
        if first_line.starts_with(marker) {
            return true;
        }
    }
    // 有序列表: "1. " / "23) " 等
    let digits: String = first_line
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if !digits.is_empty() {
        let after = &first_line[digits.len()..];
        if after.starts_with(". ") || after.starts_with(") ") {
            return true;
        }
    }
    false
}

/// Render markdown content incrementally by splitting into stable and unstable parts.
///
/// During streaming, content grows token by token. Instead of re-parsing the
/// entire content every frame, this function:
/// 1. Finds the last paragraph boundary (double newline)
/// 2. Treats everything before as "stable" (already rendered correctly)
/// 3. Only re-parses the "unstable" tail (current paragraph being streamed)
///
/// `stable_cache`: previously rendered stable lines (if available).
/// Returns `(stable_lines, unstable_lines)` where `stable_lines` should be
/// cached and reused, and `unstable_lines` should be re-rendered each frame.
pub fn render_markdown_incremental(
    content: &str,
    wrap_width: Option<usize>,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    // 防御：边界必须落在内容范围内且是 UTF-8 字符边界，否则整体视为 unstable
    let boundary = {
        let b = last_paragraph_boundary(content);
        if b == 0 || b > content.len() || !content.is_char_boundary(b) {
            0
        } else {
            b
        }
    };

    if boundary == 0 {
        // No paragraph boundary found — entire content is unstable
        let lines = render_markdown(content, wrap_width);
        return (Vec::new(), lines);
    }

    let stable_text = &content[..boundary];
    let unstable_text = &content[boundary..];

    let stable_lines = render_markdown(stable_text, wrap_width);
    let unstable_lines = if unstable_text.is_empty() {
        Vec::new()
    } else {
        render_markdown(unstable_text, wrap_width)
    };

    (stable_lines, unstable_lines)
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn debug_table_streaming() {
        // 模拟流式：表格前有完整段落，表格正在输出
        let content = "前面段落\n\n| A | B |\n|---|---|\n| 1 | 2 |";
        let (stable, unstable) = render_markdown_incremental(content, Some(80));
        let all: Vec<String> = stable
            .iter()
            .chain(unstable.iter())
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect();
        for s in &all {
            println!("S: [{}]", s);
        }
        assert!(
            all.iter().any(|s| s.contains('┌')) && all.iter().any(|s| s.contains('A')),
            "streaming table must render"
        );
    }

    #[test]
    fn debug_table_render() {
        let content = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let lines = render_markdown(content, Some(80));
        for l in &lines {
            let s: String = l.spans.iter().map(|s| s.content.to_string()).collect();
            println!("LINE: [{}]", s);
        }
        assert!(
            lines.iter().any(|l| l
                .spans
                .iter()
                .any(|s| s.content.contains('┌') || s.content.contains('│'))),
            "table borders not found"
        );
    }

    #[test]
    fn boundary_never_exceeds_content_len() {
        // 内容以空行结尾 — 旧实现会把边界推到 len+1 导致切片 panic
        let content = "para\n\n";
        let b = last_paragraph_boundary(content);
        assert!(b <= content.len());
        let content2 = "para\n\n\n";
        assert!(last_paragraph_boundary(content2) <= content2.len());
    }

    #[test]
    fn boundary_skips_blank_lines_inside_code_fence() {
        let content = "before\n\n```\ncode\n\nstill code\n```\n";
        let b = last_paragraph_boundary(content);
        // 围栏开始后的空行不能作为边界：边界必须 <= 围栏起始处
        let fence_start = content.find("```").unwrap();
        assert!(b <= fence_start, "boundary {} must not be inside fence", b);
    }

    #[test]
    fn boundary_does_not_split_list() {
        let content = "1. one\n\n2. two\n\n3. three";
        let b = last_paragraph_boundary(content);
        // 列表中间的空行不能作为边界：整个列表必须留在 unstable 段
        assert_eq!(b, 0);
    }

    #[test]
    fn boundary_after_complete_paragraph() {
        let content = "first para\n\nsecond para streaming";
        let b = last_paragraph_boundary(content);
        assert_eq!(&content[..b], "first para\n\n");
    }

    #[test]
    fn incremental_never_panics_on_multibyte() {
        // 中文内容 + 空行结尾
        let content = "第一段\n\n第二段流式中\n\n";
        let (stable, unstable) = render_markdown_incremental(content, Some(80));
        assert!(!stable.is_empty());
        let _ = unstable;
    }
}
