/// Agent 并发任务组渲染组件
///
/// 对标 Claude Code 的 `renderGroupedAgentToolUse`，实现多 Agent 并发展示：
/// - 头部汇总行：全部完成/异步/运行中
/// - 每个 Agent 两行进度：AgentProgressLine 风格
/// - 嵌套抑制：内部消息不显示展开提示
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::{AgentTaskStatus, ChatEntry, ChatEntryType};
use crate::ui::state::ChatState;

/// 渲染多个 Agent 并发任务组
///
/// 返回格式（对标 Claude Code）：
/// ```text
/// Running 3 agents…
/// ├─ AgentType (description) · 12 tool uses · 10k tokens
/// │  ⎿  Initializing…
/// ├─ AgentType (description) · 5 tool uses · 5k tokens
/// │  ⎿  Done
/// └─ AgentType (description) · 8 tool uses · 8k tokens
///    ⎿  Done
/// ```
pub fn render_agent_group(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: u16,
) -> Vec<Vec<Line<'static>>> {
    let mut blocks = Vec::new();
    let task_ids = entry.agent_task_ids.as_deref().unwrap_or(&[]);

    // 收集所有 Agent 任务的信息
    let agent_stats: Vec<AgentStat> = task_ids
        .iter()
        .filter_map(|id| state.active_agent_tasks.get(id))
        .map(|info| AgentStat::from_info(info, area_width))
        .collect();

    if agent_stats.is_empty() {
        // 没有找到 Agent 信息，显示基础汇总
        blocks.push(render_empty_group_header(task_ids.len()));
        return blocks;
    }

    // ── 1. 头部汇总行 ──
    let header = render_group_header(&agent_stats, area_width);
    blocks.push(header);

    // ── 2. 每个 Agent 两行进度 ──
    for (i, stat) in agent_stats.iter().enumerate() {
        let is_last = i == agent_stats.len() - 1;
        let lines = render_agent_progress_lines(state, stat, is_last, area_width);
        blocks.extend(lines);
    }

    blocks
}

/// 渲染组头部汇总行
///
/// 格式取决于 Agent 状态：
/// - 全部完成: `✓ 3 agents finished`
/// - 全部异步: `● 3 background agents launched (↓ manage)`
/// - 部分运行: `● Running 3 agents…`
fn render_group_header(stats: &[AgentStat], area_width: u16) -> Vec<Line<'static>> {
    let total = stats.len();
    let running = stats.iter().filter(|s| s.status == AgentTaskStatus::Running).count();
    let completed = stats.iter().filter(|s| s.status == AgentTaskStatus::Completed).count();
    let failed = stats.iter().filter(|s| s.status == AgentTaskStatus::Failed).count();
    let all_async = stats.iter().all(|s| s.is_async && s.is_resolved);
    let all_complete = running == 0;

    let mut spans = Vec::new();

    if all_complete {
        // 全部完成
        if all_async {
            spans.push(Span::styled(
                "● ",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                total.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                " background agents launched ",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                "(↓ manage)",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let icon = if failed > 0 { "⚠" } else { "✓" };
            let color = if failed > 0 { Color::Yellow } else { Color::Green };
            spans.push(Span::styled(
                format!("{} ", icon),
                Style::default().fg(color),
            ));
            spans.push(Span::styled(
                total.to_string(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                " agents finished",
                Style::default().fg(Color::DarkGray),
            ));
            if failed > 0 {
                spans.push(Span::styled(
                    format!(" ({} failed)", failed),
                    Style::default().fg(Color::Red),
                ));
            }
        }
    } else {
        // 有正在运行的
        spans.push(Span::styled(
            "● ",
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::styled(
            format!("Running {} agent{}…", total, if total == 1 { "" } else { "s" }),
            Style::default().fg(Color::DarkGray),
        ));
    }

    vec![Line::from(spans)]
}

/// 渲染空组头部（找不到 Agent 信息时）
fn render_empty_group_header(count: usize) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        format!(
            "● {} agent{} launched",
            count,
            if count == 1 { "" } else { "s" }
        ),
        Style::default().fg(Color::DarkGray),
    ))]
}

