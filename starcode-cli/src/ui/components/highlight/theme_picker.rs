/// Theme picker — visual theme selection with preview.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Theme definition
#[derive(Debug, Clone)]
pub struct ThemeInfo {
    pub name: String,
    pub description: String,
    pub preview_colors: Vec<(String, Color)>,
}

/// Available themes
pub fn available_themes() -> Vec<ThemeInfo> {
    vec![
        ThemeInfo {
            name: "dark".to_string(),
            description: "Default dark theme".to_string(),
            preview_colors: vec![
                ("Background".to_string(), Color::Black),
                ("Text".to_string(), Color::White),
                ("Accent".to_string(), Color::Cyan),
            ],
        },
        ThemeInfo {
            name: "light".to_string(),
            description: "Light theme for bright environments".to_string(),
            preview_colors: vec![
                ("Background".to_string(), Color::White),
                ("Text".to_string(), Color::Black),
                ("Accent".to_string(), Color::Blue),
            ],
        },
        ThemeInfo {
            name: "monokai".to_string(),
            description: "Monokai color scheme".to_string(),
            preview_colors: vec![
                ("Background".to_string(), Color::Rgb(39, 40, 34)),
                ("Text".to_string(), Color::Rgb(248, 248, 242)),
                ("Accent".to_string(), Color::Rgb(166, 226, 46)),
            ],
        },
        ThemeInfo {
            name: "dracula".to_string(),
            description: "Dracula color scheme".to_string(),
            preview_colors: vec![
                ("Background".to_string(), Color::Rgb(40, 42, 54)),
                ("Text".to_string(), Color::Rgb(248, 248, 242)),
                ("Accent".to_string(), Color::Rgb(189, 147, 249)),
            ],
        },
        ThemeInfo {
            name: "solarized".to_string(),
            description: "Solarized color scheme".to_string(),
            preview_colors: vec![
                ("Background".to_string(), Color::Rgb(0, 43, 54)),
                ("Text".to_string(), Color::Rgb(131, 148, 150)),
                ("Accent".to_string(), Color::Rgb(38, 139, 210)),
            ],
        },
    ]
}

/// Render theme picker
pub fn render_theme_picker(f: &mut Frame, themes: &[ThemeInfo], selected: usize, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Theme list
            Constraint::Percentage(60), // Preview
        ])
        .split(area);

    f.render_widget(Clear, area);

    // Theme list
    let items: Vec<ListItem> = themes
        .iter()
        .map(|theme| {
            let line = Line::from(vec![
                Span::styled(
                    theme.name.clone(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" - {}", theme.description),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Themes ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
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
    let theme = &themes[selected];
    let preview_lines: Vec<Line> = theme
        .preview_colors
        .iter()
        .map(|(name, color)| {
            Line::from(vec![
                Span::styled(
                    format!("  {:<12}", name),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    "████████",
                    Style::default().fg(*color),
                ),
            ])
        })
        .collect();

    let preview = Paragraph::new(preview_lines).block(
        Block::default()
            .title(format!(" Preview: {} ", theme.name))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(preview, chunks[1]);
}
