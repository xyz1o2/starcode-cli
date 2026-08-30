use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::{ChatEntryType, EntryStatus};
use crate::ui::state::ChatState;
use crate::ui::themes::theme::Theme;

pub(crate) fn is_tool_entry(entry: &crate::types::ChatEntry) -> bool {
    entry.entry_type == ChatEntryType::ToolCall
        || entry.entry_type == ChatEntryType::ToolResult
        || entry.entry_type == ChatEntryType::ToolConfirmation
}

/// 工具状态图标
pub fn get_tool_status_icon(entry: &crate::types::ChatEntry, theme: &Theme) -> (&'static str, Color) {
    match &entry.status {
        Some(EntryStatus::Success) => ("✓", theme.tool_success),
        Some(EntryStatus::Error) => ("✗", theme.tool_error),
        Some(EntryStatus::Warning) => ("⚠", theme.warning),
        Some(EntryStatus::Cancelled) => ("⊘", theme.inactive),
        Some(EntryStatus::InProgress) => ("●", theme.primary),
        Some(EntryStatus::Pending) => ("○", theme.inactive), // 排队中 — dim
        _ => {
            // 根据条目类型和结果判断状态
            if entry.entry_type == ChatEntryType::ToolResult {
                if let Some(tr) = &entry.tool_result {
                    if tr.success {
                        ("✓", theme.tool_success)
                    } else {
                        ("✗", theme.tool_error)
                    }
                } else {
                    ("⎿", theme.warning)
                }
            } else if entry.entry_type == ChatEntryType::ToolCall {
                if entry.is_streaming == Some(true) {
                    ("⎿", theme.primary)
                } else {
                    ("⎿", theme.primary)
                }
            } else {
                ("", theme.foreground)
            }
        }
    }
}

/// 工具名称颜色 — 使用热粉色边框色
pub fn get_tool_name_color(entry: &crate::types::ChatEntry, theme: &Theme) -> Color {
    if entry.entry_type == ChatEntryType::ToolResult {
        if let Some(tr) = &entry.tool_result {
            if tr.success {
                theme.tool_success
            } else {
                theme.tool_error
            }
        } else {
            theme.warning
        }
    } else {
        if entry.is_streaming == Some(true) {
            theme.warning
        } else {
            theme.primary
        }
    }
}

pub(crate) const MAX_CONFIRMATION_CARD_WRAP_WIDTH: usize = 96;

/// 工具结果预览最大行数（类似 codex 的 TOOL_CALL_MAX_LINES=5，这里更宽松一些）
/// 折叠态下所有工具输出统一截断到此行数
pub(crate) const TOOL_RESULT_PREVIEW_LINES: usize = 8;

pub(crate) fn confirmation_card_wrap_width(area_width: u16) -> usize {
    (area_width as usize)
        .saturating_sub(4)
        .min(MAX_CONFIRMATION_CARD_WRAP_WIDTH)
}

/// 对工具输出行应用预览行数限制，超出部分添加溢出提示行
/// 返回 (截断后的行, 被隐藏的行数)
fn apply_preview_limit(
    lines: &mut Vec<Line<'static>>,
    max_lines: usize,
    theme: &Theme,
) -> usize {
    let total = lines.len();
    if total > max_lines {
        let hidden = total.saturating_sub(max_lines);
        lines.truncate(max_lines);
        lines.push(Line::from(Span::styled(
            format!("... ({} more lines)", hidden),
            Style::default().fg(theme.subtle),
        )));
        hidden
    } else {
        0
    }
}

