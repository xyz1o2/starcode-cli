use crate::core::i18n;
use crate::types::{StarToolCall, ToolResult};

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

fn truncate_text(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn summarize_json_value(v: &serde_json::Value, max: usize) -> String {
    match v {
        serde_json::Value::String(s) => truncate_text(s, max),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else {
                i18n::t("ui.format.items", "[{count} items]", "[{count} items]")
                    .replace("{count}", &arr.len().to_string())
            }
        }
        serde_json::Value::Object(obj) => i18n::t(
            "ui.format.fields",
            "{{{count} fields}}",
            "{{{count} fields}}",
        )
        .replace("{count}", &obj.len().to_string()),
    }
}

fn extract_value_from_kv_like(raw: &str, key: &str) -> Option<String> {
    let patterns = [
        format!("{}=", key),
        format!("\"{}\":", key),
        format!("'{}':", key),
        format!("{}:", key),
    ];

    for pat in patterns {
        let Some(pos) = raw.find(&pat) else {
            continue;
        };
        let mut rest = raw[pos + pat.len()..].trim_start();
        if rest.is_empty() {
            continue;
        }

        if rest.starts_with("'''") || rest.starts_with("\"\"\"") {
            rest = &rest[3..];
            if let Some(end) = rest.find("'''").or_else(|| rest.find("\"\"\"")) {
                return Some(rest[..end].trim().to_string());
            }
            return Some(truncate_text(rest.trim(), 120));
        }

        if rest.starts_with('"') || rest.starts_with('\'') {
            let quote = rest.chars().next().unwrap_or('"');
            rest = &rest[quote.len_utf8()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].trim().to_string());
            }
            return Some(truncate_text(rest.trim(), 120));
        }

        let end = rest
            .find(|c: char| c == ',' || c == '\n' || c == '\r' || c == ' ' || c == '\t')
            .unwrap_or(rest.len());
        let value = rest[..end].trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }

    None
}

pub fn tool_display_name(name: &str) -> String {
    match name {
        "view_file" | "Read" => "view".into(),
        "Bash" => "bash".into(),
        "ListDir" | "list_directory" => "ls".into(),
        "Grep" | "search_file_content" | "grep_search" => "search".into(),
        "find_by_name" => "glob".into(),
        "Edit" | "str_replace_editor" | "edit_file" | "smart_edit" => "edit".into(),
        "create_file" | "Write" => "write".into(),
        "complete_task" => "done".into(),
        "Todo" => "tasks".into(),
        "enter_plan_mode" | "exit_plan_mode" => "plan".into(),
        "ask_user" | "user_prompt" => "ask".into(),
        _ => name.to_string(),
    }
}

pub fn canonical_task_action_from_value(args: &serde_json::Value) -> &'static str {
    let raw_action = args
        .get("action")
        .and_then(|value| value.as_str())
        .or_else(|| {
            args.get("operation").and_then(|operation| {
                operation.as_str().or_else(|| {
                    operation
                        .get("action")
                        .or_else(|| operation.get("operation"))
                        .or_else(|| operation.get("type"))
                        .or_else(|| operation.get("op"))
                        .or_else(|| operation.get("command"))
                        .and_then(|value| value.as_str())
                })
            })
        })
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();

    match raw_action.as_str() {
        "add" | "create" | "new" | "add_task" | "create_task" => "add",
        "update" | "set" | "update_task" => "update",
        "delete" | "remove" | "delete_task" => "delete",
        "move" | "reorder" => "move",
        "execute" | "run" => "execute",
        "archive" => "archive",
        "list" | "show" | "view" | "unknown" | "" => "list",
        _ => "list",
    }
}

pub fn task_action_display_label(action: &str) -> &'static str {
    match action {
        "add" => i18n::t("ui.task.add", "Add task", "Add task").leak(),
        "update" => i18n::t("ui.task.update", "Update task", "Update task").leak(),
        "delete" => i18n::t("ui.task.delete", "Delete task", "Delete task").leak(),
        "move" => i18n::t("ui.task.move", "Move task", "Move task").leak(),
        "execute" => i18n::t("ui.task.execute", "Execute task", "Execute task").leak(),
        "archive" => i18n::t("ui.task.archive", "Archive task", "Archive task").leak(),
        _ => i18n::t("ui.task.list", "List tasks", "List tasks").leak(),
    }
}

