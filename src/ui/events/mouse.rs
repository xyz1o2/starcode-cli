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

/// 拖选的每一步：视口外自动滚屏 + 钳进视口 + 更新选区终点。
///
/// `Drag(Left)` 与拖选中的 `Moved` 共用这一条路径——后者是同一动作在不支持
/// SGR 鼠标模式（`?1006h`）的终端上的另一种编码（见 `Moved` 分支）。
fn drag_to(state: &mut ChatState, col: u16, row: u16) {
    // Auto-scroll logic when dragging outside viewport (velocity-based)
    if let Some(area) = state.last_chat_area {
        if row < area.y {
            let dist = area.y.saturating_sub(row) as i32;
            let speed = (dist / 2 + 1).min(8);
            scroll_chat_by_lines(state, -speed);
        } else if row >= area.y + area.height {
            let dist = row.saturating_sub(area.y + area.height.saturating_sub(1)) as i32;
            let speed = (dist / 2 + 1).min(8);
            scroll_chat_by_lines(state, speed);
        }
    }

    // Map mouse position (clamped to viewport if necessary)
    let (mut col, mut row) = (col, row);
    if let Some(area) = state.last_chat_area {
        col = col.max(area.x).min(area.x + area.width.saturating_sub(1));
        row = row.max(area.y).min(area.y + area.height.saturating_sub(1));
    }

    move_selection_end_to(state, col, row);
}

