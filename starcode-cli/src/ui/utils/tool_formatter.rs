/// Tool output formatting — concise, user-friendly display.
///
/// # Design Principles (from Claude Code)
/// 1. **Layered truncation**: file → line → character level
/// 2. **Smart summaries**: show what matters, hide the rest
/// 3. **Relative paths**: shorter, cleaner display
/// 4. **Consistent format**: same pattern for all tools

/// Maximum lines to display in tool output
const MAX_DISPLAY_LINES: usize = 15;

/// Maximum characters for command display
const MAX_COMMAND_DISPLAY_CHARS: usize = 120;

/// Maximum characters for error display
const MAX_ERROR_DISPLAY_CHARS: usize = 500;

/// Format file path for display (relative to cwd, use ~ for home)
pub fn format_display_path(path: &str) -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Try relative to cwd first
    if path.starts_with(&cwd) {
        let relative = &path[cwd.len()..];
        return relative.strip_prefix('/').unwrap_or(relative).to_string();
    }

    // Try relative to home
    if path.starts_with(&home) {
        return format!("~{}", &path[home.len()..]);
    }

    path.to_string()
}

/// Format tool call summary (tool name + short args)
pub fn format_tool_call_summary(tool_name: &str, args: &str) -> String {
    let args_summary = truncate_args(tool_name, args);
    if args_summary.is_empty() {
        tool_name.to_string()
    } else {
        format!("{}({})", tool_name, args_summary)
    }
}

/// Truncate tool arguments for display
fn truncate_args(tool_name: &str, args: &str) -> String {
    // Parse args and extract key information
    let parsed: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(_) => return truncate_string(args, MAX_COMMAND_DISPLAY_CHARS),
    };

    match tool_name {
        "Read" | "view_file" => {
            if let Some(path) = parsed.get("path").or_else(|| parsed.get("file_path")) {
                let display_path = format_display_path(path.as_str().unwrap_or("?"));
                if let Some(offset) = parsed.get("offset").and_then(|v| v.as_u64()) {
                    return format!("{}:{}", display_path, offset);
                }
                return display_path;
            }
        }
        "Write" => {
            if let Some(path) = parsed.get("path").or_else(|| parsed.get("file_path")) {
                return format_display_path(path.as_str().unwrap_or("?"));
            }
        }
        "Edit" | "edit" => {
            if let Some(path) = parsed.get("path").or_else(|| parsed.get("file_path")) {
                let display_path = format_display_path(path.as_str().unwrap_or("?"));
                if let Some(old) = parsed.get("old_string").or_else(|| parsed.get("old_str")) {
                    let preview = truncate_string(old.as_str().unwrap_or(""), 30);
                    return format!("{}: '{}'", display_path, preview);
                }
                return display_path;
            }
        }
        "Bash" | "shell" => {
            if let Some(cmd) = parsed.get("command").or_else(|| parsed.get("cmd")) {
                let cmd_str = cmd.as_str().unwrap_or("");
                return truncate_string(cmd_str, MAX_COMMAND_DISPLAY_CHARS);
            }
        }
        "Grep" => {
            if let Some(query) = parsed.get("query").or_else(|| parsed.get("pattern")) {
                let q = query.as_str().unwrap_or("");
                let preview = truncate_string(q, 40);
                if let Some(path) = parsed.get("path").or_else(|| parsed.get("glob")) {
                    return format!("{} in {}", preview, format_display_path(path.as_str().unwrap_or(".")));
                }
                return preview;
            }
        }
        "Glob" => {
            if let Some(pattern) = parsed.get("pattern") {
                return pattern.as_str().unwrap_or("").to_string();
            }
        }
        "multi_edit" => {
            if let Some(path) = parsed.get("path").or_else(|| parsed.get("file_path")) {
                let display_path = format_display_path(path.as_str().unwrap_or("?"));
                if let Some(edits) = parsed.get("edits").and_then(|v| v.as_array()) {
                    return format!("{} ({} edits)", display_path, edits.len());
                }
                return display_path;
            }
        }
        "TodoWrite" | "Todo" => {
            if let Some(todos) = parsed.get("todos").and_then(|v| v.as_array()) {
                return format!("{} items", todos.len());
            }
        }
        "SemanticSearch" => {
            if let Some(query) = parsed.get("query") {
                return truncate_string(query.as_str().unwrap_or(""), 40);
            }
        }
        "ProjectMap" => {
            return "project structure".to_string();
        }
        _ => {}
    }

    // Fallback: truncate raw args
    truncate_string(args, MAX_COMMAND_DISPLAY_CHARS)
}

/// Truncate string to max length with ellipsis
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

/// Format tool result for display
pub fn format_tool_result_display(tool_name: &str, success: bool, output: &str) -> String {
    if !success {
        return format_error_output(output);
    }

    match tool_name {
        "Read" | "view_file" => {
            let line_count = output.lines().count();
            if line_count > MAX_DISPLAY_LINES {
                let preview: Vec<&str> = output.lines().take(MAX_DISPLAY_LINES).collect();
                let remaining = line_count - MAX_DISPLAY_LINES;
                format!("{}…\n  +{} more lines", preview.join("\n"), remaining)
            } else {
                output.to_string()
            }
        }
        "Bash" | "shell" => {
            let line_count = output.lines().count();
            if line_count > MAX_DISPLAY_LINES {
                // For bash, show last N lines (usually more useful)
                let lines: Vec<&str> = output.lines().collect();
                let start = lines.len().saturating_sub(MAX_DISPLAY_LINES);
                let preview: Vec<&str> = lines[start..].to_vec();
                let remaining = start;
                format!("… ({} lines above)\n{}", remaining, preview.join("\n"))
            } else {
                output.to_string()
            }
        }
        "Grep" => {
            let match_count = output.lines().count();
            if match_count > MAX_DISPLAY_LINES {
                let preview: Vec<&str> = output.lines().take(MAX_DISPLAY_LINES).collect();
                let remaining = match_count - MAX_DISPLAY_LINES;
                format!("{}…\n  +{} more matches", preview.join("\n"), remaining)
            } else {
                output.to_string()
            }
        }
        "Glob" => {
            let file_count = output.lines().count();
            if file_count > MAX_DISPLAY_LINES {
                let preview: Vec<&str> = output.lines().take(MAX_DISPLAY_LINES).collect();
                let remaining = file_count - MAX_DISPLAY_LINES;
                format!("{}…\n  +{} more files", preview.join("\n"), remaining)
            } else {
                output.to_string()
            }
        }
        _ => {
            if output.len() > MAX_ERROR_DISPLAY_CHARS {
                format!("{}…", &output[..MAX_ERROR_DISPLAY_CHARS])
            } else {
                output.to_string()
            }
        }
    }
}

/// Format error output (concise)
fn format_error_output(error: &str) -> String {
    let lines: Vec<&str> = error.lines().collect();
    if lines.len() <= 3 {
        error.to_string()
    } else {
        // Show first 3 lines + count
        let preview: Vec<&str> = lines[..3].to_vec();
        let remaining = lines.len() - 3;
        format!("{}…\n  +{} more lines", preview.join("\n"), remaining)
    }
}

/// Format elapsed time for tool execution
pub fn format_tool_elapsed(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// Format status message for tool execution
pub fn format_status_message(tool_name: &str, status: &str) -> String {
    match status {
        "running" => format!("Running {}…", tool_name),
        "done" => format!("Done: {}", tool_name),
        "error" => format!("Error: {}", tool_name),
        _ => format!("{}: {}", tool_name, status),
    }
}