fn tool_call_brief(tc: &StarToolCall) -> String {
    let name = tc.function.name.as_str();
    let raw_args = tc.function.arguments.as_str();

    // Key fix: if args look like XML/DSML format (contain DSML tags), return brief tool name
    if raw_args.contains("DSML")
        || raw_args.contains("<")
        || raw_args.contains(">")
        || raw_args.contains("function_calls")
    {
        // For XML/DSML format, only return tool name, avoid showing complex XML structure
        return tool_display_name(name);
    }

    // Normal JSON parsing
    let v: Option<serde_json::Value> = serde_json::from_str::<serde_json::Value>(raw_args).ok();
    let get_str = |k: &str| -> Option<String> {
        v.as_ref()
            .and_then(|vv| vv.get(k))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
    };
    let get_u64 =
        |k: &str| -> Option<u64> { v.as_ref().and_then(|vv| vv.get(k)).and_then(|x| x.as_u64()) };

    match name {
        "Todo" => {
            return tool_display_name("tasks").to_string();
        }
        "Edit" | "str_replace_editor" => {
            if let Some(p) = get_str("file_path").or_else(|| get_str("path")) {
                let replace_all = v
                    .as_ref()
                    .and_then(|vv| vv.get("replace_all"))
                    .and_then(|x| x.as_bool());
                let mut s = shorten_path_for_display(&p);
                if let Some(ra) = replace_all {
                    s.push_str(&format!(" (all={})", ra));
                }
                return s;
            }
        }
        "create_file" => {
            if let Some(p) = get_str("path") {
                return shorten_path_for_display(&p);
            }
        }
        "edit_file" => {
            if let Some(p) = get_str("target_file") {
                return shorten_path_for_display(&p);
            }
        }
        "smart_edit" => {
            let p = get_str("file_path")
                .or_else(|| get_str("path"))
                .or_else(|| get_str("target_file"))
                .unwrap_or_else(|| "(unknown)".to_string());
            let old_lines = get_str("old_string")
                .or_else(|| get_str("old_str"))
                .or_else(|| get_str("old"))
                .map(|s| s.lines().count())
                .unwrap_or(0);
            let new_lines = get_str("new_string")
                .or_else(|| get_str("new_str"))
                .or_else(|| get_str("new"))
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if old_lines > 0 || new_lines > 0 {
                return format!(
                    "{} (old:{} lines, new:{} lines)",
                    shorten_path_for_display(&p),
                    old_lines,
                    new_lines
                );
            }
            return shorten_path_for_display(&p);
        }
        "view_file" | "Read" => {
            if let Some(p) = get_str("path").or_else(|| get_str("file_path")) {
                let mut s = shorten_path_for_display(&p);
                if let Some(off) = get_u64("offset") {
                    s.push_str(&format!(" [offset={}]", off));
                }
                if let Some(lim) = get_u64("limit") {
                    s.push_str(&format!(" [limit={}]", lim));
                }
                return s;
            }
        }
        "grep_search" => {
            let q = get_str("Query").unwrap_or_else(|| "".to_string());
            let p = get_str("SearchPath").unwrap_or_else(|| "".to_string());
            if !q.is_empty() || !p.is_empty() {
                if p.is_empty() {
                    return truncate_text(&q, 200);
                }
                return format!(
                    "{} in {}",
                    truncate_text(&q, 200),
                    shorten_path_for_display(&p)
                );
            }
        }
        "find_by_name" => {
            let pat = get_str("Pattern").unwrap_or_else(|| "".to_string());
            let dir = get_str("SearchDirectory").unwrap_or_else(|| "".to_string());
            if !pat.is_empty() || !dir.is_empty() {
                if dir.is_empty() {
                    return truncate_text(&pat, 200);
                }
                return format!(
                    "{} in {}",
                    truncate_text(&pat, 200),
                    shorten_path_for_display(&dir)
                );
            }
        }
        "list_directory" | "ListDir" => {
            if let Some(dir) = get_str("directory") {
                return shorten_path_for_display(&dir);
            }
        }
        "Bash" => {
            if let Some(cmd) = get_str("command").or_else(|| get_str("CommandLine")) {
                return summarize_bash_command(&cmd);
            }
        }
        "enter_plan_mode" => {
            let reason = get_str("reason").unwrap_or_else(|| "".to_string());
            if !reason.is_empty() {
                return truncate_text(&reason, 220);
            }
            return "".to_string();
        }
        "exit_plan_mode" => {
            if let Some(plan) = get_str("plan") {
                let first = plan.lines().next().unwrap_or("").trim();
                if !first.is_empty() {
                    return truncate_text(first, 220);
                }
            }
            return "".to_string();
        }
        "complete_task" => {
            if let Some(result) = get_str("result") {
                let first_line = result.lines().next().unwrap_or("").trim();
                if !first_line.is_empty() {
                    return truncate_text(first_line, 200);
                }
            }
            return "".to_string();
        }
        _ => {}
    }

    if let Some(vv) = v.as_ref() {
        if let Some(obj) = vv.as_object() {
            let mut parts = Vec::new();
            for (k, v) in obj.iter().take(3) {
                parts.push(format!("{}={}", k, summarize_json_value(v, 40)));
            }
            if obj.len() > 3 {
                parts.push(format!("+{} items", obj.len() - 3));
            }
            if !parts.is_empty() {
                return parts.join(", ");
            }
        }
        let compact = vv.to_string();
        return truncate_text(&compact, 200);
    }
    "".to_string()
}

