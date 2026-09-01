use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// 对话框配置
pub struct DialogConfig {
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub border_color: Color,
    pub title_color: Color,
    pub show_close_hint: bool,
}

impl Default for DialogConfig {
    fn default() -> Self {
        Self {
            title: String::new(),
            width: 60,
            height: 20,
            border_color: Color::DarkGray,
            title_color: Color::Cyan,
            show_close_hint: true,
        }
    }
}

/// 渲染对话框
pub fn render_dialog(
    f: &mut Frame,
    config: &DialogConfig,
    content: Vec<Line<'static>>,
    area: Rect,
) {
    // 计算居中位置
    let x = area.x + (area.width.saturating_sub(config.width)) / 2;
    let y = area.y + (area.height.saturating_sub(config.height)) / 2;
    
    let popup_area = Rect {
        x,
        y,
        width: config.width,
        height: config.height,
    };
    
    // 清除背景
    f.render_widget(Clear, popup_area);
    
    // 构建标题
    let mut title = config.title.clone();
    if config.show_close_hint {
        title.push_str(" (ESC to close)");
    }
    
    // 构建块
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::default()
                .fg(config.title_color)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(config.border_color));
    
    // 渲染内容
    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: false });
    
    f.render_widget(paragraph, popup_area);
}

/// 渲染确认对话框
pub fn render_confirmation_dialog(
    f: &mut Frame,
    title: &str,
    message: &str,
    options: &[(&str, &str)],  // (key, label)
    selected: usize,
    area: Rect,
) {
    let mut content = Vec::new();
    
    // 消息
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        message.to_string(),
        Style::default().fg(Color::White),
    )));
    content.push(Line::from(""));
    
    // 选项
    for (i, (key, label)) in options.iter().enumerate() {
        let is_selected = i == selected;
        let style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        
        content.push(Line::from(vec![
            Span::styled(
                if is_selected { "❯ " } else { "  " },
                Style::default().fg(Color::Blue),
            ),
            Span::styled(format!("{}. ", key), style),
            Span::styled(label.to_string(), style),
        ]));
    }
    
    // 提示
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "Press number to select, ESC to cancel",
        Style::default().fg(Color::DarkGray),
    )));
    
    let config = DialogConfig {
        title: title.to_string(),
        width: 50,
        height: (content.len() as u16) + 2,
        border_color: Color::DarkGray,
        title_color: Color::Cyan,
        show_close_hint: false,
    };
    
    render_dialog(f, &config, content, area);
}

/// 渲染输入对话框
pub fn render_input_dialog(
    f: &mut Frame,
    title: &str,
    prompt: &str,
    input: &str,
    cursor_pos: usize,
    area: Rect,
) {
    let mut content = Vec::new();
    
    // 提示
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        prompt.to_string(),
        Style::default().fg(Color::White),
    )));
    content.push(Line::from(""));
    
    // 输入框
    let input_line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Blue)),
        Span::styled(input.to_string(), Style::default().fg(Color::White)),
        Span::styled("▌", Style::default().fg(Color::White)),
    ]);
    content.push(input_line);
    
    // 提示
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(
        "Press Enter to confirm, ESC to cancel",
        Style::default().fg(Color::DarkGray),
    )));
    
    let config = DialogConfig {
        title: title.to_string(),
        width: 60,
        height: 8,
        border_color: Color::DarkGray,
        title_color: Color::Cyan,
        show_close_hint: false,
    };
    
    render_dialog(f, &config, content, area);
}
