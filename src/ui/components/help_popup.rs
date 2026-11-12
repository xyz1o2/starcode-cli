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
            i18n::t1("ui.help.section.nav"),
            vec![
                ("PgUp/PgDn".to_string(), i18n::t1("ui.help.nav.page")),
                ("Home/End".to_string(), i18n::t1("ui.help.nav.home_end")),
                ("↑/↓".to_string(), i18n::t1("ui.help.nav.arrows")),
            ],
        ),
        (
            i18n::t1("ui.help.section.input"),
            vec![
                (
                    "Tab".to_string(),
                    // 这里此前写的是「补全/接受建议」，实际主输入区的 Tab 是折叠/展开
                    // 当前视口中心的工具输出块（`input.rs` 的 `KeyCode::Tab` 分支）。
                    // 补全走的是提示浮层的 ↑/↓ + Enter，写错等于教用户按一个不存在的键。
                    i18n::t1("ui.help.input.tab"),
                ),
                ("Enter".to_string(), i18n::t1("ui.help.input.enter")),
                ("Alt+P".to_string(), i18n::t1("ui.help.input.fold")),
                ("Backspace".to_string(), i18n::t1("ui.help.input.backspace")),
            ],
        ),
        (
            i18n::t1("ui.help.section.control"),
            // 由按键注册表生成，不要在这里手抄 —— 此前两份清单已经漂移出
            // 「有键没文档」和「文档写了键却没有」的多处不一致。
            // 新增全局绑定只需改 `events/keymap.rs` 的 GLOBAL_BINDINGS。
            crate::ui::events::keymap::GLOBAL_BINDINGS
                .iter()
                .map(|d| (d.binding.label().to_string(), i18n::t1(d.key)))
                .collect(),
        ),
        (
            i18n::t1("ui.help.section.confirm"),
            vec![
                ("1/2/3/4".to_string(), i18n::t1("ui.help.approve.choose")),
                (
                    "Ctrl+Shift+D".to_string(),
                    i18n::t1("ui.help.control.perm_debug"),
                ),
            ],
        ),
        (
            i18n::t1("ui.help.section.command"),
            vec![
                ("/help".to_string(), i18n::t1("ui.help.cmd.help")),
                ("/clear".to_string(), i18n::t1("ui.help.cmd.clear")),
                ("/model".to_string(), i18n::t1("ui.help.cmd.model")),
                ("/effort".to_string(), i18n::t1("ui.help.cmd.effort")),
                ("/settings".to_string(), i18n::t1("ui.help.cmd.settings")),
                (
                    "/permissions".to_string(),
                    i18n::t1("ui.help.cmd.permissions"),
                ),
                ("/loop".to_string(), i18n::t1("ui.help.cmd.loop")),
                ("/agents".to_string(), i18n::t1("ui.help.cmd.agents")),
                ("/plugin".to_string(), i18n::t1("ui.help.cmd.plugin")),
                ("/remote".to_string(), i18n::t1("ui.help.cmd.remote")),
                ("/hooks".to_string(), i18n::t1("ui.help.cmd.hooks")),
                ("/mcp".to_string(), i18n::t1("ui.help.cmd.mcp")),
                ("/restore".to_string(), i18n::t1("ui.help.cmd.restore")),
            ],
        ),
        (
            i18n::t1("ui.help.section.other"),
            vec![
                ("?".to_string(), i18n::t1("ui.help.other.toggle")),
                ("@filename".to_string(), i18n::t1("ui.help.other.at")),
                ("!cmd".to_string(), i18n::t1("ui.help.other.bang")),
                ("!!cmd".to_string(), i18n::t1("ui.help.other.bangbang")),
            ],
        ),
    ];

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                i18n::t1("ui.help.title"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(
                i18n::t1("ui.help.close"),
                crate::ui::themes::theme::muted_style(),
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
        Span::styled(i18n::t1("ui.help.tip"), Style::default().fg(Color::Cyan)),
        Span::styled(
            i18n::t1("ui.help.tip.confirm"),
            crate::ui::themes::theme::muted_style(),
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
            .title(i18n::t1("ui.help.block")),
    );

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}
