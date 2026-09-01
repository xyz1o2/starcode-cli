use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::i18n;

/// 权限请求类型
#[derive(Debug, Clone)]
pub enum PermissionRequestType {
    /// 文件编辑权限
    FileEdit {
        file_path: String,
        diff_preview: String,
    },
    /// Shell命令执行权限
    ShellCommand {
        command: String,
        working_dir: String,
        risk_level: RiskLevel,
    },
    /// 文件写入权限
    FileWrite {
        file_path: String,
        content_preview: String,
    },
    /// 网页抓取权限
    WebFetch { url: String },
    /// 通用权限请求
    Generic { title: String, description: String },
}

/// 风险等级
#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Safe,
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn label(&self) -> &str {
        match self {
            RiskLevel::Safe => "Safe",
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
            RiskLevel::Critical => "Critical",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            RiskLevel::Safe => Color::Green,
            RiskLevel::Low => Color::Cyan,
            RiskLevel::Medium => Color::Yellow,
            RiskLevel::High => Color::LightRed,
            RiskLevel::Critical => Color::Red,
        }
    }
}

/// 权限对话框选项
#[derive(Debug, Clone)]
pub struct PermissionOption {
    pub key: char,
    pub label: String,
    pub description: String,
    pub is_dangerous: bool,
}

/// 权限对话框状态
#[derive(Debug)]
pub struct PermissionDialogState {
    pub request: PermissionRequestType,
    pub options: Vec<PermissionOption>,
    pub selected_option: usize,
    pub show_details: bool,
    pub feedback: Option<String>,
}

impl PermissionDialogState {
    pub fn new(request: PermissionRequestType) -> Self {
        let options = match &request {
            PermissionRequestType::FileEdit { .. } => vec![
                PermissionOption {
                    key: 'Y',
                    label: "Allow once".to_string(),
                    description: "Allow this edit only".to_string(),
                    is_dangerous: false,
                },
                PermissionOption {
                    key: 'S',
                    label: "Allow for session".to_string(),
                    description: "Allow edits for this session".to_string(),
                    is_dangerous: false,
                },
                PermissionOption {
                    key: 'A',
                    label: "Always allow".to_string(),
                    description: "Always allow edits to this file".to_string(),
                    is_dangerous: true,
                },
                PermissionOption {
                    key: 'D',
                    label: "Deny".to_string(),
                    description: "Reject this edit".to_string(),
                    is_dangerous: false,
                },
            ],
            PermissionRequestType::ShellCommand { risk_level, .. } => {
                let mut opts = vec![
                    PermissionOption {
                        key: 'Y',
                        label: "Allow once".to_string(),
                        description: "Run this command only".to_string(),
                        is_dangerous: false,
                    },
                    PermissionOption {
                        key: 'S',
                        label: "Allow for session".to_string(),
                        description: "Allow similar commands for this session".to_string(),
                        is_dangerous: false,
                    },
                ];

                if *risk_level == RiskLevel::Safe || *risk_level == RiskLevel::Low {
                    opts.push(PermissionOption {
                        key: 'A',
                        label: "Always allow".to_string(),
                        description: "Always allow this command pattern".to_string(),
                        is_dangerous: true,
                    });
                }

                opts.push(PermissionOption {
                    key: 'D',
                    label: "Deny".to_string(),
                    description: "Do not run this command".to_string(),
                    is_dangerous: false,
                });

                opts
            }
            _ => vec![
                PermissionOption {
                    key: 'Y',
                    label: "Allow".to_string(),
                    description: "Allow this action".to_string(),
                    is_dangerous: false,
                },
                PermissionOption {
                    key: 'D',
                    label: "Deny".to_string(),
                    description: "Deny this action".to_string(),
                    is_dangerous: false,
                },
            ],
        };

        Self {
            request,
            options,
            selected_option: 0,
            show_details: false,
            feedback: None,
        }
    }

    pub fn select_next(&mut self) {
        self.selected_option = (self.selected_option + 1) % self.options.len();
    }

    pub fn select_previous(&mut self) {
        self.selected_option = if self.selected_option == 0 {
            self.options.len() - 1
        } else {
            self.selected_option - 1
        };
    }

    pub fn toggle_details(&mut self) {
        self.show_details = !self.show_details;
    }

    pub fn get_selected_key(&self) -> char {
        self.options[self.selected_option].key
    }
}

/// 渲染权限对话框
pub fn render_permission_dialog(f: &mut Frame, state: &PermissionDialogState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 标题
            Constraint::Min(5),    // 内容
            Constraint::Length(3), // 操作提示
        ])
        .split(area);

    // 渲染标题
    render_title(f, &state.request, chunks[0]);

    // 渲染内容
    render_content(f, state, chunks[1]);

    // 渲染操作提示
    render_footer(f, state, chunks[2]);
}

