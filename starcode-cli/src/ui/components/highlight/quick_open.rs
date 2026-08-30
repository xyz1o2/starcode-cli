/// Quick open dialog — fuzzy file finder with preview.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// File entry
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub size: String,
    pub modified: String,
}

/// Quick open state
#[derive(Debug)]
pub struct QuickOpenState {
    pub query: String,
    pub files: Vec<FileEntry>,
    pub selected_index: usize,
    pub preview_content: Option<String>,
}

impl QuickOpenState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            files: Vec::new(),
            selected_index: 0,
            preview_content: None,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.files.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn selected_file(&self) -> Option<&FileEntry> {
        self.files.get(self.selected_index)
    }
}

/// Search files with fuzzy matching
pub async fn search_files(query: &str, cwd: &str, max_results: usize) -> Vec<FileEntry> {
    if query.is_empty() {
        return Vec::new();
    }

    let output = tokio::process::Command::new("fd")
        .args(&[
            "--type", "f",
            "--max-results", &max_results.to_string(),
            query,
            cwd,
        ])
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .take(max_results)
                .map(|path| {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string());
                    FileEntry {
                        path: path.to_string(),
                        name,
                        size: String::new(),
                        modified: String::new(),
                    }
                })
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// Render quick open dialog
pub fn render_quick_open(f: &mut Frame, state: &QuickOpenState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search input
            Constraint::Min(5),   // File list
        ])
        .split(area);

    f.render_widget(Clear, area);

    // Search input
    let input_block = Block::default()
        .title(" Quick Open ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let input_text = if state.query.is_empty() {
        "Type to search files...".to_string()
    } else {
        state.query.clone()
    };
    let input = Paragraph::new(input_text)
        .block(input_block)
        .style(if state.query.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        });
    f.render_widget(input, chunks[0]);

    // File list
    let items: Vec<ListItem> = state
        .files
        .iter()
        .map(|file| {
            let line = Line::from(vec![
                Span::styled(
                    file.name.clone(),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("  {}", file.path),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Files ({}) ", state.files.len()))
                .borders(Borders::ALL)
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
