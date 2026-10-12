use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::i18n;

pub fn render_help_popup(f: &mut Frame, area: Rect) {
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
            // 由按键注册表生成，不要在这里手抄 —— 此前两份清单已经漂移出
            // 「有键没文档」和「文档写了键却没有」的多处不一致。
            // 新增全局绑定只需改 `events/keymap.rs` 的 GLOBAL_BINDINGS。
            crate::ui::events::keymap::GLOBAL_BINDINGS
                .iter()
                .map(|d| {
                    (
                        d.binding.label().to_string(),
                        i18n::t("ui.help.control", d.zh, d.en),
                    )
                })
                .collect(),
        ),
        (
            i18n::t("ui.help.section.confirm", "审批时", "While approving"),
            vec![
                (
                    "1/2/3/4".to_string(),
                    i18n::t("ui.help.approve.choose", "选择处理方式", "Choose an action"),
                ),
                (
                    "Ctrl+Shift+D".to_string(),
                    i18n::t(
                        "ui.help.control.perm_debug",
                        "切换权限调试面板",
                        "Toggle permission debug panel",
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
                    "/effort".to_string(),
                    i18n::t(
                        "ui.help.cmd.effort",
                        "设置思考深度（off/low/medium/high）",
                        "Set thinking depth (off/low/medium/high)",
                    ),
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
                        "权限规则与审批模式",
                        "Permission rules and approval mode",
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
                (
                    "!cmd".to_string(),
                    i18n::t(
                        "ui.help.other.bang",
                        "本地跑命令，输出进上下文",
                        "Run locally, output goes to the model",
                    ),
                ),
                (
                    "!!cmd".to_string(),
                    i18n::t(
                        "ui.help.other.bangbang",
                        "本地跑命令，输出只给你看",
                        "Run locally, output stays out of context",
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

    // 弹窗按内容定高，不再写死 28 行。写死的高度早就装不下了（光命令区就十几条），
    // Paragraph 又没有滚动，超出的部分是静默切掉的 —— 排在后半截的条目等于没写。
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(4));
    let width = 80.min(area.width.saturating_sub(8));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width,
        height,
    };

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(i18n::t("ui.help.block", "帮助", "Help")),
    );

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}
