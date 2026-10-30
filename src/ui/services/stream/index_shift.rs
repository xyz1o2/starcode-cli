//! 条目增删之后，渲染缓存 / 行高 / 索引表要整体位移。
//!
use super::*;

/// 在 pos 处向 chat_history 中插入条目后，修正所有按索引的缓存/映射（>=pos 的 +1）。
/// 否则中插会让 rendered_cache、流式键、消息起止索引等全部错位。
pub(super) fn shift_index_caches_after_insert(state: &mut ChatState, pos: usize) {
    state.virtual_list.insert_at(pos);
    // 选区按 entry 索引存储：中插后把锚点索引自适应 +1，保住进行中的拖选。
    // 此前直接 clear——agent 流式期间每次插入条目（工具块等）选区就被清掉，
    // 表现为“拖选总是断”。插入点之前的锚点不受影响；插入点落在选区中间时，
    // 新条目按“中间条目”语义整体高亮，可接受。
    if let Some(s) = state.text_selection.start_entry_idx.as_mut() {
        if *s >= pos {
            *s += 1;
        }
    }
    if let Some(e) = state.text_selection.end_entry_idx.as_mut() {
        if *e >= pos {
            *e += 1;
        }
    }
    if state.last_item_heights.len() >= pos {
        state.last_item_heights.insert(pos, 0);
    } else {
        state.last_item_heights.push(0);
    }

    // rendered_cache: HashMap<usize, (u16, Vec<Line>)>
    let remixed: std::collections::HashMap<usize, _> = state
        .rendered_cache
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.rendered_cache = remixed;

    // last_rendered_stream_key
    let remixed: std::collections::HashMap<usize, _> = state
        .last_rendered_stream_key
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.last_rendered_stream_key = remixed;

    // streaming_height_floor
    let remixed: std::collections::HashMap<usize, _> = state
        .streaming_height_floor
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.streaming_height_floor = remixed;

    // 按消息 id 存的索引值
    for v in state.message_start_indices.values_mut() {
        if *v >= pos {
            *v += 1;
        }
    }
    for v in state.stream_targets.values_mut() {
        if *v >= pos {
            *v += 1;
        }
    }
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx >= pos {
            *idx += 1;
        }
    }
    for queued in state.pending_confirmation_queue.iter_mut() {
        if queued.entry_idx >= pos {
            queued.entry_idx += 1;
        }
    }
}

pub(super) fn shift_indexed_cache_after_removal<T>(
    cache: &mut std::collections::HashMap<usize, T>,
    pos: usize,
) {
    *cache = cache
        .drain()
        .filter_map(|(idx, value)| match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some((idx, value)),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some((idx - 1, value)),
        })
        .collect();
}

/// 在 pos 处从 chat_history 移除条目后，修正所有按索引的缓存/映射（>pos 的 -1）。
/// 删除目标自身的渲染、流式和选择状态，防止后续更新指向相邻条目。
pub(super) fn remove_chat_entry_and_shift_indices(state: &mut ChatState, pos: usize) {
    if pos >= state.chat_history.len() {
        return;
    }

    state.chat_history.remove(pos);
    state.virtual_list.remove_at(pos);
    state.virtual_list.mark_dirty_all();
    // 选区锚点指向被删除条目时无法挽救，清除；否则索引自适应 -1，
    // 保住其余部分的拖选（此前无条件 clear）。
    let sel_hits_removed = matches!(state.text_selection.start_entry_idx, Some(i) if i == pos)
        || matches!(state.text_selection.end_entry_idx, Some(i) if i == pos);
    if sel_hits_removed {
        state.text_selection.clear();
    } else {
        if let Some(s) = state.text_selection.start_entry_idx.as_mut() {
            if *s > pos {
                *s -= 1;
            }
        }
        if let Some(e) = state.text_selection.end_entry_idx.as_mut() {
            if *e > pos {
                *e -= 1;
            }
        }
    }
    state.expanded_thinking_indices = state
        .expanded_thinking_indices
        .drain()
        .filter_map(|idx| match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some(idx),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(idx - 1),
        })
        .collect();

    if pos < state.last_item_heights.len() {
        state.last_item_heights.remove(pos);
    }

    shift_indexed_cache_after_removal(&mut state.rendered_cache, pos);
    shift_indexed_cache_after_removal(&mut state.last_rendered_stream_key, pos);
    shift_indexed_cache_after_removal(&mut state.streaming_height_floor, pos);

    for idx in state.message_start_indices.values_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            *idx = pos.min(state.chat_history.len());
        }
    }
    for idx in state.stream_targets.values_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            *idx = pos.min(state.chat_history.len());
        }
    }
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            state.pending_confirmation_entry_idx = None;
        }
    }
    state
        .pending_confirmation_queue
        .retain(|queued| queued.entry_idx != pos);
    for queued in state.pending_confirmation_queue.iter_mut() {
        if queued.entry_idx > pos {
            queued.entry_idx -= 1;
        }
    }
    if let Some(idx) = state.preview_last_entry_idx.as_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            state.preview_last_entry_idx = None;
        }
    }
    if let Some(idx) = state.chat_list_state.selected() {
        let remapped = match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some(idx),
            std::cmp::Ordering::Equal => {
                (!state.chat_history.is_empty()).then(|| pos.min(state.chat_history.len() - 1))
            }
            std::cmp::Ordering::Greater => Some(idx - 1),
        };
        state.chat_list_state.select(remapped);
    }
    for info in state.active_agent_tasks.values_mut() {
        if info.entry_idx > pos {
            info.entry_idx -= 1;
        }
    }

    state.total_rendered_lines = state.virtual_list.total_lines();
    state.scroll = state.scroll.min(state.total_rendered_lines);
}
