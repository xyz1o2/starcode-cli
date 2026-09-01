use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use crate::types::{ChatEntry, ChatEntryType, EntryStatus};
use crate::ui::state::ChatState;

/// 折叠组的最大预览行数
const MAX_PREVIEW_LINES: usize = 3;

/// 折叠组的最大摘要长度
const MAX_SUMMARY_LENGTH: usize = 80;

/// 渲染折叠组
pub fn render_collapsed_group(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: usize,
) -> Vec<Line<'static>> {
    let is_expanded = entry.is_collapsed == Some(false);
    let entries = entry.collapsed_entries.as_ref();
    let summary = entry.collapse_summary.as_ref();
    
    if is_expanded {
        // 展开状态：渲染所有子条目
        render_expanded_group(state, entry, area_width)
    } else {
        // 折叠状态：显示摘要
        render_collapsed_summary(state, entry, area_width)
    }
}

/// 渲染折叠状态的摘要
fn render_collapsed_summary(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let entries = entry.collapsed_entries.as_ref();
    let summary = entry.collapse_summary.as_ref();
    
    // 计算子条目数量
    let entry_count = entries.map(|e| e.len()).unwrap_or(0);
    
    // 获取摘要文本
    let summary_text = summary
        .map(|s| truncate_str(s, MAX_SUMMARY_LENGTH))
        .unwrap_or_else(|| {
            // 自动生成摘要
            if let Some(entries) = entries {
                generate_summary(entries)
            } else {
                "No content".to_string()
            }
        });
    
    // 统计不同类型的消息
    let (tool_calls, errors, successes) = count_entry_types(entries.unwrap_or(&Vec::new()));
    
    // 构建状态指示器
    let status_indicators = build_status_indicators(tool_calls, errors, successes);
    
    // 渲染折叠行
    let line = Line::from(vec![
        // 展开/折叠图标
        Span::styled(
            "▸ ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        // 摘要文本
        Span::styled(
            summary_text,
            Style::default().fg(Color::Gray),
        ),
        // 状态指示器
        if !status_indicators.is_empty() {
            Span::styled(
                format!(" {}", status_indicators),
                Style::default().fg(Color::DarkGray),
            )
        } else {
            Span::raw("")
        },
        // 条目数量
        Span::styled(
            format!(" ({} items)", entry_count),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    
    lines.push(line);
    
    // 如果有错误，显示错误摘要
    if errors > 0 {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("⚠ {} error(s) occurred", errors),
                Style::default().fg(Color::Red),
            ),
        ]));
    }
    
    lines
}

/// 渲染展开状态的所有子条目
fn render_expanded_group(
    state: &ChatState,
    entry: &ChatEntry,
    area_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    // 折叠组标题（可点击折叠）
    let title_line = Line::from(vec![
        Span::styled(
            "▾ ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Collapse",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    lines.push(title_line);
    
    // 渲染所有子条目
    if let Some(entries) = &entry.collapsed_entries {
        for (i, sub_entry) in entries.iter().enumerate() {
            // 为每个子条目添加缩进
            let sub_lines = render_sub_entry(state, sub_entry, area_width.saturating_sub(2));
            
            for (j, line) in sub_lines.into_iter().enumerate() {
                let mut indented_spans = vec![
                    Span::styled("  ", Style::default()),
                ];
                indented_spans.extend(line.spans);
                lines.push(Line::from(indented_spans));
            }
            
            // 在子条目之间添加分隔线
            if i < entries.len() - 1 {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        "─".repeat(area_width.saturating_sub(4)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }
    
    lines
}

/// 渲染单个子条目
fn render_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    match entry.entry_type {
        ChatEntryType::ToolCall => render_tool_call_sub_entry(state, entry, width),
        ChatEntryType::ToolResult => render_tool_result_sub_entry(state, entry, width),
        ChatEntryType::Assistant => render_assistant_sub_entry(state, entry, width),
        ChatEntryType::User => render_user_sub_entry(state, entry, width),
        _ => render_generic_sub_entry(state, entry, width),
    }
}

/// 渲染工具调用子条目
fn render_tool_call_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    if let Some(tc) = &entry.tool_call {
        // 工具名称
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::Blue)),
            Span::styled(
                tc.function.name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        
        // 参数预览
        let args_preview = truncate_str(&tc.function.arguments, width.saturating_sub(4));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(args_preview, Style::default().fg(Color::DarkGray)),
        ]));
    }
    
    lines
}

