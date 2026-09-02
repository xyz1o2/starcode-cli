//! Plugin manager modal — Claude Code style `/plugin` UI.
//! Tabs: Discover / Installed / Marketplaces / Errors.

use super::super::common::modal_shell::*;
use crate::ui::state::modal::{PluginConfirmKind, PluginTab};
use crate::ui::state::{ChatState, Modal};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn render_plugins_modal(f: &mut ratatui::Frame<'_>, area: ratatui::prelude::Rect, state: &ChatState) {
    let Some(Modal::Plugins { tab }) = state.top_modal() else {
        return;
    };
    let tab = *tab;

    let inner = modal_shell(
        f,
        area,
        &ModalSpec {
            title: " Plugins ".to_string(),
            ..ModalSpec::default()
        },
    );
    let (body, footer) = with_footer(inner);

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(tabs_line(tab));

    match tab {
        // 详情页覆盖 Discover 列表（对标 Claude Code Enter 进详情）
        PluginTab::Discover if state.plugin_detail.is_some() => {
            lines.extend(detail_lines(state, body.height))
        }
        PluginTab::Discover => lines.extend(discover_lines(state, body.height)),
        PluginTab::Installed => lines.extend(installed_lines(state, body.height)),
        PluginTab::Marketplaces => lines.extend(marketplaces_lines(state, body.height)),
        PluginTab::Errors => lines.extend(errors_lines(state, body.height)),
    }

    if let Some(confirm) = &state.plugin_confirm {
        let (text, hint) = match &confirm.kind {
            PluginConfirmKind::Install { plugin, scope } => (
                format!("  Install plugin '{}' to {}?", plugin.name, scope),
                "  Enter=confirm  Esc=cancel".to_string(),
            ),
            PluginConfirmKind::InstallScope { plugin } => (
                format!("  Install plugin '{}' to:", plugin.name),
                "  U=user (all projects)  P=this project  Esc=cancel".to_string(),
            ),
            PluginConfirmKind::Uninstall { name } => (
                format!("  Uninstall plugin '{}'?", name),
                "  Enter=confirm  Esc=cancel".to_string(),
            ),
            PluginConfirmKind::RemoveMarketplace { name } => (
                format!("  Remove marketplace '{}'?", name),
                "  Enter=confirm  Esc=cancel".to_string(),
            ),
        };
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                text,
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
    }

    render_body(f, body, lines);

    let hints: &[(&str, &str)] = match tab {
        PluginTab::Discover if state.plugin_detail.is_some() => &[
            ("Enter/i", "install"),
            ("Esc", "back to list"),
        ],
        PluginTab::Discover => &[
            ("↑↓", "select"),
            ("Type", "filter"),
            ("Space", "toggle"),
            ("i", "install selected"),
            ("Enter", "details"),
            ("Esc", "clear search / back"),
        ],
        PluginTab::Installed => &[
            ("↑↓", "select"),
            ("Enter", "enable/disable"),
            ("U", "uninstall"),
            ("Tab", "switch tab"),
            ("Esc", "close"),
        ],
        PluginTab::Marketplaces => &[
            ("↑↓", "select"),
            ("A", "add marketplace"),
            ("u", "update"),
            ("Enter", "remove"),
            ("Tab", "switch tab"),
            ("Esc", "close"),
        ],
        PluginTab::Errors => &[("R", "retry"), ("Tab", "switch tab"), ("Esc", "close")],
    };
    render_body(f, footer, vec![footer_hints(hints)]);
}

fn tabs_line(active: PluginTab) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, tab) in PluginTab::ALL.iter().enumerate() {
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

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn plugin_status_icon(enabled: bool) -> Span<'static> {
    if enabled {
        Span::styled("●", Style::default().fg(Color::Green))
    } else {
        Span::styled("○", Style::default().fg(Color::DarkGray))
    }
}