// ============ UX improvement: modern tool status display ============
fn format_tool_call_running(tc: &StarToolCall) -> String {
    tool_call_brief(tc)
}

/// Summarize a bash command line for compact display.
/// Long commands (pipelines, multi-arg) are truncated to keep the
/// tool header readable.
fn summarize_bash_command(cmd: &str) -> String {
    let cmd = cmd.trim();
    // Extract just the first command name (before pipe/redirect/semicolon)
    let first_segment = cmd
        .split(&['|', ';', '>', '<', '&'][..])
        .next()
        .unwrap_or(cmd)
        .trim();
    // Take first few words: cmd + 2 args max
    let words: Vec<&str> = first_segment.split_whitespace().collect();
    if words.len() <= 3 {
        truncate_text(cmd, 100)
    } else {
        let brief = format!("{} {} ...", words[0], words[1]);
        truncate_text(&brief, 100)
    }
}
// ============================================

pub fn format_tool_call(tc: &StarToolCall) -> String {
    format_tool_call_running(tc)
}

// ============ UX improvement: StarCode-style result display ============
pub fn format_tool_result(_tc: &StarToolCall, tr: &ToolResult) -> String {
    // Edit tools: show diff with +N/-M summary like openclaude
    if let Some(data) = &tr.data {
        if let Some(diff) = data.get("diff").and_then(|v| v.as_str()) {
            if !diff.trim().is_empty() {
                let mut added = 0;
                let mut removed = 0;
                for line in diff.lines() {
                    if line.starts_with('+') && !line.starts_with("+++") {
                        added += 1;
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        removed += 1;
                    }
                }
                let summary = format!("+{} -{}", added, removed);
                let d = truncate_text(diff.trim(), 2000);
                return format!("{}\n{}", summary, d);
            }
        }
    }

    let out_raw = if tr.success {
        tr.output.as_deref().unwrap_or("")
    } else {
        tr.error.as_deref().unwrap_or("")
    };
    let out_clean = crate::ui::utils::text::sanitize_for_tui(out_raw);
    let out = truncate_text(&out_clean, 1200);

    let lines: Vec<&str> = out.trim().lines().collect();
    let n = lines.len();

    if !tr.success {
        return truncate_text(out_raw, 100).to_string();
    }
    if n == 0 {
        return String::new();
    }
    if n == 1 {
        return out.trim().to_string();
    }
    format!("{} lines", n)
}
// ============================================

pub fn format_tool_result_with_saved_path(
    tc: &StarToolCall,
    tr: &ToolResult,
    saved_path: &str,
) -> String {
    let base = format_tool_result(tc, tr);
    let mut it = base.lines();
    let first = it.next().unwrap_or("");
    let rest = it.collect::<Vec<_>>().join("\n");

    // Generate friendlier save notification
    let path_display = truncate_text(saved_path, 100);
    let filename = std::path::Path::new(saved_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(saved_path);
    let viewer = if cfg!(unix) { "less" } else { "type" };

    let first_with_path = if rest.trim().is_empty() {
        let template = i18n::t(
            "ui.tool.output.saved.short",
            "{first}\n\n[FILE] Output saved to: {file}\n   Tip: view with external editor ({viewer} / .star/tool_outputs/)",
            "{first}\n\n[FILE] Output saved to: {file}\n   Tip: view with external editor ({viewer} / .star/tool_outputs/)",
        );
        template
            .replace("{first}", first)
            .replace("{file}", filename)
            .replace("{viewer}", viewer)
    } else {
        let template = i18n::t(
            "ui.tool.output.saved.tag",
            "{first} [saved: {file}]",
            "{first} [saved: {file}]",
        );
        template
            .replace("{first}", first)
            .replace("{file}", filename)
    };

    if rest.trim().is_empty() {
        let template = i18n::t(
            "ui.tool.output.saved.full",
            "{first}\n\n[FILE] Full output saved: {path}",
            "{first}\n\n[FILE] Full output saved: {path}",
        );
        template
            .replace("{first}", &first_with_path)
            .replace("{path}", &path_display)
    } else {
        let template = i18n::t(
            "ui.tool.output.saved.full_with_body",
            "{first}\n{rest}\n\n[FILE] Full output saved: {path}",
            "{first}\n{rest}\n\n[FILE] Full output saved: {path}",
        );
        template
            .replace("{first}", &first_with_path)
            .replace("{rest}", &rest)
            .replace("{path}", &path_display)
    }
}
