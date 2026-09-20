use crate::ui::state::ChatState;

use arboard::Clipboard;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

pub fn scroll_chat_by_lines(state: &mut ChatState, delta_lines: i32) {
    crate::ui::state::scroll_chat(state, delta_lines);
}

fn map_mouse_to_chat_position(
    state: &ChatState,
    mouse_col: u16,
    mouse_row: u16,
) -> Option<(usize, usize, usize)> {
    let area = state.last_chat_area?;

    if mouse_col < area.x
        || mouse_col >= area.x.saturating_add(area.width)
        || mouse_row < area.y
        || mouse_row >= area.y.saturating_add(area.height)
    {
        return None;
    }

    let rel_x = mouse_col.saturating_sub(area.x) as usize;
    let rel_y = mouse_row.saturating_sub(area.y) as usize;

    // Absolute line index in the virtual document
    let abs_line_idx = state.scroll.saturating_add(rel_y);

    if abs_line_idx >= state.total_rendered_lines {
        return None;
    }

    // Find which entry contains this line
    let mut current_line = 0;
    for (entry_idx, &height) in state.last_item_heights.iter().enumerate() {
        let h = height as usize;
        let next_boundary = current_line + h;

        if abs_line_idx < next_boundary {
            let row_in_item = abs_line_idx - current_line;
            return Some((entry_idx, row_in_item, rel_x));
        }

        current_line = next_boundary;
    }

    None
}

/// 把屏幕坐标换算成选区终点并写入（Drag 与 Up 共用同一套映射与钳制规则）。
///
/// 坐标调用方负责先钳进聊天区。落在内容末尾之下时 `map_mouse_to_chat_position`
/// 会因超出总行数返回 None，此时钳到最后一行，让选区跟随到底而不是冻结在原地。
fn move_selection_end_to(state: &mut ChatState, col: u16, row: u16) {
    if let Some((entry_idx, r, c)) = map_mouse_to_chat_position(state, col, row) {
        state.text_selection.update_selection(entry_idx, r, c);
    } else if let Some((last_idx, &last_h)) = state
        .last_item_heights
        .iter()
        .enumerate()
        .rev()
        .find(|(_, &h)| h > 0)
    {
        let rel_x = state
            .last_chat_area
            .map(|a| col.saturating_sub(a.x) as usize)
            .unwrap_or(0);
        state
            .text_selection
            .update_selection(last_idx, last_h as usize - 1, rel_x);
    }
}

/// 单击（按下到松开未拖动）时的折叠/展开切换，返回是否发生了切换。
/// 判定规则与旧版「Down 即切换」完全一致：
/// - thinking 块：展开时点任意行折叠；折叠时点头部或预览行展开
/// - 工具块：仅点头部行（row 0）切换
fn try_toggle_on_click(state: &mut ChatState, entry_idx: usize, row: usize) -> bool {
    let Some(entry) = state.chat_history.get(entry_idx) else {
        return false;
    };

    if let Some(reasoning) = &entry.reasoning_content {
        if !reasoning.is_empty() {
            let is_expanded = state.expanded_thinking_indices.contains(&entry_idx);
            let should_toggle = if is_expanded {
                true
            } else {
                let preview_lines = reasoning.lines().take(3).count();
                row <= preview_lines
            };
            if should_toggle {
                if is_expanded {
                    state.expanded_thinking_indices.remove(&entry_idx);
                } else {
                    state.expanded_thinking_indices.insert(entry_idx);
                }
                state.rendered_cache.remove(&entry_idx);
                return true;
            }
            return false;
        }
    }

    if let Some(tc) = &entry.tool_call {
        if row == 0 {
            if state.expanded_tool_call_ids.contains(&tc.id) {
                state.expanded_tool_call_ids.remove(&tc.id);
            } else {
                state.expanded_tool_call_ids.insert(tc.id.clone());
            }
            state.rendered_cache.remove(&entry_idx);
            return true;
        }
    }

    false
}

