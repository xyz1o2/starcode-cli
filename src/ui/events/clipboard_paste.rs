use super::*;
use std::time::{Duration, Instant};
use tui_textarea::TextArea;

use arboard::Clipboard;
use chrono::Local;
use image::ImageFormat;
use std::fs;
use crate::ui::state::ChatState;

/// Maximum image dimensions (matching Claude Code's IMAGE_MAX_WIDTH/HEIGHT)
const IMAGE_MAX_WIDTH: u32 = 2000;
const IMAGE_MAX_HEIGHT: u32 = 2000;
/// Maximum file size in bytes (5MB, matching Claude Code's API limit)
const IMAGE_MAX_SIZE_BYTES: u64 = 5 * 1024 * 1024;

pub(crate) fn save_clipboard_image() -> Option<(String, u32, u32)> {
    let mut clipboard = Clipboard::new().ok()?;
    let image_data = clipboard.get_image().ok()?;
    let current_dir = std::env::current_dir().ok()?;
    let images_dir = current_dir.join(".star").join("images");
    fs::create_dir_all(&images_dir).ok()?;
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("image_{}.png", timestamp);
    let file_path = images_dir.join(&filename);
    let width = image_data.width as u32;
    let height = image_data.height as u32;
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        width,
        height,
        image_data.bytes.into_owned(),
    )?;

    // Resize if dimensions exceed limits
    let (final_width, final_height, final_img) = if width > IMAGE_MAX_WIDTH || height > IMAGE_MAX_HEIGHT {
        let ratio_w = IMAGE_MAX_WIDTH as f64 / width as f64;
        let ratio_h = IMAGE_MAX_HEIGHT as f64 / height as f64;
        let ratio = ratio_w.min(ratio_h);
        let new_width = (width as f64 * ratio).round() as u32;
        let new_height = (height as f64 * ratio).round() as u32;
        let resized = image::imageops::resize(&img, new_width, new_height, image::imageops::FilterType::Lanczos3);
        (new_width, new_height, resized)
    } else {
        (width, height, img)
    };

    // Save as PNG first
    final_img.save_with_format(&file_path, ImageFormat::Png).ok()?;

    // Check file size and compress to JPEG if too large
    let file_size = fs::metadata(&file_path).ok()?.len();
    if file_size > IMAGE_MAX_SIZE_BYTES {
        let jpg_path = images_dir.join(format!("image_{}.jpg", timestamp));
        let dynamic_img = image::DynamicImage::ImageRgba8(final_img);
        dynamic_img.save_with_format(&jpg_path, ImageFormat::Jpeg).ok()?;
        // Remove the PNG file and use JPEG instead
        let _ = fs::remove_file(&file_path);
        Some((format!(".star/images/image_{}.jpg", timestamp), final_width, final_height))
    } else {
        Some((format!(".star/images/{}", filename), final_width, final_height))
    }
}

/// 检测文本是否全部为已存在的绝对路径，是则返回路径列表
pub(crate) fn detect_file_paths(text: &str) -> Option<Vec<String>> {
    let non_empty: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if non_empty.is_empty() || non_empty.len() > 20 {
        return None;
    }
    let valid: Vec<String> = non_empty
        .iter()
        .filter_map(|line| {
            let t = line.trim();
            let p = std::path::Path::new(t);
            if p.is_absolute() && p.exists() {
                Some(t.to_string())
            } else {
                None
            }
        })
        .collect();
    if valid.len() == non_empty.len() {
        Some(valid)
    } else {
        None
    }
}

/// 创建图片粘贴块（始终创建，无行数限制）
pub(crate) fn insert_image_paste_block(state: &mut ChatState, path: String, width: u32, height: u32) {
    let content = format!("![Image]({})", path);
    let id = state.paste_segments.len();
    let placeholder = crate::ui::state::format_image_paste_ref(id);
    state.paste_segments.push(crate::ui::state::PasteSegment {
        id,
        content,
        line_count: 1,
        kind: crate::ui::state::PasteKind::Image {
            path: path.clone(),
            width,
            height,
        },
    });
    let (_, cur_col) = state.textarea.cursor();
    if cur_col > 0 {
        state.textarea.insert_newline();
    }
    state.textarea.insert_str(&placeholder);
    state.textarea.insert_newline();
    state.current_status_line = Some(format!("已粘贴图片（{}×{} px → {}）", width, height, path));
}

/// 创建文件路径粘贴块
pub(crate) fn insert_file_paste_block(state: &mut ChatState, paths: Vec<String>) {
    let content = paths.join("\n");
    let id = state.paste_segments.len();
    let placeholder = crate::ui::state::format_files_paste_ref(id);
    let names: Vec<String> = paths
        .iter()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p.as_str())
                .to_string()
        })
        .collect();
    let count = paths.len();
    state.paste_segments.push(crate::ui::state::PasteSegment {
        id,
        content,
        line_count: count,
        kind: crate::ui::state::PasteKind::Files(paths),
    });
    let (_, cur_col) = state.textarea.cursor();
    if cur_col > 0 {
        state.textarea.insert_newline();
    }
    state.textarea.insert_str(&placeholder);
    state.textarea.insert_newline();
    state.current_status_line = Some(format!("已粘贴 {} 个文件路径", count));
    let _ = names;
}


