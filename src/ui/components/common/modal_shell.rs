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

/// Geometry spec for a modal shell.
pub struct ModalSpec {
    pub percent_x: u16,
    pub percent_y: u16,
    pub title: String,
    pub accent: Color,
}

impl Default for ModalSpec {
    fn default() -> Self {
        Self {
            percent_x: 70,
            percent_y: 60,
            title: String::new(),
            accent: Color::Cyan,
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
    let area = centered_rect(spec.percent_x, spec.percent_y, area);
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
pub fn footer_hints(hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                "  ·  ",
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {}", label),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

/// Status icon for list rows: connected / error / disabled (shape + color so
/// it also works in colorblind mode).
pub fn status_spans(connected: bool, disabled: bool) -> Vec<Span<'static>> {
    if disabled {
        return vec![Span::styled(
            "○",
            Style::default().fg(Color::DarkGray),
        )];
    }
    if connected {
        return vec![Span::styled(
            "●",
            Style::default().fg(Color::Green),
        )];
    }
    vec![Span::styled("✗", Style::default().fg(Color::Red))]
}

/// Build highlight/normal styles for row `index` given `selected`.
pub fn row_style(index: usize, selected: usize) -> Style {
    if index == selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
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
