//! 补全建议（`/` 命令提示 + `@` 文件提示）的状态机与按键处理 —— TEA 的 update 侧。
//!
//! 这里只做"输入 → 状态变更"，不碰 Buffer、不做布局。
//! 绘制在 `crate::ui::components::command_suggestions`。

use std::time::Duration;

use crate::ui::state::ChatState;
const PASTE_HINT_GUARD_MS: u64 = 350;

fn extract_hint_primary_text(hint: &str) -> &str {
    hint.split(" - ").next().unwrap_or("").trim()
}

fn extract_mention_path(hint: &str) -> &str {
    hint.split("  ").next().unwrap_or(hint).trim()
}

/// 定位输入末尾正在输入的 @ token。
/// 返回 Some((start, quoted))：start 是 '@' 的字节位置，quoted 表示 @" 引号形式。
/// 引号形式允许 token 内含空格；普通形式以空白结尾即失效。
fn last_at_token_start(input: &str) -> Option<(usize, bool)> {
    let bytes = input.as_bytes();
    let mut at = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'@' && (i == 0 || (bytes[i - 1] as char).is_whitespace()) {
            at = Some(i);
        }
    }
    let at = at?;
    let quoted = bytes.get(at + 1) == Some(&b'"');
    if !quoted && input[at + 1..].contains(char::is_whitespace) {
        return None; // 普通 token 已被空白终结，不再激活
    }
    Some((at, quoted))
}

/// 从 @ token 提取搜索片段（已接受时替换、输入时刷新共用）
fn at_token_frag(input: &str, at: usize, quoted: bool) -> &str {
    let after = &input[at + 1..];
    if quoted {
        // 去掉开头引号；未闭合时取到末尾
        let body = after.strip_prefix('"').unwrap_or(after);
        match body.find('"') {
            Some(i) => &body[..i],
            None => body,
        }
    } else {
        // 到空白为止
        match after.find(char::is_whitespace) {
            Some(i) => &after[..i],
            None => after,
        }
    }
}

/// 把 @ token 替换为指定路径。路径含空格时使用 @"..." 引号形式（对标 Claude Code）。
/// is_dir 为真时以 `/` 结尾且不关闭提示（继续下钻），否则补空格。
fn replace_at_token_with_path(input: &mut String, path: &str, is_dir: bool) {
    let Some((at, _quoted)) = last_at_token_start(input) else {
        return;
    };
    // 计算原 token 的结束位置：引号形式到闭合引号或末尾；普通形式到空白或末尾
    let end = {
        let after = &input[at + 1..];
        if after.starts_with('"') {
            let body = &after[1..];
            body.find('"')
                .map(|i| at + 1 + 1 + i + 1)
                .unwrap_or(input.len())
        } else {
            after
                .find(char::is_whitespace)
                .map(|i| at + 1 + i)
                .unwrap_or(input.len())
        }
    };

    let suffix = input[end..].to_string();
    input.truncate(at);

    let clean = path.trim_end_matches('/');
    if clean.chars().any(char::is_whitespace) {
        // 引号形式，目录的 '/' 放在引号内
        if is_dir {
            input.push_str(&format!("@\"{}/\"", clean));
        } else {
            input.push_str(&format!("@\"{}\"", clean));
        }
    } else if is_dir {
        input.push_str(&format!("@{}/", clean));
    } else {
        input.push_str(&format!("@{}", clean));
    }

    if suffix.is_empty() {
        // 文件补空格；目录不补（等待继续输入或下钻刷新）
        if !is_dir {
            input.push(' ');
        }
    } else if !suffix.starts_with(' ') && !is_dir {
        input.push(' ');
        input.push_str(&suffix);
    } else {
        input.push_str(&suffix);
    }
}

pub fn on_input_changed(state: &mut ChatState) {
    if should_suppress_hints(state) {
        state.show_command_hints = false;
        state.command_hints.clear();
        state.show_mention_hints = false;
        state.mention_hints.clear();
        return;
    }

    refresh_mention_hints(state);

    if state.input.starts_with('/') && !state.show_mention_hints {
        state.show_command_hints = true;
        state.command_hints = crate::commands::system::get_command_hints(&state.input);
        state.selected_hint = 0;
    } else {
        state.show_command_hints = false;
        state.command_hints.clear();
    }
}

