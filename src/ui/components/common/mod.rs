pub mod badge;
pub mod dialog;
pub mod divider;
pub mod modal_shell;
pub mod progress;
pub mod spinner;
pub mod tooltip;

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// 通用的确认/取消按钮样式
pub fn render_action_buttons(
    actions: &[(&str, &str)], // (key, label)
    selected: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans = Vec::new();

    for (i, (key, label)) in actions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }

        let is_selected = i == selected;
        let style = if is_selected {
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        spans.push(Span::styled(format!(" {} ", key), style));
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(
            label.to_string(),
            if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
    }

    lines.push(Line::from(spans));
    lines
}

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

/// 渲染状态图标
pub fn render_status_icon(status: &str) -> Span<'static> {
    match status {
        "success" | "✓" => Span::styled("✓", Style::default().fg(Color::Green)),
        "error" | "✗" => Span::styled("✗", Style::default().fg(Color::Red)),
        "warning" | "⚠" => Span::styled("⚠", Style::default().fg(Color::Yellow)),
        "info" | "ℹ" => Span::styled("ℹ", Style::default().fg(Color::Blue)),
        "loading" | "●" => Span::styled("●", Style::default().fg(Color::Blue)),
        "pending" | "○" => Span::styled("○", Style::default().fg(Color::DarkGray)),
        _ => Span::styled("·", Style::default().fg(Color::DarkGray)),
    }
}

/// 渲染标签
pub fn render_label(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {} ", text),
        Style::default()
            .fg(Color::White)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

/// 渲染徽章
pub fn render_badge(text: &str, color: Color) -> Span<'static> {
    Span::styled(format!(" {} ", text), Style::default().fg(color))
}

/// 渲染计数器
pub fn render_counter(count: usize, color: Color) -> Span<'static> {
    if count > 0 {
        Span::styled(
            format!(" {} ", count),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    }
}

/// 渲染进度指示器
pub fn render_progress_indicator(progress: f32, width: usize) -> String {
    let filled = (progress * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);

    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

/// 渲染时间戳
pub fn render_timestamp(timestamp: &chrono::DateTime<chrono::Utc>) -> Span<'static> {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(timestamp);

    let time_str = if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else {
        format!("{}d ago", duration.num_days())
    };

    Span::styled(time_str, Style::default().fg(Color::DarkGray))
}

/// 渲染截断文本
pub fn render_truncated_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

/// 渲染换行文本
pub fn render_wrapped_text(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for line in text.lines() {
        if line.len() <= width {
            lines.push(Line::from(line.to_string()));
        } else {
            // 简单的换行逻辑
            let mut current_line = String::new();
            for word in line.split_whitespace() {
                if current_line.is_empty() {
                    current_line.push_str(word);
                } else if current_line.len() + 1 + word.len() <= width {
                    current_line.push(' ');
                    current_line.push_str(word);
                } else {
                    lines.push(Line::from(current_line));
                    current_line = word.to_string();
                }
            }
            if !current_line.is_empty() {
                lines.push(Line::from(current_line));
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}