/// 渲染单个 Agent 进度行（两行）
///
/// 对标 Claude Code 的 AgentProgressLine：
/// 第一行: `├─ AgentType (description) · N tool uses · Nk tokens`
/// 第二行: `│  ⎿  Initializing… / LastToolInfo / Done`
fn render_agent_progress_lines(
    _state: &ChatState,
    stat: &AgentStat,
    is_last: bool,
    area_width: u16,
) -> Vec<Vec<Line<'static>>> {
    let tree_char = if is_last { "└─" } else { "├─" };
    let is_backgrounded = stat.is_async && stat.is_resolved;

    // ── 第一行：Agent 类型 + 描述 + 统计 ──
    let mut line1_spans = Vec::new();

    // 树形连接符
    line1_spans.push(Span::styled(
        format!("{} ", tree_char),
        Style::default().fg(Color::DarkGray),
    ));

    // Agent 类型（粗体，带颜色）
    let type_color = match stat.status {
        AgentTaskStatus::Running => Color::Yellow,
        AgentTaskStatus::Completed => Color::Green,
        AgentTaskStatus::Failed => Color::Red,
        AgentTaskStatus::Background => Color::DarkGray,
    };
    let display_name = stat.name.as_deref().unwrap_or(&stat.agent_type);
    line1_spans.push(Span::styled(
        display_name.to_string(),
        Style::default()
            .fg(type_color)
            .add_modifier(Modifier::BOLD),
    ));

    // 描述（灰色括号）
    if let Some(desc) = &stat.description {
        let max_desc_width = (area_width as usize).saturating_sub(40);
        let truncated = if desc.len() > max_desc_width {
            format!(" ({}…)", &desc[..max_desc_width.saturating_sub(1)])
        } else {
            format!(" ({})", desc)
        };
        line1_spans.push(Span::styled(
            truncated,
            Style::default().fg(Color::DarkGray),
        ));
    }

    // 统计信息（非 backgrounded 时显示）
    if !is_backgrounded {
        if stat.tool_use_count > 0 || stat.tokens.unwrap_or(0) > 0 {
            line1_spans.push(Span::styled(
                " · ".to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            if stat.tool_use_count > 0 {
                line1_spans.push(Span::styled(
                    format!("{} tool {}", stat.tool_use_count, if stat.tool_use_count == 1 { "use" } else { "uses" }),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            if let Some(tokens) = stat.tokens {
                if tokens > 0 {
                    if stat.tool_use_count > 0 {
                        line1_spans.push(Span::styled(
                            " · ".to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    line1_spans.push(Span::styled(
                        format_tokens(tokens),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
        }
    }

    // ── 第二行：状态信息 ──
    let mut line2_spans = Vec::new();

    // 树形前缀（├─ 变成 │，└─ 变成空格）
    let prefix = if is_last { "   " } else { "│  " };
    line2_spans.push(Span::styled(
        prefix.to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    line2_spans.push(Span::styled(
        "⎿  ".to_string(),
        Style::default().fg(Color::DarkGray),
    ));

    // 状态文本
    let (status_text, status_color) = if is_backgrounded {
        let text = stat.task_description.as_deref().unwrap_or("Running in the background");
        (text.to_string(), Color::DarkGray)
    } else {
        match stat.status {
            AgentTaskStatus::Running => {
                let text = stat.last_tool_info.as_deref().unwrap_or("Initializing…");
                (text.to_string(), Color::Yellow)
            }
            AgentTaskStatus::Completed => ("Done".to_string(), Color::Green),
            AgentTaskStatus::Failed => ("Failed".to_string(), Color::Red),
            AgentTaskStatus::Background => {
                let text = stat.task_description.as_deref().unwrap_or("Running in the background");
                (text.to_string(), Color::DarkGray)
            }
        }
    };
    line2_spans.push(Span::styled(status_text, Style::default().fg(status_color)));

    vec![vec![Line::from(line1_spans)], vec![Line::from(line2_spans)]]
}

/// Agent 统计信息（从 AgentTaskInfo 提取）
struct AgentStat {
    agent_type: String,
    description: Option<String>,
    name: Option<String>,
    task_description: Option<String>,
    status: AgentTaskStatus,
    tool_use_count: u32,
    tokens: Option<u32>,
    is_resolved: bool,
    is_error: bool,
    is_async: bool,
    last_tool_info: Option<String>,
}

impl AgentStat {
    fn from_info(info: &crate::ui::state::AgentTaskInfo, _area_width: u16) -> Self {
        Self {
            agent_type: info.agent_type.clone(),
            description: Some(info.description.clone()),
            name: None,
            task_description: None,
            status: info.status.clone(),
            tool_use_count: info.tool_use_count,
            tokens: Some(info.tokens),
            is_resolved: info.is_resolved,
            is_error: info.is_error,
            is_async: info.is_async,
            last_tool_info: info.last_tool_info.clone(),
        }
    }
}

/// 格式化 token 数量
fn format_tokens(count: u32) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}
