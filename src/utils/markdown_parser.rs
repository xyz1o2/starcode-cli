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

/// 代码块正文的一行 → 渲染行（可能因折行变成多行）。
///
/// 左侧统一缩进 2 列，与 `  ┌` / `  └` 边框对齐（此前正文顶到第 0 列，
/// 边框却从第 2 列开始，看起来像框歪了）。折行宽度相应减 2。
fn code_block_lines(
    line_str: &str,
    language: &str,
    wrap_width: Option<usize>,
) -> Vec<Line<'static>> {
    const GUTTER: &str = "  ";
    let mut out = Vec::new();
    if line_str.trim().is_empty() {
        out.push(Line::from(""));
        return out;
    }
    let pieces = match wrap_width {
        Some(max_w) => {
            crate::ui::utils::render::wrap_text_to_width(line_str, max_w.saturating_sub(2))
        }
        None => vec![line_str.to_string()],
    };
    for piece in pieces {
        let highlighted = highlight_code_line(&piece, language);
        let mut line_spans: Vec<Span<'static>> = vec![Span::raw(GUTTER)];
        line_spans.extend(highlighted.spans.iter().cloned());
        out.push(Line::from(line_spans));
    }
    out
}

/// 把累积的内联 spans 按显示宽度折行，样式与空格原样保留。
///
/// 折行必须在"整行内联内容收齐之后"做，不能在每个 `Event::Text` 里各自折 ——
/// 一个带 `**bold**`/`` `code` ``/链接的段落会拆成多个 Text 事件，逐事件折行时
/// 每个片段单看都不超宽，拼到同一行后整行远超终端宽度（右侧被裁掉）。
///
/// `cont_prefix` 是续行前缀（列表悬挂缩进 / 引用竖线），首行不加。
fn wrap_spans_to_lines(
    spans: &[Span<'static>],
    wrap_width: Option<usize>,
    cont_prefix: &[Span<'static>],
) -> Vec<Line<'static>> {
    if spans.is_empty() {
        return Vec::new();
    }
    let Some(total) = wrap_width else {
        return vec![Line::from(spans.to_vec())];
    };
    // 展平成字符流（样式逐字符对齐），折行后再按样式合并回 span
    let mut chars: Vec<char> = Vec::new();
    let mut styles: Vec<Style> = Vec::new();
    for span in spans {
        for c in span.content.chars() {
            chars.push(c);
            styles.push(span.style);
        }
    }
    let prefix_w: usize = cont_prefix
        .iter()
        .map(|s| UnicodeWidthStr::width_cjk(s.content.as_ref()))
        .sum();
    let ranges = crate::ui::utils::render::wrap_char_ranges(
        &chars,
        total.max(1),
        total.saturating_sub(prefix_w).max(1),
    );

    let mut out = Vec::with_capacity(ranges.len());
    for (idx, (s, e)) in ranges.into_iter().enumerate() {
        let mut line_spans: Vec<Span<'static>> = Vec::new();
        if idx > 0 {
            line_spans.extend(cont_prefix.iter().cloned());
        }
        let mut k = s;
        while k < e {
            let style = styles[k];
            let mut text = String::new();
            while k < e && styles[k] == style {
                text.push(chars[k]);
                k += 1;
            }
            line_spans.push(Span::styled(text, style));
        }
        out.push(Line::from(line_spans));
    }
    out
}

/// 输出当前累积的内联内容（折行 + 续行前缀）。
///
/// 无内容时什么都不做：空行一律由块级处理显式插入。旧实现在这里无条件补一个
/// 空行，嵌套列表结束时会凭空多出一行，把同级的下一项与前面隔开。
fn flush_inline(
    spans: &mut Vec<Span<'static>>,
    lines: &mut Vec<Line<'static>>,
    wrap_width: Option<usize>,
    cont_prefix: &[Span<'static>],
) {
    if spans.is_empty() {
        return;
    }
    lines.extend(wrap_spans_to_lines(spans, wrap_width, cont_prefix));
    spans.clear();
}

/// 续行前缀：引用块用竖线，列表用与标记等宽的缩进。
fn continuation_prefix(blockquote_depth: usize, list_marker_width: usize) -> Vec<Span<'static>> {
    if blockquote_depth > 0 {
        vec![Span::styled(
            "│ ".repeat(blockquote_depth),
            Style::default().fg(Color::DarkGray),
        )]
    } else if list_marker_width > 0 {
        vec![Span::raw(" ".repeat(list_marker_width))]
    } else {
        Vec::new()
    }
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
    // 当前列表项标记（"• " / "1. " 含缩进）的显示宽度，用于续行悬挂缩进
    let mut list_marker_width: usize = 0;
    let mut language = String::new();
    let mut code_line_number: usize = 0;
    let mut blockquote_depth: usize = 0;
    let mut link_url_stack: Vec<String> = Vec::new();
    let mut table_alignments: Vec<pulldown_cmark::Alignment> = Vec::new();

    let current_style = |stack: &[Style]| stack.last().copied().unwrap_or_default();

    // 折行发生在这里（整行内联内容收齐后），续行前缀按当前引用/列表状态计算
    macro_rules! flush_line {
        () => {{
            let cp = continuation_prefix(blockquote_depth, list_marker_width);
            flush_inline(&mut current_spans, &mut lines, wrap_width, &cp);
        }};
    }

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    // 引用块内每个段落自带竖线前缀（在 BlockQuote 起始处只加一次的话，
                    // 多段引用的第二段起就没有前缀了）
                    if blockquote_depth > 0 && current_spans.is_empty() {
                        current_spans.push(Span::styled(
                            "│ ".repeat(blockquote_depth),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
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
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_line_number = 0;
                    language = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    flush_line!();
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
                        flush_line!();
                    }
                }
                Tag::Item => {
                    if list_depth > 0 {
                        let indent = "  ".repeat(list_depth.saturating_sub(1));
                        let marker = if list_is_ordered.last().copied().unwrap_or(false) {
                            let n = list_counters
                                .last_mut()
                                .map(|c| {
                                    *c += 1;
                                    *c
                                })
                                .unwrap_or(1);
                            format!("{}{}. ", indent, n)
                        } else {
                            format!("{}• ", indent)
                        };
                        // 续行按标记宽度悬挂缩进，长条目折行后与首行文字左对齐。
                        // 这里用 width（非 width_cjk）：ratatui 与终端按 width 排版，
                        // "• " 这类 Ambiguous 字符用 width_cjk 会多算一列、缩进歪一格
                        list_marker_width = UnicodeWidthStr::width(marker.as_str());
                        current_spans
                            .push(Span::styled(marker, Style::default().fg(Color::Yellow)));
                    }
                }
                Tag::FootnoteDefinition(_) => {}
                Tag::Table(alignments) => {
                    in_table = true;
                    table_rows.clear();
                    // 必须连同 current_row 一起清：上一张表最后一行结束后没人清 current_row
                    // （清除只发生在下一行开始时），残留行会被下一张表的表头单元格追加，
                    // 渲染出 [上一表末行 | 下一表表头] 的合并表头
                    current_row.clear();
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
                    flush_line!();
                    // 引用块内的段落不各自留空行（空行没有竖线前缀，会把引用切开），
                    // 块间空行统一由 TagEnd::BlockQuote 补
                    if blockquote_depth == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    flush_line!();
                    lines.push(Line::from(""));
                }
                TagEnd::BlockQuote(_) => {
                    // 先 flush 再降深度：续行前缀依赖当前深度
                    flush_line!();
                    blockquote_depth = blockquote_depth.saturating_sub(1);
                    if blockquote_depth == 0 {
                        lines.push(Line::from(""));
                    }
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
                    flush_line!();
                    if list_depth == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Item => {
                    flush_line!();
                    list_marker_width = 0;
                }
                TagEnd::Table => {
                    if !table_rows.is_empty() {
                        render_table(&mut lines, &table_rows, &table_alignments, wrap_width);
                    }
                    in_table = false;
                    table_rows.clear();
                    // 表格结束即清残留行，不把状态带进后续块（与 Tag::Table 处双保险）
                    current_row.clear();
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
                        lines.extend(code_block_lines(line_str, &language, wrap_width));
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
                        lines.extend(code_block_lines(line_str, &language, wrap_width));
                    }
                } else if in_table_head || in_table {
                    current_cell.push_str(text);
                } else {
                    // 不在这里折行：本段的内联片段还没收齐（见 wrap_spans_to_lines）
                    let style = current_style(&style_stack);
                    current_spans.push(Span::styled(text.to_string(), style));
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
                if current_spans.is_empty() {
                    lines.push(Line::from(""));
                } else {
                    flush_line!();
                }
                if blockquote_depth > 0 {
                    let prefix = "│ ".repeat(blockquote_depth);
                    current_spans.push(Span::styled(prefix, Style::default().fg(Color::DarkGray)));
                }
            }
            Event::Rule => {
                flush_line!();
                // 铺满可用宽度，而不是固定 60 列（窄终端会溢出、宽终端又太短）
                let rule = "─".repeat(wrap_width.unwrap_or(60).clamp(3, 200));
                lines.push(Line::from(Span::styled(
                    rule,
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
            }
            Event::TaskListMarker(checked) => {
                // 复选框取代圆点标记，避免渲染成 "• [ ] 任务"
                if let Some(last) = current_spans.last_mut() {
                    if last.content.ends_with("• ") {
                        let indent = last.content[..last.content.len() - "• ".len()].to_string();
                        list_marker_width = UnicodeWidthStr::width(indent.as_str()) + 4;
                        *last = Span::raw(indent);
                    }
                }
                let marker = if checked { "[x] " } else { "[ ] " };
                current_spans.push(Span::styled(marker, Style::default().fg(Color::Yellow)));
            }
        }
    }

    flush_line!();

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
        // 每列渲染为 " 内容(w) "（恰占 w+2 列）+ "│"，与边框 ─(w+2)+┬ 逐列严格对齐，
        // 行尾不再多出一列空格
        spans.push(Span::styled("│", border_style));
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
                Alignment::Left => format!("{}{}", display_cell, " ".repeat(pad)),
                Alignment::Center => {
                    let left_pad = pad / 2;
                    let right_pad = pad - left_pad;
                    format!(
                        "{}{}{}",
                        " ".repeat(left_pad),
                        display_cell,
                        " ".repeat(right_pad)
                    )
                }
                Alignment::Right => format!("{}{}", " ".repeat(pad), display_cell),
            };
            spans.push(Span::styled(format!(" {} ", padded), cell_style));
            spans.push(Span::styled("│", border_style));
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

#[cfg(test)]
mod table_render_tests {
    use super::*;

    fn rendered(md: &str, width: usize) -> Vec<String> {
        render_markdown(md, Some(width))
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect()
    }

    #[test]
    fn consecutive_tables_keep_headers_separate() {
        // 回归：表格结束后 current_row 残留上一张表的最后一行，下一张表的表头
        // 单元格会追加到它上面，渲染出 [上一表末行 | 下一表表头] 的合并表头。
        // 典型场景：相邻两个小节各带一张表，中间只隔一个标题。
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n\n# Title\n\n| C | D |\n|---|---|\n| 3 | 4 |\n";
        let lines = rendered(md, 80);

        // 含 "C" 的行必然是第二张表的表头，其中不允许出现第一张表的数据 "1"
        let head2 = lines
            .iter()
            .find(|l| l.contains('C'))
            .expect("second table header must render");
        assert!(
            !head2.contains(" 1 "),
            "下一张表的表头混入了上一张表的末行: {:?}",
            head2
        );
        // 标题独立成行，两张表各自有完整的上边框
        assert!(lines.iter().any(|l| l.trim() == "Title"));
        assert_eq!(lines.iter().filter(|l| l.contains('┌')).count(), 2);
    }

    #[test]
    fn consecutive_tables_with_cjk_cells_keep_headers_separate() {
        // CJK 单元格 + 行内代码 + emoji（真实对话回复的构成）同样不能串行
        let md = "| 模块 | 结果 |\n|---|---|\n| 前端 | 正常 |\n\n### 下节\n\n| 端点 | 状态 |\n|---|---|\n| `GET /x` | ✅ |\n";
        let lines = rendered(md, 80);

        let head2 = lines
            .iter()
            .find(|l| l.contains("端点"))
            .expect("second table header must render");
        assert!(
            !head2.contains("前端"),
            "下一张表的表头混入了上一张表的末行: {:?}",
            head2
        );
    }

    #[test]
    fn table_rows_align_with_borders() {
        // 内容行与边框行必须等宽：行尾不允许再多一列（旧实现在每个单元格后
        // 追加 "│ "，导致最右侧竖线落在 ┐/┘ 之外一格）
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let lines = rendered(md, 80);
        let border = lines
            .iter()
            .find(|l| l.starts_with('┌'))
            .expect("top border must render");
        let row = lines
            .iter()
            .find(|l| l.starts_with('│'))
            .expect("table row must render");
        assert_eq!(
            row.chars().count(),
            border.chars().count(),
            "row {:?} vs border {:?}",
            row,
            border
        );
    }
}

#[cfg(test)]
mod inline_layout_tests {
    use super::*;

    fn rendered(md: &str, width: usize) -> Vec<String> {
        let blocks = parse_markdown_content_ext(md, false);
        render_content_blocks(&blocks, Some(width))
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect()
    }

    #[test]
    fn inline_marks_keep_surrounding_spaces() {
        // 回归：逐 Text 事件调用 split_whitespace 折行会吃掉片段首尾空格，
        // "with **bold** inside" 被粘成 "withboldinside"
        let lines = rendered("with **bold** inside and `code` here\n", 80);
        let body = lines.join("\n");
        assert!(
            body.contains("with bold inside and code here"),
            "行内标记两侧空格丢失: {:?}",
            body
        );
    }

    #[test]
    fn paragraph_with_inline_marks_wraps_within_width() {
        // 回归：折行在每个 Text 事件里各自进行，片段单看都不超宽，
        // 拼回同一行后整段远超终端宽度，右侧被裁掉
        let md = "这是一个比较长的段落，包含 **加粗文字** 和 *斜体* 以及 `inline_code`，\
                  还有一个 [链接](https://example.com/very/long/path)，用来测试折行。\n";
        for &w in &[40usize, 60, 80] {
            for line in rendered(md, w) {
                assert!(
                    UnicodeWidthStr::width_cjk(line.as_str()) <= w,
                    "宽度 {} 下溢出: {:?}",
                    w,
                    line
                );
            }
        }
    }

    #[test]
    fn code_block_keeps_indentation_and_aligns_with_border() {
        // 回归：wrap_text_to_width 曾用 split_whitespace 重组文本，代码块的
        // 前导缩进全部被吃掉，整段代码顶到最左侧
        let md = "```rust\nfn main() {\n    let x = 42;\n}\n```\n";
        let lines = rendered(md, 80);
        assert!(
            lines.iter().any(|l| l == "      let x = 42;"),
            "代码缩进丢失或未与边框对齐: {:?}",
            lines
        );
        // 正文与 "  ┌" / "  └" 边框同起于第 2 列
        assert!(lines.iter().any(|l| l.starts_with("  fn main() {")));
    }

    #[test]
    fn nested_list_keeps_siblings_adjacent() {
        // 回归：flush 在无内容时也塞一个空行，子列表结束后同级下一项被空行隔开
        let md = "- A\n  - A1\n  - A2\n- B\n";
        let lines = rendered(md, 80);
        let b = lines
            .iter()
            .position(|l| l.contains('B'))
            .expect("最后一项必须渲染");
        assert!(
            !lines[b - 1].trim().is_empty(),
            "子列表结束后多出空行: {:?}",
            lines
        );
    }

    #[test]
    fn long_list_item_wraps_with_hanging_indent() {
        let md = "- 这一项内容很长很长很长很长很长很长很长很长很长很长很长很长很长很长，需要折行\n";
        let lines = rendered(md, 40);
        assert!(lines.len() >= 2, "长条目必须折行: {:?}", lines);
        assert!(
            lines[1].starts_with("  ") && !lines[1].starts_with("  •"),
            "续行未做悬挂缩进: {:?}",
            lines[1]
        );
    }

    #[test]
    fn blockquote_continuation_keeps_bar() {
        let md = "> 引用内容很长很长很长很长很长很长很长很长很长很长很长很长很长很长很长\n";
        let lines = rendered(md, 40);
        let quoted: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
        assert!(quoted.len() >= 2, "长引用必须折行: {:?}", lines);
        assert!(
            quoted.iter().all(|l| l.starts_with('│')),
            "折行后的引用缺少竖线前缀: {:?}",
            quoted
        );
    }
}
