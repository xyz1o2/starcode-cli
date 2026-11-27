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

    state.plugin_commands.inline_selection_lost = false;
    refresh_command_hints(state);
}

/// 完整命令名后的参数或子命令输入只显示用法，不自动接受候选。
fn is_command_argument_input(state: &ChatState) -> bool {
    state.input.chars().any(char::is_whitespace)
        || crate::commands::system::command_argument_hint_with_plugins(
            &state.input,
            state.plugin_commands.candidates(),
        )
        .is_some()
}

pub(crate) fn refresh_command_hints(state: &mut ChatState) {
    if state.input_intent == crate::types::InputIntent::CommandEligible
        && state.input.starts_with('/')
        && !state.show_mention_hints
        && !should_suppress_hints(state)
    {
        state.show_command_hints = true;
        state.command_hints = if is_command_argument_input(state) {
            Vec::new()
        } else {
            crate::commands::system::get_command_hints_with_plugins(
                &state.input,
                state.plugin_commands.candidates(),
            )
        };
        state.selected_hint = 0;
    } else {
        state.show_command_hints = false;
        state.command_hints.clear();
    }
}

/// 粘贴保护到期本身也是状态变化，必须恢复当时被隐藏的候选并触发重绘。
pub(crate) fn expire_paste_guard(state: &mut ChatState) -> bool {
    if state.pending_clipboard_read.is_some()
        || !state
            .paste_end_time
            .is_some_and(|ended| ended.elapsed() >= Duration::from_millis(PASTE_HINT_GUARD_MS))
    {
        return false;
    }
    state.paste_in_progress = false;
    state.paste_end_time = None;
    on_input_changed(state);
    true
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
        state.plugin_commands.inline_selection_lost = false;
        if state.selected_hint > 0 {
            state.selected_hint -= 1;
        }
        return true;
    }
    false
}