fn should_suppress_hints(state: &ChatState) -> bool {
    if state.paste_in_progress {
        return true;
    }
    if let Some(end_time) = state.paste_end_time {
        if end_time.elapsed() < Duration::from_millis(PASTE_HINT_GUARD_MS) {
            return true;
        }
    }
    // 不再因为多行输入而抑制提示，允许用户在多行输入时使用 @ 文件引用
    false
}

pub fn handle_up(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        if state.selected_session_index > 0 {
            state.selected_session_index -= 1;
        }
        return true;
    }
    if state.show_provider_menu {
        if state.selected_provider_index > 0 {
            state.selected_provider_index -= 1;
        }
        return true;
    }
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        if state.selected_mention_hint > 0 {
            state.selected_mention_hint -= 1;
        }
        return true;
    }
    if state.show_command_hints && !state.command_hints.is_empty() {
        if state.selected_hint > 0 {
            state.selected_hint -= 1;
        }
        return true;
    }
    false
}

pub fn handle_down(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        let sessions_len =
            crate::ui::components::palette::get_session_quick_items(&state.available_sessions)
                .len();
        if state.selected_session_index + 1 < sessions_len {
            state.selected_session_index += 1;
        }
        return true;
    }
    if state.show_provider_menu {
        let providers_len =
            crate::ui::components::palette::get_provider_quick_items(&state.configured_providers)
                .len();
        if state.selected_provider_index + 1 < providers_len {
            state.selected_provider_index += 1;
        }
        return true;
    }
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        if state.selected_mention_hint + 1 < state.mention_hints.len() {
            state.selected_mention_hint += 1;
        }
        return true;
    }
    if state.show_command_hints && !state.command_hints.is_empty() {
        if state.selected_hint + 1 < state.command_hints.len() {
            state.selected_hint += 1;
        }
        return true;
    }
    false
}

fn update_textarea_from_input(state: &mut ChatState) {
    let lines: Vec<String> = state.input.lines().map(|s| s.to_string()).collect();
    state.input_line_count = lines.len();
    let mut textarea = if lines.is_empty() {
        tui_textarea::TextArea::default()
    } else {
        tui_textarea::TextArea::new(lines)
    };
    textarea.set_placeholder_text("Type a message...");
    textarea.set_cursor_line_style(ratatui::style::Style::default());

    // Move cursor to end
    textarea.move_cursor(tui_textarea::CursorMove::Bottom);
    textarea.move_cursor(tui_textarea::CursorMove::End);

    // If input ends with a newline that lines() ate, or if we want to ensure cursor is after the text
    if state.input.ends_with('\n') {
        textarea.insert_newline();
        state.input_line_count += 1;
    }

    state.textarea = textarea;
}

pub fn handle_enter(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        let sessions =
            crate::ui::components::palette::get_session_quick_items(&state.available_sessions);
        if state.selected_session_index < sessions.len() {
            state.pending_palette_action =
                Some(sessions[state.selected_session_index].action.clone());
            state.show_session_menu = false;
        }
        return true;
    }
    if state.show_provider_menu {
        let providers =
            crate::ui::components::palette::get_provider_quick_items(&state.configured_providers);
        if state.selected_provider_index < providers.len() {
            state.pending_palette_action =
                Some(providers[state.selected_provider_index].action.clone());
            state.show_provider_menu = false;
        }
        return true;
    }

    if state.show_mention_hints && !state.mention_hints.is_empty() {
        accept_mention_hint(state);
        return true;
    }

    if state.show_command_hints && !state.command_hints.is_empty() {
        let selected = &state.command_hints[state.selected_hint];
        let cmd_text = extract_hint_primary_text(selected);

        // 如果提取出的命令文本已经是一个完整的命令路径（例如 /memory add），
        // 且它不是一个子命令片段（例如仅 add），则直接替换
        if cmd_text.starts_with('/') {
            state.input = cmd_text.to_string();
            state.input.push(' '); // 自动添加空格方便后续输入参数
        } else if state.input.to_lowercase().starts_with("/mcp") {
            // 特殊处理 /mcp 子命令
            state.input = format!("/mcp {} ", cmd_text);
        } else {
            // 普通文本补全
            state.input = format!("/{} ", cmd_text);
        }

        update_textarea_from_input(state);
        state.show_command_hints = false;
        state.command_hints.clear();
        return true;
    }

    false
}

