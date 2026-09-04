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
///
/// 对标 CCB HistorySearchDialog: 响应式布局 —
/// columns >= 100 时预览在右侧，否则在底部。
pub fn render_history_search(f: &mut Frame, state: &HistorySearchState, area: Rect) {
    use super::fuzzy_picker;

    f.render_widget(Clear, area);

    // Pane 分割线（对标 CCB Pane Divider）
    fuzzy_picker::render_pane_divider(f, area, Color::Cyan);

    // 计算布局（对标 CCB FuzzyPicker 布局）
    let (areas, _preview_pos, _content_area) = fuzzy_picker::compute_layout(area, 100, 6);

    // Search input（对标 CCB SearchBox）
    fuzzy_picker::render_search_input(
        f,
        areas.search,
        "Search prompts",
        &state.query,
        "Filter history…",
    );

    // Results list（对标 CCB FuzzyPicker List + ListItem）
    let match_label =
        fuzzy_picker::format_match_label(state.results.len(), false, false, "prompts");
    let results_block = Block::default()
        .title(format!(" Results ({}) ", match_label))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    if state.results.is_empty() {
        let empty_msg = if state.query.is_empty() {
            "No history yet"
        } else {
            "No matching prompts"
        };
        fuzzy_picker::render_empty_state(f, areas.list, results_block, empty_msg);
    } else {
        fuzzy_picker::render_scrolling_list(
            f,
            areas.list,
            results_block,
            &state.results,
            state.selected_index,
            |entry, _is_focused| {
                let role_color = if entry.role == "user" {
                    Color::Cyan
                } else {
                    Color::Green
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:<8}", entry.role),
                        Style::default().fg(role_color),
                    ),
                    Span::styled(entry.content.clone(), Style::default().fg(Color::White)),
                ])
            },
        );
    }

    // Preview（对标 CCB HistorySearchDialog renderPreview — 圆角边框）
    let selected = state.results.get(state.selected_index);
    let preview_lines: Vec<Line> = if let Some(entry) = selected {
        entry
            .content
            .chars()
            .collect::<Vec<_>>()
            .chunks(80)
            .map(|chunk| {
                Line::from(Span::styled(
                    chunk.iter().collect::<String>(),
                    Style::default().fg(Color::White),
                ))
            })
            .collect()
    } else {
        vec![]
    };
    let preview = Paragraph::new(preview_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(preview, areas.preview);

    // Byline（对标 CCB FuzzyPicker byline）
    fuzzy_picker::render_byline(
        f,
        area,
        &[("↑/↓", "navigate"), ("Enter", "use"), ("Esc", "cancel")],
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}