pub(crate) fn render_tool_entry_blocks(
    state: &ChatState,
    entry: &crate::types::ChatEntry,
    _entry_idx: usize,
    area_width: u16,
    _prev_is_tool: bool,
    _next_is_tool: bool,
    _prev_is_confirmation: bool,
) -> Vec<Vec<Line<'static>>> {
    let theme = state.theme_manager.current();
    let mut blocks = Vec::new();
    let is_tool_result = entry.entry_type == ChatEntryType::ToolResult;
    let tool_box_width = area_width as usize;
    // 统一使用 5 字符前缀宽度，确保 ToolCall 和 ToolResult 对齐
    // "⎿ " (2) + 3空格缩进 = 5 字符
    let prefix_width: usize = 5;
    let tool_inner_width = tool_box_width.saturating_sub(prefix_width);

    let expanded = entry
        .tool_call
        .as_ref()
        .map(|tc| {
            // ToolCall 默认展开；ToolResult 有 diff 时也默认展开
            let has_diff = entry.tool_result.as_ref()
                .and_then(|tr| tr.data.as_ref())
                .and_then(|d| d.get("diff"))
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let default_expanded = matches!(entry.entry_type, ChatEntryType::ToolCall)
                || (matches!(entry.entry_type, ChatEntryType::ToolResult) && has_diff);
            let toggled = state.expanded_tool_call_ids.contains(&tc.id);
            default_expanded ^ toggled
        })
        .unwrap_or(true);

    // ToolCall: 只显示工具名称和参数摘要（单行）
    // ToolResult: 显示结果内容
    // ToolConfirmation: 显示确认卡片
    let content_lines = if entry.entry_type == ChatEntryType::ToolConfirmation {
        if let Some(ref conf) = entry.confirmation {
            if matches!(conf.operation_type, crate::types::ConfirmationType::AskUserQuestion) {
                crate::ui::components::confirmation_dialog::build_ask_user_question_card(
                    conf,
                    confirmation_card_wrap_width(area_width),
                    state.pending_confirmation_choice,
                    &state.pending_question_selections,
                    &state.pending_other_input,
                )
            } else {
                crate::ui::components::confirmation_dialog::build_confirmation_card_block(
                    conf,
                    confirmation_card_wrap_width(area_width),
                    state.pending_confirmation_choice,
                    state.show_permission_explanation,
                    state.show_permission_debug,
                )
            }
        } else {
            vec![Line::from(Span::styled(
                "Error: confirmation data missing",
                Style::default().fg(Color::Red),
            ))]
        }
    } else if entry.entry_type == ChatEntryType::ToolCall {
        // ToolCall: 只显示工具名称和参数摘要
        if let Some(tc) = &entry.tool_call {
            let (summary, _extra_info) = build_tool_argument_display(tc, state.ui_verbose);
            let tool_name = crate::ui::utils::format::tool_display_name(tc.function.name.as_str());
            let short_summary = if !state.ui_verbose && summary.chars().count() > 60 {
                format!("{}...", summary.chars().take(57).collect::<String>())
            } else {
                summary
            };
            vec![Line::from(vec![
                Span::styled(tool_name, Style::default().add_modifier(Modifier::BOLD).fg(theme.primary)),
                Span::raw(" "),
                Span::styled(short_summary, Style::default().fg(Color::Gray)),
            ])]
        } else {
            vec![]
        }
    } else if entry.entry_type == ChatEntryType::ToolResult {
        // ToolResult: 显示结果内容
        if let Some(tc) = &entry.tool_call {
            render_rich_tool_content(entry, tc, tool_inner_width, expanded, _prev_is_confirmation, theme)
        } else if !entry.content.trim().is_empty() {
            crate::ui::utils::render::build_tool_body_block(
                &entry.content,
                tool_inner_width,
                expanded,
            )
        } else {
            vec![]
        }
    } else {
        crate::ui::utils::render::build_tool_body_block(
            &entry.content,
            tool_inner_width,
            expanded,
        )
    };

    let mut boxed_lines = Vec::new();
    let is_streaming = entry.is_streaming == Some(true);
    let cancelling = state.cancelling_since.is_some() && is_streaming;
    // 使用 600ms 间隔闪烁，和 Claude Code 一致
    // animation_tick 每帧递增（约 30fps），600ms ≈ 18 帧
    let blink_visible = (state.animation_tick / 18) % 2 == 0;

    // ToolCall: ● (闪烁) + ToolName (args)
    // ToolResult: 无标记，直接显示结果内容
    if entry.entry_type == ChatEntryType::ToolCall {
        // ToolCall 行：圆点 + 工具名称 + 参数
        let marker = if cancelling {
            "⊘"
        } else if is_streaming {
            if blink_visible { "●" } else { " " }
        } else {
            "●"  // 完成后也显示圆点，但颜色不同
        };
        let marker_color = if cancelling {
            theme.warning
        } else if is_streaming {
            theme.primary
        } else {
            theme.tool_success  // 完成后用绿色
        };

        // 第一行：圆点 + 工具信息（在 content_lines 中已经渲染好了）
        // 这里只需要添加前缀
        for (i, line) in content_lines.into_iter().enumerate() {
            let line_is_blank = line.spans.iter().all(|s| s.content.trim().is_empty());
            if line_is_blank {
                boxed_lines.push(Line::from(""));
                continue;
            }

            let mut spans = Vec::new();
            if i == 0 {
                // 首行：● + 空格
                spans.push(Span::styled(
                    format!("{} ", marker),
                    Style::default()
                        .fg(marker_color)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                // 续行：2空格缩进
                spans.push(Span::raw("  "));
            }
            spans.extend(line.spans);
            boxed_lines.push(Line::from(spans));
        }
    } else if entry.entry_type == ChatEntryType::ToolResult {
        // ToolResult: 无标记，直接显示结果内容
        for (i, line) in content_lines.into_iter().enumerate() {
            let line_is_blank = line.spans.iter().all(|s| s.content.trim().is_empty());
            if line_is_blank {
                boxed_lines.push(Line::from(""));
                continue;
            }

            let mut spans = Vec::new();
            if i == 0 {
                // 首行：2空格缩进（与 ToolCall 的内容对齐）
                spans.push(Span::raw("  "));
            }
            // 续行无缩进，保持内容原始格式
            spans.extend(line.spans);
            boxed_lines.push(Line::from(spans));
        }
    } else {
        // 其他类型（ToolConfirmation 等）
        for line in content_lines.into_iter() {
            boxed_lines.push(line);
        }
    }

    if !boxed_lines.is_empty() {
        blocks.push(boxed_lines);
    }

    blocks
}


fn truncate_chars_with_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn shorten_path_for_display(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let cwd = crate::core::utils::paths::current_dir_cached();
    let p = std::path::Path::new(path);
    if let Ok(rel) = p.strip_prefix(&cwd) {
        rel.to_string_lossy().to_string()
    } else {
        path.to_string()
    }
}

fn summarize_arg_value(v: &serde_json::Value, max_chars: usize) -> String {
    match v {
        serde_json::Value::String(s) => truncate_chars_with_ellipsis(s, max_chars),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else {
                format!("[{} items]", arr.len())
            }
        }
        serde_json::Value::Object(obj) => format!("{{{} fields}}", obj.len()),
    }
}

