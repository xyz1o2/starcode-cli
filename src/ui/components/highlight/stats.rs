/// Usage statistics — session stats, model usage, cost tracking.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::ui::themes::theme::Theme;

/// Usage statistics
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    pub session_count: usize,
    pub total_messages: usize,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub models_used: Vec<(String, u64)>,
    pub session_duration: String,
}

impl UsageStats {
    pub fn format_total_tokens(&self) -> String {
        if self.total_tokens >= 1_000_000 {
            format!("{:.1}M", self.total_tokens as f64 / 1_000_000.0)
        } else if self.total_tokens >= 1_000 {
            format!("{:.1}k", self.total_tokens as f64 / 1_000.0)
        } else {
            self.total_tokens.to_string()
        }
    }
}

/// Render usage statistics
pub fn render_usage_stats(f: &mut Frame, stats: &UsageStats, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Session info
            Constraint::Length(2), // Token info
            Constraint::Length(2), // Cost info
            Constraint::Length(4), // Model usage
        ])
        .split(area);

    let block = Block::default()
        .title(" Usage Statistics ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    // Session info
    let session_lines = vec![
        Line::from(vec![
            Span::styled("  Sessions: ", Style::default().fg(theme.secondary)),
            Span::styled(
                format!("{}", stats.session_count),
                Style::default().fg(theme.foreground),
            ),
            Span::styled("  Duration: ", Style::default().fg(theme.secondary)),
            Span::styled(
                stats.session_duration.clone(),
                Style::default().fg(theme.foreground),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(session_lines).block(block.clone()), chunks[0]);

    // Token info
    let token_lines = vec![
        Line::from(vec![
            Span::styled("  Total Tokens: ", Style::default().fg(theme.secondary)),
            Span::styled(
                stats.format_total_tokens(),
                Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Messages: ", Style::default().fg(theme.secondary)),
            Span::styled(
                format!("{}", stats.total_messages),
                Style::default().fg(theme.foreground),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(token_lines).block(block.clone()), chunks[1]);

    // Cost info
    let cost_lines = vec![
        Line::from(vec![
            Span::styled("  Total Cost: ", Style::default().fg(theme.secondary)),
            Span::styled(
                format!("${:.2}", stats.total_cost),
                Style::default().fg(theme.success).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(cost_lines).block(block.clone()), chunks[2]);

    // Model usage
    let mut model_lines = vec![Line::from(vec![
        Span::styled("  Model Usage:", Style::default().fg(theme.secondary),
        ),
    ])];
    for (model, tokens) in &stats.models_used {
        let token_str = if *tokens >= 1_000_000 {
            format!("{:.1}M", *tokens as f64 / 1_000_000.0)
        } else if *tokens >= 1_000 {
            format!("{:.1}k", *tokens as f64 / 1_000.0)
        } else {
            tokens.to_string()
        };
        model_lines.push(Line::from(vec![
            Span::styled(format!("    {}: ", model), Style::default().fg(theme.secondary)),
            Span::styled(token_str, Style::default().fg(theme.primary)),
        ]));
    }
    f.render_widget(Paragraph::new(model_lines).block(block.clone()), chunks[3]);
}