/// 把屏幕坐标换算成选区终点并写入（drag_to 与 Up 共用同一套映射与钳制规则）。
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
/// - 工具块：点头部行切换
pub(crate) fn try_toggle_on_click(state: &mut ChatState, entry_idx: usize, row: usize) -> bool {
    let Some(entry) = state.chat_history.get(entry_idx) else {
        return false;
    };

    // Debug: 记录点击信息，帮助诊断"点工具块却展开思考块"的问题
    crate::utils::logging::append_debug_log_line(&format!(
        "[TOGGLE_CLICK] entry_idx={} row={} entry_type={:?} has_reasoning={} reasoning_len={} has_tool_call={} tool_name={:?}",
        entry_idx,
        row,
        entry.entry_type,
        entry.reasoning_content.is_some(),
        entry.reasoning_content.as_ref().map(|s| s.len()).unwrap_or(0),
        entry.tool_call.is_some(),
        entry.tool_call.as_ref().map(|tc| tc.function.name.as_str())
    ));

    if let Some(reasoning) = &entry.reasoning_content {
        // 可见性判定必须与渲染一致：有正文，**或**正在流式（此时正文可能还没到，
        // 但标题行已经在屏幕上）。以前只认"有正文"，于是流式刚开始、正文尚空时
        // 那行"∴ Thinking…"看得见却点不动 —— 点了没反应，用户只会当成卡住。
        let block_visible = !reasoning.is_empty() || entry.is_streaming == Some(true);
        if block_visible {
            // 展开状态问的是**同一个**判定入口（`is_thinking_expanded`）：
            // 流式期间块是自动展开的，单看 `expanded_thinking_indices` 会
            // 得出"折叠"的结论，于是点一下反而变成"展开"（其实早就展开着），
            // 用户的直觉（点一下收起来）被反转。
            let is_expanded =
                state.is_thinking_expanded(entry_idx, entry.is_streaming == Some(true));
            // 思考块折叠时只渲染 1 行标题："∴ Thought for 14s  (ctrl+o to expand)"
            // 点击这一行（row 0）应该展开；展开后点任意行都收起。
            //
            // 之前的 `row <= preview_lines` 判断有问题：折叠态的 heading 占 1 行，
            // preview_lines 却统计的是 reasoning 内容的前 3 行（折叠时根本不渲染），
            // 导致 row 0 可能大于 preview_lines（reasoning 内容少于 1 行时），
            // 无法展开。正确的逻辑：折叠时点任意位置（实际只有 row 0）都展开。
            let should_toggle = if is_expanded {
                true // 展开时点任意行都收起
            } else {
                true // 折叠时点标题行（唯一可见的那一行）展开
            };
            if should_toggle {
                // 走统一入口：它会记录"用户手动干预"，此后自动展开/收起
                // 对这个块永久失效（手动意图压过自动化）。
                state.toggle_thinking_expansion(entry_idx);
                state.rendered_cache.remove(&entry_idx);
                // 折叠/展开是用户明确的布局意图，不是流式内容导致的抖动，
                // `streaming_height_floor` 的"只增不减"下限从这一帧起作废，
                // 否则折叠后的 1 行标题会被垫回展开时刻的高度（大片空白行）。
                // 反抖动保护在下一个 stale 帧按折叠后的新高度重新钉底。
                state.streaming_height_floor.remove(&entry_idx);
                return true;
            }
            return false;
        }
    }

    // 工具块：点工具名展开/收起；正文展开后点正文里任意一行都收起。
    //
    // 一个工具块在 `chat_history` 里是**两条** entry：工具头（ToolCall，工具名
    // 那一行）和工具体（ToolResult，正文）。两条共用同一个 tool_call.id，
    // 折叠位是同一份 —— 所以从哪一条点都是切同一块。
    //
    // 命中规则与 thinking 块一致（见上）：**展开着时点块内任意一行都收起**。
    // 展开的正文可能有两百行，工具名远在屏幕外 —— 不这样，用户得一路滚回顶部
    // 才收得起来（这正是本次要修的操作痛点）。
    //
    // 折叠着时点正文任意一行（摘要行 / 预览行）都展开；点工具名那一行同样展开。
    let Some(entry) = state.chat_history.get(entry_idx) else {
        return false;
    };
    if entry.tool_call.is_none() {
        return false;
    }
    // TodoWrite 的正文是任务清单，整份渲染、不看展开位 —— 对它做折叠/展开
    // 没有意义，点它必须什么都不发生。
    if !state.tool_block_is_foldable(entry) {
        return false;
    }
    // 工具名那一行的行号：entry 前面可能垫了一行分隔空行（`leading_blank_rows`
    // 与渲染同源），所以工具名不一定是第 0 行 —— 以前按第 0 行判，命中的是
    // 那行看不见的空行，于是"点工具名"没反应、"点工具名上面那行空白"才有反应。
    let first_content_row =
        crate::ui::components::chat_history::leading_blank_rows(state, entry_idx);
    let should_toggle = match entry.entry_type {
        crate::types::ChatEntryType::ToolCall => row == first_content_row,
        crate::types::ChatEntryType::ToolResult => row >= first_content_row,
        _ => false,
    };
    if should_toggle {
        return state.toggle_tool_expansion(entry_idx);
    }

    false
}

