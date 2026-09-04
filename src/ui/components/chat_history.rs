use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{types::ChatEntryType, ui::state::ChatState};

/// Apply selection highlight to text lines (Optimized)
fn apply_selection_highlight(
    lines: &mut [Line],
    entry_idx: usize,
    line_offset: usize,
    state: &ChatState,
) {
    if !state.text_selection.has_selection() {
        return;
    }

    // 1. Get Global Selection Bounds
    let (start_entry, end_entry) = match (
        state.text_selection.start_entry_idx,
        state.text_selection.end_entry_idx,
    ) {
        (Some(s), Some(e)) => {
            if s <= e {
                (s, e)
            } else {
                (e, s)
            }
        }
        _ => return,
    };

    if entry_idx < start_entry || entry_idx > end_entry {
        return;
    }

    let Some(((start_row, start_col), (end_row, end_col))) =
        state.text_selection.get_selection_range()
    else {
        return;
    };

    for (i, line) in lines.iter_mut().enumerate() {
        let row_idx = i + line_offset;

        // 2. Determine Selected Column Range for this specific line
        let (sel_start_col, sel_end_col) = if entry_idx == start_entry && entry_idx == end_entry {
            // Single entry selection
            if row_idx < start_row || row_idx > end_row {
                continue;
            }
            let s = if row_idx == start_row { start_col } else { 0 };
            let e = if row_idx == end_row {
                end_col
            } else {
                usize::MAX
            };
            (s, e)
        } else if entry_idx == start_entry {
            // Start entry (multi-entry)
            if row_idx < start_row {
                continue;
            }
            let s = if row_idx == start_row { start_col } else { 0 };
            (s, usize::MAX)
        } else if entry_idx == end_entry {
            // End entry (multi-entry)
            if row_idx > end_row {
                continue;
            }
            let e = if row_idx == end_row {
                end_col
            } else {
                usize::MAX
            };
            (0, e)
        } else {
            // Middle entry
            (0, usize::MAX)
        };

        // 3. Rebuild spans only if overlapping
        let mut new_spans: Vec<Span> = Vec::with_capacity(line.spans.len() * 2);
        let mut current_col = 0;

        for span in &line.spans {
            let span_text = span.content.as_ref();
            // Use Unicode width for column calculation to match terminal display
            let span_len = span_text.width();
            let span_end_col = current_col + span_len;

            // Check overlap: [current_col, span_end_col) vs [sel_start_col, sel_end_col]
            // Note: sel_end_col is INCLUSIVE in is_position_selected logic (<= end_col)
            // But for range slicing, we need to be careful.

            // If the span is completely outside selection
            if span_end_col <= sel_start_col || current_col > sel_end_col {
                new_spans.push(span.clone());
            } else {
                // Partial or full overlap
                // Calculate relative split points using visual width
                let rel_start = if current_col < sel_start_col {
                    sel_start_col - current_col
                } else {
                    0
                };

                // sel_end_col is inclusive; compute exclusive end safely to avoid
                // overflow when sel_end_col == usize::MAX (meaning "end of line").
                let sel_end_exclusive = sel_end_col.saturating_add(1);
                let rel_end = if span_end_col > sel_end_exclusive {
                    sel_end_exclusive.saturating_sub(current_col)
                } else {
                    span_len
                };

                // Split by visual width, not char count
                let (prefix, selected, suffix) = split_by_width(span_text, rel_start, rel_end);

                // 1. Unselected Prefix
                if !prefix.is_empty() {
                    new_spans.push(Span::styled(prefix, span.style));
                }

                // 2. Selected Middle
                if !selected.is_empty() {
                    new_spans.push(Span::styled(
                        selected,
                        span.style
                            .bg(Color::Blue)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                }

                // 3. Unselected Suffix
                if !suffix.is_empty() {
                    new_spans.push(Span::styled(suffix, span.style));
                }
            }
            current_col += span_len;
        }
        line.spans = new_spans;
    }
}

/// Split text by visual width into (prefix, middle, suffix)
/// prefix: first `start_width` visual columns
/// middle: next `end_width - start_width` visual columns
/// suffix: remaining text
fn split_by_width(text: &str, start_width: usize, end_width: usize) -> (String, String, String) {
    let mut prefix = String::new();
    let mut middle = String::new();
    let mut suffix = String::new();
    let mut current_width = 0;
    let mut in_prefix = true;
    let mut in_middle = false;

    for ch in text.chars() {
        // 度量口径必须与 ratatui 的缓冲区一致（见 render::display_width），
        // 否则选区高亮的起止列会随 Ambiguous 字符逐个右移
        let ch_width = crate::ui::utils::render::char_display_width(ch).max(1);

        if in_prefix && current_width < start_width {
            prefix.push(ch);
            current_width += ch_width;
            if current_width >= start_width {
                in_prefix = false;
                in_middle = true;
            }
        } else if in_middle && current_width < end_width {
            middle.push(ch);
            current_width += ch_width;
            if current_width >= end_width {
                in_middle = false;
            }
        } else {
            suffix.push(ch);
            current_width += ch_width;
        }
    }

    (prefix, middle, suffix)
}

fn find_scroll_anchor(heights: &[u16], scroll_top: usize) -> Option<(usize, usize)> {
    let mut current_y = 0usize;
    for (idx, &h) in heights.iter().enumerate() {
        let h = h as usize;
        if h == 0 {
            continue;
        }
        if scroll_top < current_y + h {
            return Some((idx, scroll_top.saturating_sub(current_y)));
        }
        current_y += h;
    }
    None
}

fn scroll_top_for_anchor(heights: &[u16], entry_idx: usize, row: usize) -> Option<usize> {
    if entry_idx >= heights.len() {
        return None;
    }
    let mut current_y = 0usize;
    for &h in heights.iter().take(entry_idx) {
        current_y += h as usize;
    }
    let h = heights[entry_idx] as usize;
    let clamped_row = if h == 0 {
        0
    } else {
        row.min(h.saturating_sub(1))
    };
    Some(current_y + clamped_row)
}

/// Render chat entries into lines for the full-page scrollable document.
/// Returns the rendered lines WITHOUT drawing to the frame.
pub fn render_chat_lines(state: &mut ChatState, area_width: u16) -> Vec<Line<'static>> {
    // 详情视图优先：`viewing_agent_task_id` 有值时整块聊天区换成那个后台代理的输出。
    // 刻意不走 virtual_list / rendered_cache —— 那两者的下标是按 `chat_history` 尺寸分配的，
    // 塞进另一份条目列表会互相污染高度缓存。详情视图行数有限，每帧全量渲染即可。
    if state.viewing_agent_task_id.is_some() {
        if let Some(lines) = render_teammate_view(state, area_width) {
            return lines;
        }
        // 任务已经不在 active_agent_tasks 里（理论上不会发生，只插不删）——退回主会话
        state.exit_teammate_view();
    }

    let history_len = state.chat_history.len();
    if history_len == 0 {
        return Vec::new();
    }

    state.virtual_list.auto_follow = state.auto_follow;
    state.virtual_list.resize(history_len);

    // Clear cache if terminal width changed
    if state.last_terminal_width != area_width {
        state.clear_cache();
        state.last_terminal_width = area_width;
    }

    // Ensure last_item_heights has correct capacity
    if state.last_item_heights.len() != history_len {
        state.last_item_heights.resize(history_len, 0);
    }

    // Single pass: render once, use for both height tracking and output.
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for idx in 0..history_len {
        let entry = &state.chat_history[idx];
        let is_streaming = entry.is_streaming == Some(true);
        let reasoning_len = entry
            .reasoning_content
            .as_ref()
            .map(|r| r.len())
            .unwrap_or(0);
        let content_len = entry.content.len();
        let current_key = (reasoning_len, content_len);

        let stale = if is_streaming {
            state.virtual_list.item_height(idx) == 0
                || state
                    .last_rendered_stream_key
                    .get(&idx)
                    .map(|k| *k != current_key)
                    .unwrap_or(true)
        } else {
            state.virtual_list.is_dirty(idx) || !state.rendered_cache.contains_key(&idx)
        };

        if stale {
            let mut lines = render_entry_lines(state, idx, area_width);
            if is_streaming {
                // 高度单调下限：流式期间渲染高度只增不减。
                // 增量 markdown 在段落边界跨越时可能重排导致高度回缩（-N 又 +N），
                // 视口又锚定在底部，会引起整页上下跳动。补空行保持高度稳定。
                let floor = state.streaming_height_floor.get(&idx).copied().unwrap_or(0);
                if (lines.len() as u16) < floor {
                    lines.resize(floor as usize, Line::from(""));
                }
                state
                    .streaming_height_floor
                    .insert(idx, lines.len().min(u16::MAX as usize) as u16);
            } else {
                state.streaming_height_floor.remove(&idx);
            }
            let h = lines.len().min(u16::MAX as usize) as u16;
            state.virtual_list.set_height(idx, h);
            // Ensure last_item_heights has capacity before setting
            while state.last_item_heights.len() <= idx {
                state.last_item_heights.push(0);
            }
            state.last_item_heights[idx] = h;
            if !is_streaming {
                state.rendered_cache.insert(idx, (h, lines.clone()));
                state.last_rendered_stream_key.remove(&idx);
                all_lines.extend(lines);
            } else {
                state.rendered_cache.remove(&idx);
                state.last_rendered_stream_key.insert(idx, current_key);
                all_lines.extend(lines); // streaming: use the freshly rendered lines
            }
        } else {
            // Not stale: use cached (non-streaming) or re-render (streaming)
            if is_streaming {
                let mut lines = render_entry_lines(state, idx, area_width);
                // 与 stale 分支相同的高度下限，保证同帧高度一致
                let floor = state.streaming_height_floor.get(&idx).copied().unwrap_or(0);
                if (lines.len() as u16) < floor {
                    lines.resize(floor as usize, Line::from(""));
                }
                all_lines.extend(lines);
            } else if let Some((_, lines)) = state.rendered_cache.get(&idx) {
                all_lines.extend(lines.clone());
            }
        }
    }

    // Apply text selection highlighting to visible entries
    if state.text_selection.has_selection() {
        let mut offset = 0usize;
        for (entry_idx, &height) in state.last_item_heights.iter().enumerate() {
            let h = height as usize;
            if h > 0 && offset + h <= all_lines.len() {
                apply_selection_highlight(&mut all_lines[offset..offset + h], entry_idx, 0, state);
            }
            offset += h;
        }
    }

    // Note: last_item_heights is already set correctly in the loop above (line 267)
    // Do NOT sync from virtual_list here as it may have stale values

    all_lines
}

pub fn render_chat_history(
    f: &mut Frame,
    state: &mut ChatState,
    chat_area: Rect,
    _scrollbar_area: Rect,
) {
    // ============ UX improvement: render cache ============
    // Clear cache if terminal width changes
    if state.last_terminal_width != chat_area.width {
        state.clear_cache();
        state.last_terminal_width = chat_area.width;
    }
    // =========================================

    // ── Cancel transition expiry cleanup: auto-clear is_streaming 1.5s after ESC/Ctrl+C ──
    if let Some(t) = state.cancelling_since {
        if t.elapsed() >= std::time::Duration::from_millis(1500) {
            for e in state.chat_history.iter_mut() {
                if e.is_streaming == Some(true) {
                    e.is_streaming = Some(false);
                }
            }
            state.cancelling_since = None;
            state.is_streaming = false;
            state.current_status_line = None;
        }
    }

    // ── VirtualList: per-entry dirty tracking + viewport clipping ──
    let history_len = state.chat_history.len();
    state.virtual_list.auto_follow = state.auto_follow;
    state.virtual_list.resize(history_len);

    // Save scroll anchor when not auto-following
    let anchor_idx = if !state.auto_follow && state.virtual_list.total_lines() > 0 {
        state.virtual_list.item_at_scroll(state.scroll)
    } else {
        None
    };

    // Re-render dirty entries
    for idx in 0..history_len {
        let entry = &state.chat_history[idx];
        let is_streaming = entry.is_streaming == Some(true);
        let reasoning_len = entry
            .reasoning_content
            .as_ref()
            .map(|r| r.len())
            .unwrap_or(0);
        let content_len = entry.content.len();
        let current_key = (reasoning_len, content_len);

        // Per-entry dirty: streaming OR content changed OR first render
        let stale = if is_streaming {
            state.virtual_list.item_height(idx) == 0
                || state
                    .last_rendered_stream_key
                    .get(&idx)
                    .map(|k| *k != current_key)
                    .unwrap_or(true)
        } else {
            state.virtual_list.is_dirty(idx) || !state.rendered_cache.contains_key(&idx)
        };

        if !stale {
            continue;
        }

        let lines = render_entry_lines(state, idx, chat_area.width);
        let h = lines.len().min(u16::MAX as usize) as u16;
        state.virtual_list.set_height(idx, h);
        // Sync legacy height tracking
        if idx < state.last_item_heights.len() {
            state.last_item_heights[idx] = h;
        }

        if is_streaming {
            state.rendered_cache.remove(&idx);
            state.last_rendered_stream_key.insert(idx, current_key);
        } else {
            state.rendered_cache.insert(idx, (h, lines));
            state.last_rendered_stream_key.remove(&idx);
            state.virtual_list.mark_dirty(idx); // will be cleaned after this frame
        }
    }

    state.total_rendered_lines = state.virtual_list.total_lines();

    // Scroll: auto-follow or anchor-preserve.
    // Subtract 1 from viewport so the last chat line doesn't touch the input.
    let viewport_height = chat_area.height.saturating_sub(1) as usize;
    let max_scroll = state
        .virtual_list
        .total_lines()
        .saturating_sub(viewport_height);
    if !state.auto_follow {
        if let Some(entry_idx) = anchor_idx {
            state.scroll = state
                .virtual_list
                .anchor_scroll(entry_idx, 0)
                .min(max_scroll);
        }
    }
    if state.auto_follow {
        state.scroll = max_scroll;
    } else if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }

    // Use VirtualList's visible range for viewport clipping
    let (start_idx, end_idx, _skip) = state
        .virtual_list
        .visible_range(chat_area.height, state.scroll);
    let scroll_top = state.scroll;
    let scroll_bottom = scroll_top + viewport_height;

    let mut visible_lines: Vec<Line> = Vec::with_capacity(viewport_height);
    let mut current_y = 0usize;
    // Calculate y-offset up to start_idx
    for i in 0..start_idx {
        current_y += state.virtual_list.item_height(i) as usize;
    }

    for entry_idx in start_idx..end_idx.min(history_len) {
        let height = state.virtual_list.item_height(entry_idx) as usize;
        let entry_start = current_y;

        if entry_start < scroll_bottom {
            let is_streaming = state
                .chat_history
                .get(entry_idx)
                .map(|e| e.is_streaming == Some(true))
                .unwrap_or(false);

            let lines_cow = if !is_streaming {
                if let Some((_, lines)) = state.rendered_cache.get(&entry_idx) {
                    std::borrow::Cow::Borrowed(lines)
                } else {
                    let lines = render_entry_lines(state, entry_idx, chat_area.width);
                    std::borrow::Cow::Owned(lines)
                }
            } else {
                let lines = render_entry_lines(state, entry_idx, chat_area.width);
                std::borrow::Cow::Owned(lines)
            };

            let skip = if scroll_top > entry_start {
                scroll_top - entry_start
            } else {
                0
            };
            let take = (scroll_bottom - (entry_start + skip)).min(height - skip);
            let chunk_iter = lines_cow.iter().skip(skip).take(take);

            if state.text_selection.has_selection() {
                let mut chunk: Vec<Line> = chunk_iter.cloned().collect();
                apply_selection_highlight(&mut chunk, entry_idx, skip, state);
                visible_lines.extend(chunk);
            } else {
                visible_lines.extend(chunk_iter.cloned());
            }
        } else {
            break;
        }
        current_y += height;
    }

    // Bottom-up fill: insert empty lines at the top so content
    // sits at the bottom (near the input area), like Claude Code.
    while visible_lines.len() < viewport_height {
        visible_lines.insert(0, Line::from(""));
    }

    let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });

    f.render_widget(paragraph, chat_area);

    // Update state for next frame calculations
    state.last_chat_area = Some(chat_area);
    state.last_chat_height = chat_area.height;
    state.last_max_scroll = 0;

    // Update Scrollbar
    state.chat_scrollbar_state = state
        .chat_scrollbar_state
        .content_length(state.total_rendered_lines)
        .position(state.scroll);

    // Render scrollbar indicator on the right edge
    if chat_area.width > 2 && state.total_rendered_lines > chat_area.height as usize {
        let scrollbar_area = Rect {
            x: chat_area.x + chat_area.width - 1,
            y: chat_area.y,
            width: 1,
            height: chat_area.height,
        };
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            scrollbar_area,
            &mut state.chat_scrollbar_state,
        );
    }
}

