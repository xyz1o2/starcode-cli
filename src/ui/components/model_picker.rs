use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::core::i18n;

/// 模型信息
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub description: String,
    pub max_tokens: Option<u64>,
    pub supports_vision: bool,
    pub supports_tools: bool,
}

/// 模型选择器状态
#[derive(Debug)]
pub struct ModelPickerState {
    pub models: Vec<ModelInfo>,
    pub selected_index: usize,
    pub filter: String,
    pub show_details: bool,
    pub current_model: Option<String>,
}

impl ModelPickerState {
    pub fn new(models: Vec<ModelInfo>, current_model: Option<String>) -> Self {
        Self {
            models,
            selected_index: 0,
            filter: String::new(),
            show_details: false,
            current_model,
        }
    }

    pub fn get_filtered_models(&self) -> Vec<&ModelInfo> {
        if self.filter.is_empty() {
            self.models.iter().collect()
        } else {
            let filter_lower = self.filter.to_lowercase();
            self.models
                .iter()
                .filter(|m| {
                    m.name.to_lowercase().contains(&filter_lower)
                        || m.provider.to_lowercase().contains(&filter_lower)
                        || m.description.to_lowercase().contains(&filter_lower)
                })
                .collect()
        }
    }

    pub fn select_next(&mut self) {
        let filtered = self.get_filtered_models();
        if !filtered.is_empty() {
            self.selected_index = (self.selected_index + 1) % filtered.len();
        }
    }

    pub fn select_previous(&mut self) {
        let filtered = self.get_filtered_models();
        if !filtered.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                filtered.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn get_selected_model(&self) -> Option<&ModelInfo> {
        let filtered = self.get_filtered_models();
        filtered.get(self.selected_index).map(|m| *m)
    }

    pub fn toggle_details(&mut self) {
        self.show_details = !self.show_details;
    }

    pub fn add_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected_index = 0;
    }

    pub fn remove_filter_char(&mut self) {
        self.filter.pop();
        self.selected_index = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected_index = 0;
    }
}

/// 渲染模型选择器
pub fn render_model_picker(f: &mut Frame, state: &ModelPickerState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 标题和过滤器
            Constraint::Min(5),    // 模型列表
            Constraint::Length(3), // 详情和操作提示
        ])
        .split(area);

    // 渲染标题和过滤器
    render_header(f, state, chunks[0]);

    // 渲染模型列表
    render_model_list(f, state, chunks[1]);

    // 渲染详情和操作提示
    render_footer(f, state, chunks[2]);
}

fn render_header(f: &mut Frame, state: &ModelPickerState, area: Rect) {
    let filter_text = if state.filter.is_empty() {
        "Type to filter models...".to_string()
    } else {
        format!("Filter: {}", state.filter)
    };

    let current_model_text = if let Some(current) = &state.current_model {
        format!("Current: {}", current)
    } else {
        "No model selected".to_string()
    };

    let block = Block::default()
        .title("Model Picker")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Filter: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if state.filter.is_empty() {
                    "Type to filter...".to_string()
                } else {
                    state.filter.clone()
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Current: ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                state.current_model.as_deref().unwrap_or("None"),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ])
    .block(block);

    f.render_widget(paragraph, area);
}

fn render_model_list(f: &mut Frame, state: &ModelPickerState, area: Rect) {
    let filtered_models = state.get_filtered_models();

    let items: Vec<ListItem> = filtered_models
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let is_selected = i == state.selected_index;
            let is_current = state.current_model.as_deref() == Some(&model.id);

            let style = if is_selected {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = if is_selected { "> " } else { "  " };
            let current_marker = if is_current { " *" } else { "" };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{}{}", model.name, current_marker), style),
                Span::styled(
                    format!(" ({})", model.provider),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title("Available Models")
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

fn render_footer(f: &mut Frame, state: &ModelPickerState, area: Rect) {
    let mut lines = Vec::new();

    if let Some(model) = state.get_selected_model() {
        if state.show_details {
            lines.push(Line::from(vec![
                Span::styled(
                    "Description: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(model.description.clone(), Style::default().fg(Color::White)),
            ]));

            if let Some(max_tokens) = model.max_tokens {
                lines.push(Line::from(vec![
                    Span::styled(
                        "Max Tokens: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{}", max_tokens), Style::default().fg(Color::White)),
                ]));
            }

            lines.push(Line::from(vec![
                Span::styled(
                    "Vision: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if model.supports_vision { "Yes" } else { "No" },
                    Style::default().fg(if model.supports_vision {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
                Span::styled(
                    "  Tools: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if model.supports_tools { "Yes" } else { "No" },
                    Style::default().fg(if model.supports_tools {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "Press [D] to show details",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "↑/↓: Navigate | Enter: Select | D: Details | Esc: Cancel | Type to filter",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(paragraph, area);
}

/// 处理模型选择器输入
pub fn handle_model_picker_input(state: &mut ModelPickerState, key: char) -> Option<ModelInfo> {
    match key {
        '\n' | '\r' => state.get_selected_model().cloned(),
        '\x1b' => None, // Esc
        '\t' => {
            state.toggle_details();
            None
        }
        'd' | 'D' => {
            state.toggle_details();
            None
        }
        '\x7f' => {
            // Backspace
            state.remove_filter_char();
            None
        }
        _ => {
            if key.is_ascii_graphic() || key == ' ' {
                state.add_filter_char(key);
            }
            None
        }
    }
}

/// 创建默认模型列表
pub fn create_default_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
            provider: "OpenAI".to_string(),
            description: "Most capable model, great for complex tasks".to_string(),
            max_tokens: Some(128000),
            supports_vision: true,
            supports_tools: true,
        },
        ModelInfo {
            id: "gpt-4o-mini".to_string(),
            name: "GPT-4o Mini".to_string(),
            provider: "OpenAI".to_string(),
            description: "Fast and efficient, good for most tasks".to_string(),
            max_tokens: Some(128000),
            supports_vision: true,
            supports_tools: true,
        },
        ModelInfo {
            id: "claude-3-5-sonnet".to_string(),
            name: "Claude 3.5 Sonnet".to_string(),
            provider: "Anthropic".to_string(),
            description: "Excellent for coding and analysis".to_string(),
            max_tokens: Some(200000),
            supports_vision: true,
            supports_tools: true,
        },
        ModelInfo {
            id: "deepseek-chat".to_string(),
            name: "DeepSeek Chat".to_string(),
            provider: "DeepSeek".to_string(),
            description: "Cost-effective, good for general tasks".to_string(),
            max_tokens: Some(32000),
            supports_vision: false,
            supports_tools: true,
        },
        ModelInfo {
            id: "moonshot-v1-8k".to_string(),
            name: "Moonshot v1 8K".to_string(),
            provider: "Moonshot".to_string(),
            description: "Fast Chinese model, good for short contexts".to_string(),
            max_tokens: Some(8000),
            supports_vision: false,
            supports_tools: true,
        },
    ]
}