pub fn handle_tab(state: &mut ChatState) -> bool {
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        accept_mention_hint(state);
        return true;
    }

    if state.show_command_hints && !state.command_hints.is_empty() {
        let selected = &state.command_hints[state.selected_hint];
        let cmd_text = extract_hint_primary_text(selected);

        if cmd_text.starts_with('/') {
            state.input = cmd_text.to_string();
        } else if state.input.to_lowercase().starts_with("/mcp") {
            state.input = format!("/mcp {}", cmd_text);
        } else {
            state.input = format!("/{}", cmd_text);
        }

        update_textarea_from_input(state);
        state.show_command_hints = false;
        state.command_hints.clear();
        return true;
    }

    false
}

/// 接受当前选中的 @ 文件提示。
/// 目录：替换为 `dir/` 并立即刷新提示（下钻）；文件：替换 + 空格并关闭提示。
fn accept_mention_hint(state: &mut ChatState) {
    let selected = state.mention_hints[state.selected_mention_hint].clone();
    let display = extract_mention_path(&selected).to_string();
    if display.is_empty() {
        return;
    }
    let is_dir = display.ends_with('/');
    let at_input = last_at_token_start(&state.input).is_some();

    if at_input {
        replace_at_token_with_path(&mut state.input, &display, is_dir);
        update_textarea_from_input(state);
        if is_dir {
            // 下钻：保持提示打开并刷新子项列表
            state.selected_mention_hint = 0;
            refresh_mention_hints(state);
        } else {
            state.show_mention_hints = false;
            state.mention_hints.clear();
        }
    } else {
        // 裸路径补全（无 @ 前缀）
        let trimmed = display.trim_end_matches('/').to_string();
        state.input = trimmed;
        state.input.push(' ');
        update_textarea_from_input(state);
        state.show_mention_hints = false;
        state.mention_hints.clear();
    }
}

fn refresh_mention_hints(state: &mut ChatState) {
    // @ token（含 @" 引号形式）优先：token 可跨空格
    if let Some((at, quoted)) = last_at_token_start(&state.input) {
        let frag = at_token_frag(&state.input, at, quoted).trim();
        let hints = crate::ui::services::file_search::search_files(frag);
        if hints.is_empty() {
            // 无匹配：关闭弹窗，不提供可选中的占位行
            state.show_mention_hints = false;
            state.mention_hints.clear();
        } else {
            state.show_mention_hints = true;
            state.mention_hints = hints;
            state.selected_mention_hint = 0;
            state.show_command_hints = false;
        }
        return;
    }

    let token_start = state
        .input
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    let token = state.input.get(token_start..).unwrap_or("");
    let t = token.trim();

    // 修复：避免将以 / 开头的命令（位于输入开头）误判为文件路径
    // 如果 token 位于开头且以 / 起始，优先视为命令
    let is_command_candidate = token_start == 0 && t.starts_with('/');

    // 优化：仅在明确可能是文件路径时触发（包含路径分隔符或以.开头）
    if !t.is_empty()
        && !is_command_candidate
        && (t.contains('/') || t.contains('\\') || t.starts_with('.'))
    {
        let hints = crate::ui::services::file_search::search_files(t);
        if hints.is_empty() {
            state.show_mention_hints = false;
            state.mention_hints.clear();
        } else {
            state.show_mention_hints = true;
            state.mention_hints = hints;
            state.selected_mention_hint = 0;
            state.show_command_hints = false;
        }
        return;
    }

    state.show_mention_hints = false;
    state.mention_hints.clear();
}