/// 渲染某个后台代理的详情视图（对标 teammate view）。
///
/// `viewing_agent_task_id` 命中 `active_agent_tasks` 时返回「标题行 + 该任务的 sub_entries」，
/// 否则返回 `None` 让调用方退回主会话。子条目复用 `tool_render` / `message_render` 的
/// 同一套 block 构造器，不另写渲染逻辑。
fn render_teammate_view(state: &ChatState, area_width: u16) -> Option<Vec<Line<'static>>> {
    let task_id = state.viewing_agent_task_id.as_deref()?;
    let info = state.active_agent_tasks.get(task_id)?;
    let theme = state.theme_manager.current();

    let label = info.name.as_deref().unwrap_or(info.agent_type.as_str());
    let desc = info
        .task_description
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(info.description.as_str());

    let status_color = match info.status {
        crate::types::AgentTaskStatus::Running => theme.warning,
        crate::types::AgentTaskStatus::Completed => theme.success,
        crate::types::AgentTaskStatus::Failed | crate::types::AgentTaskStatus::Rejected => {
            theme.error
        }
        crate::types::AgentTaskStatus::Background => theme.info,
    };

    let hint = "Esc to return";
    // 标题行：⏵ <label> — <desc> · <耗时> · <N tool uses>            Esc to return
    let tool_uses = info.tool_use_count;
    let plural = if tool_uses == 1 { "use" } else { "uses" };
    let stats = format!(
        "{} · {} tool {}",
        super::agent_group_render::format_duration(info.elapsed()),
        tool_uses,
        plural,
    );
    let fixed = 2 + label.chars().count() + 3 + stats.chars().count() + hint.chars().count() + 4;
    let desc_width = (area_width as usize).saturating_sub(fixed);
    let desc_shown = crate::ui::utils::render::truncate_to_display_width(desc, desc_width);

    let mut header = vec![
        Span::styled("⏵ ", Style::default().fg(status_color)),
        Span::styled(
            label.to_string(),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if !desc_shown.is_empty() {
        header.push(Span::styled(
            format!(" — {}", desc_shown),
            Style::default().fg(theme.foreground),
        ));
    }
    header.push(Span::styled(
        format!(" · {}", stats),
        Style::default().fg(theme.inactive),
    ));
    let used: usize = header.iter().map(|s| s.content.chars().count()).sum();
    if (area_width as usize) > used + hint.chars().count() + 2 {
        let pad = area_width as usize - used - hint.chars().count() - 1;
        header.push(Span::styled(" ".repeat(pad), Style::default()));
    } else {
        header.push(Span::styled("  ", Style::default()));
    }
    header.push(Span::styled(hint, Style::default().fg(theme.inactive)));

    let mut lines = vec![Line::from(header), Line::from("")];

    if info.sub_entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Waiting for the first tool call…",
            Style::default().fg(theme.inactive),
        )));
        return Some(lines);
    }

    let wrap_width = area_width.saturating_sub(2) as usize;
    for sub in &info.sub_entries {
        let blocks = if super::tool_render::is_tool_entry(sub) {
            super::tool_render::render_tool_entry_blocks(
                state,
                sub,
                usize::MAX,
                area_width,
                false,
                false,
                false,
            )
        } else {
            // `usize::MAX` 只用于 thinking 展开态的键；子条目不参与主历史的展开状态，
            // 传一个不会与真实下标碰撞的哨兵即可。
            super::message_render::render_non_tool_entry_blocks(state, sub, usize::MAX, wrap_width)
        };
        for b in blocks {
            lines.extend(b);
        }
    }

    Some(lines)
}

