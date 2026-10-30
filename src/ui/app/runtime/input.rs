//! 输入缓冲：一批字符攒成整段插入，还是一行一行攒到回车为止。

use super::*;

/// Flush character batch buffer as paste block (>= 8 lines) or direct text insertion.
/// Used for capturing large paste from terminals that don't support Bracketed Paste.
pub(super) fn flush_input_batch(state: &mut ChatState, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    // Skip if Event::Paste was just processed — prevents double-paste when
    // WSL2/terminal sends both Event::Paste and individual char events for one action.
    if let Some(end_time) = state.paste_end_time {
        if end_time.elapsed() < Duration::from_millis(PASTE_DEBOUNCE_MS) {
            buf.clear();
            return;
        }
    }
    let text = std::mem::take(buf);

    // When the palette is open, route chars to the search filter, not the main textarea.
    if state.is_palette_open() {
        for c in text.chars() {
            if !c.is_control() {
                state.palette_filter.push(c);
            }
        }
        state.selected_palette_index = 0;
        return;
    }

    let line_count = text.chars().filter(|&c| c == '\n').count() + 1;
    let is_large = line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES
        || text.len() >= crate::ui::events::clipboard_paste::PASTE_CHAR_THRESHOLD;
    if !state.is_input_modal_open() && is_large {
        state.paste_in_progress = true;
        state.paste_end_time = Some(Instant::now());
        let id = state.paste_segments.len();
        let placeholder = crate::ui::state::format_text_paste_ref(id, line_count);
        state.paste_segments.push(crate::ui::state::PasteSegment {
            id,
            content: text,
            line_count,
            kind: crate::ui::state::PasteKind::Text,
        });
        let (_, cur_col) = state.textarea.cursor();
        if cur_col > 0 {
            state.textarea.insert_newline();
        }
        state.textarea.insert_str(&placeholder);
        state.textarea.insert_newline();
        state.current_status_line = Some(format!(
            "Pasted block #{}: {} lines (continue typing or paste again)",
            id + 1,
            line_count
        ));
    } else if state.is_input_modal_open() {
        state.modal_textarea.insert_str(&text);
    } else {
        crate::ui::events::input::push_cursor_off_sentinel_pub(state);
        state.textarea.insert_str(&text);
        state.input_line_count = state.textarea.lines().len();
        state.input = state.textarea.lines().join("\n");
        crate::ui::services::suggestions::on_input_changed(state);
    }
}
