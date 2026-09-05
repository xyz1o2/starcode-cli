//! Marketplace modal — tabbed browser (Browse / Installed / Sources),
//! mirroring Claude Code's PluginSettings tab layout.

use super::super::common::modal_shell::*;
use crate::core::extensions::types::ExtensionType;
use crate::ui::state::modal::MarketTab;
use crate::ui::state::{ChatState, Modal};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn render_market_modal(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::prelude::Rect,
    state: &ChatState,
) {
    let Some(Modal::Market { tab }) = state.top_modal() else {
        return;
    };
    let tab = *tab;

    let inner = modal_shell(
        f,
        area,
        &ModalSpec {
            title: " Extension Marketplace ".to_string(),
            accent: Color::Magenta,
            ..ModalSpec::default()
        },
    );
    let (body, footer) = with_footer(inner);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(tabs_line(tab));

    match tab {
        MarketTab::Browse => lines.extend(browse_lines(state, body.height)),
        MarketTab::Installed => lines.extend(installed_lines(state, body.height)),
        MarketTab::Sources => lines.extend(sources_lines(state)),
    }

    if let Some(confirm) = &state.market_confirm {
        let verb = if confirm.install {
            "Install"
        } else {
            "Uninstall"
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  ⚠ {} '{}'? ", verb, confirm.name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Enter=confirm  Esc=cancel",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    render_body(f, body, lines);

    let hints: &[(&str, &str)] = match tab {
        MarketTab::Browse => &[
            ("↑↓", "select"),
            ("Enter", "install"),
            ("Tab", "switch tab"),
            ("/", "search"),
            ("Esc", "close"),
        ],
        MarketTab::Installed => &[
            ("↑↓", "select"),
            ("Enter", "enable/disable"),
            ("U", "uninstall"),
            ("Tab", "switch tab"),
            ("Esc", "close"),
        ],
        MarketTab::Sources => &[("Tab", "switch tab"), ("Esc", "close")],
    };
    render_body(f, footer, vec![footer_hints(hints)]);
}

fn tabs_line(active: MarketTab) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, tab) in MarketTab::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        let label = tab.label();
        if *tab == active {
            spans.push(Span::styled(
                format!(" [{}] ", label),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("  {}  ", label),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    Line::from(spans)
}

fn type_label(t: &ExtensionType) -> &'static str {
    match t {
        ExtensionType::Skill => "skill",
        ExtensionType::Plugin => "plugin",
        ExtensionType::Mcp => "mcp",
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn entry_rows(
    state: &ChatState,
    body_h: u16,
    installed: std::collections::HashSet<String>,
    enabled_of: impl Fn(&str) -> Option<bool>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if state.market_loading {
        lines.push(Line::from(Span::styled(
            "  Loading marketplace…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    if matches!(
        state.top_modal(),
        Some(Modal::Market {
            tab: MarketTab::Browse
        })
    ) {
        let query_disp = if state.market_query.is_empty() {
            "  / search…".to_string()
        } else {
            format!("  / {}", state.market_query)
        };
        lines.push(Line::from(Span::styled(
            query_disp,
            Style::default().fg(Color::Yellow),
        )));
    }

    if state.market_entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (empty)",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for (i, entry) in state.market_entries.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(3) as usize {
            break;
        }
        let selected = i == state.market_index;
        let style = row_style(i, state.market_index);
        let mut spans = vec![Span::raw(" ")];

        match enabled_of(&entry.name) {
            Some(true) => spans.push(Span::styled("●", Style::default().fg(Color::Green))),
            Some(false) => spans.push(Span::styled("○", Style::default().fg(Color::DarkGray))),
            None => {
                if installed.contains(&entry.name) {
                    spans.push(Span::styled("✓", Style::default().fg(Color::Green)));
                } else {
                    spans.push(Span::styled(" ", Style::default()));
                }
            }
        }

        spans.push(Span::styled(
            format!(" {:<20}", truncate_str(&entry.name, 20)),
            style,
        ));
        spans.push(Span::styled(
            format!(" {:<7}", type_label(&entry.extension_type)),
            if selected {
                style
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        spans.push(Span::styled(
            format!(" v{:<6}", truncate_str(&entry.version, 6)),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            format!(" {}", truncate_str(&entry.description, 40)),
            if selected {
                style
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(Span::styled(
        format!("  {} items", state.market_entries.len()),
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn browse_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    entry_rows(state, body_h, state.market_installed_names.clone(), |_| {
        None
    })
}

fn installed_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    // 启用状态来自 reload 时快照的合并表（extensions 注册表 + plugins 系统）
    let enabled = state.market_enabled_map.clone();
    let lookup = move |name: &str| enabled.get(name).copied();
    entry_rows(state, body_h, std::collections::HashSet::new(), lookup)
}

fn sources_lines(state: &ChatState) -> Vec<Line<'static>> {
    use crate::core::extensions::marketplace::Marketplace;
    use crate::core::extensions::registry::ExtensionRegistryManager;

    let marketplace = Marketplace::new();
    let index = marketplace.load_index();
    let dir = ExtensionRegistryManager::marketplace_dir();

    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Marketplace dir:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(dir.display().to_string(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Index entries:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                index.entries.len().to_string(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Updated at:       ", Style::default().fg(Color::DarkGray)),
            Span::styled(index.updated_at.clone(), Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Install from Browse tab; registry lives under the same directory.",
            Style::default().fg(Color::Gray),
        )),
    ]
}
