use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::ui::themes::theme::Theme;

/// 徽章样式
pub enum BadgeStyle {
    Success,
    Error,
    Warning,
    Info,
    Neutral,
    Custom(Color),
}

impl BadgeStyle {
    pub fn color(&self, theme: &Theme) -> Color {
        match self {
            BadgeStyle::Success => theme.tool_success,
            BadgeStyle::Error => theme.tool_error,
            BadgeStyle::Warning => theme.warning,
            BadgeStyle::Info => theme.info,
            BadgeStyle::Neutral => theme.inactive,
            BadgeStyle::Custom(color) => *color,
        }
    }
}

/// 渲染徽章
pub fn render_badge(text: &str, style: BadgeStyle, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {} ", text),
        Style::default()
            .fg(style.color(theme))
            .add_modifier(Modifier::BOLD),
    )
}

/// 渲染小徽章（无背景）
pub fn render_small_badge(text: &str, style: BadgeStyle, theme: &Theme) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::default()
            .fg(style.color(theme))
            .add_modifier(Modifier::BOLD),
    )
}

/// 渲染计数徽章
pub fn render_count_badge(count: usize, style: BadgeStyle, theme: &Theme) -> Span<'static> {
    if count > 0 {
        render_badge(&count.to_string(), style, theme)
    } else {
        Span::raw("")
    }
}

/// 渲染状态徽章
pub fn render_status_badge(success: bool, theme: &Theme) -> Span<'static> {
    if success {
        render_badge("✓", BadgeStyle::Success, theme)
    } else {
        render_badge("✗", BadgeStyle::Error, theme)
    }
}

/// 渲染工具状态徽章
pub fn render_tool_status_badge(status: &str, theme: &Theme) -> Span<'static> {
    match status {
        "success" => render_badge("✓", BadgeStyle::Success, theme),
        "error" => render_badge("✗", BadgeStyle::Error, theme),
        "warning" => render_badge("⚠", BadgeStyle::Warning, theme),
        "info" => render_badge("ℹ", BadgeStyle::Info, theme),
        "loading" => render_badge("●", BadgeStyle::Info, theme),
        "pending" => render_badge("○", BadgeStyle::Neutral, theme),
        _ => render_badge("·", BadgeStyle::Neutral, theme),
    }
}

/// 渲染优先级徽章
pub fn render_priority_badge(priority: &str, theme: &Theme) -> Span<'static> {
    match priority {
        "high" => render_badge("HIGH", BadgeStyle::Error, theme),
        "medium" => render_badge("MED", BadgeStyle::Warning, theme),
        "low" => render_badge("LOW", BadgeStyle::Info, theme),
        _ => render_badge(priority, BadgeStyle::Neutral, theme),
    }
}

/// 渲染标签徽章
pub fn render_tag_badge(tag: &str, theme: &Theme) -> Span<'static> {
    Span::styled(format!(" {} ", tag), Style::default().fg(theme.primary))
}
