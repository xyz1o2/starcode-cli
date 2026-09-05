/// Agent 任务渲染组件 — 单 Agent 渐进式折叠展示
///
/// 对标 Claude Code 的 `renderToolUseProgressMessage`，实现三级展示：
/// 1. 精简模式 (Condensed): 一行汇总 "In progress... · N tools · tokens"
/// 2. 分组模式 (Grouped): 最后 N 条进度 + "+N more" 提示
/// 3. Verbose/Transcript: 完整子消息列表
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::{AgentTaskStatus, ChatEntry};
use crate::ui::state::ChatState;
use crate::ui::themes::Theme;

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
    let theme = state.theme_manager.current();

    // ── 1. 头部汇总行 ──
    let header = render_agent_header(entry, area_width, theme);
    blocks.push(header);

    // ── 2. 子消息列表 ──
    let sub_entries = entry.agent_sub_entries.as_deref().unwrap_or(&[]);
    let is_resolved = entry.agent_is_resolved.unwrap_or(false);
    let is_async = entry.agent_is_async.unwrap_or(false);

    if is_transcript {
        // Verbose/Transcript 模式：显示全部子消息
        let sub_lines = render_sub_entries_verbose(state, sub_entries, area_width);
        if !sub_lines.is_empty() {
            blocks.extend(sub_lines);
        }
    } else if is_resolved {
        // 已完成：精简一行 "Done"（含耗时）
        blocks.push(render_done_line(state, entry));
    } else if sub_entries.is_empty() {
        // 初始化中
        blocks.push(vec![Line::from(Span::styled(
            "      Initializing…",
            Style::default().fg(theme.inactive),
        ))]);
    } else {
        // 分组模式：先把子条目折叠成“工具活动”行，再取尾部 N 条 + "+N more"。
        // ToolCall 条目不占进度行（结果行会给出同一活动的描述，保留会把
        // "+N more" 按 call+result 双倍计数）；连续相同的活动合并为 ×N。
        let activities = condense_sub_activities(sub_entries);
        let hidden_count = activities
            .len()
            .saturating_sub(MAX_PROGRESS_MESSAGES_TO_SHOW);
        let start = activities
            .len()
            .saturating_sub(MAX_PROGRESS_MESSAGES_TO_SHOW);

        for item in &activities[start..] {
            blocks.push(render_activity_line(state, item, area_width));
        }

        if hidden_count > 0 {
            blocks.push(vec![Line::from(Span::styled(
                format!(
                    "      +{} more tool {}",
                    hidden_count,
                    if hidden_count == 1 { "use" } else { "uses" }
                ),
                Style::default().fg(theme.inactive),
            ))]);
        }
    }

    // ── 3. 尾部提示 ──
    if !is_transcript && !is_async {
        blocks.push(vec![Line::from(Span::styled(
            "      (ctrl+o to expand)",
            Style::default().fg(theme.inactive),
        ))]);
    }

    blocks
}

