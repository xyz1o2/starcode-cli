//! Unified modal shell — the shared chrome every modal renders inside of.
//!
//! Extracts the previously copy-pasted `centered_rect` + rounded-border +
//! title + footer hint-bar pattern (input_modal / status_modal / palette all
//! had private copies). A modal only produces its body lines; the shell owns
//! geometry, clearing, borders and the key-hint footer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear},
    Frame,
};

use crate::ui::themes::theme::Theme;

/// Geometry spec for a modal shell.
pub struct ModalSpec {
    pub percent_x: u16,
    pub percent_y: u16,
    pub title: String,
    pub accent: Color,
    /// Fixed cell size override. 小对话框（错误浮层这类 60 列左右的确认弹窗）
    /// 不适合百分比几何：屏一大弹窗就被拉成横幅。给了 size 就按固定格子居中，
    /// 并 clamp 进 `area`；不给则走 `percent_x/percent_y`。
    pub size: Option<(u16, u16)>,
}

impl Default for ModalSpec {
    fn default() -> Self {
        Self {
            percent_x: 70,
            percent_y: 60,
            title: String::new(),
            accent: Color::Cyan,
            size: None,
        }
    }
}

/// Centered rect (single shared implementation).
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Draw the modal chrome (clear + rounded border + title) and return the
/// inner area the caller should render its body into.
pub fn modal_shell(f: &mut Frame<'_>, area: Rect, spec: &ModalSpec) -> Rect {
    let area = match spec.size {
        Some((width, height)) => {
            let width = width.min(area.width.saturating_sub(2)).max(4);
            let height = height.min(area.height.saturating_sub(2)).max(3);
            Rect {
                x: area.x + area.width.saturating_sub(width) / 2,
                y: area.y + area.height.saturating_sub(height) / 2,
                width,
                height,
            }
        }
        None => centered_rect(spec.percent_x, spec.percent_y, area),
    };
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", spec.title),
            Style::default()
                .fg(spec.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(spec.accent));

    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let inner = area;
    Rect {
        x: inner.x.saturating_add(1),
        y: inner.y.saturating_add(1),
        width: inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(2),
    }
}

/// Reserve the last row of `inner` for the footer and return the body area.
pub fn with_footer(inner: Rect) -> (Rect, Rect) {
    if inner.height <= 2 {
        return (inner, Rect::default());
    }
    let body = Rect {
        height: inner.height - 1,
        ..inner
    };
    let footer = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    (body, footer)
}

/// Footer key-hint bar: `[(key, label), ...]` rendered dim with bright keys.
/// 使用当前会话的主题（通过全局默认主题获取）。
pub fn footer_hints(hints: &[(&str, &str)]) -> Line<'static> {
    // 使用默认主题颜色。大多数调用点都有 state，但签名已经在很多地方使用，
    // 为了避免破坏性更改，使用 primary 色的合理默认值。
    use crate::ui::themes::theme::Theme;
    let theme = Theme::default_dark();  // 回退到默认深色主题
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                "  ·  ",
                crate::ui::themes::theme::muted_style(),
            ));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {}", label),
            crate::ui::themes::theme::muted_style(),
        ));
    }
    Line::from(spans)
}

/// Status icon for list rows: connected / error / disabled (shape + color so
/// it also works in colorblind mode).
pub fn status_spans(connected: bool, disabled: bool) -> Vec<Span<'static>> {
    use crate::ui::themes::theme::Theme;
    let theme = Theme::default_dark();
    if disabled {
        return vec![Span::styled("○", crate::ui::themes::theme::muted_style())];
    }
    if connected {
        return vec![Span::styled("●", Style::default().fg(theme.success))];
    }
    vec![Span::styled("✗", Style::default().fg(theme.error))]
}

/// Build highlight/normal styles for row `index` given `selected`.
/// 使用主题颜色而不是硬编码，与主 UI 保持一致。
pub fn row_style(index: usize, selected: usize) -> Style {
    use crate::ui::themes::theme::Theme;
    let theme = Theme::default_dark();
    if index == selected {
        Style::default()
            .bg(theme.selection_bg)
            .fg(theme.foreground)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.foreground)
    }
}

/// Render body lines inside the shell body area (clipped, no wrap).
pub fn render_body(f: &mut Frame<'_>, body: Rect, lines: Vec<Line<'static>>) {
    if body.area() == 0 {
        return;
    }
    let paragraph = ratatui::widgets::Paragraph::new(lines);
    f.render_widget(paragraph, body);
}
