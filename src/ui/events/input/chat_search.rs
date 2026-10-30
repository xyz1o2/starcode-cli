//! 会话内搜索（Ctrl+F）：在已经发生过的对话里找关键词。
//!
use super::*;

/// 打开对话内搜索面板（Ctrl+F）。
///
/// 搜的是**已经渲染出来的对话行**，不是 `chat_history` 里的结构化条目 ——
/// 用户记得的是"屏幕上那行写着 `cargo test`"，让他按渲染行去搜才对得上。
/// 命中行号落在 `ChatSearchState::matches` 里，与滚动/高亮共用一套坐标。
pub fn open_chat_search(state: &mut ChatState) {
    let scroll = state.scroll;
    state.chat_search.open(scroll);
    rerun_chat_search(state);
    // 面板是叠加层，不抢输入焦点：主输入框保持可打字
    state.auto_follow = false;
}

/// 关闭面板并回到打开前的滚动位置。
pub fn close_chat_search(state: &mut ChatState) {
    let restore = state.chat_search.restore_scroll;
    state.chat_search.close();
    let max_scroll = state
        .total_rendered_lines
        .saturating_sub(state.last_chat_height as usize);
    state.scroll = restore.min(max_scroll);
    // 只有原本就贴着底部才恢复跟随，否则会把用户正在读的历史拽走
    state.auto_follow = restore >= max_scroll;
}

/// 用当前 query 重跑搜索，并跳到视口内的第一个命中。
pub(super) fn rerun_chat_search(state: &mut ChatState) {
    let haystack = chat_search_haystack(state);
    state.chat_search.run(&haystack);

    let height = state.last_chat_height as usize;
    let top = state.scroll;
    let bottom = top + height;
    let viewport = |row: usize| row >= top && row < bottom;
    // 视口内没有命中时跳第一个，否则跳第一个落在视口里的
    let target = state
        .chat_search
        .matches
        .iter()
        .position(|row| viewport(*row))
        .unwrap_or(0);
    state.chat_search.selected = target;
    reveal_chat_search_match(state);
}

/// 把命中行滚进视口中央。
pub(super) fn reveal_chat_search_match(state: &mut ChatState) {
    let Some(row) = state.chat_search.current_match() else {
        return;
    };
    let height = state.last_chat_height as usize;
    if height == 0 {
        return;
    }
    state.scroll = row.saturating_sub(height / 3);
    let max_scroll = state.total_rendered_lines.saturating_sub(height);
    state.scroll = state.scroll.min(max_scroll);
    state.auto_follow = false;
}

/// 搜索用的逐行文本：与 `render_chat_lines` 的输出逐行对齐。
///
/// 必须与渲染同源，否则命中行号会整体错位（用户跳过去看到的是隔壁那条）。
/// 因此这里**按 entry 下标顺序**遍历 `rendered_cache` —— 它是 `HashMap`，
/// 直接 `.values()` 拿到的是任意顺序，命中会全落到错误的行上。
pub(super) fn chat_search_haystack(state: &ChatState) -> Vec<String> {
    let mut lines = Vec::with_capacity(state.total_rendered_lines);
    for idx in 0..state.chat_history.len() {
        // 缓存里没有的条目（尚未渲染过）当空行跳过：它们的行数在缓存缺失时不可知，
        // 猜一个高度只会让后面的行号全错。
        let Some((_, entry_lines)) = state.rendered_cache.get(&idx) else {
            continue;
        };
        for line in entry_lines {
            lines.push(
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>(),
            );
        }
    }
    lines
}

/// 搜索面板的按键处理。返回 `true` 表示这个键已被面板消费；`false` 表示应当
/// 继续走后面的处理层（目前只有 `Ctrl+Shift+F`：先收面板，再开全局文件搜索）。
///
/// 面板打开期间吞掉除下面这些键以外的一切 —— 漏一个就会表现为"打一个字进了主输入框"。
pub(super) fn handle_chat_search_key(state: &mut ChatState, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            close_chat_search(state);
            true
        }
        // Ctrl+F 再按一次关掉自己；带 Shift 时顺手让全局文件搜索接手
        KeyCode::Char('f') if ctrl => {
            close_chat_search(state);
            !key.modifiers.contains(KeyModifiers::SHIFT)
        }
        KeyCode::Enter => {
            if state.chat_search.step(true).is_some() {
                reveal_chat_search_match(state);
            }
            true
        }
        KeyCode::Backspace => {
            if state.chat_search.cursor > 0 {
                let cursor = state.chat_search.cursor - 1;
                state.chat_search.query.remove(cursor);
                state.chat_search.cursor = cursor;
                // Ctrl+Backspace 按词退格：连着退到分隔符之前（与编辑器一致）
                if ctrl {
                    let head = state.chat_search.query[..cursor].to_string();
                    let trimmed = head.trim_end();
                    let removed = head.len() - trimmed.len();
                    state.chat_search.query.truncate(cursor - removed);
                    state.chat_search.cursor = cursor - removed;
                }
                rerun_chat_search(state);
            }
            true
        }
        KeyCode::Char(c) => {
            state.chat_search.query.insert(state.chat_search.cursor, c);
            state.chat_search.cursor += 1;
            rerun_chat_search(state);
            true
        }
        KeyCode::Tab | KeyCode::Down => {
            if state.chat_search.step(true).is_some() {
                reveal_chat_search_match(state);
            }
            true
        }
        KeyCode::Up => {
            state.chat_search.step(false);
            reveal_chat_search_match(state);
            true
        }
        _ => true,
    }
}