/// 渲染 Agent 头部汇总行
///
/// 对标 Claude Code 的 AgentProgressLine 第一行格式:
/// `├─ AgentType (description) · N tool uses · Nk tokens`
fn render_agent_header(entry: &ChatEntry, area_width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let agent_type = entry.agent_type.as_deref().unwrap_or("agent");
    let description = entry.agent_description.as_deref().unwrap_or("");
    let status = entry
        .agent_status
        .as_ref()
        .unwrap_or(&AgentTaskStatus::Running);
    let tool_count = entry.agent_tool_use_count.unwrap_or(0);
    let tokens = entry.agent_tokens.unwrap_or(0);
    // teammate 的 `@name` 优先于类型名显示（对标 renderGroupedAgentToolUse）
    let display_name = entry.agent_name.as_deref().unwrap_or(agent_type);

    let mut spans = Vec::new();

    // 树形字符（单 Agent 用 ├─）
    spans.push(Span::styled("├─ ", Style::default().fg(theme.inactive)));

    // Agent 类型（粗体，带颜色）
    let type_color = match status {
        AgentTaskStatus::Running => theme.warning,
        AgentTaskStatus::Completed => theme.success,
        AgentTaskStatus::Failed | AgentTaskStatus::Rejected => theme.error,
        AgentTaskStatus::Background => theme.inactive,
    };
    spans.push(Span::styled(
        display_name.to_string(),
        Style::default().fg(type_color).add_modifier(Modifier::BOLD),
    ));

    // 描述（灰色括号）
    if !description.is_empty() {
        let max_desc_width = (area_width as usize).saturating_sub(40);
        spans.push(Span::styled(
            format!(" ({})", truncate_str(description, max_desc_width)),
            Style::default().fg(theme.inactive),
        ));
    }

    // 统计信息：工具使用次数 + Token 使用量
    if tool_count > 0 || tokens > 0 {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(theme.inactive),
        ));
        if tool_count > 0 {
            spans.push(Span::styled(
                format!(
                    "{} tool {}",
                    tool_count,
                    if tool_count == 1 { "use" } else { "uses" }
                ),
                Style::default().fg(theme.inactive),
            ));
        }
        if tokens > 0 {
            if tool_count > 0 {
                spans.push(Span::styled(
                    " · ".to_string(),
                    Style::default().fg(theme.inactive),
                ));
            }
            spans.push(Span::styled(
                format_tokens(tokens),
                Style::default().fg(theme.inactive),
            ));
        }
    }

    vec![Line::from(spans)]
}

/// 渲染完成行
///
/// 对标 Claude Code 的 Done 格式:
/// `│  ⎿  Done (N tool uses · Nk tokens · Xs)`
fn render_done_line(state: &ChatState, entry: &ChatEntry) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
    let tool_count = entry.agent_tool_use_count.unwrap_or(0);
    let tokens = entry.agent_tokens.unwrap_or(0);
    let status = entry
        .agent_status
        .as_ref()
        .unwrap_or(&AgentTaskStatus::Completed);

    let (icon, color, label) = match status {
        // 用户拒绝授权（对标 renderToolUseRejectedMessage）
        AgentTaskStatus::Rejected => ("✗", theme.error, "Rejected"),
        AgentTaskStatus::Failed => ("✗", theme.error, "Failed"),
        // 后台 agent 已交回控制权，不算"完成"
        AgentTaskStatus::Background => ("●", theme.inactive, "Running in the background"),
        _ if entry.agent_is_error.unwrap_or(false) => ("✗", theme.error, "Failed"),
        _ => ("✓", theme.success, "Done"),
    };

    let mut spans = vec![
        // 树形前缀
        Span::styled("│  ", Style::default().fg(theme.inactive)),
        Span::styled("⎿  ", Style::default().fg(theme.inactive)),
        // 状态图标和标签
        Span::styled(format!("{} ", icon), Style::default().fg(color)),
        Span::styled(label.to_string(), Style::default().fg(color)),
    ];

    // 统计信息
    let mut stats = Vec::new();
    if tool_count > 0 {
        stats.push(format!(
            "{} tool {}",
            tool_count,
            if tool_count == 1 { "use" } else { "uses" }
        ));
    }
    if tokens > 0 {
        stats.push(format_tokens(tokens));
    }
    // 耗时取 AgentTaskInfo 的冻结值，完成后不再增长
    if let Some(info) = entry
        .agent_task_id
        .as_ref()
        .and_then(|id| state.active_agent_tasks.get(id))
    {
        stats.push(super::agent_group_render::format_duration(info.elapsed()));
    }
    if !stats.is_empty() {
        spans.push(Span::styled(" (", Style::default().fg(theme.inactive)));
        spans.push(Span::styled(
            stats.join(" · "),
            Style::default().fg(theme.inactive),
        ));
        spans.push(Span::styled(")", Style::default().fg(theme.inactive)));
    }

    vec![Line::from(spans)]
}

/// 折叠后的工具活动行（分组进度条目）
struct SubActivity {
    text: String,
    success: bool,
    is_assistant: bool,
    repeats: usize,
}