fn summarize_object_inline(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut parts = Vec::new();
    for (k, v) in obj.iter().take(3) {
        parts.push(format!("{}={}", k, summarize_arg_value(v, 40)));
    }
    if obj.len() > 3 {
        parts.push(format!("+{} items", obj.len() - 3));
    }
    parts.join(", ")
}

fn build_tool_argument_display(
    tc: &crate::types::StarToolCall,
    verbose: bool,
) -> (String, Vec<String>) {
    // verbose 模式不截断、不缩短路径（对标 Claude Code verbose 显示）
    let lim = |n: usize| if verbose { usize::MAX } else { n };
    let shorten = |p: &str| if verbose { p.to_string() } else { shorten_path_for_display(p) };
    let args = match serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
        Ok(v) => v,
        Err(_) => {
            let raw = tc.function.arguments.trim();
            return (truncate_chars_with_ellipsis(raw, lim(140)), Vec::new());
        }
    };

    let obj = if let Some(o) = args.as_object() {
        o
    } else {
        return (summarize_arg_value(&args, 140), Vec::new());
    };

    let get_str = |keys: &[&str]| -> Option<String> {
        for k in keys {
            if let Some(v) = obj.get(*k).and_then(|x| x.as_str()) {
                return Some(v.to_string());
            }
        }
        None
    };
    let get_u64 = |keys: &[&str]| -> Option<u64> {
        for k in keys {
            if let Some(v) = obj.get(*k).and_then(|x| x.as_u64()) {
                return Some(v);
            }
        }
        None
    };

    // summary: 头部显示的简短摘要
    // extra_info: 展开时显示的额外信息（summary 中未包含的）
    let mut extra_info: Vec<String> = Vec::new();

    let summary = match tc.function.name.as_str() {
        "enter_plan_mode" => {
            let reason = get_str(&["reason"]).unwrap_or_else(|| "No reason provided".to_string());
            extra_info.push(reason);
            "Enter plan mode".to_string()
        }
        "exit_plan_mode" => {
            let plan = get_str(&["plan"]).unwrap_or_default();
            if !plan.trim().is_empty() {
                extra_info.push(truncate_chars_with_ellipsis(&plan, lim(200)));
            }
            "Exit plan mode".to_string()
        }
        "view_file" | "Read" => {
            let path = get_str(&["path", "file_path", "target_file"])
                .map(|p| shorten(&p))
                .unwrap_or_else(|| "(no path provided)".to_string());
            let range = if let (Some(s), Some(e)) =
                (get_u64(&["start_line"]), get_u64(&["end_line"]))
            {
                format!(" [{}-{}]", s, e)
            } else if let (Some(off), Some(limit)) = (get_u64(&["offset"]), get_u64(&["limit"])) {
                format!(" [offset={}, limit={}]", off, limit)
            } else {
                String::new()
            };
            format!("{}{}", path, range)
        }
        "Bash" => {
            let cmd = get_str(&["command", "CommandLine"]).unwrap_or_default();
            let dir = get_str(&["dir_path", "working_dir"])
                .map(|p| shorten(&p))
                .unwrap_or_default();
            if !dir.is_empty() {
                extra_info.push(format!("directory: {}", dir));
            }
            truncate_chars_with_ellipsis(&cmd, lim(180))
        }
        "create_file" | "edit_file" | "Edit" | "str_replace_editor" | "smart_edit" => {
            let path = get_str(&["path", "target_file", "file_path"])
                .map(|p| shorten(&p))
                .unwrap_or_else(|| "(no path provided)".to_string());
            if let Some(rep) = obj.get("replace_all").and_then(|x| x.as_bool()) {
                extra_info.push(format!("replace_all: {}", rep));
            }
            path
        }
        "Grep" | "search_file_content" | "grep_search" => {
            let q = get_str(&["query", "Query", "pattern", "Pattern"]).unwrap_or_default();
            let p = get_str(&["path", "Path", "SearchPath", "glob", "include_pattern"])
                .unwrap_or_default();
            if p.is_empty() {
                q
            } else {
                format!("{} in {}", q, shorten(&p))
            }
        }
        "Todo" => {
            let action = crate::ui::utils::format::canonical_task_action_from_value(&args);
            let action_label = crate::ui::utils::format::task_action_display_label(action);
            if let Some(todos) = obj.get("todos").and_then(|v| v.as_array()) {
                extra_info.push(format!("{} tasks", todos.len()));
            }
            if let Some(updates) = obj.get("updates").and_then(|v| v.as_array()) {
                extra_info.push(format!("{} updates", updates.len()));
            }
            action_label.to_string()
        }
        "complete_task" => {
            let first = get_str(&["result"])
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            truncate_chars_with_ellipsis(&first, lim(140))
        }
        "find_by_name" => {
            let pat = get_str(&["Pattern"]).unwrap_or_default();
            let dir = get_str(&["SearchDirectory"]).unwrap_or_default();
            if dir.is_empty() {
                pat
            } else {
                format!("{} in {}", pat, shorten(&dir))
            }
        }
        "list_directory" | "ListDir" => {
            get_str(&["directory", "path"])
                .map(|p| shorten(&p))
                .unwrap_or_else(|| ".".to_string())
        }
        _ => summarize_object_inline(obj),
    };

    (summary, extra_info)
}

