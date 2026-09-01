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
                // Check for Thinking Process Toggle
                let mut toggled = false;
                if let Some(entry) = state.chat_history.get(entry_idx) {
                    if let Some(reasoning) = &entry.reasoning_content {
                        if !reasoning.is_empty() {
                            let is_expanded = state.expanded_thinking_indices.contains(&entry_idx);

                            // 点击 thinking 块任意位置都可以切换展开/折叠
                            // 展开时：点击任意行折叠
                            // 折叠时：点击标题或预览行展开
                            let should_toggle = if is_expanded {
                                true // 展开时点击任意位置都折叠
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
                                toggled = true;
                            }
                        }
                    }
                }

                // Check for Tool expand/collapse toggle
                if !toggled {
                    if let Some(entry) = state.chat_history.get(entry_idx) {
                        if let Some(tc) = &entry.tool_call {
                            // Click on header row toggles tool expansion
                            if row == 0 {
                                if state.expanded_tool_call_ids.contains(&tc.id) {
                                    state.expanded_tool_call_ids.remove(&tc.id);
                                } else {
                                    state.expanded_tool_call_ids.insert(tc.id.clone());
                                }
                                state.rendered_cache.remove(&entry_idx);
                                toggled = true;
                            }
                        }
                    }
                }

                if !toggled {
                    state.text_selection.start_selection(entry_idx, row, col);
                }
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

            if let Some((entry_idx, r, c)) = map_mouse_to_chat_position(state, col, row) {
                state.text_selection.update_selection(entry_idx, r, c);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if m.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                return;
            }
            // 如果有选中文本，自动复制到剪贴板
            if state.text_selection.has_selection() {
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
