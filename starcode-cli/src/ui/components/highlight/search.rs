/// Global search dialog — ripgrep-based workspace search with preview.
///
/// Provides:
/// - Real-time search as you type
/// - File path and line number display
/// - Syntax-highlighted preview
/// - Keyboard navigation
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Search result entry
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file: String,
    pub line_number: usize,
    pub content: String,
    pub score: i32,
}

/// Global search state
#[derive(Debug)]
pub struct GlobalSearchState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected_index: usize,
    pub is_searching: bool,
    pub search_error: Option<String>,
}

impl GlobalSearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            is_searching: false,
            search_error: None,
        }
    }

    pub fn reset(&mut self) {
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.is_searching = false;
        self.search_error = None;
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

    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected_index)
    }
}

/// Execute a ripgrep search
pub async fn execute_search(query: &str, cwd: &str, max_results: usize) -> Vec<SearchResult> {
    if query.is_empty() {
        return Vec::new();
    }

    let output = tokio::process::Command::new("rg")
        .args(&[
            "--line-number",
            "--column",
            "--no-heading",
            "--color=never",
            "--max-count=1",
            &format!("--max-count={}", max_results),
            query,
            cwd,
        ])
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_rg_output(&stdout, max_results)
        }
        Err(_) => Vec::new(),
    }
}

/// Parse ripgrep output into SearchResult entries
fn parse_rg_output(output: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    for line in output.lines().take(max_results) {
        // Format: file:line:col:content
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() >= 3 {
            let file = parts[0].to_string();
            let line_number = parts[1].parse().unwrap_or(0);
            let content = parts[2].to_string();

            results.push(SearchResult {
                file,
                line_number,
                content,
                score: 0,
            });
        }
    }

    results
}

/// Render the global search dialog
pub fn render_global_search(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Search input
            Constraint::Min(5),    // Results list
            Constraint::Length(3), // Preview
        ])
        .split(area);

    // Clear background
    f.render_widget(Clear, area);

    // Search input
    let input_block = Block::default()
        .title(" Search ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let input_text = if state.query.is_empty() {
        "Type to search...".to_string()
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

    // Results list
    let results_block = Block::default()
        .title(format!(" Results ({}) ", state.results.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if state.results.is_empty() {
        let empty_msg = if state.is_searching {
            "Searching..."
        } else if state.query.is_empty() {
            "Enter a search query"
        } else {
            "No results found"
        };
        let empty = Paragraph::new(empty_msg)
            .block(results_block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .results
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let file_name = std::path::Path::new(&result.file)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| result.file.clone());
                let line = Line::from(vec![
                    Span::styled(
                        format!("{}:", file_name),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        format!("{} ", result.line_number),
                        Style::default().fg(Color::Green),
                    ),
                    Span::styled(
                        result.content.clone(),
                        Style::default().fg(Color::White),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(results_block)
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

    // Preview / Help
    let preview_text = if let Some(result) = state.selected_result() {
        format!("{}:{}", result.file, result.line_number)
    } else {
        "↑↓ Navigate  Enter Open  Esc Close".to_string()
    };
    let preview = Paragraph::new(preview_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(preview, chunks[2]);
}
