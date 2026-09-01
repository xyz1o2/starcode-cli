use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 渲染分隔线
pub fn render_separator(width: usize, color: Color) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width), Style::default().fg(color)))
}

/// 渲染带标题的分隔线
pub fn render_titled_separator(title: &str, width: usize, color: Color) -> Line<'static> {
    let title_len = title.len();
    let separator_len = width.saturating_sub(title_len + 4); // 两边各2个空格

    Line::from(vec![
        Span::styled("─".repeat(separator_len / 2), Style::default().fg(color)),
        Span::styled("  ", Style::default()),
        Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            "─".repeat(separator_len - separator_len / 2),
            Style::default().fg(color),
        ),
    ])
}

/// 渲染双线分隔符
pub fn render_double_separator(width: usize, color: Color) -> Line<'static> {
    Line::from(Span::styled("═".repeat(width), Style::default().fg(color)))
}

/// 渲染点状分隔符
pub fn render_dotted_separator(width: usize, color: Color) -> Line<'static> {
    Line::from(Span::styled("·".repeat(width), Style::default().fg(color)))
}

/// 渲染空行
pub fn render_empty_line() -> Line<'static> {
    Line::from("")
}

/// 渲染带内容的行
pub fn render_content_line(content: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        content.to_string(),
        Style::default().fg(color),
    ))
}

/// 渲染带图标的行
pub fn render_icon_line(
    icon: &str,
    content: &str,
    icon_color: Color,
    content_color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
        Span::styled(content.to_string(), Style::default().fg(content_color)),
    ])
}

/// 渲染带缩进的行
pub fn render_indented_line(content: &str, indent: usize, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ".repeat(indent), Style::default()),
        Span::styled(content.to_string(), Style::default().fg(color)),
    ])
}

/// 渲染带边框的内容块
pub fn render_bordered_block(
    title: &str,
    content: Vec<Line<'static>>,
    width: usize,
    border_color: Color,
    title_color: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // 顶部边框
    let top_line = if title.is_empty() {
        render_separator(width, border_color)
    } else {
        render_titled_separator(title, width, border_color)
    };
    lines.push(top_line);

    // 内容
    for line in content {
        let mut new_spans = vec![Span::styled("│ ", Style::default().fg(border_color))];
        new_spans.extend(line.spans);
        lines.push(Line::from(new_spans));
    }

    // 底部边框
    lines.push(render_separator(width, border_color));

    lines
}
