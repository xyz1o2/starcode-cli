/// Compression visualization — shows context compression status.
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Compression state
#[derive(Debug, Clone, Default)]
pub struct CompressionState {
    pub is_compressed: bool,
    pub original_tokens: u32,
    pub compressed_tokens: u32,
    pub compression_ratio: f64,
    pub turn_number: usize,
}

impl CompressionState {
    pub fn format_status(&self) -> String {
        if !self.is_compressed {
            return "No compression".to_string();
        }
        format!(
            "Compressed: {} → {} ({:.1}% reduction)",
            format_tokens(self.original_tokens),
            format_tokens(self.compressed_tokens),
            (1.0 - self.compression_ratio) * 100.0
        )
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

/// Render compression status
pub fn render_compression_status(f: &mut Frame, state: &CompressionState, area: Rect) {
    let lines = if state.is_compressed {
        vec![
            Line::from(vec![
                Span::styled("  Compression: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Active",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Turn: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", state.turn_number),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Original: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format_tokens(state.original_tokens),
                    Style::default().fg(Color::Red),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Compressed: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format_tokens(state.compressed_tokens),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Reduction: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:.1}%", (1.0 - state.compression_ratio) * 100.0),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("  Compression: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Not needed",
                    Style::default().fg(Color::Green),
                ),
            ]),
        ]
    };

    let block = Block::default()
        .title(" Context Compression ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    f.render_widget(Paragraph::new(lines).block(block), area);
}
