/// Session management — enhanced session list with preview and metadata.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Session entry with metadata
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub message_count: usize,
    pub preview: String,
}

/// Render session list with preview
pub fn render_session_list(f: &mut Frame, sessions: &[SessionEntry], selected: usize, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Session list
            Constraint::Percentage(60), // Preview
        ])
        .split(area);

    f.render_widget(Clear, area);

    // Session list
    let items: Vec<ListItem> = sessions
        .iter()
        .map(|s| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20}", truncate(&s.title, 20)),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" {} msgs", s.message_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Sessions ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 60))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(selected));
    f.render_stateful_widget(list, chunks[0], &mut state);

    // Preview
    let preview_text = sessions
        .get(selected)
        .map(|s| s.preview.as_str())
        .unwrap_or("Select a session to preview");
    let preview = Paragraph::new(preview_text)
        .block(
            Block::default()
                .title(" Preview ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::DarkGray))
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(preview, chunks[1]);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
