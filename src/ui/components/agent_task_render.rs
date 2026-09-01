/// Agent 任务渲染组件 — 单 Agent 渐进式折叠展示
///
/// 对标 Claude Code 的 `renderToolUseProgressMessage`，实现三级展示：
/// 1. 精简模式 (Condensed): 一行汇总 "In progress... · N tools · tokens"
/// 2. 分组模式 (Grouped): 最后 N 条进度 + "+N more" 提示
/// 3. Verbose/Transcript: 完整子消息列表
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::{AgentTaskStatus, ChatEntry};
use crate::ui::state::ChatState;

/// 分组模式下最多显示的进度消息数（对标 Claude Code 的 MAX_PROGRESS_MESSAGES_TO_SHOW = 3）
const MAX_PROGRESS_MESSAGES_TO_SHOW: usize = 3;

/// 渲染单个 Agent 任务条目
///
/// 返回多行 `Vec<Vec<Line>>`，每行是一个独立的渲染块：
/// - 头部汇总行（Agent 类型 + 描述 + 状态）
/// - 子消息列表（根据模式折叠/展开）
/// - 尾部提示（"+N more" 或 "Ctrl+O 展开"）
pub fn render_agent_task_entry(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: u16,
    _entry_idx: usize,
) -> Vec<Vec<Line<'static>>> {
    let mut blocks = Vec::new();
    let is_transcript = state.is_transcript_mode;

    // ── 1. 头部汇总行 ──
    let header = render_agent_header(state, entry, area_width);
    blocks.push(header);

    // ── 2. 子消息列表 ──
    let sub_entries = entry.agent_sub_entries.as_deref().unwrap_or(&[]);
    let is_resolved = entry.agent_is_resolved.unwrap_or(false);
    let is_error = entry.agent_is_error.unwrap_or(false);
    let is_async = entry.agent_is_async.unwrap_or(false);

    if is_transcript {
        // Verbose/Transcript 模式：显示全部子消息
        let sub_lines = render_sub_entries_verbose(state, sub_entries, area_width);
        if !sub_lines.is_empty() {
            blocks.extend(sub_lines);
        }
    } else if is_resolved {
        // 已完成：精简一行 "Done"（含耗时）
        blocks.push(render_done_line(state, entry, is_error));
    } else if sub_entries.is_empty() {
        // 初始化中
        blocks.push(vec![Line::from(Span::styled(
            "      Initializing…",
            Style::default().fg(Color::DarkGray),
        ))]);
    } else {
        // 分组模式：显示最后 N 条 + "+N more" 提示
        let (displayed, hidden_count) = if sub_entries.len() > MAX_PROGRESS_MESSAGES_TO_SHOW {
            let displayed = &sub_entries[sub_entries.len() - MAX_PROGRESS_MESSAGES_TO_SHOW..];
            let hidden = sub_entries.len() - MAX_PROGRESS_MESSAGES_TO_SHOW;
            (displayed, hidden)
        } else {
            (sub_entries, 0)
        };

        for sub in displayed {
            let line = render_sub_entry_condensed(state, sub, area_width);
            blocks.push(line);
        }

        if hidden_count > 0 {
            blocks.push(vec![Line::from(Span::styled(
                format!("      +{} more tool {}", hidden_count, if hidden_count == 1 { "use" } else { "uses" }),
                Style::default().fg(Color::DarkGray),
            ))]);
        }
    }

    // ── 3. 尾部提示 ──
    if !is_transcript && !is_async {
        blocks.push(vec![Line::from(Span::styled(
            "      (ctrl+o to expand)",
            Style::default().fg(Color::DarkGray),
        ))]);
    }

    blocks
}

