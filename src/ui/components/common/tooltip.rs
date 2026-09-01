use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::themes::theme::Theme;

/// 工具提示位置
pub enum TooltipPosition {
    Top,
    Bottom,
    Left,
    Right,
}

/// 渲染工具提示
pub fn render_tooltip(
    text: &str,
    position: TooltipPosition,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    match position {
        TooltipPosition::Top => {
            lines.push(render_tooltip_content(text, width, theme));
            lines.push(render_tooltip_arrow(position, theme));
        }
        TooltipPosition::Bottom => {
            lines.push(render_tooltip_arrow(position, theme));
            lines.push(render_tooltip_content(text, width, theme));
        }
        TooltipPosition::Left => {
            lines.push(render_tooltip_content(text, width, theme));
        }
        TooltipPosition::Right => {
            lines.push(render_tooltip_content(text, width, theme));
        }
    }
    
    lines
}

/// 渲染工具提示内容
fn render_tooltip_content(text: &str, width: usize, theme: &Theme) -> Line<'static> {
    let truncated = if text.len() > width {
        format!("{}...", &text[..width.saturating_sub(3)])
    } else {
        text.to_string()
    };
    
    Line::from(Span::styled(
        format!(" {} ", truncated),
        Style::default()
            .fg(theme.foreground)
            .bg(theme.border),
    ))
}

/// 渲染工具提示箭头
fn render_tooltip_arrow(position: TooltipPosition, theme: &Theme) -> Line<'static> {
    match position {
        TooltipPosition::Top => {
            Line::from(Span::styled(
                "▲",
                Style::default().fg(theme.border),
            ))
        }
        TooltipPosition::Bottom => {
            Line::from(Span::styled(
                "▼",
                Style::default().fg(theme.border),
            ))
        }
        TooltipPosition::Left => {
            Line::from(Span::styled(
                "◄",
                Style::default().fg(theme.border),
            ))
        }
        TooltipPosition::Right => {
            Line::from(Span::styled(
                "►",
                Style::default().fg(theme.border),
            ))
        }
    }
}

/// 渲染快捷键提示
pub fn render_key_hint(key: &str, description: &str, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {} ", key),
        Style::default()
            .fg(theme.foreground)
            .bg(theme.border)
            .add_modifier(Modifier::BOLD),
    )
}

/// 渲染多个快捷键提示
pub fn render_key_hints(hints: &[(&str, &str)], theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    
    for (i, (key, description)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        
        spans.push(render_key_hint(key, description, theme));
        spans.push(Span::styled(
            format!(" {}", description),
            Style::default().fg(theme.secondary),
        ));
    }
    
    Line::from(spans)
}

/// 渲染帮助文本
pub fn render_help_text(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::DarkGray),
    ))
}

/// 渲染错误提示
pub fn render_error_tooltip(text: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" ✗ {} ", text),
            Style::default()
                .fg(Color::White)
                .bg(Color::Red),
        )),
    ]
}

/// 渲染成功提示
pub fn render_success_tooltip(text: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" ✓ {} ", text),
            Style::default()
                .fg(Color::White)
                .bg(Color::Green),
        )),
    ]
}

/// 渲染警告提示
pub fn render_warning_tooltip(text: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" ⚠ {} ", text),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow),
        )),
    ]
}

/// 渲染信息提示
pub fn render_info_tooltip(text: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!(" ℹ {} ", text),
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue),
        )),
    ]
}