pub fn handle_mouse_event(state: &mut ChatState, m: MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollUp => {
            // 检查鼠标是否在 input 区域
            if let Some(input_area) = state.last_input_area {
                if m.column >= input_area.x
                    && m.column < input_area.x + input_area.width
                    && m.row >= input_area.y
                    && m.row < input_area.y + input_area.height
                {
                    // 鼠标在 input 区域，不处理滚动事件（避免 textarea 内部滚动）
                    return;
                }
            }
            // Scroll up by 3 lines (standard mouse wheel increment)
            let speed_multiplier = if m
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
            {
                5 // Turbo mode with Ctrl
            } else {
                1
            };

            scroll_chat_by_lines(state, -3 * speed_multiplier);
        }
        MouseEventKind::ScrollDown => {
            // 检查鼠标是否在 input 区域
            if let Some(input_area) = state.last_input_area {
                if m.column >= input_area.x
                    && m.column < input_area.x + input_area.width
                    && m.row >= input_area.y
                    && m.row < input_area.y + input_area.height
                {
                    // 鼠标在 input 区域，不处理滚动事件（避免 textarea 内部滚动）
                    return;
                }
            }
            // Scroll down by 3 lines (standard mouse wheel increment)
            let speed_multiplier = if m
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
            {
                5 // Turbo mode with Ctrl
            } else {
                1
            };

            scroll_chat_by_lines(state, 3 * speed_multiplier);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Shift+Click bypasses TUI mouse capture to allow native terminal text selection
            if m.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                return;
            }
            if let Some((entry_idx, row, col)) = map_mouse_to_chat_position(state, m.column, m.row)
            {
                // 一律先起选区。thinking / 工具头部的折叠切换延迟到松开且未拖动时
                // 才执行（见 Up 分支的 try_toggle_on_click）。此前 Down 即切换：
                // 1) 按在 thinking 标签上想拖选正文时选区永远起不来；
                // 2) 按下瞬间折叠/展开内容、行号整体位移，正在显示的选区错位，
                //    表现为“拖选总是断”。
                state.text_selection.start_selection(entry_idx, row, col);
            } else {
                state.text_selection.clear();
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if m.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                return;
            }
            // Auto-scroll logic when dragging outside viewport (velocity-based)
            let mut _scrolled = false;
            if let Some(area) = state.last_chat_area {
                if m.row < area.y {
                    let dist = area.y.saturating_sub(m.row) as i32;
                    let speed = (dist / 2 + 1).min(8);
                    scroll_chat_by_lines(state, -speed);
                    _scrolled = true;
                } else if m.row >= area.y + area.height {
                    let dist = m.row.saturating_sub(area.y + area.height.saturating_sub(1)) as i32;
                    let speed = (dist / 2 + 1).min(8);
                    scroll_chat_by_lines(state, speed);
                    _scrolled = true;
                }
            }

            // Map mouse position (clamped to viewport if necessary)
            let (mut col, mut row) = (m.column, m.row);
            if let Some(area) = state.last_chat_area {
                col = col.max(area.x).min(area.x + area.width.saturating_sub(1));
                row = row.max(area.y).min(area.y + area.height.saturating_sub(1));
            }

            move_selection_end_to(state, col, row);
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if m.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                return;
            }
            // 先用松开位置补一次选区终点，再判定单击/拖选。
            // 终端经常不上报、或在上报途中丢失中间的 Drag 事件（tmux 转发、
            // 非 SGR 鼠标模式、事件被节流/合并），此时终点还停在按下位置，
            // 下面 start==end 的单击判定会把真正的拖选误判成单击并清掉选区——
            // 表现就是「左键拖选永远选不中」。松开在聊天区之外（输入框、状态栏）
            // 时不外推，保留拖动期间已记下的终点。
            if state.text_selection.is_selecting {
                let inside_chat = match state.last_chat_area {
                    Some(area) => {
                        m.column >= area.x
                            && m.column < area.x + area.width
                            && m.row >= area.y
                            && m.row < area.y + area.height
                    }
                    None => true,
                };
                if inside_chat {
                    move_selection_end_to(state, m.column, m.row);
                }
            }
            // 未拖动 = 单击（起止锚点重合）：执行折叠/展开切换，并清掉单格
            // 选区高亮。切换会改变条目行数，残留锚点会指向错误的行。
            let clicked = match (
                state.text_selection.start_entry_idx,
                state.text_selection.end_entry_idx,
                state.text_selection.start,
                state.text_selection.end,
            ) {
                (Some(se), Some(ee), Some(s), Some(e)) if se == ee && s == e => Some((se, s.0)),
                _ => None,
            };

            if let Some((entry_idx, row)) = clicked {
                let _toggled = try_toggle_on_click(state, entry_idx, row);
                state.text_selection.clear();
            } else if state.text_selection.has_selection() {
                // 如果有选中文本，自动复制到剪贴板
                if let Some(selected_text) = state.get_selected_text() {
                    match Clipboard::new() {
                        Ok(mut clipboard) => {
                            if let Err(e) = clipboard.set_text(&selected_text) {
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[COPY] clipboard set_text failed: {}",
                                    e
                                ));
                            }
                        }
                        Err(e) => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[COPY] clipboard init failed: {}",
                                e
                            ));
                        }
                    }
                }
            }
            state.text_selection.end_selection();
        }
        MouseEventKind::Down(MouseButton::Right) => {
            state.text_selection.clear();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::ChatState;
    use ratatui::layout::Rect;

    fn mouse_event(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::empty(),
        }
    }

    /// ChatState::new() 自带欢迎条目（高 1 行），思考条目在其后（3 行）。
    /// 思考条目占文档第 1..=3 行。
    fn setup_state() -> ChatState {
        let mut state = ChatState::new();
        let mut entry = crate::types::ChatEntry::assistant("");
        entry.reasoning_content = Some("first step\n\nsecond step".to_string());
        state.chat_history.push(entry);
        state.last_chat_area = Some(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        });
        state.last_item_heights = vec![1, 3];
        state.total_rendered_lines = 4;
        state
    }

    /// 单击 thinking 头部：Down 只起选区不切换；Up（未拖动）才切换。
    /// 此前 Down 即切换会吞掉拖选起点，且按下瞬间行号位移让选区错位。
    #[test]
    fn click_on_thinking_header_toggles_on_up_not_down() {
        let mut state = setup_state();
        let idx = state.chat_history.len() - 1;

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        assert!(
            !state.expanded_thinking_indices.contains(&idx),
            "Down 不得立即切换"
        );
        assert!(state.text_selection.is_selecting, "按下必须能起选区");

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 0, 1),
        );
        assert!(state.expanded_thinking_indices.contains(&idx), "单击应展开");
        assert!(!state.text_selection.is_selecting);
        assert!(
            state.text_selection.start.is_none(),
            "单击后的单格选区必须清掉"
        );
    }

    /// 按住拖动不触发切换，且从 thinking 头部起拖也能正常选区。
    #[test]
    fn drag_from_thinking_header_selects_without_toggling() {
        let mut state = setup_state();
        let idx = state.chat_history.len() - 1;

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 2),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 10, 2),
        );

        assert!(
            state.expanded_thinking_indices.is_empty(),
            "拖动不得触发折叠/展开"
        );
        assert_eq!(state.text_selection.start_entry_idx, Some(idx));
        assert_eq!(state.text_selection.start, Some((0, 0)));
        assert_eq!(state.text_selection.end, Some((1, 10)));
    }

    /// 拖到内容末尾之下（超出总行数）时锚点钳制到最后一行，
    /// 选区跟随到底而不是冻结在原地。
    #[test]
    fn drag_below_content_clamps_to_last_line() {
        let mut state = setup_state();

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 10, 20),
        );

        assert_eq!(state.text_selection.end_entry_idx, Some(1));
        assert_eq!(state.text_selection.end, Some((2, 10)), "钳到最后一行");
    }

    /// 终端只上报 Down/Up、不上报中间 Drag 事件时（tmux 转发、非 SGR 鼠标
    /// 模式、事件被节流），松开位置只要与按下位置不同就必须算拖选。
    /// 此前 Up 只看 Drag 留下的锚点，终点停在按下处 → start==end 被判成
    /// 单击 → 选区被清空，表现为「左键拖选永远选不中」。
    #[test]
    fn drag_without_motion_events_still_selects() {
        let mut state = setup_state();

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        // 注意：完全没有 Drag 事件
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 10, 2),
        );

        assert!(
            state.text_selection.has_selection(),
            "缺 Drag 事件时也必须保留选区"
        );
        assert_eq!(state.text_selection.start, Some((0, 0)));
        assert_eq!(state.text_selection.end, Some((1, 10)), "终点取自松开位置");
        assert!(
            state.expanded_thinking_indices.is_empty(),
            "未拖动判定不得因补终点而误触发折叠/展开"
        );
    }

    /// 松开位置落在聊天区之外（输入框/状态栏）时不外推终点，
    /// 保留拖动期间已记下的位置。
    #[test]
    fn release_outside_chat_keeps_last_drag_end() {
        let mut state = setup_state();

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 5, 2),
        );
        // 松开在第 99 行——远在 24 行高的聊天区之外
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 5, 99),
        );

        assert!(state.text_selection.has_selection());
        assert_eq!(state.text_selection.end, Some((1, 5)), "终点保持拖动时的值");
    }
}
