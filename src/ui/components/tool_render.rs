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
pub fn get_tool_status_icon(
    entry: &crate::types::ChatEntry,
    theme: &Theme,
) -> (&'static str, Color) {
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
fn apply_preview_limit(lines: &mut Vec<Line<'static>>, max_lines: usize, theme: &Theme) -> usize {
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
    // "  ⎿  " (5) —— 对标 Claude Code MessageResponse 前缀
    // 内容宽度再留 1 列右边距，避免与滚动条列重叠
    let prefix_width: usize = 5;
    let tool_inner_width = tool_box_width.saturating_sub(prefix_width + 1);

    let expanded = entry
        .tool_call
        .as_ref()
        .map(|tc| {
            // ToolCall 默认展开
            // ToolResult: 编辑类工具默认展开，其他工具默认折叠
            let is_edit_tool = matches!(
                tc.function.name.as_str(),
                "Edit"
                    | "create_file"
                    | "edit_file"
                    | "str_replace_editor"
                    | "smart_edit"
                    | "Write"
                    | "TodoWrite"
            );

            let default_expanded = matches!(entry.entry_type, ChatEntryType::ToolCall)
                || (matches!(entry.entry_type, ChatEntryType::ToolResult) && is_edit_tool);
            let toggled = state.expanded_tool_call_ids.contains(&tc.id);
            default_expanded ^ toggled
        })
        .unwrap_or(true);

    // ToolCall: 只显示工具名称和参数摘要（单行）
    // ToolResult: 显示结果内容
    // ToolConfirmation: 显示确认卡片
    let content_lines = if entry.entry_type == ChatEntryType::ToolConfirmation {
        if let Some(ref conf) = entry.confirmation {
            if matches!(
                conf.operation_type,
                crate::types::ConfirmationType::AskUserQuestion
            ) {
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
        // ToolCall: 显示工具名称和参数，格式为 ● ToolName(args)
        if let Some(tc) = &entry.tool_call {
            let (args_str, _extra_info) = build_tool_argument_display(tc, state.ui_verbose);
            let tool_name = crate::ui::utils::format::tool_display_name(tc.function.name.as_str());

            // 构建完整行：工具名(参数)；参数为空时不带括号（如 TodoWrite → "Update Todos"）
            let full_line = if args_str.trim().is_empty() {
                tool_name.clone()
            } else {
                format!("{}({})", tool_name, args_str)
            };

            // 按终端宽度截断（对标 Claude Code wrap="truncate-end"）
            let display_args = if !state.ui_verbose {
                let truncated = crate::ui::utils::render::truncate_to_display_width(
                    &full_line,
                    tool_inner_width,
                );
                // 从截断后的行中提取参数部分
                truncated
                    .strip_prefix(&tool_name)
                    .and_then(|s| s.strip_prefix('('))
                    .and_then(|s| s.strip_suffix(')'))
                    .unwrap_or(&args_str)
                    .to_string()
            } else {
                args_str
            };

            vec![Line::from(vec![
                Span::styled(
                    tool_name,
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(theme.primary),
                ),
                Span::raw("("),
                Span::styled(display_args, Style::default()),
                Span::raw(")"),
            ])]
        } else {
            vec![]
        }
    } else if entry.entry_type == ChatEntryType::ToolResult {
        // ToolResult: 显示结果内容
        if let Some(tc) = &entry.tool_call {
            render_rich_tool_content(
                entry,
                tc,
                tool_inner_width,
                expanded,
                _prev_is_confirmation,
                theme,
            )
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
        crate::ui::utils::render::build_tool_body_block(&entry.content, tool_inner_width, expanded)
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
        // 注意：闪烁通过颜色明暗切换实现，字符恒为 ●。
        // 不能用空格替代 ● —— CJK 终端下 ● 是双宽、空格是单宽，
        // 交替会导致整行文字左右移动（所有工具行都会抖动）。
        let marker = if cancelling { "⊘" } else { "●" };
        let marker_color = if cancelling {
            theme.warning
        } else if is_streaming {
            if blink_visible {
                theme.primary
            } else {
                theme.inactive // 闪烁"灭"态：暗色圆点，宽度不变
            }
        } else {
            theme.tool_success // 完成后用绿色
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
        // ToolResult: 对标 Claude Code MessageResponse 的 "  ⎿ " 前缀（5 列）——
        // 首行带 ⎿ 标记，续行用等宽空格缩进，保证整个输出块左缘对齐，
        // 而不是只有首行缩进、其余行顶到行首。
        let mut first_line = true;
        for line in content_lines.into_iter() {
            let line_is_blank = line.spans.iter().all(|s| s.content.trim().is_empty());
            if line_is_blank {
                boxed_lines.push(Line::from(""));
                continue;
            }

            let mut spans = Vec::with_capacity(line.spans.len() + 1);
            if first_line {
                spans.push(Span::styled("  ⎿  ", Style::default().fg(theme.subtle)));
                first_line = false;
            } else {
                spans.push(Span::raw("     "));
            }
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
    let shorten = |p: &str| {
        if verbose {
            p.to_string()
        } else {
            shorten_path_for_display(p)
        }
    };
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
        "TodoWrite" => {
            // 对标 CC：调用行只显示工具名，参数以 "N items" 摘要挂在 extra_info
            if let Some(todos) = obj.get("todos").and_then(|v| v.as_array()) {
                extra_info.push(format!("{} items", todos.len()));
            }
            String::new()
        }
        "Agent" | "agent" => {
            // 对标 CC：Agent 调用行只显示 description，不倾倒 prompt 全文
            get_str(&["description"]).unwrap_or_default()
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
        "list_directory" | "ListDir" => get_str(&["directory", "path"])
            .map(|p| shorten(&p))
            .unwrap_or_else(|| ".".to_string()),
        _ => summarize_object_inline(obj),
    };

    (summary, extra_info)
}

/// TodoWrite 结果块：把 args 里的 todos 渲染成清单（对标 Claude Code TaskListV2::TaskItem）。
/// completed `✔` + 删除线暗色；in_progress `▪` 高亮加粗（优先显示 activeForm）；
/// pending `▫` 默认色。args 解析不出条目时返回空，由调用方回退到通用渲染。
fn render_todo_checklist(
    tc: &crate::types::StarToolCall,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let todos = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
        .ok()
        .and_then(|v| v.get("todos").and_then(|t| t.as_array()).cloned())
        .unwrap_or_default();

    let mut lines = Vec::with_capacity(todos.len());
    for todo in &todos {
        let status = todo
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("pending");
        let content = todo.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let active_form = todo
            .get("activeForm")
            .or_else(|| todo.get("active_form"))
            .and_then(|a| a.as_str())
            .filter(|s| !s.trim().is_empty());

        let (icon, icon_style, text_style) = match status {
            "completed" => (
                "✔",
                Style::default().fg(theme.success),
                Style::default()
                    .fg(theme.inactive)
                    .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
            ),
            "in_progress" => (
                "▪",
                Style::default().fg(theme.primary),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            _ => (
                "▫",
                Style::default().fg(theme.secondary),
                Style::default().fg(theme.foreground),
            ),
        };

        // 进行中的行显示 activeForm（"Running tests"），其余显示 content
        let label = if status == "in_progress" {
            active_form.unwrap_or(content)
        } else {
            content
        };
        // 续行有 2 空格缩进 + 图标 + 空格，文本宽度按此收敛
        let label =
            crate::ui::utils::render::truncate_to_display_width(label, width.saturating_sub(4));
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{} ", icon), icon_style),
            Span::styled(label.to_string(), text_style),
        ]));
    }
    lines
}

fn render_rich_tool_content(
    entry: &crate::types::ChatEntry,
    tc: &crate::types::StarToolCall,
    tool_inner_width: usize,
    expanded: bool,
    _prev_is_confirmation: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    // ===== TodoWrite：结果块直接渲染 todos 清单（对标 Claude Code TaskListV2） =====
    if tc.function.name == "TodoWrite" {
        if let Some(tr) = &entry.tool_result {
            if tr.success {
                return render_todo_checklist(tc, tool_inner_width, theme);
            }
        }
    }

    let mut lines = Vec::new();

    // 此函数只处理 ToolResult
    let tool_color = if let Some(tr) = &entry.tool_result {
        if tr.success {
            theme.tool_success
        } else {
            theme.tool_error
        }
    } else {
        theme.warning
    };

    let _tool_name = crate::ui::utils::format::tool_display_name(tc.function.name.as_str());

    // ===== Write/create_file 工具：折叠态显示 Wrote N lines to path =====
    if !expanded && matches!(tc.function.name.as_str(), "create_file" | "Write") {
        if let Some(tr) = &entry.tool_result {
            if tr.success {
                // 从 args 提取文件路径和内容行数
                if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                {
                    let path = args
                        .get("path")
                        .or_else(|| args.get("file_path"))
                        .or_else(|| args.get("target_file"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("(unknown)");

                    // 优先从 args.content 获取行数，其次从 output 获取
                    let line_count = args
                        .get("content")
                        .or_else(|| args.get("file_text"))
                        .and_then(|v| v.as_str())
                        .map(|c| c.lines().count())
                        .or_else(|| tr.output.as_deref().map(|o| o.lines().count()))
                        .unwrap_or(0);

                    let short_path = shorten_path_for_display(path);
                    lines.push(Line::from(vec![
                        Span::raw("Wrote "),
                        Span::styled(
                            format!("{}", line_count),
                            Style::default().fg(tool_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" lines to "),
                        Span::styled(short_path, Style::default().add_modifier(Modifier::BOLD)),
                    ]));
                    return lines;
                }
            }
        }
    }

    // ===== ToolResult 路径 =====
    if let Some(tr) = &entry.tool_result {
        let diff_str = tr
            .data
            .as_ref()
            .and_then(|d| d.get("diff").and_then(|v| v.as_str()));
        let has_diff = diff_str.is_some() && !diff_str.unwrap_or("").trim().is_empty();

        // 编辑工具：始终显示摘要行 + 完整 diff
        if has_diff {
            let diff_content = diff_str.unwrap();
            // 计算新增/删除行数
            let mut added = 0usize;
            let mut removed = 0usize;
            for line in diff_content.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    added += 1;
                } else if line.starts_with('-') && !line.starts_with("---") {
                    removed += 1;
                }
            }
            // 摘要行（对标 FileEditToolUpdatedMessage）
            lines.push(Line::from(vec![
                Span::raw("Added "),
                Span::styled(
                    format!("{}", added),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" lines, removed "),
                Span::styled(
                    format!("{}", removed),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" lines"),
            ]));
            // 完整 diff 内容
            lines.extend(crate::ui::utils::render::build_diff_block(
                diff_content,
                tool_inner_width,
            ));
            return lines;
        }

        // 非 diff 工具：根据工具类型决定折叠态显示方式
        let text = if tr.success {
            tr.output.as_deref().unwrap_or("")
        } else {
            tr.error.as_deref().unwrap_or("")
        };

        // ===== 查看类工具：折叠态只显示摘要行 =====
        if !expanded && !text.trim().is_empty() && tr.success {
            match tc.function.name.as_str() {
                // Read/view_file: "Read N lines"
                "view_file" | "Read" => {
                    let line_count = text.lines().count();
                    lines.push(Line::from(vec![
                        Span::raw("Read "),
                        Span::styled(
                            format!("{}", line_count),
                            Style::default().fg(tool_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" lines"),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "  (Tab to expand)",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    )));
                    return lines;
                }
                // Grep/search: "Found N matches"
                "Grep" | "search_file_content" | "grep_search" => {
                    let match_count = text.lines().filter(|l| !l.trim().is_empty()).count();
                    lines.push(Line::from(vec![
                        Span::raw("Found "),
                        Span::styled(
                            format!("{}", match_count),
                            Style::default().fg(tool_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" matches"),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "  (Tab to expand)",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    )));
                    return lines;
                }
                // find_by_name/list_directory/ListDir: "Found N files"
                "find_by_name" | "list_directory" | "ListDir" => {
                    let file_count = text.lines().filter(|l| !l.trim().is_empty()).count();
                    lines.push(Line::from(vec![
                        Span::raw("Found "),
                        Span::styled(
                            format!("{}", file_count),
                            Style::default().fg(tool_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" files"),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "  (Tab to expand)",
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    )));
                    return lines;
                }
                // 其他工具：维持预览逻辑
                _ => {}
            }
        }

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

    // ===== Bash 无输出时显示 Done =====
    if success && text.trim().is_empty() && !expanded {
        lines.push(Line::from(Span::styled(
            "Done",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
        return;
    }

    // ===== Bash 后台任务显示 =====
    // 检查 text 是否包含后台任务标识
    if success && text.contains("Running in the background") && !expanded {
        lines.push(Line::from(Span::styled(
            "Running in the background (↓ to manage)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
        return;
    }

    if !expanded {
        // 折叠态：预览前几行真实内容 + "Tab 展开"提示（而非只有行数）。
        // 之前只显示第一行 + N lines，用户看不到任何正文，误以为输出被截断/不显示。
        // 带颜色的行保留 ANSI 颜色（对标参考实现折叠输出仍显示彩色）。
        let mut shown = 0usize;
        for raw_line in text.lines() {
            if shown >= TOOL_RESULT_PREVIEW_LINES {
                break;
            }
            if raw_line.trim().is_empty() {
                continue;
            }
            if raw_line.contains('\x1b') {
                let spans = crate::ui::utils::render::parse_ansi_text(raw_line);
                let truncated = crate::ui::utils::render::truncate_spans_to_width(&spans, width);
                if !truncated.is_empty() {
                    lines.push(Line::from(truncated));
                    shown += 1;
                }
                continue;
            }
            let clean = crate::ui::utils::render::strip_ansi_codes(raw_line);
            let preview = crate::ui::utils::render::truncate_to_display_width(clean.trim(), width);
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
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
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
            // 错误输出同样保留 ANSI 颜色，无色部分补红色（对标 OutputLine isError）
            if line.contains('\x1b') {
                let spans = crate::ui::utils::render::parse_ansi_text(line);
                let rows = if crate::ui::utils::render::line_spans_display_width(&spans) > width {
                    crate::ui::utils::render::wrap_spans_to_width(spans, width)
                } else {
                    vec![spans]
                };
                for mut row in rows {
                    for span in row.iter_mut() {
                        if span.style == Style::default() {
                            let content = span.content.clone();
                            *span = Span::styled(content, Style::default().fg(Color::Red));
                        }
                    }
                    lines.push(Line::from(row));
                }
            } else {
                let stripped = crate::ui::utils::render::strip_ansi_codes(line);
                lines.push(Line::from(vec![Span::styled(
                    stripped,
                    Style::default().fg(Color::Red),
                )]));
            }
        }
    } else {
        lines.extend(crate::ui::utils::render::build_tool_body_block(
            &result_text,
            width,
            true,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo_call(args: &str) -> crate::types::StarToolCall {
        crate::types::StarToolCall {
            id: "t1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "TodoWrite".to_string(),
                arguments: args.to_string(),
            },
        }
    }

    fn theme() -> Theme {
        // 测试只取色值字段，用内置暗色主题即可
        crate::ui::themes::theme::Theme::default_dark()
    }

    /// 三种状态按 CC TaskListV2 渲染：pending ▫、in_progress ▪ + activeForm、
    /// completed ✔ + 删除线
    #[test]
    fn todo_checklist_renders_cc_styling() {
        let tc = todo_call(
            r#"{"todos":[
                {"content":"安装 recharts 图表库","status":"pending","activeForm":"安装中"},
                {"content":"跑回测","status":"in_progress","activeForm":"跑回测中"},
                {"content":"接入真实历史数据","status":"completed","activeForm":"接入中"}
            ]}"#,
        );
        let lines = render_todo_checklist(&tc, 80, &theme());
        assert_eq!(lines.len(), 3);

        // pending：▫ + content（祈使句）
        assert_eq!(lines[0].spans[1].content, "▫ ");
        assert_eq!(lines[0].spans[2].content, "安装 recharts 图表库");

        // in_progress：▪ + activeForm（进行时）
        assert_eq!(lines[1].spans[1].content, "▪ ");
        assert_eq!(lines[1].spans[2].content, "跑回测中");

        // completed：✔ + 删除线
        assert_eq!(lines[2].spans[1].content, "✔ ");
        assert!(lines[2].spans[2]
            .style
            .add_modifier
            .contains(Modifier::CROSSED_OUT));
    }

    /// 窄宽度下中文标签被截断但不能 panic
    #[test]
    fn todo_checklist_truncates_cjk_at_narrow_width() {
        let tc = todo_call(
            r#"{"todos":[{"content":"安装 recharts 图表库到前端项目","status":"pending","activeForm":"安装中"}]}"#,
        );
        for width in 0..=30usize {
            let lines = render_todo_checklist(&tc, width, &theme());
            assert_eq!(lines.len(), 1, "width = {width}");
        }
    }

    /// 解析不出 todos（如空参数）时返回空，由调用方回退到通用结果渲染
    #[test]
    fn todo_checklist_without_todos_yields_empty() {
        let tc = todo_call(r#"{"todos":[]}"#);
        assert!(render_todo_checklist(&tc, 80, &theme()).is_empty());
        let tc = todo_call("not json");
        assert!(render_todo_checklist(&tc, 80, &theme()).is_empty());
    }
}
