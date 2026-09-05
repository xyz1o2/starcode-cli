use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::core::i18n;

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub line_number: usize,
    pub content: String,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
}

/// 全局搜索对话框状态
#[derive(Debug)]
pub struct GlobalSearchState {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub selected_index: usize,
    pub is_searching: bool,
    pub show_preview: bool,
    pub max_results: usize,
    pub error: Option<String>,
}

impl GlobalSearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            is_searching: false,
            show_preview: true,
            max_results: 100,
            error: None,
        }
    }

    pub fn add_query_char(&mut self, c: char) {
        self.query.push(c);
        self.selected_index = 0;
    }

    pub fn remove_query_char(&mut self) {
        self.query.pop();
        self.selected_index = 0;
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.error = None;
    }

    pub fn select_next(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.results.len();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.results.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn get_selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected_index)
    }

    pub fn toggle_preview(&mut self) {
        self.show_preview = !self.show_preview;
    }

    pub fn set_results(&mut self, results: Vec<SearchResult>) {
        self.results = results;
        self.selected_index = 0;
        self.is_searching = false;
        self.error = None;
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.is_searching = false;
    }

    pub fn set_searching(&mut self, searching: bool) {
        self.is_searching = searching;
        if searching {
            self.error = None;
        }
    }
}

/// 渲染全局搜索对话框
pub fn render_global_search(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 搜索输入
            Constraint::Min(5),    // 结果列表
            Constraint::Length(3), // 预览和状态
        ])
        .split(area);

    // 渲染搜索输入
    render_search_input(f, state, chunks[0]);

    // 渲染结果列表
    render_results_list(f, state, chunks[1]);

    // 渲染预览和状态
    render_footer(f, state, chunks[2]);
}

fn render_search_input(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    let search_text = if state.query.is_empty() {
        "Type to search...".to_string()
    } else {
        state.query.clone()
    };

    let status = if state.is_searching {
        " (searching...)"
    } else if state.results.is_empty() && !state.query.is_empty() {
        " (no results)"
    } else {
        ""
    };

    let block = Block::default()
        .title("Global Search")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Blue));

    let paragraph = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Search: ",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(search_text, Style::default().fg(Color::White)),
            Span::styled(status, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(Span::styled(
            "Type to search | ↑/↓: Navigate | Enter: Open | Esc: Close",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(block);

    f.render_widget(paragraph, area);
}

fn render_results_list(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    if let Some(error) = &state.error {
        let block = Block::default()
            .title("Error")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Red));

        let paragraph = Paragraph::new(vec![
            Line::from(Span::styled(error.clone(), Style::default().fg(Color::Red))),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block);

        f.render_widget(paragraph, area);
        return;
    }

    if state.results.is_empty() {
        let block = Block::default()
            .title("Results")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray));

        let message = if state.query.is_empty() {
            "Enter a search query to find files"
        } else if state.is_searching {
            "Searching..."
        } else {
            "No results found"
        };

        let paragraph = Paragraph::new(vec![Line::from(Span::styled(
            message,
            Style::default().fg(Color::Gray),
        ))])
        .block(block);

        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .results
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let is_selected = i == state.selected_index;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };

            // 截断长路径 / 长内容。
            // 原来两处都是字节切片，搜到含中文的行就会在渲染线程里 panic ——
            // 而这个仓库自己的注释就是中文。
            let file_path = crate::utils::string_utils::truncate_start_chars(&result.file_path, 37);
            let content = crate::utils::string_utils::truncate_with_ellipsis(&result.content, 60);

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(file_path, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!(":{} ", result.line_number),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(content, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("Results ({})", state.results.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_index));

    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_footer(f: &mut Frame, state: &GlobalSearchState, area: Rect) {
    let mut lines = Vec::new();

    if let Some(result) = state.get_selected_result() {
        if state.show_preview {
            // 显示预览
            lines.push(Line::from(vec![
                Span::styled(
                    "File: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(result.file_path.clone(), Style::default().fg(Color::Yellow)),
            ]));

            lines.push(Line::from(vec![
                Span::styled(
                    "Line: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}", result.line_number),
                    Style::default().fg(Color::White),
                ),
            ]));

            // 显示上下文
            if let Some(before) = &result.context_before {
                lines.push(Line::from(Span::styled(
                    before.clone(),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            lines.push(Line::from(Span::styled(
                result.content.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));

            if let Some(after) = &result.context_after {
                lines.push(Line::from(Span::styled(
                    after.clone(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "Press [P] to show preview",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "↑/↓: Navigate | Enter: Open File | P: Toggle Preview | Esc: Close",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(paragraph, area);
}

/// 处理全局搜索输入
pub fn handle_global_search_input(
    state: &mut GlobalSearchState,
    key: char,
) -> Option<SearchResult> {
    match key {
        '\n' | '\r' => state.get_selected_result().cloned(),
        '\x1b' => None, // Esc
        '\t' => {
            state.toggle_preview();
            None
        }
        'p' | 'P' => {
            state.toggle_preview();
            None
        }
        '\x7f' => {
            // Backspace
            state.remove_query_char();
            None
        }
        _ => {
            if key.is_ascii_graphic() || key == ' ' {
                state.add_query_char(key);
            }
            None
        }
    }
}

/// 执行搜索
pub async fn perform_search(
    state: &mut GlobalSearchState,
    search_fn: impl Fn(&str) -> Vec<SearchResult>,
) {
    if state.query.is_empty() {
        state.results.clear();
        return;
    }

    state.set_searching(true);

    // 在实际实现中，这里应该调用ripgrep或其他搜索工具
    let results = search_fn(&state.query);

    state.set_results(results);
}