/// 把子条目折叠成活动列表（对标参考实现：进度行只描述“做了什么”，不倾倒结果内容）：
/// - ToolCall 条目跳过 —— 结果行携带同一活动的描述，保留会把 "+N more" 双倍计数
/// - ToolResult → `describe_tool_use` 的 "Read: b.txt" 式活动文案
/// - Assistant → 首行文本
/// - 连续相同的活动合并为一行，计 `repeats`
fn condense_sub_activities(sub_entries: &[ChatEntry]) -> Vec<SubActivity> {
    use crate::types::ChatEntryType;

    let mut items: Vec<SubActivity> = Vec::new();
    for entry in sub_entries {
        let text: String = match entry.entry_type {
            ChatEntryType::ToolResult => entry
                .tool_call
                .as_ref()
                .map(|tc| {
                    crate::agent::subagent::progress::describe_tool_use(
                        &tc.function.name,
                        &tc.function.arguments,
                    )
                })
                .unwrap_or_else(|| first_line_of(&entry.content)),
            ChatEntryType::Assistant => first_line_of(&entry.content),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let success = entry
            .tool_result
            .as_ref()
            .map(|tr| tr.success)
            .unwrap_or(true);
        let is_assistant = entry.entry_type == ChatEntryType::Assistant;
        // 连续重复的活动（如同一个文件被连续读三次）合并为 "text ×N"
        if let Some(last) = items.last_mut() {
            if last.text == text && last.success == success && last.is_assistant == is_assistant {
                last.repeats += 1;
                continue;
            }
        }
        items.push(SubActivity {
            text,
            success,
            is_assistant,
            repeats: 1,
        });
    }
    items
}

fn first_line_of(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// 渲染一条活动行：`│  ⎿  Read: b.txt ×3`
fn render_activity_line(
    state: &ChatState,
    item: &SubActivity,
    area_width: u16,
) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
    let inner_width = (area_width as usize).saturating_sub(8); // 缩进 + 树形符

    let suffix = if item.repeats > 1 {
        format!(" ×{}", item.repeats)
    } else {
        String::new()
    };
    let budget = inner_width
        .saturating_sub(2)
        .saturating_sub(suffix.chars().count());
    let text = format!("{}{}", truncate_str(&item.text, budget), suffix);

    let color = if item.is_assistant {
        theme.foreground
    } else if item.success {
        theme.success
    } else {
        theme.error
    };

    vec![Line::from(vec![
        Span::styled("│  ", Style::default().fg(theme.inactive)),
        Span::styled("⎿  ", Style::default().fg(theme.inactive)),
        Span::styled(text, Style::default().fg(color)),
    ])]
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

    let theme = state.theme_manager.current();
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
                Span::styled("  ● ", Style::default().fg(theme.info)),
                Span::styled(
                    tool_name.to_string(),
                    Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
                ),
            ])];

            // 参数预览（最多显示 3 行）
            let args_lines: Vec<&str> = args.lines().take(3).collect();
            for arg_line in args_lines {
                let truncated = truncate_str(arg_line, inner_width.saturating_sub(4));
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(truncated, Style::default().fg(theme.inactive)),
                ]));
            }
            if args.lines().count() > 3 {
                lines.push(Line::from(Span::styled(
                    format!("    ... ({} more lines)", args.lines().count() - 3),
                    Style::default().fg(theme.inactive),
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
            let icon_color = if success { theme.success } else { theme.error };
            let content = &entry.content;

            let mut lines = Vec::new();
            for (i, line) in content.lines().take(10).enumerate() {
                let truncated = truncate_str(line, inner_width.saturating_sub(4));
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::styled("  ⎿ ", Style::default().fg(icon_color)),
                        Span::styled(truncated, Style::default().fg(theme.foreground)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(truncated, Style::default().fg(theme.foreground)),
                    ]));
                }
            }
            if content.lines().count() > 10 {
                lines.push(Line::from(Span::styled(
                    format!("    ... ({} more lines)", content.lines().count() - 10),
                    Style::default().fg(theme.inactive),
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
            let md_lines =
                crate::utils::markdown_parser::render_content_blocks(&blocks, Some(inner_width));

            // 添加缩进前缀
            let mut lines = Vec::new();
            for md_line in md_lines.into_iter().take(50) {
                // 最多显示 50 行
                let mut prefixed_spans =
                    vec![Span::styled("  ✦ ", Style::default().fg(theme.agent_blue))];
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
                    Span::styled(content, Style::default().fg(theme.foreground)),
                ])]
            }
        }
    }
}