fn render_rich_tool_content(
    entry: &crate::types::ChatEntry,
    tc: &crate::types::StarToolCall,
    tool_inner_width: usize,
    expanded: bool,
    _prev_is_confirmation: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // 此函数只处理 ToolResult
    let tool_color = if let Some(tr) = &entry.tool_result {
        if tr.success { theme.tool_success } else { theme.tool_error }
    } else {
        theme.warning
    };

    let _tool_name = crate::ui::utils::format::tool_display_name(tc.function.name.as_str());

    // ===== ToolResult 路径 =====
    if let Some(tr) = &entry.tool_result {
        let diff_str = tr.data.as_ref()
            .and_then(|d| d.get("diff").and_then(|v| v.as_str()));
        let has_diff = diff_str.is_some() && !diff_str.unwrap_or("").trim().is_empty();

        // 编辑工具：折叠态显示 +N/-M，展开态显示完整 diff
        if has_diff {
            let diff_content = diff_str.unwrap();
            if !expanded {
                let mut added = 0usize;
                let mut removed = 0usize;
                for line in diff_content.lines() {
                    if line.starts_with('+') && !line.starts_with("+++") { added += 1; }
                    else if line.starts_with('-') && !line.starts_with("---") { removed += 1; }
                }
                lines.push(Line::from(vec![
                    Span::raw("⎿ Added "),
                    Span::styled(format!("{}", added), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw(" lines, removed "),
                    Span::styled(format!("{}", removed), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw(" lines"),
                ]));
            } else {
                lines.extend(crate::ui::utils::render::build_diff_block(diff_content, tool_inner_width));
            }
            return lines;
        }

        // 非 diff 工具：统一的折叠/展开渲染
        let text = if tr.success {
            tr.output.as_deref().unwrap_or("")
        } else {
            tr.error.as_deref().unwrap_or("")
        };
        render_tool_result_text(&mut lines, text, tr.success, tool_inner_width, expanded);
    } else {
        // 回退：tool_result 字段缺失时（如会话恢复），使用 entry.content 渲染
        if !entry.content.trim().is_empty() {
            render_tool_result_text(&mut lines, &entry.content, true, tool_inner_width, expanded);
        }
    }

    lines
}

/// ToolResult 文本渲染的公共函数（折叠态：第一行预览 + 行数；展开态：完整内容）
fn render_tool_result_text(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    success: bool,
    width: usize,
    expanded: bool,
) {
    let total = text.lines().count();

    if !expanded {
        // 折叠态：预览前几行真实内容 + "Tab 展开"提示（而非只有行数）。
        // 之前只显示第一行 + N lines，用户看不到任何正文，误以为输出被截断/不显示。
        let mut shown = 0usize;
        for raw_line in text.lines() {
            if shown >= TOOL_RESULT_PREVIEW_LINES {
                break;
            }
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let clean = crate::ui::utils::render::strip_ansi_codes(line);
            let preview = if clean.chars().count() > width {
                let truncated: String = clean.chars().take(width.saturating_sub(3)).collect();
                format!("{}...", truncated)
            } else {
                clean
            };
            if !preview.is_empty() {
                lines.push(Line::from(Span::styled(
                    preview,
                    Style::default().fg(Color::DarkGray),
                )));
                shown += 1;
            }
        }
        // 折叠提示行（内容更多时给出明确指引）
        if total > shown.max(1) {
            let hidden = total.saturating_sub(shown);
            lines.push(Line::from(Span::styled(
                format!("... +{} lines  (press Tab to expand)", hidden),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )));
        }
        return;
    }

    // 展开态：完整内容
    let result_text = crate::ui::utils::text::sanitize_for_tui(text);
    if result_text.trim().is_empty() {
        return;
    }

    if !success {
        for line in result_text.lines() {
            let stripped = crate::ui::utils::render::strip_ansi_codes(line);
            lines.push(Line::from(vec![
                Span::styled(stripped, Style::default().fg(Color::Red)),
            ]));
        }
    } else {
        lines.extend(crate::ui::utils::render::build_tool_body_block(&result_text, width, true));
    }
}