/// 如果光标当前在占位符行，将光标移到下一行，避免在 sentinel 行插入文字导致其损坏
pub(crate) fn push_cursor_off_sentinel_pub(state: &mut ChatState) {
    push_cursor_off_sentinel(state);
}

pub(super) fn push_cursor_off_sentinel(state: &mut ChatState) {
    if state.paste_segments.is_empty() {
        return;
    }
    let (row, _) = state.textarea.cursor();
    if state
        .textarea
        .lines()
        .get(row)
        .and_then(|l| crate::ui::state::parse_paste_reference(l.as_str()))
        .is_some()
    {
        state
            .textarea
            .move_cursor(tui_textarea::CursorMove::Down);
    }
}

pub(super) const PASTE_ENTER_GUARD_MS: u64 = 350;
pub(super) const RAPID_PASTE_KEY_INTERVAL_MS: u64 = 60;

pub(super) fn reset_main_textarea(state: &mut ChatState) {
    state.textarea = TextArea::default();
    state.textarea.set_placeholder_text("Type a message...");
    state
        .textarea
        .set_cursor_line_style(ratatui::style::Style::default());
    state.textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );
    state.input.clear();
    state.input_line_count = 0;
    state.input_folded = false;
    state.paste_segments.clear();
}

pub(crate) fn sync_input_from_textarea(state: &mut ChatState) {
    state.input_line_count = state.textarea.lines().len();
    state.input = state.textarea.lines().join("\n");
    if state.input_line_count < crate::ui::state::INPUT_FOLD_MIN_LINES {
        state.input_folded = false;
    }
    // Auto-save draft
    state.save_draft();
}

pub(crate) fn collect_modal_input(textarea: &TextArea<'_>) -> String {
    textarea.lines().join("\n")
}

pub(super) fn normalize_modal_api_key(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(super) fn normalize_modal_base_url(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(super) fn needs_manual_base_url_confirmation(provider_id: &str, saved_base_url: Option<&str>) -> bool {
    crate::core::config::providers::provider_requires_manual_base_url(provider_id)
        && saved_base_url
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
}

pub(crate) fn maybe_auto_fold_input(state: &mut ChatState) {
    state.input_line_count = state.textarea.lines().len();
    if state.paste_segments.is_empty()
        && state.input_line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES
    {
        state.input_folded = true;
    }
}

pub(crate) fn insert_paste_block(state: &mut ChatState, text: String) {
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("\u{2028}", "\n")
        .replace("\u{2029}", "\n");
    let line_count = normalized.lines().count();

    // Large paste warning: ask for confirmation when pasting 50+ lines
    const PASTE_WARNING_LINES: usize = 50;
    if line_count >= PASTE_WARNING_LINES && !state.show_paste_confirmation {
        state.pending_paste = Some(normalized);
        state.show_paste_confirmation = true;
        return;
    }

    if line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES {
        let id = state.paste_segments.len();
        state.paste_segments.push(crate::ui::state::PasteSegment {
            id,
            content: normalized.clone(),
            line_count,
            kind: crate::ui::state::PasteKind::Text,
        });
        let placeholder = crate::ui::state::format_text_paste_ref(id, line_count);
        let (_, cur_col) = state.textarea.cursor();
        if cur_col > 0 {
            state.textarea.insert_newline();
        }
        state.textarea.insert_str(&placeholder);
        state.textarea.insert_newline();
        state.current_status_line = Some(format!(
            "已粘贴块 #{}: {} 行（继续输入或再次粘贴）",
            id + 1,
            line_count
        ));
    } else {
        state.textarea.insert_str(&normalized);
    }
}

/// 确认后直接插入粘贴块（跳过大段粘贴警告）
pub(crate) fn insert_paste_block_confirmed(state: &mut ChatState, text: String) {
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace("\u{2028}", "\n")
        .replace("\u{2029}", "\n");
    let line_count = normalized.lines().count();
    if line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES {
        let id = state.paste_segments.len();
        state.paste_segments.push(crate::ui::state::PasteSegment {
            id,
            content: normalized.clone(),
            line_count,
            kind: crate::ui::state::PasteKind::Text,
        });
        let placeholder = crate::ui::state::format_text_paste_ref(id, line_count);
        let (_, cur_col) = state.textarea.cursor();
        if cur_col > 0 {
            state.textarea.insert_newline();
        }
        state.textarea.insert_str(&placeholder);
        state.textarea.insert_newline();
        state.current_status_line = Some(format!(
            "已粘贴块 #{}: {} 行（继续输入或再次粘贴）",
            id + 1,
            line_count
        ));
    } else {
        state.textarea.insert_str(&normalized);
    }
}