fn render_entry_lines(state: &ChatState, entry_idx: usize, area_width: u16) -> Vec<Line<'static>> {
    let entry = &state.chat_history[entry_idx];
    let mut entry_lines: Vec<Line<'static>> = Vec::new();

    let prev_is_tool = entry_idx
        .checked_sub(1)
        .and_then(|i| state.chat_history.get(i))
        .map(|e| super::tool_render::is_tool_entry(e))
        .unwrap_or(false);

    let prev_is_confirmation = entry_idx
        .checked_sub(1)
        .and_then(|i| state.chat_history.get(i))
        .map(|e| e.entry_type == ChatEntryType::ToolConfirmation)
        .unwrap_or(false);

    let next_is_tool = state
        .chat_history
        .get(entry_idx + 1)
        .map(|e| super::tool_render::is_tool_entry(e))
        .unwrap_or(false);

    let is_tool = super::tool_render::is_tool_entry(entry);
    let wrap_width = area_width.saturating_sub(2) as usize;

    let mut blocks: Vec<Vec<Line<'static>>> = Vec::new();

    // ── 特殊条目分发 ──
    if entry.entry_type == ChatEntryType::AgentTask {
        blocks.extend(super::agent_task_render::render_agent_task_entry(
            state,
            entry,
            area_width,
            entry_idx,
        ));
    } else if entry.entry_type == ChatEntryType::AgentGroup {
        blocks.extend(super::agent_group_render::render_agent_group(
            state,
            entry,
            area_width,
        ));
    } else if entry.entry_type == ChatEntryType::CollapsedGroup {
        blocks.extend(super::collapsed_group::render_collapsed_group(
            state,
            entry,
            area_width as usize,
        ).into_iter().map(|line| vec![line]));
    } else if entry.entry_type == ChatEntryType::GroupedToolUse {
        // 分组工具调用：展开显示子条目
        if let Some(sub_entries) = &entry.collapsed_entries {
            for sub in sub_entries {
                if super::tool_render::is_tool_entry(sub) {
                    blocks.extend(super::tool_render::render_tool_entry_blocks(
                        state,
                        sub,
                        entry_idx,
                        area_width,
                        false,
                        false,
                        false,
                    ));
                }
            }
        }
    } else if is_tool {
        blocks.extend(super::tool_render::render_tool_entry_blocks(
            state,
            entry,
            entry_idx,
            area_width,
            prev_is_tool,
            next_is_tool,
            prev_is_confirmation,
        ));
    } else {
        blocks.extend(super::message_render::render_non_tool_entry_blocks(
            state, entry, entry_idx, wrap_width,
        ));
    }

    for b in blocks {
        entry_lines.extend(b);
    }

    // ── Uniform leading spacing ──
    // Unified rule: all entries get 1 blank line separator, EXCEPT:
    //   - First entry (no blank)
    //   - ToolResult/ToolConfirmation immediately after ToolCall (flow together)
    //   - ToolCall immediately after ToolCall (multiple tool calls in sequence)
    //   - ToolCall immediately after ToolResult (continuation of tool sequence)
    //   - AgentTask/AgentGroup entries (always flow together)
    let add_leading_blank = if entry_idx == 0 {
        false
    } else {
        let prev = &state.chat_history[entry_idx - 1];
        // 工具调用序列：ToolCall/ToolResult/ToolConfirmation 之间不加空行
        let is_tool_sequence = matches!(
            entry.entry_type,
            ChatEntryType::ToolCall | ChatEntryType::ToolResult | ChatEntryType::ToolConfirmation
        ) && matches!(
            prev.entry_type,
            ChatEntryType::ToolCall | ChatEntryType::ToolResult | ChatEntryType::ToolConfirmation
        );
        // Agent 任务条目之间不加空行
        let is_agent_sequence = matches!(
            entry.entry_type,
            ChatEntryType::AgentTask | ChatEntryType::AgentGroup
        ) && matches!(
            prev.entry_type,
            ChatEntryType::AgentTask | ChatEntryType::AgentGroup
        );
        !is_tool_sequence && !is_agent_sequence
    };
    if add_leading_blank {
        entry_lines.insert(0, Line::from(""));
    }

    // Coalesce adjacent spans with same style to reduce rendering overhead
    for line in &mut entry_lines {
        if line.spans.len() > 1 {
            let mut new_spans = Vec::with_capacity(line.spans.len());
            let mut current_span: Option<Span> = None;

            for span in line.spans.drain(..) {
                if span.content.is_empty() {
                    continue;
                }

                if let Some(mut curr) = current_span.take() {
                    if curr.style == span.style {
                        // Merge
                        curr.content = format!("{}{}", curr.content, span.content).into();
                        current_span = Some(curr);
                    } else {
                        // Push previous, start new
                        new_spans.push(curr);
                        current_span = Some(span);
                    }
                } else {
                    current_span = Some(span);
                }
            }
            if let Some(curr) = current_span {
                new_spans.push(curr);
            }
            line.spans = new_spans;
        }
    }

    entry_lines
}

// ── Animation helper ──────────────────────────────────────────────

/// Calculate animation frame: returns (elapsed ms, frame index 0/1/2 for dot cycling, cursor visible)
pub(crate) fn animation_state(state: &ChatState) -> (u128, usize) {
    let elapsed_ms = state
        .processing_started_at
        .map(|t| t.elapsed().as_millis())
        .unwrap_or(0);
    let frame = (elapsed_ms / 400) as usize % 3; // 0, 1, 2 — dot cycling
    (elapsed_ms, frame)
}

pub(crate) fn format_elapsed(elapsed_ms: u128) -> String {
    if elapsed_ms < 1000 {
        format!("{}ms", elapsed_ms)
    } else if elapsed_ms < 60_000 {
        format!("{:.1}s", elapsed_ms as f64 / 1000.0)
    } else {
        let mins = elapsed_ms / 60_000;
        let secs = (elapsed_ms % 60_000) / 1000;
        format!("{}m{}s", mins, secs)
    }
}

pub(crate) fn format_token_count(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}
