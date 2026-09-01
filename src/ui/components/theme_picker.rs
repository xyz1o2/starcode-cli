use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::core::i18n;

/// 主题信息
#[derive(Debug, Clone)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub background: Color,
    pub foreground: Color,
    pub accent: Color,
    pub is_dark: bool,
}

/// 主题选择器状态
#[derive(Debug)]
pub struct ThemePickerState {
    pub themes: Vec<ThemeInfo>,
    pub selected_index: usize,
    pub current_theme: Option<String>,
    pub preview_theme: Option<String>,
}

impl ThemePickerState {
    pub fn new(themes: Vec<ThemeInfo>, current_theme: Option<String>) -> Self {
        Self {
            themes,
            selected_index: 0,
            current_theme,
            preview_theme: None,
        }
    }

    pub fn select_next(&mut self) {
        if !self.themes.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.themes.len();
            self.update_preview();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.themes.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.themes.len() - 1
            } else {
                self.selected_index - 1
            };
            self.update_preview();
        }
    }

    pub fn get_selected_theme(&self) -> Option<&ThemeInfo> {
        self.themes.get(self.selected_index)
    }

    pub fn update_preview(&mut self) {
        if let Some(theme) = self.get_selected_theme() {
            self.preview_theme = Some(theme.id.clone());
        }
    }

    pub fn confirm_selection(&mut self) -> Option<ThemeInfo> {
        self.get_selected_theme().cloned()
    }

    pub fn cancel_preview(&mut self) {
        self.preview_theme = None;
    }
}

/// 渲染主题选择器
pub fn render_theme_picker(f: &mut Frame, state: &ThemePickerState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 标题
            Constraint::Min(5),    // 主题列表
            Constraint::Length(3), // 预览和操作提示
        ])
        .split(area);

    // 渲染标题
    render_title(f, state, chunks[0]);

    // 渲染主题列表
    render_theme_list(f, state, chunks[1]);

    // 渲染预览和操作提示
    render_footer(f, state, chunks[2]);
}

fn render_title(f: &mut Frame, state: &ThemePickerState, area: Rect) {
    let current_theme_text = if let Some(current) = &state.current_theme {
        format!("Current: {}", current)
    } else {
        "No theme selected".to_string()
    };

    let block = Block::default()
        .title("Theme Picker")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Select a theme: ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                state.current_theme.as_deref().unwrap_or("None"),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(Span::styled(
            "Use ↑/↓ to navigate, Enter to select, Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(block);

    f.render_widget(paragraph, area);
}

fn render_theme_list(f: &mut Frame, state: &ThemePickerState, area: Rect) {
    let items: Vec<ListItem> = state
        .themes
        .iter()
        .enumerate()
        .map(|(i, theme)| {
            let is_selected = i == state.selected_index;
            let is_current = state.current_theme.as_deref() == Some(&theme.id);
            let is_preview = state.preview_theme.as_deref() == Some(&theme.id);

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
            let preview_marker = if is_preview { " (preview)" } else { "" };

            let dark_light = if theme.is_dark { "Dark" } else { "Light" };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(
                    format!("{}{}{}", theme.name, current_marker, preview_marker),
                    style,
                ),
                Span::styled(
                    format!(" ({})", dark_light),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title("Available Themes")
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

fn render_footer(f: &mut Frame, state: &ThemePickerState, area: Rect) {
    let mut lines = Vec::new();

    if let Some(theme) = state.get_selected_theme() {
        lines.push(Line::from(vec![
            Span::styled(
                "Description: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(theme.description.clone(), Style::default().fg(Color::White)),
        ]));

        // 显示主题颜色预览
        lines.push(Line::from(vec![
            Span::styled(
                "Preview: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("■", Style::default().fg(theme.background)),
            Span::raw(" "),
            Span::styled("■", Style::default().fg(theme.foreground)),
            Span::raw(" "),
            Span::styled("■", Style::default().fg(theme.accent)),
        ]));
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "↑/↓: Navigate | Enter: Select | Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(paragraph, area);
}

/// 处理主题选择器输入
pub fn handle_theme_picker_input(state: &mut ThemePickerState, key: char) -> Option<ThemeInfo> {
    match key {
        '\n' | '\r' => state.confirm_selection(),
        '\x1b' => {
            // Esc
            state.cancel_preview();
            None
        }
        _ => None,
    }
}

/// 创建默认主题列表
pub fn create_default_themes() -> Vec<ThemeInfo> {
    vec![
        ThemeInfo {
            id: "dark".to_string(),
            name: "Dark".to_string(),
            description: "Default dark theme, easy on the eyes".to_string(),
            background: Color::Black,
            foreground: Color::White,
            accent: Color::Cyan,
            is_dark: true,
        },
        ThemeInfo {
            id: "light".to_string(),
            name: "Light".to_string(),
            description: "Clean light theme for bright environments".to_string(),
            background: Color::White,
            foreground: Color::Black,
            accent: Color::Blue,
            is_dark: false,
        },
        ThemeInfo {
            id: "monokai".to_string(),
            name: "Monokai".to_string(),
            description: "Classic Monokai color scheme".to_string(),
            background: Color::Rgb(39, 40, 34),
            foreground: Color::Rgb(248, 248, 242),
            accent: Color::Rgb(166, 226, 46),
            is_dark: true,
        },
        ThemeInfo {
            id: "solarized-dark".to_string(),
            name: "Solarized Dark".to_string(),
            description: "Solarized dark theme, low contrast".to_string(),
            background: Color::Rgb(0, 43, 54),
            foreground: Color::Rgb(131, 148, 150),
            accent: Color::Rgb(38, 139, 210),
            is_dark: true,
        },
        ThemeInfo {
            id: "solarized-light".to_string(),
            name: "Solarized Light".to_string(),
            description: "Solarized light theme, easy to read".to_string(),
            background: Color::Rgb(253, 246, 227),
            foreground: Color::Rgb(101, 123, 131),
            accent: Color::Rgb(38, 139, 210),
            is_dark: false,
        },
        ThemeInfo {
            id: "dracula".to_string(),
            name: "Dracula".to_string(),
            description: "Popular Dracula color scheme".to_string(),
            background: Color::Rgb(40, 42, 54),
            foreground: Color::Rgb(248, 248, 242),
            accent: Color::Rgb(189, 147, 249),
            is_dark: true,
        },
        ThemeInfo {
            id: "nord".to_string(),
            name: "Nord".to_string(),
            description: "Arctic, north-bluish color palette".to_string(),
            background: Color::Rgb(46, 52, 64),
            foreground: Color::Rgb(216, 222, 233),
            accent: Color::Rgb(136, 192, 208),
            is_dark: true,
        },
        ThemeInfo {
            id: "gruvbox-dark".to_string(),
            name: "Gruvbox Dark".to_string(),
            description: "Retro groove dark theme".to_string(),
            background: Color::Rgb(29, 32, 33),
            foreground: Color::Rgb(235, 219, 178),
            accent: Color::Rgb(214, 153, 67),
            is_dark: true,
        },
    ]
}
