/// Context visualization — shows token usage breakdown by category.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

/// Token usage breakdown
#[derive(Debug, Clone, Default)]
pub struct TokenBreakdown {
    pub system_prompt: u32,
    pub conversation: u32,
    pub tool_outputs: u32,
    pub context_files: u32,
    pub total: u32,
    pub max_context: u32,
}

impl TokenBreakdown {
    pub fn usage_percent(&self) -> f64 {
        if self.max_context == 0 {
            0.0
        } else {
            (self.total as f64 / self.max_context as f64) * 100.0
        }
    }

    pub fn format_total(&self) -> String {
        format_tokens(self.total)
    }

    pub fn format_max(&self) -> String {
        format_tokens(self.max_context)
    }
}

fn format_tokens(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Render context visualization
pub fn render_context_visualization(f: &mut Frame, breakdown: &TokenBreakdown, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Progress bar
            Constraint::Length(4), // Breakdown
        ])
        .split(area);

    // Title
    let pct = breakdown.usage_percent();
    let title_color = if pct >= 90.0 {
        Color::Red
    } else if pct >= 75.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let title = Line::from(vec![
        Span::styled(" Context Window: ", Style::default().fg(Color::White)),
        Span::styled(
            format!("{}/{}", breakdown.format_total(), breakdown.format_max()),
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({:.1}%)", pct),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // Progress bar
    let ratio = (pct / 100.0).min(1.0) as f64;
    let bar_color = if pct >= 90.0 {
        Color::Red
    } else if pct >= 75.0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let gauge = Gauge::default()
        .ratio(ratio)
        .style(Style::default().fg(bar_color));
    f.render_widget(gauge, chunks[1]);

    // Breakdown
    let breakdown_lines = vec![
        breakdown_line(
            "System",
            breakdown.system_prompt,
            breakdown.total,
            Color::Cyan,
        ),
        breakdown_line(
            "Conversation",
            breakdown.conversation,
            breakdown.total,
            Color::White,
        ),
        breakdown_line(
            "Tool Outputs",
            breakdown.tool_outputs,
            breakdown.total,
            Color::Yellow,
        ),
        breakdown_line(
            "Context Files",
            breakdown.context_files,
            breakdown.total,
            Color::Green,
        ),
    ];
    f.render_widget(Paragraph::new(breakdown_lines), chunks[2]);
}

fn breakdown_line(label: &str, tokens: u32, total: u32, color: Color) -> Line<'static> {
    let pct = if total > 0 {
        (tokens as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    Line::from(vec![
        Span::styled(
            format!("  {:<15}", label),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{:>8}", format_tokens(tokens)),
            Style::default().fg(color),
        ),
        Span::styled(
            format!("  ({:.1}%)", pct),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}