/// 渲染 Agent 头部汇总行
///
/// 对标 Claude Code 的 AgentProgressLine 第一行格式:
/// `├─ AgentType (description) · N tool uses · Nk tokens`
fn render_agent_header(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: u16,
) -> Vec<Line<'static>> {
    let agent_type = entry.agent_type.as_deref().unwrap_or("agent");
    let description = entry.agent_description.as_deref().unwrap_or("");
    let status = entry.agent_status.as_ref().unwrap_or(&AgentTaskStatus::Running);
    let tool_count = entry.agent_tool_use_count.unwrap_or(0);
    let tokens = entry.agent_tokens.unwrap_or(0);
    let is_error = entry.agent_is_error.unwrap_or(false);
    let last_tool = entry.agent_last_tool_info.as_deref();

    let mut spans = Vec::new();

    // 树形字符（单 Agent 用 ├─）
    spans.push(Span::styled(
        "├─ ",
        Style::default().fg(Color::DarkGray),
    ));

    // Agent 类型（粗体，带颜色）
    let type_color = match status {
        AgentTaskStatus::Running => Color::Yellow,
        AgentTaskStatus::Completed => Color::Green,
        AgentTaskStatus::Failed => Color::Red,
        AgentTaskStatus::Background => Color::DarkGray,
    };
    spans.push(Span::styled(
        agent_type.to_string(),
        Style::default()
            .fg(type_color)
            .add_modifier(Modifier::BOLD),
    ));

    // 描述（灰色括号）
    if !description.is_empty() {
        let max_desc_width = (area_width as usize).saturating_sub(40);
        let truncated_desc = if description.len() > max_desc_width {
            format!(" ({}…)", &description[..max_desc_width.saturating_sub(1)])
        } else {
            format!(" ({})", description)
        };
        spans.push(Span::styled(
            truncated_desc,
            Style::default().fg(Color::DarkGray),
        ));
    }

    // 统计信息：工具使用次数 + Token 使用量
    if tool_count > 0 || tokens > 0 {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        if tool_count > 0 {
            spans.push(Span::styled(
                format!("{} tool {}", tool_count, if tool_count == 1 { "use" } else { "uses" }),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if tokens > 0 {
            if tool_count > 0 {
                spans.push(Span::styled(
                    " · ".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.push(Span::styled(
                format_tokens(tokens),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    vec![Line::from(spans)]
}

/// 渲染完成行
///
/// 对标 Claude Code 的 Done 格式:
/// `│  ⎿  Done (N tool uses · Nk tokens · Xs)`
fn render_done_line(state: &ChatState, entry: &ChatEntry, is_error: bool) -> Vec<Line<'static>> {
    let tool_count = entry.agent_tool_use_count.unwrap_or(0);
    let tokens = entry.agent_tokens.unwrap_or(0);

    let (icon, color, label) = if is_error {
        ("✗", Color::Red, "Failed")
    } else {
        ("✓", Color::Green, "Done")
    };

    let mut spans = vec![
        // 树形前缀
        Span::styled("│  ", Style::default().fg(Color::DarkGray)),
        Span::styled("⎿  ", Style::default().fg(Color::DarkGray)),
        // 状态图标和标签
        Span::styled(format!("{} ", icon), Style::default().fg(color)),
        Span::styled(label.to_string(), Style::default().fg(color)),
    ];

    // 统计信息
    if tool_count > 0 || tokens > 0 {
        spans.push(Span::styled(" (", Style::default().fg(Color::DarkGray)));
        let mut stats = Vec::new();
        if tool_count > 0 {
            stats.push(format!("{} tool {}", tool_count, if tool_count == 1 { "use" } else { "uses" }));
        }
        if tokens > 0 {
            stats.push(format_tokens(tokens));
        }
        // 添加耗时
        if let Some(task_id) = &entry.agent_task_id {
            if let Some(task_info) = state.active_agent_tasks.get(task_id) {
                let elapsed = task_info.started_at.elapsed();
                let dur_str = if elapsed.as_secs() >= 60 {
                    format!("{}m{}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
                } else if elapsed.as_millis() >= 1000 {
                    format!("{:.1}s", elapsed.as_secs_f64())
                } else {
                    format!("{}ms", elapsed.as_millis())
                };
                stats.push(dur_str);
            }
        }
        spans.push(Span::styled(
            stats.join(" · "),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(")", Style::default().fg(Color::DarkGray)));
    }

    vec![Line::from(spans)]
}

/// 精简模式下渲染子条目
///
/// 对标 Claude Code 的 AgentProgressLine 第二行格式:
/// `│  ⎿  ToolName: summary` 或 `│  ⎿  Initializing…`
fn render_sub_entry_condensed(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: u16,
) -> Vec<Line<'static>> {
    use crate::types::ChatEntryType;

    let inner_width = (area_width as usize).saturating_sub(8); // 缩进 + 树形符

    let mut spans = vec![
        // 树形前缀
        Span::styled("│  ", Style::default().fg(Color::DarkGray)),
        Span::styled("⎿  ", Style::default().fg(Color::DarkGray)),
    ];

    match entry.entry_type {
        ChatEntryType::ToolCall => {
            let tool_name = entry
                .tool_call
                .as_ref()
                .map(|tc| tc.function.name.as_str())
                .unwrap_or("unknown");
            let args_preview = truncate_str(
                &entry.tool_call
                    .as_ref()
                    .map(|tc| tc.function.arguments.as_str())
                    .unwrap_or(""),
                inner_width.saturating_sub(tool_name.len() + 3),
            );
            spans.push(Span::styled(
                tool_name.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            if !args_preview.is_empty() {
                spans.push(Span::styled(
                    format!(": {}", args_preview),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        ChatEntryType::ToolResult => {
            let success = entry
                .tool_result
                .as_ref()
                .map(|tr| tr.success)
                .unwrap_or(true);
            let content_preview = truncate_str(&entry.content, inner_width.saturating_sub(2));
            if success {
                spans.push(Span::styled(
                    content_preview,
                    Style::default().fg(Color::Green),
                ));
            } else {
                spans.push(Span::styled(
                    content_preview,
                    Style::default().fg(Color::Red),
                ));
            }
        }
        ChatEntryType::Assistant => {
            let preview = truncate_str(&entry.content, inner_width.saturating_sub(2));
            spans.push(Span::styled(preview, Style::default().fg(Color::White)));
        }
        _ => {
            let preview = truncate_str(&entry.content, inner_width.saturating_sub(2));
            spans.push(Span::styled(preview, Style::default().fg(Color::DarkGray)));
        }
    };

    vec![Line::from(spans)]
}

/// Transcript 模式下渲染完整子消息列表
///
/// 直接使用 tool_render / message_render 的完整渲染，无截断
fn render_sub_entries_verbose(
    state: &ChatState,
    sub_entries: &[ChatEntry],
    area_width: u16,
) -> Vec<Vec<Line<'static>>> {
    let mut all_blocks = Vec::new();
    let inner_width = area_width.saturating_sub(4); // 缩进

    for sub in sub_entries {
        let lines = render_sub_entry_verbose(state, sub, inner_width);
        if !lines.is_empty() {
            all_blocks.push(lines);
        }
    }

    all_blocks
}

/// Transcript 模式下单个子条目的完整渲染
fn render_sub_entry_verbose(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: u16,
) -> Vec<Line<'static>> {
    use crate::types::ChatEntryType;

    let inner_width = (area_width as usize).saturating_sub(4);

    match entry.entry_type {
        ChatEntryType::ToolCall => {
            let tool_name = entry
                .tool_call
                .as_ref()
                .map(|tc| tc.function.name.as_str())
                .unwrap_or("unknown");
            let args = entry
                .tool_call
                .as_ref()
                .map(|tc| tc.function.arguments.as_str())
                .unwrap_or("{}");

            let mut lines = vec![Line::from(vec![
                Span::styled(
                    "  ● ",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    tool_name.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ])];

            // 参数预览（最多显示 3 行）
            let args_lines: Vec<&str> = args.lines().take(3).collect();
            for arg_line in args_lines {
                let truncated = truncate_str(arg_line, inner_width.saturating_sub(4));
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(truncated, Style::default().fg(Color::DarkGray)),
                ]));
            }
            if args.lines().count() > 3 {
                lines.push(Line::from(Span::styled(
                    format!("    ... ({} more lines)", args.lines().count() - 3),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            lines
        }
        ChatEntryType::ToolResult => {
            let success = entry
                .tool_result
                .as_ref()
                .map(|tr| tr.success)
                .unwrap_or(true);
            let icon_color = if success { Color::Green } else { Color::Red };
            let content = &entry.content;

            let mut lines = Vec::new();
            for (i, line) in content.lines().take(10).enumerate() {
                let truncated = truncate_str(line, inner_width.saturating_sub(4));
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::styled("  ⎿ ", Style::default().fg(icon_color)),
                        Span::styled(truncated, Style::default().fg(Color::White)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(truncated, Style::default().fg(Color::White)),
                    ]));
                }
            }
            if content.lines().count() > 10 {
                lines.push(Line::from(Span::styled(
                    format!("    ... ({} more lines)", content.lines().count() - 10),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines
        }
        ChatEntryType::Assistant => {
            // 使用 markdown 解析器渲染内容（支持表格、代码块等）
            let content = &entry.content;
            if content.trim().is_empty() {
                return Vec::new();
            }

            let blocks = crate::utils::markdown_parser::parse_markdown_content(content);
            let md_lines = crate::utils::markdown_parser::render_content_blocks(
                &blocks,
                Some(inner_width),
            );

            // 添加缩进前缀
            let mut lines = Vec::new();
            for md_line in md_lines.into_iter().take(50) {
                // 最多显示 50 行
                let mut prefixed_spans = vec![
                    Span::styled("  ✦ ", Style::default().fg(Color::Blue)),
                ];
                // 将原始 spans 添加缩进
                for span in md_line.spans {
                    prefixed_spans.push(span);
                }
                lines.push(Line::from(prefixed_spans));
            }
            lines
        }
        _ => {
            let content = truncate_str(&entry.content, inner_width.saturating_sub(4));
            if content.is_empty() {
                Vec::new()
            } else {
                vec![Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(content, Style::default().fg(Color::White)),
                ])]
            }
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

/// 截断字符串
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}