pub fn handle_mouse_event(state: &mut ChatState, m: MouseEvent) {
    // 终端鼠标编码差异大（SGR / X10 / RXVT、tmux 转发），左键拖选失效时
    // 先看这条轨迹确认到达的事件种类与坐标，而不是猜映射逻辑。
    // 拖选期间 Moved 频率可达每秒数百条，默认不开，避免写盘开销拖慢渲染。
    if std::env::var("STAR_MOUSE_TRACE").is_ok() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[MOUSE] {:?} at ({},{}) selecting={}",
            m.kind, m.column, m.row, state.text_selection.is_selecting
        ));
    }
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
            drag_to(state, m.column, m.row);
        }
        MouseEventKind::Moved => {
            // Shift 修饰的移动同样交给终端原生选择，不接管。
            if m.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                return;
            }
            // 拖选进行中，把悬停移动当拖动处理。
            //
            // SGR 鼠标模式（?1006h）下按住左键移动编码为 button 0 + motion，
            // crossterm 解成 Drag(Left)；但不支持 SGR 的终端（老 xterm、部分
            // tmux 配置）把它编码成 button 3 + motion，crossterm 解成 Moved。
            // 事件循环（runtime.rs / mod.rs 的 Moved 过滤）只放行拖选中的 Moved，
            // 其余一律丢弃以避免悬停重绘风暴——所以能进到这里的 Moved 必定是
            // 正在拖选。若这里也忽略，这类终端上选区终点永远停在按下处，
            // 表现为「左键拖选选不中，一拖就失去焦点」。
            if state.text_selection.is_selecting {
                drag_to(state, m.column, m.row);
            }
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
            // 未拖动 = 单击（起止锚点重合，或偏移不足 2 列）：执行折叠/展开切换，
            // 并清掉单格选区高亮。
            //
            // 「不足 2 列也算单击」修的是另一个问题：鼠标/触摸板按下到松开之间
            // 有 1 列级抖动是常态，旧判定 `s == e` 只要差 1 列就落进
            // `has_selection()` 分支 → 松开即复制把 `∴ Thin` 这种几字符的碎片
            // 塞进剪贴板。用户的意图是单击，不是复制。
            let clicked = match (
                state.text_selection.start_entry_idx,
                state.text_selection.end_entry_idx,
                state.text_selection.start,
                state.text_selection.end,
            ) {
                (Some(se), Some(ee), Some(s), Some(e))
                    if se == ee && s.0 == e.0 && e.1.abs_diff(s.1) <= 1 =>
                {
                    Some((se, s.0))
                }
                _ => None,
            };

            if let Some((entry_idx, row)) = clicked {
                let _toggled = try_toggle_on_click(state, entry_idx, row);
                state.text_selection.clear();
            } else if state.text_selection.has_selection() {
                // 松开即复制：走 `ui::selection::copy_selection`（和 Ctrl+C 同一个
                // 入口），失败要说给用户听，且**保留选区**好让用户重试（Ctrl+C）。
                // 此前这里只写日志文件，界面上毫无反馈。
                match crate::ui::selection::copy_selection(state) {
                    Ok(()) => {
                        // 刻意**不**清选区：保留高亮，用户能看到复制了哪一段，
                        // 剪贴板被占用时还能按 Ctrl+C 重试。
                        state.current_status_line = Some("Copied".to_string());
                    }
                    Err(e) => {
                        state.current_status_line =
                            Some(format!("Copy failed: {e} — press Ctrl+C to retry"));
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[COPY] failed: {}",
                            e
                        ));
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

    /// 鼠标/触摸板按下到松开之间的 1 列级抖动不得触发「松开即复制」。
    ///
    /// 真实事故：用户单击 thinking 折叠头（`∴ Thinking…`），Down 在第 5 列、
    /// Up 在第 6 列，旧判定 `s == e` 不成立 → 落进 `has_selection()` 分支 →
    /// `copy_selection` 把 `∴ Thin` 这 6 个字符塞进剪贴板。用户的意图是单击，
    /// 不是复制。
    #[test]
    fn a_one_column_jitter_is_a_click_not_a_copy() {
        let mut state = setup_state();
        let idx = state.chat_history.len() - 1;

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 5, 1),
        );
        // 松开时向右偏了 1 列——触摸板/鼠标的常规抖动
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 6, 1),
        );

        assert!(
            state.expanded_thinking_indices.contains(&idx),
            "1 列抖动应按单击处理：触发折叠/展开"
        );
        assert!(
            state.text_selection.start.is_none(),
            "抖动选区必须被清掉，不得残留成可复制选区"
        );
    }

    /// 真正的拖选（≥2 列位移）仍然走「松开即复制」，不受抖动阈值影响。
    #[test]
    fn a_real_drag_still_selects_for_copy() {
        let mut state = setup_state();
        let idx = state.chat_history.len() - 1;

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Drag(MouseButton::Left), 8, 1),
        );
        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 8, 1),
        );

        assert!(
            state.expanded_thinking_indices.is_empty(),
            "拖选不得触发折叠/展开"
        );
        assert!(
            state.text_selection.has_selection(),
            "拖选后必须保留选区供复制"
        );
        assert_eq!(state.text_selection.start, Some((0, 0)));
        assert_eq!(state.text_selection.end, Some((0, 8)));
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

    /// 不支持 SGR 鼠标模式（?1006h）的终端把「按住左键移动」编码成
    /// button 3 + motion，crossterm 解成 Moved 而非 Drag(Left)。
    /// 事件循环只放行拖选中的 Moved，处理逻辑必须把它当拖动走，
    /// 否则这类终端上选区终点永远停在按下处——正是「一拖就失去焦点」。
    #[test]
    fn moved_during_selection_is_treated_as_drag() {
        let mut state = setup_state();

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        // 注意：拖动事件全部以 Moved 形式到达，一个 Drag 都没有
        handle_mouse_event(&mut state, mouse_event(MouseEventKind::Moved, 10, 2));
        handle_mouse_event(&mut state, mouse_event(MouseEventKind::Moved, 15, 3));

        assert!(
            state.text_selection.has_selection(),
            "Moved 必须能推进选区终点"
        );
        assert_eq!(state.text_selection.start, Some((0, 0)));
        assert_eq!(
            state.text_selection.end,
            Some((2, 15)),
            "终点跟随最后一个 Moved"
        );

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 15, 3),
        );
        assert!(state.text_selection.has_selection());
    }

    /// 没有按下过的纯悬停 Moved 不得起选区——它是无按钮的移动事件。
    #[test]
    fn moved_without_press_does_not_start_selection() {
        let mut state = setup_state();

        handle_mouse_event(&mut state, mouse_event(MouseEventKind::Moved, 10, 2));
        handle_mouse_event(&mut state, mouse_event(MouseEventKind::Moved, 20, 3));

        assert!(
            !state.text_selection.has_selection(),
            "纯悬停 Moved 不得产生选区"
        );
        assert!(state.text_selection.start.is_none());
    }

    /// 拖选中的 Moved 越出聊天区下方时，和 Drag 一样钳到最后一行。
    #[test]
    fn moved_below_content_clamps_like_drag() {
        let mut state = setup_state();

        handle_mouse_event(
            &mut state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 1),
        );
        handle_mouse_event(&mut state, mouse_event(MouseEventKind::Moved, 10, 20));

        assert_eq!(state.text_selection.end_entry_idx, Some(1));
        assert_eq!(
            state.text_selection.end,
            Some((2, 10)),
            "Moved 越界与 Drag 走同一套钳制"
        );
    }

    // ══════════════════════════════════════════════════════════════════════
    // 工具块的展开 / 收起
    //
    // 用户反馈（原文）：「展开工具视图之后要收起它非常难 —— 得一路滚回顶部
    // 去点那个显示出来的工具名」。这里锁住三条规则：
    //   1. 点**看得见的工具名**（不是它上面那行空白）能切换
    //   2. 正文展开着时，点正文里任意一行都能收起（不必滚回顶部）
    //   3. TodoWrite 的任务清单不可折叠，点它什么也不发生
    // ══════════════════════════════════════════════════════════════════════

    fn tool_call_entry(id: &str, name: &str) -> crate::types::ChatEntry {
        crate::types::ChatEntry::tool_call(
            "",
            crate::types::StarToolCall {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: name.to_string(),
                    arguments: r#"{"file_path":"app/main.py"}"#.to_string(),
                },
            },
        )
    }

    fn tool_result_entry(id: &str, name: &str, output: &str) -> crate::types::ChatEntry {
        crate::types::ChatEntry::tool_result(
            "",
            crate::types::StarToolCall {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: name.to_string(),
                    arguments: r#"{"file_path":"app/main.py"}"#.to_string(),
                },
            },
            crate::types::ToolResult {
                success: true,
                output: Some(output.to_string()),
                error: None,
                data: None,
                llm_content: None,
            },
        )
    }

    /// 一个会话：用户消息 → 助手消息 → 工具头 → 工具体。
    /// 渲染后工具名那一行在文档里的绝对行号由 `tool_header_row` 给出。
    fn tool_block_state() -> ChatState {
        let mut state = ChatState::new();
        state.chat_history = vec![
            crate::types::ChatEntry::user("看看文件"),
            crate::types::ChatEntry::assistant("好"),
            tool_call_entry("t1", "Read"),
            tool_result_entry("t1", "Read", "line A\nline B\nline C"),
        ];
        state.last_chat_area = Some(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        });
        state
    }

    /// 渲染一遍并把渲染结果写回 state（与真实渲染路径同源）。
    fn render(state: &mut ChatState) -> Vec<ratatui::text::Line<'static>> {
        let lines = crate::ui::components::chat_history::render_chat_lines(state, 80);
        state.total_rendered_lines = lines.len();
        lines.as_ref().clone()
    }

    /// 找出含 `needle` 的那一行的绝对行号。
    fn row_containing(lines: &[ratatui::text::Line<'static>], needle: &str) -> usize {
        lines
            .iter()
            .position(|l| {
                let t: String = l
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref().to_string())
                    .collect();
                t.contains(needle)
            })
            .unwrap_or_else(|| panic!("渲染结果里没有 {needle:?} 那一行"))
    }

    /// 工具名那一行（工具名被渲染成 "view(...)" —— `tool_display_name` 的映射）。
    fn tool_header_row(state: &mut ChatState) -> usize {
        let lines = render(state);
        row_containing(&lines, "view(")
    }

    fn click_at(state: &mut ChatState, row: u16) {
        handle_mouse_event(
            state,
            mouse_event(MouseEventKind::Down(MouseButton::Left), 0, row),
        );
        handle_mouse_event(
            state,
            mouse_event(MouseEventKind::Up(MouseButton::Left), 0, row),
        );
    }

    /// 回归：点**看得见的工具名**那一行能展开。
    ///
    /// 旧实现判的是「entry 的第 0 行」——工具头 entry 渲染出来是
    /// `[前导空行] + [工具名行]`，所以第 0 行是那行看不见的空白。
    /// 于是用户的体验是「点工具名没反应，点工具名上面那行空白才有反应」。
    #[test]
    fn clicking_the_visible_tool_name_toggles_the_block() {
        let mut state = tool_block_state();
        let header_row = tool_header_row(&mut state);

        assert!(!state.expanded_tool_call_ids.contains("t1"), "初始是折叠态");

        click_at(&mut state, header_row as u16);

        assert!(
            state.expanded_tool_call_ids.contains("t1"),
            "点工具名必须能展开（而不是点它上面那行空白）"
        );
    }

    /// 回归：工具名上面那行空白**不是**切换目标。
    ///
    /// 这行是分隔用的空行，点了它什么也不该发生 —— 否则"点工具名没反应"
    /// 这个症状会被"点空白有反应"掩盖过去。
    #[test]
    fn clicking_the_blank_above_the_tool_name_does_nothing() {
        let mut state = tool_block_state();
        let header_row = tool_header_row(&mut state);
        assert!(header_row > 0, "工具名上面应当有一行分隔空行");

        click_at(&mut state, (header_row - 1) as u16);

        assert!(
            !state.expanded_tool_call_ids.contains("t1"),
            "点工具名上面那行空白不得切换"
        );
    }

    /// 回归：正文展开着时，点正文里任意一行都能收起。
    ///
    /// 这条是用户反馈的核心：展开后的正文可能几十上百行，工具名远在屏幕外，
    /// 不这样就得一路滚回顶部才收得起来。
    #[test]
    fn clicking_inside_an_expanded_body_collapses_it() {
        let mut state = tool_block_state();
        let header_row = tool_header_row(&mut state);

        // 先展开（点工具名）
        click_at(&mut state, header_row as u16);
        assert!(state.expanded_tool_call_ids.contains("t1"), "先展开");

        // 重新渲染 —— 真实流程里两次点击之间一定有一帧
        let lines = render(&mut state);
        let body_row = row_containing(&lines, "line B");
        assert!(
            body_row > header_row,
            "正文行必须在工具名下面（body_row={body_row}, header_row={header_row}）"
        );

        // 再点正文中间那一行
        click_at(&mut state, body_row as u16);

        assert!(
            !state.expanded_tool_call_ids.contains("t1"),
            "点正文里任意一行必须能收起，不必滚回工具名"
        );
    }

    /// 回归：折叠着的正文，点它也能展开。
    #[test]
    fn clicking_the_collapsed_summary_expands_it() {
        let mut state = tool_block_state();
        let header_row = tool_header_row(&mut state);

        // 工具体紧跟在工具头后面（工具序列不加空行），摘要行就在它下面
        click_at(&mut state, (header_row + 1) as u16);

        assert!(
            state.expanded_tool_call_ids.contains("t1"),
            "点折叠态的摘要行必须能展开"
        );
    }

    /// 回归：TodoWrite 的任务清单不可折叠 —— 点它什么也不发生。
    ///
    /// 清单整份渲染、不看展开位（对标 CC 的 TaskList），对它做折叠/展开
    /// 没有意义；真让它翻起来，屏幕上还是同一份清单，纯属骗人。
    #[test]
    fn clicking_a_todowrite_block_does_nothing() {
        let mut state = ChatState::new();
        state.chat_history = vec![
            crate::types::ChatEntry::user("列个计划"),
            tool_call_entry("t2", "TodoWrite"),
            tool_result_entry("t2", "TodoWrite", "ok"),
        ];
        state.last_chat_area = Some(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        });
        let lines = render(&mut state);
        let header_row = row_containing(&lines, "Update Todos");

        click_at(&mut state, header_row as u16);
        assert!(
            !state.expanded_tool_call_ids.contains("t2"),
            "TodoWrite 不得被折叠/展开"
        );

        click_at(&mut state, (header_row + 1) as u16);
        assert!(
            !state.expanded_tool_call_ids.contains("t2"),
            "TodoWrite 的清单不得被折叠/展开"
        );
    }

    /// 回归：收起之后，工具名那一行必须还在视口里。
    ///
    /// 收起时正文整段消失，下面的内容会上移；视口若停在原处，
    /// 用户看到的是一段毫不相干的内容 —— 像是"点了没反应"。
    #[test]
    fn collapsing_keeps_the_tool_name_in_view() {
        // 正文要够长，长到"用户正在正文中间"时工具名已经滚出视口
        let long_output = (1..=40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = ChatState::new();
        state.chat_history = vec![
            crate::types::ChatEntry::user("看看文件"),
            crate::types::ChatEntry::assistant("好"),
            tool_call_entry("t1", "Read"),
            tool_result_entry("t1", "Read", &long_output),
        ];
        state.last_chat_area = Some(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        });
        state.last_chat_height = 6;

        let header_row = tool_header_row(&mut state);
        click_at(&mut state, header_row as u16);
        assert!(state.expanded_tool_call_ids.contains("t1"), "先展开");

        // 重新渲染，再把视口滚到正文中间 —— 此刻工具名已滚出视口
        let lines = render(&mut state);
        let body_row = row_containing(&lines, "line 20");
        state.scroll = body_row - 2;
        let screen_row = (body_row - state.scroll) as u16;

        // 点正文里那一行 → 收起
        click_at(&mut state, screen_row);
        assert!(!state.expanded_tool_call_ids.contains("t1"), "已收起");

        // 收起后工具名必须回到视口里
        let scroll = state.scroll;
        let viewport = state.last_chat_height as usize;
        assert!(
            header_row >= scroll && header_row < scroll + viewport,
            "收起后工具名必须仍在视口内（scroll={scroll}, header_row={header_row}）"
        );
    }
}