fn render_title(f: &mut Frame, request: &PermissionRequestType, area: Rect) {
    let title = match request {
        PermissionRequestType::FileEdit { .. } => Span::styled(
            "File Edit Permission",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        PermissionRequestType::ShellCommand { .. } => Span::styled(
            "Shell Command Permission",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        PermissionRequestType::FileWrite { .. } => Span::styled(
            "File Write Permission",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        PermissionRequestType::WebFetch { .. } => Span::styled(
            "Web Fetch Permission",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        PermissionRequestType::Generic { title, .. } => Span::styled(
            title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(Line::from(vec![Span::raw("  "), title])).block(block);

    f.render_widget(paragraph, area);
}

fn render_content(f: &mut Frame, state: &PermissionDialogState, area: Rect) {
    let mut lines = Vec::new();

    // 根据请求类型渲染内容
    match &state.request {
        PermissionRequestType::FileEdit {
            file_path,
            diff_preview,
        } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "Edit ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(file_path.clone(), Style::default().fg(Color::Yellow)),
            ]));

            if state.show_details {
                lines.push(Line::from(Span::raw("")));
                for diff_line in diff_preview.lines().take(10) {
                    let color = if diff_line.starts_with('+') {
                        Color::Green
                    } else if diff_line.starts_with('-') {
                        Color::Red
                    } else {
                        Color::Gray
                    };
                    lines.push(Line::from(Span::styled(
                        diff_line.to_string(),
                        Style::default().fg(color),
                    )));
                }
            }
        }
        PermissionRequestType::ShellCommand {
            command,
            working_dir,
            risk_level,
        } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "Run: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if command.len() > 60 {
                        format!("{}...", &command[..60])
                    } else {
                        command.clone()
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]));

            lines.push(Line::from(vec![
                Span::styled("Risk: ", Style::default().fg(Color::Gray)),
                Span::styled(risk_level.label(), Style::default().fg(risk_level.color())),
            ]));

            lines.push(Line::from(vec![
                Span::styled("Dir: ", Style::default().fg(Color::Gray)),
                Span::styled(working_dir.clone(), Style::default().fg(Color::DarkGray)),
            ]));
        }
        PermissionRequestType::FileWrite {
            file_path,
            content_preview,
        } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "Write to ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(file_path.clone(), Style::default().fg(Color::Yellow)),
            ]));

            if state.show_details {
                lines.push(Line::from(Span::raw("")));
                for preview_line in content_preview.lines().take(5) {
                    lines.push(Line::from(Span::styled(
                        preview_line.to_string(),
                        Style::default().fg(Color::Gray),
                    )));
                }
            }
        }
        PermissionRequestType::WebFetch { url } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "Fetch: ",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(url.clone(), Style::default().fg(Color::Cyan)),
            ]));
        }
        PermissionRequestType::Generic {
            title: _,
            description,
        } => {
            lines.push(Line::from(Span::styled(
                description.clone(),
                Style::default().fg(Color::White),
            )));
        }
    }

    // 添加详细信息切换提示
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        if state.show_details {
            "Press [D] to hide details"
        } else {
            "Press [D] to show details"
        },
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, state: &PermissionDialogState, area: Rect) {
    let mut lines = Vec::new();

    // 渲染选项
    for (i, option) in state.options.iter().enumerate() {
        let selected = i == state.selected_option;
        let (marker, key_style, label_style) = if selected {
            (
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                Span::styled("  ", Style::default()),
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::Gray),
            )
        };

        lines.push(Line::from(vec![
            marker,
            Span::styled(format!("[{}]", option.key), key_style),
            Span::styled(format!(" {}", option.label), label_style),
        ]));
    }

    // 添加键盘提示
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "Y/S/A/D or 1-4 + Enter, Esc to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(paragraph, area);
}

/// 处理权限对话框输入
pub fn handle_permission_input(state: &mut PermissionDialogState, key: char) -> Option<char> {
    match key {
        'y' | 'Y' => Some('Y'),
        's' | 'S' => Some('S'),
        'a' | 'A' => Some('A'),
        'd' | 'D' => {
            if state.show_details {
                state.toggle_details();
                None
            } else {
                Some('D')
            }
        }
        '1' => {
            if state.options.len() > 0 {
                state.selected_option = 0;
                Some(state.options[0].key)
            } else {
                None
            }
        }
        '2' => {
            if state.options.len() > 1 {
                state.selected_option = 1;
                Some(state.options[1].key)
            } else {
                None
            }
        }
        '3' => {
            if state.options.len() > 2 {
                state.selected_option = 2;
                Some(state.options[2].key)
            } else {
                None
            }
        }
        '4' => {
            if state.options.len() > 3 {
                state.selected_option = 3;
                Some(state.options[3].key)
            } else {
                None
            }
        }
        '\n' | '\r' => Some(state.get_selected_key()),
        _ => None,
    }
}
