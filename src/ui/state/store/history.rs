//! 会话历史的整体操作：滚动、索引表位移。
//!
use super::*;

pub fn scroll_chat(state: &mut ChatState, delta: i32) {
    if state.total_rendered_lines == 0 {
        state.scroll = 0;
        return;
    }

    // Viewport height is needed to calculate max scroll.
    let viewport_height = if state.last_chat_height > 0 {
        state.last_chat_height as usize
    } else {
        state
            .last_chat_area
            .map(|a| a.height as usize)
            .unwrap_or(0usize)
    };
    let max_scroll = state.total_rendered_lines.saturating_sub(viewport_height);

    // Keep scroll in valid range before applying new delta.
    if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }

    // Direct scroll application (no momentum for predictable behavior)
    if delta < 0 {
        // Scrolling up — pause auto-follow so user can read history
        let d = delta.unsigned_abs() as usize;
        state.scroll = state.scroll.saturating_sub(d);
        state.auto_follow = false;
    } else {
        // Scrolling down
        let d = delta as usize;
        state.scroll = state.scroll.saturating_add(d).min(max_scroll);
        if state.scroll >= max_scroll {
            state.auto_follow = true;
            state.show_scroll_to_bottom = false;
        }
    }
}

pub fn bump_index_map(map: &mut HashMap<u64, usize>, from: usize, delta: usize) {
    for v in map.values_mut() {
        if *v >= from {
            *v += delta;
        }
    }
}

pub fn bump_indices_after_insert(state: &mut ChatState, from: usize, delta: usize) {
    // 插入元素会导致后续索引变化，需要平移渲染缓存而不是全部清理
    let mut new_cache = HashMap::new();
    for (idx, value) in state.rendered_cache.drain() {
        if idx >= from {
            new_cache.insert(idx + delta, value);
        } else {
            new_cache.insert(idx, value);
        }
    }
    state.rendered_cache = new_cache;

    bump_index_map(&mut state.stream_targets, from, delta);
    bump_index_map(&mut state.message_start_indices, from, delta);
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx >= from {
            *idx += delta;
        }
    }
    for queued in state.pending_confirmation_queue.iter_mut() {
        if queued.entry_idx >= from {
            queued.entry_idx += delta;
        }
    }
}
