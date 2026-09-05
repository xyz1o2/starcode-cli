//! MCP manager modal — view state machine renderer (list → server menu →
//! tools → tool detail → confirm-remove), mirroring Claude Code's MCPSettings.

use super::super::common::modal_shell::*;
use crate::ui::state::{ChatState, McpView, Modal};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn render_mcp_modal(
    f: &mut ratatui::Frame<'_>,
    area: ratatui::prelude::Rect,
    state: &ChatState,
) {
    let Some(Modal::Mcp { view }) = state.top_modal() else {
        return;
    };
    let view = view.clone();

    let inner = modal_shell(
        f,
        area,
        &ModalSpec {
            title: mcp_title(&view),
            ..ModalSpec::default()
        },
    );
    let (body, footer) = with_footer(inner);

    let mut lines: Vec<Line<'static>> = Vec::new();
    match &view {
        McpView::List => lines = mcp_list_lines(state, body.height),
        McpView::ServerMenu { name } => lines = mcp_menu_lines(state, name),
        McpView::Tools { name } => lines = mcp_tools_lines(state, name, body.height),
        McpView::ToolDetail { name, index } => lines = mcp_detail_lines(state, name, *index),
        McpView::ConfirmRemove { name } => lines = mcp_confirm_lines(name),
    }
    render_body(f, body, lines);

    let hints: &[(&str, &str)] = match &view {
        McpView::List => &[
            ("↑↓", "select"),
            ("Enter", "details"),
            ("R", "refresh"),
            ("N", "add"),
            ("Esc", "close"),
        ],
        McpView::ServerMenu { .. } => &[("↑↓", "select"), ("Enter", "run"), ("Esc", "back")],
        McpView::Tools { .. } => &[("↑↓", "select"), ("Enter", "detail"), ("Esc", "back")],
        McpView::ToolDetail { .. } => &[("Esc", "back")],
        McpView::ConfirmRemove { .. } => &[("Enter", "confirm remove"), ("Esc", "cancel")],
    };
    render_body(f, footer, vec![footer_hints(hints)]);
}

fn mcp_title(view: &McpView) -> String {
    match view {
        McpView::List => " MCP Servers ".to_string(),
        McpView::ServerMenu { name } => format!(" MCP · {} ", name),
        McpView::Tools { name } => format!(" MCP · {} · Tools ", name),
        McpView::ToolDetail { .. } => " MCP · Tool Detail ".to_string(),
        McpView::ConfirmRemove { .. } => " MCP · Remove Server ".to_string(),
    }
}