pub fn handle_down(state: &mut ChatState) -> bool {
    if state.show_session_menu {
        let sessions_len = state.available_sessions.len();
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
        state.plugin_commands.inline_selection_lost = false;
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
    textarea.set_cursor_render_mode(tui_textarea::CursorRenderMode::Hidden);
    textarea.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);

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

fn discard_literal_command_hints(state: &mut ChatState) {
    if state.input_intent == crate::types::InputIntent::Literal {
        state.show_command_hints = false;
        state.command_hints.clear();
        state.plugin_commands.inline_selection_lost = false;
    }
}

pub fn handle_enter(state: &mut ChatState) -> bool {
    crate::ui::services::plugin_commands::invalidate_changed_root(state);
    discard_literal_command_hints(state);
    if state.show_session_menu {
        if crate::ui::services::session_list::invalidate_changed_directory(state) {
            return true;
        }
        if let Some(session) = state.available_sessions.get(state.selected_session_index) {
            state.pending_palette_action = Some(crate::ui::state::PaletteAction::ExecuteCommand(
                format!("/chat resume {}", session.id),
            ));
            state.close_quick_menus();
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

    // 参数提示不参与接受动作，完整命令必须原样交给提交分流。
    if state.show_command_hints && is_command_argument_input(state) {
        state.plugin_commands.inline_selection_lost = false;
        return false;
    }
    if state.show_command_hints && state.plugin_commands.inline_selection_lost {
        state.plugin_commands.inline_selection_lost = false;
        return true;
    }
    if state.show_command_hints && !state.command_hints.is_empty() {
        let Some(selected) = state.command_hints.get(state.selected_hint) else {
            return true;
        };
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
        on_input_changed(state);
        return true;
    }

    false
}

pub fn handle_tab(state: &mut ChatState) -> bool {
    crate::ui::services::plugin_commands::invalidate_changed_root(state);
    discard_literal_command_hints(state);
    if state.show_mention_hints && !state.mention_hints.is_empty() {
        accept_mention_hint(state);
        return true;
    }

    let argument_input = is_command_argument_input(state);
    if state.show_command_hints && argument_input {
        state.plugin_commands.inline_selection_lost = false;
    } else if state.show_command_hints && state.plugin_commands.inline_selection_lost {
        state.plugin_commands.inline_selection_lost = false;
        return true;
    }
    // 参数阶段只在显式 Tab 时查询候选，不把子命令放进默认选中列表。
    let explicit_hints = if state.show_command_hints && argument_input {
        crate::commands::system::get_command_hints_with_plugins(
            &state.input,
            state.plugin_commands.candidates(),
        )
    } else {
        Vec::new()
    };
    let hints = if argument_input {
        &explicit_hints
    } else {
        &state.command_hints
    };
    if state.show_command_hints && !hints.is_empty() {
        let index = if argument_input {
            0
        } else {
            state.selected_hint
        };
        let Some(selected) = hints.get(index) else {
            return true;
        };
        let cmd_text = extract_hint_primary_text(selected);

        if cmd_text.starts_with('/') {
            state.input = cmd_text.to_string();
        } else if state.input.to_lowercase().starts_with("/mcp") {
            state.input = format!("/mcp {}", cmd_text);
        } else {
            state.input = format!("/{}", cmd_text);
        }

        update_textarea_from_input(state);
        on_input_changed(state);
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

    #[test]
    fn literal_input_does_not_offer_or_accept_command_completions() {
        for stale_hints in [false, true] {
            for use_tab in [false, true] {
                let mut state = ChatState::test_fixture();
                state.set_user_input("/resum".into());
                if stale_hints {
                    state.show_command_hints = true;
                    state.command_hints = vec!["/resume - Resume a session".into()];
                } else {
                    on_input_changed(&mut state);
                    assert!(!state.show_command_hints);
                    assert!(state.command_hints.is_empty());
                }
                let accepted = if use_tab {
                    handle_tab(&mut state)
                } else {
                    handle_enter(&mut state)
                };
                assert!(
                    !accepted,
                    "literal input must not consume command completion"
                );
                assert_eq!(state.input, "/resum");
                assert_eq!(state.textarea.lines().join("\n"), "/resum");
                assert_eq!(state.input_intent, crate::types::InputIntent::Literal);
                assert!(!state.show_command_hints);
                assert!(state.command_hints.is_empty());
            }
        }
    }

    #[test]
    fn deleting_literal_prompt_restores_completion_only_after_full_clear() {
        let mut state = ChatState::test_fixture();
        state.set_user_input("/resumX".into());
        state.textarea.delete_char();
        crate::ui::events::clipboard_paste::sync_input_from_textarea(&mut state);
        on_input_changed(&mut state);
        assert_eq!(state.input, "/resum");
        assert_eq!(state.input_intent, crate::types::InputIntent::Literal);
        assert!(!state.show_command_hints);
        assert!(state.command_hints.is_empty());
        state.textarea.select_all();
        state.textarea.cut();
        crate::ui::events::clipboard_paste::sync_input_from_textarea(&mut state);
        assert_eq!(
            state.input_intent,
            crate::types::InputIntent::CommandEligible
        );
        assert!(
            !state.textarea.undo(),
            "cleared literal text must not return as a command"
        );
        state.textarea.insert_str("/resum");
        crate::ui::events::clipboard_paste::sync_input_from_textarea(&mut state);
        on_input_changed(&mut state);
        assert!(state.show_command_hints);
        assert!(handle_tab(&mut state));
        assert_eq!(state.input, "/resume");
    }

    #[test]
    fn literal_input_still_offers_file_mentions() {
        let mut state = ChatState::test_fixture();
        state.set_user_input("See @Cargo".into());
        on_input_changed(&mut state);
        assert!(state.show_mention_hints);
        assert!(state
            .mention_hints
            .iter()
            .any(|hint| hint.contains("Cargo.toml")));
    }

    #[test]
    fn unmatched_command_completion_preserves_input_on_enter_and_tab() {
        for input in ["/zzzzzzzzzz", "/mcp zzzzzzzzzz"] {
            for use_tab in [false, true] {
                let mut state = ChatState::test_fixture();
                state.set_input_text(input);
                on_input_changed(&mut state);
                let accepted = if use_tab {
                    handle_tab(&mut state)
                } else {
                    handle_enter(&mut state)
                };
                assert!(
                    !accepted,
                    "unknown input must not accept an unrelated completion: {input}, tab={use_tab}"
                );
                assert_eq!(state.input, input, "tab={use_tab}");
                assert_eq!(state.textarea.lines().join("\n"), input, "tab={use_tab}");
                assert!(state.command_hints.is_empty(), "input={input}");
            }
        }
    }

    #[test]
    fn empty_command_queries_still_offer_completions() {
        for input in ["/", "/mc"] {
            let mut state = ChatState::test_fixture();
            state.set_input_text(input);
            on_input_changed(&mut state);
            assert!(state.show_command_hints, "input={input}");
            assert!(!state.command_hints.is_empty(), "input={input}");
            let palette = crate::commands::system::command_palette_items(input);
            assert!(
                palette.iter().all(|item| item.label.starts_with('/')),
                "completion entries must remain slash commands: {input}"
            );
            if input == "/mc" {
                assert!(palette.iter().any(|item| item.label == "/mcp"));
            }
        }

        let mut state = ChatState::test_fixture();
        state.set_input_text("/mcp ");
        on_input_changed(&mut state);
        assert!(state.show_command_hints);
        assert!(state.command_hints.is_empty());
        assert!(!crate::commands::system::get_command_hints("/mcp ").is_empty());
    }

    #[test]
    fn complete_builtin_names_and_aliases_never_require_accepting_arguments() {
        let mut state = ChatState::test_fixture();
        for command in crate::commands::system::ALL_COMMANDS {
            for name in std::iter::once(command.name).chain(command.alt_names.iter().copied()) {
                for suffix in ["", " ", " 分析项目"] {
                    let input = format!("/{name}{suffix}");
                    state.set_input_text(&input);
                    on_input_changed(&mut state);
                    assert!(state.show_command_hints, "input={input}");
                    assert!(state.command_hints.is_empty(), "input={input}");
                    assert!(!handle_up(&mut state), "input={input}");
                    assert!(!handle_down(&mut state), "input={input}");
                    // 后台刷新遗留的选择失效标记也不能拦截完整命令的提交。
                    state.plugin_commands.inline_selection_lost = true;
                    assert!(!handle_enter(&mut state), "input={input}");
                    assert_eq!(state.input, input);
                    assert_eq!(state.textarea.lines().join("\n"), input);
                }
            }
        }
    }

    #[test]
    fn explicit_tab_completes_subcommands_but_enter_keeps_partial_arguments() {
        for (input, completed) in [
            ("/loop ", "/loop add"),
            ("/loop a", "/loop add"),
            ("/chat ", "/chat save"),
            ("/chat sa", "/chat save"),
            ("/remember ad", "/remember add"),
            ("/model us", "/model use"),
        ] {
            let mut state = ChatState::test_fixture();
            state.set_input_text(input);
            on_input_changed(&mut state);
            assert!(state.command_hints.is_empty(), "input={input}");
            assert!(!handle_enter(&mut state), "input={input}");
            assert_eq!(state.input, input);
            assert!(handle_tab(&mut state), "input={input}");
            assert_eq!(state.input, completed);
            assert_eq!(state.textarea.lines().join("\n"), completed);
            assert!(state.command_hints.is_empty());
            assert!(!handle_enter(&mut state));
        }
    }

    #[test]
    fn complete_plugin_commands_keep_arguments_and_do_not_consume_enter() {
        let mut state = ChatState::test_fixture();
        state.plugin_commands.seed_for_tests(vec![
            crate::commands::system::PluginCommandCandidate {
                name: "fixture-check".into(),
                description: "Inspect fixture metadata".into(),
            },
        ]);
        for input in [
            "/fixture-check",
            "/fixture-check ",
            "/fixture-check 分析项目",
        ] {
            state.set_input_text(input);
            on_input_changed(&mut state);
            assert!(state.command_hints.is_empty());
            assert!(!handle_enter(&mut state));
            assert_eq!(state.input, input);
            assert_eq!(state.textarea.lines().join("\n"), input);
        }
    }

    #[test]
    fn unmatched_command_queries_leave_palette_empty() {
        for input in ["/zzzzzzzzzz", "/mcp zzzzzzzzzz"] {
            assert!(
                crate::commands::system::command_palette_items(input).is_empty(),
                "unknown input must not offer unrelated palette entries: {input}"
            );
        }
    }

    #[test]
    fn accepting_command_completion_keeps_usage_without_consuming_submit() {
        for use_tab in [false, true] {
            let mut state = ChatState::test_fixture();
            state.input = "/resum".into();
            on_input_changed(&mut state);
            state.selected_hint = state
                .command_hints
                .iter()
                .position(|hint| hint.starts_with("/resume - "))
                .unwrap();
            assert!(if use_tab {
                handle_tab(&mut state)
            } else {
                handle_enter(&mut state)
            });
            assert_eq!(state.input.trim(), "/resume");
            assert!(state.show_command_hints);
            assert!(state.command_hints.is_empty());
            assert!(crate::commands::system::command_argument_hint(&state.input).is_some());
            assert!(
                !handle_enter(&mut state),
                "next Enter must reach command dispatch"
            );
        }
    }

    /// 已取回的剪贴板结果同样受短保护；仍在读取时不能提前解锁。
    #[test]
    fn paste_guard_waits_for_reader_and_refreshes_completed_paste() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("/");
        state.paste_end_time = Some(std::time::Instant::now() - Duration::from_secs(1));
        let (_tx, result) = tokio::sync::oneshot::channel();
        state.pending_clipboard_read = Some(crate::ui::state::PendingClipboardRead {
            target: crate::ui::state::ClipboardTarget::Prompt {
                session_id: state.active_session_id.clone(),
                session_started_at: state.session_started_at,
                next_message_id: state.next_message_id,
            },
            result,
        });
        assert!(!expire_paste_guard(&mut state));
        state.pending_clipboard_read = None;
        assert!(!state.paste_in_progress);
        assert!(expire_paste_guard(&mut state));
        assert!(state.show_command_hints);
        assert!(!state.command_hints.is_empty());
        assert!(!expire_paste_guard(&mut state));
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
