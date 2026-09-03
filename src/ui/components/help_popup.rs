use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::i18n;

pub fn render_help_popup(f: &mut Frame, area: Rect) {
    let height = 28.min(area.height.saturating_sub(4));
    let width = 80.min(area.width.saturating_sub(8));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    let popup_area = Rect {
        x,
        y,
        width,
        height,
    };

    let sections: Vec<(String, Vec<(String, String)>)> = vec![
        (
            i18n::t("ui.help.section.nav", "导航", "Navigation"),
            vec![
                (
                    "PgUp/PgDn".to_string(),
                    i18n::t("ui.help.nav.page", "翻页", "Page up/down"),
                ),
                (
                    "Home/End".to_string(),
                    i18n::t("ui.help.nav.home_end", "跳到开头/结尾", "Jump to start/end"),
                ),
                (
                    "↑/↓".to_string(),
                    i18n::t("ui.help.nav.arrows", "在提示中导航", "Navigate suggestions"),
                ),
            ],
        ),
        (
            i18n::t("ui.help.section.input", "输入", "Input"),
            vec![
                (
                    "Tab".to_string(),
                    i18n::t("ui.help.input.tab", "补全/接受建议", "Autocomplete/accept"),
                ),
                (
                    "Enter".to_string(),
                    i18n::t("ui.help.input.enter", "发送/确认", "Send/confirm"),
                ),
                (
                    "Alt+P".to_string(),
                    i18n::t(
                        "ui.help.input.fold",
                        "折叠/展开多行粘贴",
                        "Fold/unfold pasted text",
                    ),
                ),
                (
                    "Backspace".to_string(),
                    i18n::t("ui.help.input.backspace", "删除字符", "Delete character"),
                ),
            ],
        ),
        (
            i18n::t("ui.help.section.control", "控制", "Control"),
            vec![
                (
                    "Ctrl+P".to_string(),
                    i18n::t(
                        "ui.help.control.palette",
                        "打开命令面板",
                        "Open command palette",
                    ),
                ),
                (
                    "Ctrl+T".to_string(),
                    i18n::t("ui.help.control.tasks", "切换任务面板", "Toggle tasks"),
                ),
                (
                    "Ctrl+O".to_string(),
                    i18n::t(
                        "ui.help.control.transcript",
                        "切换详细输出（transcript）",
                        "Verbose output (transcript)",
                    ),
                ),
                (
                    "Ctrl+C".to_string(),
                    i18n::t(
                        "ui.help.control.cancel",
                        "中断生成/清空输入",
                        "Interrupt generation / clear input",
                    ),
                ),
                (
                    "Ctrl+D".to_string(),
                    i18n::t("ui.help.control.exit", "退出程序", "Exit"),
                ),
                (
                    "ESC".to_string(),
                    i18n::t("ui.help.control.esc", "取消/关闭弹窗", "Cancel/close"),
                ),
                (
                    "Shift+Tab".to_string(),
                    i18n::t(
                        "ui.help.control.plan",
                        "切换 Plan/Build",
                        "Toggle Plan/Build",
                    ),
                ),
            ],
        ),
        (
            i18n::t("ui.help.section.command", "命令", "Commands"),
            vec![
                (
                    "/help".to_string(),
                    i18n::t("ui.help.cmd.help", "显示帮助", "Show help"),
                ),
                (
                    "/clear".to_string(),
                    i18n::t("ui.help.cmd.clear", "清除对话", "Clear chat"),
                ),
                (
                    "/model".to_string(),
                    i18n::t("ui.help.cmd.model", "选择模型", "Select model"),
                ),
                (
                    "/settings".to_string(),
                    i18n::t(
                        "ui.help.cmd.settings",
                        "查看当前设置概览",
                        "Show current settings overview",
                    ),
                ),
                (
                    "/permissions".to_string(),
                    i18n::t(
                        "ui.help.cmd.permissions",
                        "切换审批模式",
                        "Change approval mode",
                    ),
                ),
                (
                    "/loop".to_string(),
                    i18n::t("ui.help.cmd.loop", "管理定时任务", "Manage loops"),
                ),
                (
                    "/agents".to_string(),
                    i18n::t(
                        "ui.help.cmd.agents",
                        "管理 agent 与 teams（含 run/apply/runs/show-run/clean）",
                        "Manage agents and teams (run/apply/runs/show-run/clean)",
                    ),
                ),
                (
                    "/plugin".to_string(),
                    i18n::t("ui.help.cmd.plugin", "管理插件", "Manage plugins"),
                ),
                (
                    "/remote".to_string(),
                    i18n::t("ui.help.cmd.remote", "远程控制收件箱", "Remote inbox"),
                ),
                (
                    "/hooks".to_string(),
                    i18n::t("ui.help.cmd.hooks", "管理 hooks", "Manage hooks"),
                ),
                (
                    "/mcp".to_string(),
                    i18n::t("ui.help.cmd.mcp", "MCP 管理命令", "MCP commands"),
                ),
                (
                    "/restore".to_string(),
                    i18n::t(
                        "ui.help.cmd.restore",
                        "恢复 checkpoint",
                        "Restore checkpoint",
                    ),
                ),
            ],
        ),
        (
            i18n::t("ui.help.section.other", "其他", "Other"),
            vec![
                (
                    "?".to_string(),
                    i18n::t(
                        "ui.help.other.toggle",
                        "显示/关闭此帮助",
                        "Toggle this help",
                    ),
                ),
                (
                    "@filename".to_string(),
                    i18n::t(
                        "ui.help.other.at",
                        "自动读取文件内容",
                        "Auto-read file content",
                    ),
                ),
            ],
        ),
    ];

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                i18n::t("ui.help.title", "快捷键帮助", "Keyboard Shortcuts"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(
                i18n::t(
                    "ui.help.close",
                    "按 ? 或 ESC 关闭",
                    "Press ? or ESC to close",
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(")"),
        ]),
        Line::from(""),
    ];

    for (section_name, keys) in sections {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}:", section_name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]));

        for (key, desc) in keys {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{:<12}", key), Style::default().fg(Color::Green)),
                Span::styled(format!(" - {}", desc), Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![
        Span::styled(
            i18n::t("ui.help.tip", "提示: ", "Tip: "),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            i18n::t(
                "ui.help.tip.confirm",
                "工具执行时会显示确认框，用 1/2/3 或方向键选择",
                "Tool execution may require confirmation: use 1/2/3 or arrows",
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(i18n::t("ui.help.block", "帮助", "Help")),
    );

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}