fn mcp_list_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if state.mcp_modal_loading {
        lines.push(Line::from(Span::styled(
            "  Loading MCP servers…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if let Some(err) = &state.mcp_modal_error {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {}", err),
            Style::default().fg(Color::Yellow),
        )));
    }
    if let Some(msg) = &state.mcp_modal_action_msg {
        lines.push(Line::from(Span::styled(
            format!("  ℹ {}", msg),
            Style::default().fg(Color::Cyan),
        )));
    }
    if state.mcp_modal_servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No MCP servers configured.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Add one with /mcp add <name> … (press N)",
            Style::default().fg(Color::Gray),
        )));
        return lines;
    }

    for (i, row) in state.mcp_modal_servers.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(2) as usize {
            break;
        }
        let selected = i == state.mcp_modal_index;
        let style = row_style(i, state.mcp_modal_index);
        let mut spans = vec![Span::raw(" ")];
        spans.extend(status_spans(row.connected, row.disabled));
        spans.push(Span::styled(
            format!(" {:<18}", truncate_str(&row.name, 18)),
            style,
        ));
        spans.push(Span::styled(
            format!(" {:<6}", truncate_str(&row.transport, 6)),
            if selected {
                style
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
        let status_text = if row.disabled {
            "disabled".to_string()
        } else if row.connected {
            format!("{} tools", row.tool_count)
        } else {
            row.error.clone().unwrap_or_else(|| "offline".to_string())
        };
        spans.push(Span::styled(
            format!(" {}", status_text),
            if row.disabled {
                Style::default().fg(Color::DarkGray)
            } else if row.connected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            },
        ));
        lines.push(Line::from(spans));
    }

    let enabled = state
        .mcp_modal_servers
        .iter()
        .filter(|r| !r.disabled)
        .count();
    let connected = state
        .mcp_modal_servers
        .iter()
        .filter(|r| r.connected)
        .count();
    lines.push(Line::from(Span::styled(
        format!(
            "  {} servers · {} connected · {} enabled",
            state.mcp_modal_servers.len(),
            connected,
            enabled
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn mcp_menu_lines(state: &ChatState, name: &str) -> Vec<Line<'static>> {
    let row = state
        .mcp_modal_servers
        .iter()
        .find(|r| r.name == name)
        .cloned();

    let mut lines = Vec::new();
    if let Some(msg) = &state.mcp_modal_action_msg {
        lines.push(Line::from(Span::styled(
            format!("  ℹ {}", msg),
            Style::default().fg(Color::Cyan),
        )));
    }
    if let Some(row) = &row {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(status_spans(row.connected, row.disabled));
        spans.push(Span::styled(
            format!(" {} · {} · ", row.transport, truncate_str(&row.command, 40)),
            Style::default().fg(Color::Gray),
        ));
        let st = if row.disabled {
            "disabled"
        } else if row.connected {
            "connected"
        } else if row.needs_auth {
            // 对标 Claude Code UnifiedInstalledCell 的 "Enter to auth"
            "needs-auth"
        } else {
            "offline"
        };
        spans.push(Span::styled(
            st.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(spans));
        lines.push(Line::from(""));
    }

    let toggle_label = match &row {
        Some(r) if r.disabled => "Enable server",
        _ => "Disable server",
    };
    // needs-auth 服务器插入 Authenticate 项（对标 Claude Code "Enter to auth"）
    let mut actions: Vec<String> = vec![
        "View tools".to_string(),
        "Reconnect".to_string(),
        toggle_label.to_string(),
    ];
    if row.as_ref().map(|r| r.needs_auth).unwrap_or(false) {
        actions.push("Authenticate… (opens browser)".to_string());
    }
    actions.push("Remove server…".to_string());
    actions.push(".. Back".to_string());
    for (i, label) in actions.iter().enumerate() {
        let style = row_style(i, state.mcp_modal_menu_index);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<16}", label), style),
        ]));
    }
    lines
}

fn mcp_tools_lines(state: &ChatState, name: &str, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  Server: {}", name),
        Style::default().fg(Color::DarkGray),
    )));

    if state.mcp_modal_loading {
        lines.push(Line::from(Span::styled(
            "  Discovering tools…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if let Some(err) = &state.mcp_modal_error {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {}", err),
            Style::default().fg(Color::Yellow),
        )));
    }
    if state.mcp_modal_tools.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no tools)",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for (i, tool) in state.mcp_modal_tools.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(1) as usize {
            break;
        }
        let style = row_style(i, state.mcp_modal_index);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<26}", truncate_str(&tool.name, 26)), style),
            Span::styled(
                truncate_str(&tool.description, 46),
                if i == state.mcp_modal_index {
                    style
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ]));
    }
    lines
}

fn mcp_detail_lines(state: &ChatState, _name: &str, index: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(tool) = state.mcp_modal_tools.get(index) else {
        return lines;
    };

    lines.push(Line::from(Span::styled(
        format!("  {}", tool.name),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    if !tool.description.is_empty() {
        for seg in tool.description.lines().take(4) {
            lines.push(Line::from(Span::styled(
                format!("  {}", seg),
                Style::default().fg(Color::White),
            )));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "  Input Schema:",
        Style::default().fg(Color::DarkGray),
    )));
    let schema = serde_json::to_string_pretty(&tool.input_schema).unwrap_or_else(|_| "{}".into());
    for seg in schema.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {}", seg),
            Style::default().fg(Color::Gray),
        )));
    }
    lines
}

fn mcp_confirm_lines(name: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("  Remove MCP server '{}' from the project config?", name),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This cannot be undone (re-add with /mcp add).",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}