/// 格式化 token 数量
fn format_tokens(count: u32) -> String {
    let scaled = if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    };
    format!("{} tokens", scaled)
}

/// 截断字符串到 `max_len` 个终端显示单元。
///
/// 原实现是 `&s[..max_len]` 这类**字节**切片。子代理的 description 与工具参数都由
/// 模型生成，中文/emoji 一律会把切点落在多字节字符中间，直接 panic 掉整个 TUI。
/// 现在统一走 CJK 宽度感知的公共实现，顺带修正宽字符的对齐。
fn truncate_str(s: &str, max_len: usize) -> String {
    crate::ui::utils::render::truncate_to_display_width(s, max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：中文描述 + 任意窄宽度都不能 panic（原字节切片实现必崩）
    #[test]
    fn truncate_str_survives_multibyte_at_every_width() {
        let cjk = "研究 src/hooks 目录下的组件实现";
        for width in 0..=cjk.len() + 4 {
            let out = truncate_str(cjk, width);
            // 只要不 panic 即通过；顺带确认输出仍是合法 UTF-8 的前缀语义
            assert!(out.chars().count() <= cjk.chars().count() + 3);
        }
        let emoji = "✅ done 🎉🎉🎉";
        for width in 0..=emoji.len() + 4 {
            let _ = truncate_str(emoji, width);
        }
    }

    #[test]
    fn truncate_str_keeps_short_input_intact() {
        assert_eq!(truncate_str("abc", 10), "abc");
        assert_eq!(truncate_str("中文", 10), "中文");
    }

    fn progress_tool_call(name: &str, args: &str) -> crate::types::StarToolCall {
        crate::types::StarToolCall {
            id: format!("{name}-1"),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: name.to_string(),
                arguments: args.to_string(),
            },
        }
    }

    /// 同一工具连读三次 → 一行活动 ×3；ToolCall 条目不产生活动行
    #[test]
    fn condense_merges_duplicates_and_skips_tool_calls() {
        let tc = progress_tool_call("Read", r#"{"file_path":"/a/b.txt"}"#);
        let mut subs = Vec::new();
        subs.push(ChatEntry::tool_call(String::new(), tc.clone()));
        for _ in 0..3 {
            subs.push(ChatEntry::tool_result(
                "import x",
                tc.clone(),
                crate::types::ToolResult {
                    success: true,
                    output: Some("import x".to_string()),
                    error: None,
                    data: None,
                },
            ));
        }
        let items = condense_sub_activities(&subs);
        assert_eq!(items.len(), 1, "call 条目被跳过，重复活动合并");
        assert_eq!(items[0].repeats, 3);
        assert!(items[0].text.starts_with("Read:"));
    }

    /// 不同活动不合并，失败结果标记 success=false
    #[test]
    fn condense_keeps_distinct_activities() {
        let mut subs = Vec::new();
        for (name, args, ok) in [
            ("Read", r#"{"file_path":"/a/b.txt"}"#, true),
            ("Bash", r#"{"command":"cargo test"}"#, true),
            ("Bash", r#"{"command":"cargo test"}"#, false),
        ] {
            let tc = progress_tool_call(name, args);
            subs.push(ChatEntry::tool_result(
                "out",
                tc,
                crate::types::ToolResult {
                    success: ok,
                    output: Some("out".to_string()),
                    error: None,
                    data: None,
                },
            ));
        }
        let items = condense_sub_activities(&subs);
        assert_eq!(items.len(), 3);
        assert!(items[0].text.starts_with("Read:"));
        assert!(items[1].text.starts_with("Bash:"));
        assert!(!items[2].success);
    }
}
