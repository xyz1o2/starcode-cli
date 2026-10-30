use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Block,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{types::ChatEntryType, ui::state::ChatState};

/// Apply selection highlight to text lines (Optimized)
///
/// 换算逻辑搬到 `ui::selection` 了 —— 高亮与复制必须用同一套列宽口径，
/// 否则屏幕上高亮的范围和复制出来的文本会不一致。
fn apply_selection_highlight(
    lines: &mut [Line],
    entry_idx: usize,
    line_offset: usize,
    state: &ChatState,
) {
    if !state.text_selection.has_selection() {
        return;
    }

    let Some(range) = crate::ui::selection::SelectionRange::resolve(&state.text_selection) else {
        return;
    };

    // `lines` 是某个条目自己的渲染行，行号要从该条目内的偏移算起。
    let shifted = crate::ui::selection::SelectionRange {
        start: (
            range.start.0,
            range.start.1.saturating_sub(line_offset),
            range.start.2,
        ),
        end: (
            range.end.0,
            range.end.1.saturating_sub(line_offset),
            range.end.2,
        ),
    };
    crate::ui::selection::apply_highlight(lines, &shifted, entry_idx);
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

    // 抬头那条条目的内容不来自 `entry.content`，而是渲染期从 state 现算（模型名 +
    // 思考档位）。而 `rendered_cache` 只在条目被标脏时才刷新 —— 所以模型或档位在别处
    // 一改，屏幕上留着的还是旧字符串。这里按内容指纹判一次，比在十几个
    // `current_model = …` 赋值点各插一次 invalidate 可靠。
    super::welcome_header::refresh_if_stale(state);

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
                all_lines.extend(lines.clone()); // streaming: use the freshly rendered lines
                                                 // 拖选松开时 get_selected_text() 依赖 rendered_cache 取文本。
                                                 // 流式渲染每帧都会刷新，这里必须同步保留一份，否则左键选择永远取不到内容。
                state.rendered_cache.insert(idx, (h, lines));
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

/// ⚠️ 曾经的死函数 `render_chat_history` 已于 2026-10-13 删除。**不要在这里找聊天区的渲染逻辑。**
///
/// 它是 `render_chat_lines` + `write_lines_to_buffer` 之前的旧实现，`ui/app/mod.rs`
/// 里真正跑的是那条路径。它内部唯一值得留的逻辑 —— 右侧滚动条与
/// `chat_scrollbar_state` 的更新 —— 也跟着一起落下了，于是长会话里没有任何位置指示。
/// 现在这段逻辑在 `ui/app/mod.rs` 的活路径上（见那里的"滚动条"注释）。
///
/// `#![allow(dead_code)]` 让它能一直躺在树里不被发现：上面那条 bug 就是这么来的。
/// 同类待查对象：`interrupted.rs`（6 个函数零调用）、`global_search.rs`、
/// `theme_picker.rs`、`memory_usage.rs`、`pr_badge.rs`、`cache_pill.rs`。
///
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
            // 传一个不会与真实下标碰撞的哨兵即可。子转录始终显示折叠头（不走隐藏规则）。
            super::message_render::render_non_tool_entry_blocks(
                state,
                sub,
                usize::MAX,
                wrap_width,
                true,
            )
        };
        for b in blocks {
            lines.extend(b);
        }
    }

    Some(lines)
}

/// 渲染单个条目的完整行序列。`pub(crate)`：`store::get_selected_text` 在
/// rendered_cache 缺失时用它兜底重渲染，否则复制会静默失败。
pub(crate) fn render_entry_lines(
    state: &ChatState,
    entry_idx: usize,
    area_width: u16,
) -> Vec<Line<'static>> {
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
            state, entry, area_width, entry_idx,
        ));
    } else if entry.entry_type == ChatEntryType::AgentGroup {
        blocks.extend(super::agent_group_render::render_agent_group(
            state, entry, area_width,
        ));
    } else if entry.entry_type == ChatEntryType::CollapsedGroup {
        blocks.extend(
            super::collapsed_group::render_collapsed_group(state, entry, area_width as usize)
                .into_iter()
                .map(|line| vec![line]),
        );
    } else if entry.entry_type == ChatEntryType::GroupedToolUse {
        // 分组工具调用：展开显示子条目
        if let Some(sub_entries) = &entry.collapsed_entries {
            for sub in sub_entries {
                if super::tool_render::is_tool_entry(sub) {
                    blocks.extend(super::tool_render::render_tool_entry_blocks(
                        state, sub, entry_idx, area_width, false, false, false,
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
            state, entry, entry_idx, wrap_width, false,
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
