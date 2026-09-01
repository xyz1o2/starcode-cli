/// History search — fuzzy search through conversation history.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// History entry
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub index: usize,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

/// History search state
#[derive(Debug)]
pub struct HistorySearchState {
    pub query: String,
    pub results: Vec<HistoryEntry>,
    pub selected_index: usize,
}

impl HistorySearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.results.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }
}

/// Search history with fuzzy matching
pub fn search_history(history: &[(String, String)], query: &str) -> Vec<HistoryEntry> {
    if query.is_empty() {
        return history
            .iter()
            .enumerate()
            .map(|(i, (role, content))| HistoryEntry {
                index: i,
                role: role.clone(),
                content: truncate(content, 100),
                timestamp: String::new(),
            })
            .collect();
    }

    let query_lower = query.to_lowercase();
    history
        .iter()
        .enumerate()
        .filter(|(_, (_, content))| content.to_lowercase().contains(&query_lower))
        .map(|(i, (role, content))| HistoryEntry {
            index: i,
            role: role.clone(),
            content: truncate(content, 100),
            timestamp: String::new(),
        })
        .collect()
}

/// Render history search dialog
pub fn render_history_search(f: &mut Frame, state: &HistorySearchState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search input
            Constraint::Min(5),    // Results
        ])
        .split(area);

    f.render_widget(Clear, area);

    // Search input
    let input_block = Block::default()
        .title(" Search History ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let input = Paragraph::new(state.query.as_str())
        .block(input_block)
        .style(Style::default().fg(Color::White));
    f.render_widget(input, chunks[0]);

    // Results
    let items: Vec<ListItem> = state
        .results
        .iter()
        .map(|entry| {
            let role_color = if entry.role == "user" {
                Color::Cyan
            } else {
                Color::Green
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<8}", entry.role),
                    Style::default().fg(role_color),
                ),
                Span::styled(entry.content.clone(), Style::default().fg(Color::White)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Results ({}) ", state.results.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 60))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_index));
    f.render_stateful_widget(list, chunks[1], &mut list_state);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