/// 渲染工具结果子条目
fn render_tool_result_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    let (icon, color) = if let Some(tr) = &entry.tool_result {
        if tr.success {
            ("⎿", Color::Green)
        } else {
            ("⎿", Color::Red)
        }
    } else {
        ("⎿", Color::Yellow)
    };
    
    // 结果内容预览
    let content_preview = truncate_str(&entry.content, width.saturating_sub(4));
    lines.push(Line::from(vec![
        Span::styled(icon, Style::default().fg(color)),
        Span::styled(" ", Style::default()),
        Span::styled(content_preview, Style::default().fg(Color::White)),
    ]));
    
    lines
}

/// 渲染助手消息子条目
fn render_assistant_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    // 助手消息预览
    let content_preview = truncate_str(&entry.content, width.saturating_sub(4));
    if !content_preview.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("✦ ", Style::default().fg(Color::Blue)),
            Span::styled(content_preview, Style::default().fg(Color::White)),
        ]));
    }
    
    lines
}

/// 渲染用户消息子条目
fn render_user_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    // 用户消息预览
    let content_preview = truncate_str(&entry.content, width.saturating_sub(4));
    if !content_preview.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("❯ ", Style::default().fg(Color::Blue)),
            Span::styled(content_preview, Style::default().fg(Color::White)),
        ]));
    }
    
    lines
}

/// 渲染通用子条目
fn render_generic_sub_entry(
    state: &ChatState,
    entry: &ChatEntry,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    let content_preview = truncate_str(&entry.content, width.saturating_sub(4));
    if !content_preview.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(content_preview, Style::default().fg(Color::White)),
        ]));
    }
    
    lines
}

/// 生成摘要文本
fn generate_summary(entries: &[ChatEntry]) -> String {
    let mut summary_parts = Vec::new();
    
    // 统计不同类型
    let (tool_calls, errors, successes) = count_entry_types(entries);
    
    if tool_calls > 0 {
        summary_parts.push(format!("{} tool call(s)", tool_calls));
    }
    
    if errors > 0 {
        summary_parts.push(format!("{} error(s)", errors));
    }
    
    if successes > 0 {
        summary_parts.push(format!("{} success", successes));
    }
    
    if summary_parts.is_empty() {
        format!("{} message(s)", entries.len())
    } else {
        summary_parts.join(", ")
    }
}

/// 统计消息类型
fn count_entry_types(entries: &[ChatEntry]) -> (usize, usize, usize) {
    let mut tool_calls = 0;
    let mut errors = 0;
    let mut successes = 0;
    
    for entry in entries {
        match entry.entry_type {
            ChatEntryType::ToolCall => tool_calls += 1,
            ChatEntryType::ToolResult => {
                if let Some(tr) = &entry.tool_result {
                    if tr.success {
                        successes += 1;
                    } else {
                        errors += 1;
                    }
                }
            }
            ChatEntryType::ErrorMessage => errors += 1,
            _ => {}
        }
    }
    
    (tool_calls, errors, successes)
}

/// 构建状态指示器
fn build_status_indicators(tool_calls: usize, errors: usize, successes: usize) -> String {
    let mut indicators = Vec::new();
    
    if tool_calls > 0 {
        indicators.push(format!("{} tools", tool_calls));
    }
    
    if successes > 0 {
        indicators.push(format!("{} ✓", successes));
    }
    
    if errors > 0 {
        indicators.push(format!("{} ✗", errors));
    }
    
    indicators.join(" · ")
}

/// 截断字符串
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// 折叠组的切换逻辑
pub fn toggle_collapsed_group(entry: &mut ChatEntry) {
    if let Some(is_collapsed) = entry.is_collapsed {
        entry.is_collapsed = Some(!is_collapsed);
    }
}

/// 创建折叠组
pub fn create_collapsed_group(
    entries: Vec<ChatEntry>,
    summary: impl Into<String>,
) -> ChatEntry {
    ChatEntry::collapsed_group(entries, summary)
}

/// 自动折叠连续的工具调用
pub fn auto_collapse_tool_calls(entries: &[ChatEntry]) -> Vec<ChatEntry> {
    let mut result = Vec::new();
    let mut current_group: Vec<ChatEntry> = Vec::new();
    
    for entry in entries {
        if entry.entry_type == ChatEntryType::ToolCall || entry.entry_type == ChatEntryType::ToolResult {
            current_group.push(entry.clone());
        } else {
            // 如果有累积的工具调用，创建折叠组
            if current_group.len() >= 3 {
                let summary = generate_summary(&current_group);
                result.push(create_collapsed_group(current_group, summary));
            } else {
                result.extend(current_group);
            }
            current_group = Vec::new();
            result.push(entry.clone());
        }
    }
    
    // 处理最后一组
    if current_group.len() >= 3 {
        let summary = generate_summary(&current_group);
        result.push(create_collapsed_group(current_group, summary));
    } else {
        result.extend(current_group);
    }
    
    result
}
