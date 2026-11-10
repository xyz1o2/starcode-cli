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
///
/// 触发规则：输入里的**每一个** `@` 都算（取最后一个）。发送侧
/// `at_processor::find_next_unescaped_at` 就是不管前缀直接解析所有 `@` 的，
/// UI 若还要求「@ 前面必须是空白」，`看看@src` 这类紧贴文字的引用就会
/// 发送时被解析、输入时却弹不出提示 —— 两侧必须同一条规则。
fn last_at_token_start(input: &str) -> Option<(usize, bool)> {
    let at = input.rfind('@')?;
    let bytes = input.as_bytes();
    let quoted = bytes.get(at + 1) == Some(&b'"');
    if quoted {
        // 引号形式：闭合引号出现即 token 已终结，不再激活 —— 否则引用完
        // 一个带空格的文件继续打字，弹窗会每敲一个字重新冒出来。
        if input[at + 2..].contains('"') {
            return None;
        }
    } else if input[at + 1..].contains(char::is_whitespace) {
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
/// is_dir 为真时以 `/` 结尾（继续敲 `/` 前的定稿形态），不补空格；文件补空格。
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
/// 目录和文件一视同仁：替换为路径并关闭提示。目录不补空格也不下钻 ——
/// 「选文件夹」就该只选中文件夹本身；要下钻继续敲 `/`（`@src/` 会重新
/// 列出子项）。之前 Enter 选目录会自动展开子项，用户想引用整个目录却
/// 被迫在弹窗里继续选，感觉是「选文件夹会连带」。
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
    }
    // 没有 @ token 时提示不可能打开（refresh_mention_hints 只认 @ token），
    // 到这里就是外部直接改了状态，稳妥收场即可。
    state.show_mention_hints = false;
    state.mention_hints.clear();
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

    // 走到这里说明没有活跃的 @ token：不再让「末尾 token 恰好含 / 或以 . 开头」
    // 触发目录弹窗 —— 之前裸 token 回退会在任意段落的词中间敲一个 `/`（甚至就是
    // 裸 `/`）时把当前词当路径去 search_files，裸 `/` 更是列出文件系统根目录的
    // 子项，表现为「打到一半突然吐出一个目录列表」。Claude Code 也只有 @ 触发。
    state.show_mention_hints = false;
    state.mention_hints.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::ChatState;

    /// 回归：任意段落中的裸 `/` 不得再弹出目录列表。
    ///
    /// 之前 `refresh_mention_hints` 的裸 token 回退会把末尾含 `/` 的词送进
    /// `search_files`，裸 `/` 经 `search_children` 列出的是文件系统根目录。
    #[test]
    fn mid_text_slash_does_not_open_mention_hints() {
        let mut state = ChatState::new();
        for input in ["用 /mode 切换模式", "mkdir dir/sub /", "路径 a /", "/"] {
            state.input = input.to_string();
            on_input_changed(&mut state);
            assert!(
                !state.show_mention_hints,
                "输入 {input:?} 不该弹目录提示，实际弹出了 {:?}",
                state.mention_hints
            );
        }
    }

    /// `@` 仍然在任意段落可用。
    #[test]
    fn at_token_mid_paragraph_still_opens() {
        let mut state = ChatState::new();
        state.input = "看看 @Cargo".to_string();
        on_input_changed(&mut state);
        assert!(state.show_mention_hints, "段落中的 @ 要能打开提示");
        assert!(
            state.mention_hints.iter().any(|h| h.contains("Cargo.toml")),
            "应命中 Cargo.toml，实际 {:?}",
            state.mention_hints
        );
    }

    /// 目录下钻：`@src/` 列出该目录的直接子项。
    #[test]
    fn at_dir_drills_into_children() {
        let mut state = ChatState::new();
        state.input = "见 @src/".to_string();
        on_input_changed(&mut state);
        assert!(
            state.show_mention_hints && !state.mention_hints.is_empty(),
            "@src/ 应列出子项"
        );
        assert!(
            state.mention_hints.iter().all(|h| h.starts_with("src/")),
            "子项都应带 src/ 前缀，实际 {:?}",
            state.mention_hints
        );
    }

    /// 紧贴文字的 `@`（前面没有空白）也要能触发 —— 发送侧
    /// `at_processor` 解析所有 `@`，UI 触发规则必须一致。
    #[test]
    fn at_attached_to_word_still_opens() {
        let mut state = ChatState::new();
        state.input = "看看@Cargo".to_string();
        on_input_changed(&mut state);
        assert!(state.show_mention_hints, "紧贴文字的 @ 要能打开提示");
        assert!(
            state.mention_hints.iter().any(|h| h.contains("Cargo.toml")),
            "应命中 Cargo.toml，实际 {:?}",
            state.mention_hints
        );
    }

    /// 选中目录 = 只选中目录本身：Enter 后提示关闭、token 定稿为 `@dir/`，
    /// 不自动展开子项（要下钻继续敲 `/`）。
    #[test]
    fn accepting_a_dir_selects_the_dir_only() {
        let mut state = ChatState::new();
        state.input = "见 @src".to_string();
        on_input_changed(&mut state);
        assert!(state.show_mention_hints, "@src 应列出候选");
        let dir_idx = state
            .mention_hints
            .iter()
            .position(|h| h.ends_with('/') && h.starts_with("src"))
            .unwrap_or_else(|| panic!("候选里应有 src/ 目录，实际 {:?}", state.mention_hints));
        state.selected_mention_hint = dir_idx;
        assert!(handle_enter(&mut state), "Enter 应被采纳");
        assert!(!state.show_mention_hints, "选中目录后提示应关闭");
        assert!(
            state.input.contains("@src/"),
            "输入应定稿为 @src/，实际 {:?}",
            state.input
        );
        // 定稿后继续敲 `/` 前的形态不会再弹提示（token 已被后续输入终结的
        // 场景由 last_at_token_start 的空白规则处理；`@src/` 尾斜杠触发下钻）。
        on_input_changed(&mut state);
        assert!(state.show_mention_hints, "@src/ 尾斜杠形态应保持下钻能力");
    }

    /// 引号形式闭合后 token 终结：继续打字不得再弹提示（乱弹回归）。
    #[test]
    fn closed_quoted_token_does_not_reopen_hints() {
        let mut state = ChatState::new();
        state.input = "读 @\"my file.txt\" 然后总结".to_string();
        on_input_changed(&mut state);
        assert!(
            !state.show_mention_hints,
            "闭合的 @\"...\" 不该再激活提示，实际 {:?}",
            state.mention_hints
        );
        // 未闭合的引号形式仍激活（用真实存在的路径，否则是「无匹配关闭」而非「token 终结」）
        state.input = "读 @\"src".to_string();
        on_input_changed(&mut state);
        assert!(state.show_mention_hints, "未闭合的 @\" 仍应激活提示");
    }

    /// 输入开头的 `/` 命令提示不受影响。
    #[test]
    fn leading_slash_still_shows_command_hints() {
        let mut state = ChatState::new();
        state.input = "/he".to_string();
        on_input_changed(&mut state);
        assert!(state.show_command_hints, "开头的 / 要出命令提示");
        assert!(!state.show_mention_hints);
    }
}