fn discover_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        "  Discover plugins",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    // 实时搜索框（输入即过滤，对标 Claude Code Discover 的 filter 行）
    let total = state.plugin_discover.len();
    let indices = state.filtered_discover_indices();
    let search_span = if state.plugin_search.is_empty() {
        Span::styled("Type to filter…", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(
            state.plugin_search.clone(),
            Style::default().fg(Color::White),
        )
    };
    let mut search_line = vec![
        Span::styled("  Search: ", Style::default().fg(Color::Gray)),
        search_span,
        Span::styled("▌", Style::default().fg(Color::Cyan)),
    ];
    if !state.plugin_search.is_empty() {
        search_line.push(Span::styled(
            format!("  ({} / {})", indices.len(), total),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(search_line));

    if state.plugin_loading {
        lines.push(Line::from(Span::styled(
            "  Loading…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if let Some(hint) = &state.plugin_errors_hint {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {}", hint),
            Style::default().fg(Color::Yellow),
        )));
    }
    if let Some(msg) = &state.plugin_message {
        lines.push(Line::from(Span::styled(
            format!("  ℹ {}", msg),
            Style::default().fg(Color::Cyan),
        )));
    }

    if state.plugin_discover.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No plugins available.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Add a marketplace first using the Marketplaces tab.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if indices.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  No plugins match '{}'.", state.plugin_search.trim()),
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    if !state.plugin_selected.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "  {} selected — press i to install",
                state.plugin_selected.len()
            ),
            Style::default().fg(Color::Yellow),
        )));
    }

    // 滑动窗口：保证当前选中行始终可见（对标 Claude Code 分页指示）
    let visible = body_h.saturating_sub((lines.len() as u16) + 2).max(3) as usize;
    let sel = state.plugin_index.min(indices.len().saturating_sub(1));
    let start = if sel >= visible { sel + 1 - visible } else { 0 };
    let end = (start + visible).min(indices.len());
    if start > 0 {
        lines.push(Line::from(Span::styled(
            "  ↑ more above",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for pos in start..end {
        let i = indices[pos];
        let row = &state.plugin_discover[i];
        let selected = pos == state.plugin_index;
        let style = row_style(pos, state.plugin_index);
        let radio = if state.plugin_selected.contains(&row.plugin.name) {
            Span::styled("◉", Style::default().fg(Color::Cyan))
        } else {
            Span::styled("○", Style::default().fg(Color::DarkGray))
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            radio,
            Span::styled(
                format!(" {:<22}", truncate_str(&row.plugin.name, 22)),
                style,
            ),
            Span::styled(
                format!(" {:<12}", truncate_str(&row.marketplace, 12)),
                if selected {
                    style
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            Span::styled(
                format!(" {}", truncate_str(&row.plugin.description, 36)),
                if selected {
                    style
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ]));
    }
    if end < indices.len() {
        lines.push(Line::from(Span::styled(
            "  ↓ more below",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn installed_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if state.plugin_loading {
        lines.push(Line::from(Span::styled(
            "  Loading…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if let Some(msg) = &state.plugin_message {
        lines.push(Line::from(Span::styled(
            format!("  ℹ {}", msg),
            Style::default().fg(Color::Cyan),
        )));
    }
    if state.plugin_installed.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No plugins installed.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Install from the Discover tab, or /plugin install <source>.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for (i, p) in state.plugin_installed.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(2) as usize {
            break;
        }
        let selected = i == state.plugin_index;
        let style = row_style(i, state.plugin_index);
        lines.push(Line::from(vec![
            Span::raw("  "),
            plugin_status_icon(p.entry.enabled),
            Span::styled(
                format!(" {:<22}", truncate_str(&p.entry.name, 22)),
                style,
            ),
            Span::styled(
                format!(" {:<8}", truncate_str(&p.entry.install_type, 8)),
                if selected {
                    style
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
            // 范围徽标（对标 Claude Code 的 [user]/[project] 标注）
            Span::styled(
                format!(" [{:<7}]", p.entry.scope),
                Style::default().fg(if p.entry.scope == "user" {
                    Color::Magenta
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!(" {}", truncate_str(&p.entry.source, 32)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    lines
}

/// 插件详情页（对标 Claude Code Enter 进详情：版本/作者/来源/安装入口）
fn detail_lines(state: &ChatState, _body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some((marketplace, p)) = &state.plugin_detail else {
        return lines;
    };

    lines.push(Line::from(Span::styled(
        format!("  {}", p.name),
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    let mut meta: Vec<String> = Vec::new();
    if !p.version.is_empty() {
        meta.push(format!("v{}", p.version));
    }
    if !p.author.is_empty() {
        meta.push(format!("by {}", p.author));
    }
    meta.push(format!("from {}", marketplace));
    lines.push(Line::from(Span::styled(
        format!("  {}", meta.join(" · ")),
        Style::default().fg(Color::Gray),
    )));

    if !p.description.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", p.description),
            Style::default().fg(Color::Gray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  Source: {}", p.source),
        Style::default().fg(Color::DarkGray),
    )));
    if !p.homepage.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Homepage: {}", p.homepage),
            Style::default().fg(Color::Cyan),
        )));
    }
    if let Some(r) = p.source_ref.as_deref().filter(|r| !r.trim().is_empty()) {
        lines.push(Line::from(Span::styled(
            format!("  Ref: {}", r),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press Enter to install — choose U (user, all projects) or P (this project)",
        Style::default().fg(Color::Yellow),
    )));
    lines
}

fn marketplaces_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if state.plugin_loading {
        lines.push(Line::from(Span::styled(
            "  Loading…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if let Some(msg) = &state.plugin_message {
        lines.push(Line::from(Span::styled(
            format!("  ℹ {}", msg),
            Style::default().fg(Color::Cyan),
        )));
    }

    // 行 0 = Add marketplace…
    {
        let selected = state.plugin_index == 0;
        let style = row_style(0, if selected { 0 } else { usize::MAX });
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "＋ Add marketplace…".to_string(),
                style,
            ),
            Span::styled(
                "  (git URL / GitHub owner/repo / local path)",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    for (i, m) in state.plugin_marketplaces.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(2) as usize {
            break;
        }
        let row = i + 1;
        let selected = row == state.plugin_index;
        let style = row_style(row, if selected { row } else { usize::MAX });
        let count = state
            .plugin_marketplace_counts
            .get(&m.name)
            .copied()
            .unwrap_or(0);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("●", Style::default().fg(Color::Green)),
            Span::styled(
                format!(" {:<22}", truncate_str(&m.name, 22)),
                style,
            ),
            Span::styled(
                format!(" {} plugins", count),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("  {}", truncate_str(&m.source, 36)),
                if selected {
                    style
                } else {
                    Style::default().fg(Color::Gray)
                },
            ),
        ]));
    }
    lines
}

/// 错误分类（对标 Claude Code PluginErrors.tsx 的错误类型 + 可重试性）
fn classify_plugin_error(err: &str) -> (&'static str, bool, Color) {
    let e = err.to_lowercase();
    if e.contains("authentication") || e.contains("auth") || e.contains("permission denied (os") && e.contains("git") {
        ("git-auth", false, Color::Red)
    } else if e.contains("timed out") || e.contains("timeout") {
        ("git-timeout", true, Color::Yellow)
    } else if e.contains("failed to connect") || e.contains("network") || e.contains("connection") {
        ("network", true, Color::Yellow)
    } else if e.contains("manifest") || e.contains("missing") {
        ("invalid-manifest", false, Color::Red)
    } else {
        ("other", false, Color::Red)
    }
}

fn errors_lines(state: &ChatState, body_h: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if state.plugin_loading {
        lines.push(Line::from(Span::styled(
            "  Loading…",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }
    if state.plugin_errors.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No plugin errors.",
            Style::default().fg(Color::Green),
        )));
        return lines;
    }

    for (i, (name, err)) in state.plugin_errors.iter().enumerate() {
        if lines.len() >= body_h.saturating_sub(2) as usize {
            break;
        }
        let selected = i == state.plugin_index;
        let style = row_style(i, if selected { i } else { usize::MAX });
        let (kind, retryable, color) = classify_plugin_error(err);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("✗", Style::default().fg(Color::Red)),
            Span::styled(format!(" {} ", kind), Style::default().fg(color)),
            Span::styled(format!(" {}: ", name), style),
            Span::styled(
                truncate_str(err, 48),
                Style::default().fg(Color::Yellow),
            ),
            if retryable {
                Span::styled("  [R]", Style::default().fg(Color::Cyan))
            } else {
                Span::raw("")
            },
        ]));
    }
    lines
}
