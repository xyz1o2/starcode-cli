use super::*;
use std::time::{Duration, Instant};
use tui_textarea::TextArea;

use crate::ui::state::ChatState;
use arboard::Clipboard;
use image::ImageFormat;
use std::fs;

/// Maximum image dimensions (matching Claude Code's IMAGE_MAX_WIDTH/HEIGHT)
const IMAGE_MAX_WIDTH: u32 = 2000;
const IMAGE_MAX_HEIGHT: u32 = 2000;
/// Maximum file size in bytes (5MB, matching Claude Code's API limit)
const IMAGE_MAX_SIZE_BYTES: u64 = 5 * 1024 * 1024 / 4 * 3;

pub(crate) fn save_clipboard_image() -> Option<(String, u32, u32)> {
    // WSL 优先走 Windows 剪贴板桥：arboard 在 WSL 里连的是 X11/Wayland 的
    // 剪贴板，而 Windows 侧复制的图片进不了那个世界 —— 用户复制截图后
    // Ctrl+V 永远拿到的是空。powershell.exe 的 `Get-Clipboard -Format Image`
    // 读的是 Windows 自己的剪贴板，才是对的源。失败再回落 arboard。
    #[cfg(target_os = "linux")]
    if crate::core::prompts::env_info::is_wsl() {
        if let Some(result) = save_clipboard_image_wsl() {
            return Some(result);
        }
    }

    let mut clipboard = Clipboard::new().ok()?;
    let image_data = clipboard.get_image().ok()?;
    let width = u32::try_from(image_data.width).ok()?;
    let height = u32::try_from(image_data.height).ok()?;
    let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        width,
        height,
        image_data.bytes.into_owned(),
    )?;

    save_pasted_image(image::DynamicImage::ImageRgba8(img))
}

/// 两条剪贴板来源共用解码后的图片处理，保存前验证尺寸和编码体积。
fn save_pasted_image(img: image::DynamicImage) -> Option<(String, u32, u32)> {
    let storage = crate::core::config::storage::Storage::new(std::env::current_dir().ok()?);
    save_pasted_image_in(img, &storage.star_dir().join("images"))
}

fn save_pasted_image_in(
    img: image::DynamicImage,
    images_dir: &std::path::Path,
) -> Option<(String, u32, u32)> {
    use std::io::{Cursor, Write};

    if img.width() == 0 || img.height() == 0 {
        return None;
    }
    let img = if img.width() > IMAGE_MAX_WIDTH || img.height() > IMAGE_MAX_HEIGHT {
        img.resize(
            IMAGE_MAX_WIDTH,
            IMAGE_MAX_HEIGHT,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let mut encoded = Cursor::new(Vec::new());
    img.write_to(&mut encoded, ImageFormat::Png).ok()?;
    let extension = if encoded.get_ref().len() as u64 > IMAGE_MAX_SIZE_BYTES {
        encoded = Cursor::new(Vec::new());
        // JPEG 编码器不接受 RGBA；显式转 RGB 后再检查体积。
        image::DynamicImage::ImageRgb8(img.to_rgb8())
            .write_to(&mut encoded, ImageFormat::Jpeg)
            .ok()?;
        "jpg"
    } else {
        "png"
    };
    if encoded.get_ref().len() as u64 > IMAGE_MAX_SIZE_BYTES {
        return None;
    }
    fs::create_dir_all(images_dir).ok()?;
    let path = images_dir.join(format!("image_{}.{}", uuid::Uuid::new_v4(), extension));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .ok()?;
    if file.write_all(encoded.get_ref()).is_err() {
        drop(file);
        let _ = fs::remove_file(&path);
        return None;
    }
    Some((
        path.to_string_lossy().into_owned(),
        img.width(),
        img.height(),
    ))
}

/// WSL 下通过 STA PowerShell 读取 Windows 图片，解码后保存到项目状态目录。
///
/// 返回 None 表示 Windows 侧没有图片（或桥失败）——调用方回落 arboard。
/// 剪贴板里是文本时不输出图片路径，同样走回落。
#[cfg(target_os = "linux")]
fn save_clipboard_image_wsl() -> Option<(String, u32, u32)> {
    use std::process::Command;

    let win_temp = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-STA",
            "-Command",
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
             Add-Type -AssemblyName System.Windows.Forms; \
             $img = [System.Windows.Forms.Clipboard]::GetImage(); \
             if ($img) { \
               $p = Join-Path $env:TEMP ('starcode_clip_' + [guid]::NewGuid().ToString() + '.png'); \
               try { $img.Save($p, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output $p } \
               finally { $img.Dispose() } \
             }",
        ])
        .output()
        .ok()?;
    if !win_temp.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&win_temp.stdout);
    let path = windows_clipboard_path(stdout.trim())?;
    let decoded = image::open(&path);
    // 只清理本次桥创建的唯一临时文件。
    let _ = fs::remove_file(&path);
    save_pasted_image(decoded.ok()?)
}

/// Windows 临时路径按实际盘符映射到 WSL 挂载点。
fn windows_clipboard_path(path: &str) -> Option<std::path::PathBuf> {
    let normalized = path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":/" {
        return None;
    }
    let name = normalized.rsplit('/').next()?;
    if !name.starts_with("starcode_clip_") || !name.ends_with(".png") {
        return None;
    }
    Some(std::path::PathBuf::from(format!(
        "/mnt/{}/{}",
        (bytes[0] as char).to_ascii_lowercase(),
        &normalized[3..]
    )))
}

pub(crate) fn clipboard_target(state: &ChatState) -> Option<crate::ui::state::ClipboardTarget> {
    use crate::ui::state::ClipboardTarget;
    if state.is_awaiting_confirmation {
        let tool_call_id = state.pending_tool_call_id.clone()?;
        let is_question = state
            .pending_confirmation_entry_idx
            .and_then(|idx| state.chat_history.get(idx))
            .and_then(|entry| entry.confirmation.as_ref())
            .is_some_and(|confirmation| {
                matches!(
                    confirmation.details,
                    crate::types::ConfirmationDetails::AskUserQuestion { .. }
                )
            });
        return Some(if is_question {
            ClipboardTarget::Question {
                tool_call_id,
                question_index: state.pending_question_index,
            }
        } else {
            ClipboardTarget::Feedback { tool_call_id }
        });
    }
    if !matches!(
        crate::ui::events::key_context::KeyContext::of(state),
        crate::ui::events::key_context::KeyContext::TextInput
            | crate::ui::events::key_context::KeyContext::Vim
            | crate::ui::events::key_context::KeyContext::TaskPanel
            | crate::ui::events::key_context::KeyContext::SuggestionOverlay
            | crate::ui::events::key_context::KeyContext::BgAgentSelector
    ) {
        return None;
    }
    Some(ClipboardTarget::Prompt {
        session_id: state.active_session_id.clone(),
        session_started_at: state.session_started_at,
        next_message_id: state.next_message_id,
    })
}

/// 确认卡的文本粘贴与 Ctrl+V 共用目标；不把内容漏进背后的主输入框。
pub(crate) fn paste_confirmation_text(state: &mut ChatState, text: &str) -> bool {
    use crate::ui::state::ClipboardTarget;
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    match clipboard_target(state) {
        Some(ClipboardTarget::Question { question_index, .. }) => {
            let other_idx = state
                .pending_confirmation_entry_idx
                .and_then(|idx| state.chat_history.get(idx))
                .and_then(|entry| entry.confirmation.as_ref())
                .and_then(|confirmation| match &confirmation.details {
                    crate::types::ConfirmationDetails::AskUserQuestion { questions } => questions
                        .get(question_index)
                        .map(|question| question.options.len()),
                    _ => None,
                });
            let Some(other_idx) = other_idx else {
                return false;
            };
            state.pending_confirmation_choice = other_idx;
            state.pending_question_other_focused = true;
            state.pending_other_input.push_str(&normalized);
        }
        Some(ClipboardTarget::Feedback { .. }) => {
            state.confirmation_feedback_mode = true;
            state.pending_confirmation_feedback.push_str(&normalized);
        }
        _ => return false,
    }
    if let Some(idx) = state.pending_confirmation_entry_idx {
        state.rendered_cache.remove(&idx);
        state.virtual_list.mark_dirty(idx);
    }
    true
}

/// 事件循环取回后台剪贴板结果；读取期间继续处理键盘和模型消息。
pub(crate) fn poll_clipboard_read(state: &mut ChatState) -> bool {
    use crate::ui::state::{ClipboardPayload, ClipboardTarget};
    use tokio::sync::oneshot::error::TryRecvError;
    let Some(pending) = state.pending_clipboard_read.as_mut() else {
        return false;
    };
    let result = match pending.result.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return false,
        Err(TryRecvError::Closed) => Err("Clipboard reader stopped".to_string()),
    };
    let target = state.pending_clipboard_read.take().unwrap().target;
    state.paste_in_progress = false;
    state.paste_end_time = Some(Instant::now());
    if clipboard_target(state).as_ref() != Some(&target) {
        state.push_toast(
            &crate::core::i18n::t1("ui.paste.target_changed"),
            crate::ui::state::ToastKind::Info,
        );
        return true;
    }
    match result {
        Ok(ClipboardPayload::Image(path, width, height)) => {
            if matches!(target, ClipboardTarget::Prompt { .. }) {
                insert_image_paste_block(state, path, width, height);
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            } else {
                insert_image_paste_block(state, path, width, height);
                sync_input_from_textarea(state);
                state.push_toast(
                    &crate::core::i18n::t1("ui.image.saved_to_prompt"),
                    crate::ui::state::ToastKind::Info,
                );
            }
        }
        Ok(ClipboardPayload::Text(text)) => {
            if !paste_confirmation_text(state, &text) {
                if let Some(paths) = detect_file_paths(&text) {
                    insert_file_paste_block(state, paths);
                } else {
                    insert_paste_block(state, text);
                    maybe_auto_fold_input(state);
                }
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        Err(error) => {
            crate::utils::logging::append_debug_log_line(&format!("[PASTE] {error}"));
            state.push_toast(
                &crate::core::i18n::t1("ui.paste.failed"),
                crate::ui::state::ToastKind::Error,
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::{ClipboardPayload, PendingClipboardRead};

    fn ready_clipboard(state: &mut ChatState, payload: ClipboardPayload) {
        let target = clipboard_target(state).expect("input target");
        let (tx, result) = tokio::sync::oneshot::channel();
        assert!(tx.send(Ok(payload)).is_ok());
        state.pending_clipboard_read = Some(PendingClipboardRead { target, result });
        state.paste_in_progress = true;
    }

    #[test]
    fn image_paste_preserves_running_status() {
        let mut state = ChatState::test_fixture();
        state.current_status_line = Some("Waiting for provider".into());
        insert_image_paste_block(&mut state, "/tmp/image.png".into(), 686, 411);
        assert_eq!(
            state.current_status_line.as_deref(),
            Some("Waiting for provider")
        );
        assert_eq!(state.paste_segments.len(), 1);
    }

    #[test]
    fn clipboard_result_does_not_cross_sessions_or_submissions() {
        let mut state = ChatState::test_fixture();
        ready_clipboard(&mut state, ClipboardPayload::Text("old clipboard".into()));
        state.active_session_id = Some("another-session".into());
        assert!(poll_clipboard_read(&mut state));
        assert!(state.input.is_empty());
        assert!(state.pending_clipboard_read.is_none());

        ready_clipboard(&mut state, ClipboardPayload::Text("old draft".into()));
        state.next_message_id += 1;
        assert!(poll_clipboard_read(&mut state));
        assert!(state.input.is_empty());
    }

    #[test]
    fn resetting_prompt_discards_pending_clipboard() {
        let mut state = ChatState::test_fixture();
        ready_clipboard(&mut state, ClipboardPayload::Text("old draft".into()));
        reset_main_textarea(&mut state);
        assert!(!poll_clipboard_read(&mut state));
        assert!(!state.paste_in_progress);
        assert!(state.input.is_empty());
    }

    #[test]
    fn pasted_question_text_selects_other_without_touching_prompt() {
        let mut state = ChatState::test_fixture();
        let mut entry = crate::types::ChatEntry::new(
            crate::types::ChatEntryType::ToolConfirmation,
            String::new(),
        );
        entry.confirmation = Some(crate::types::ToolConfirmation {
            tool_name: "AskUserQuestion".into(),
            operation_type: crate::types::ConfirmationType::AskUserQuestion,
            details: crate::types::ConfirmationDetails::AskUserQuestion {
                questions: vec![crate::types::AskUserQuestionItem {
                    question: "Choose".into(),
                    header: None,
                    options: vec![crate::types::AskUserQuestionOption {
                        label: "Default".into(),
                        description: String::new(),
                        preview: None,
                    }],
                    multi_select: false,
                }],
            },
            is_dangerous: false,
            outcome: None,
            reason: None,
        });
        let idx = state.chat_history.len();
        state.chat_history.push(entry);
        state.is_awaiting_confirmation = true;
        state.pending_tool_call_id = Some("question-1".into());
        state.pending_confirmation_entry_idx = Some(idx);
        state.pending_confirmation_choice = 0;

        assert!(paste_confirmation_text(&mut state, "custom\r\nanswer"));
        assert_eq!(state.pending_confirmation_choice, 1);
        assert!(state.pending_question_other_focused);
        assert_eq!(state.pending_other_input, "custom\nanswer");
        assert!(state.input.is_empty());
    }

    #[test]
    fn clipboard_image_files_are_unique_and_resized() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::DynamicImage::new_rgba8(2400, 1200);
        let (first, width, height) = save_pasted_image_in(image.clone(), dir.path()).unwrap();
        let (second, _, _) = save_pasted_image_in(image, dir.path()).unwrap();
        assert_ne!(first, second);
        assert_eq!((width, height), (2000, 1000));
        let decoded = image::open(first).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (width, height));
        assert!(fs::metadata(second).unwrap().len() <= IMAGE_MAX_SIZE_BYTES);
    }

    #[test]
    fn windows_clipboard_path_uses_actual_drive() {
        assert_eq!(
            windows_clipboard_path(r"D:\Temp\starcode_clip_example.png"),
            Some(std::path::PathBuf::from(
                "/mnt/d/Temp/starcode_clip_example.png"
            ))
        );
        assert!(windows_clipboard_path(r"D:\Temp\other.png").is_none());
        assert!(windows_clipboard_path("/tmp/starcode_clip_example.png").is_none());
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

/// 撤回或重试时直接恢复已保存的图片内容，不依赖原临时文件。
pub(crate) fn restore_image_attachments(
    state: &mut ChatState,
    images: Vec<crate::types::ImageAttachment>,
) {
    if images.is_empty() {
        return;
    }
    let intent = state.input_intent;
    let existing_segments = std::mem::take(&mut state.paste_segments);
    let mut text = state.textarea.lines().join("\n");
    let offset = existing_segments.len();
    let mut segments = existing_segments;
    for (index, image) in images.into_iter().enumerate() {
        let id = offset + index;
        let reference = crate::ui::state::format_image_paste_ref(id);
        if let Some(marker) = text.find("[Image]") {
            text.replace_range(marker..marker + "[Image]".len(), &reference);
        } else {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&reference);
        }
        segments.push(crate::ui::state::PasteSegment {
            id,
            content: "[Image]".to_string(),
            line_count: 1,
            kind: crate::ui::state::PasteKind::AttachedImage(image),
        });
    }
    state.set_input_text(&text);
    state.input_intent = intent;
    state.paste_segments = segments;
    sync_input_from_textarea(state);
}

/// 创建图片粘贴块（始终创建，无行数限制）
pub(crate) fn insert_image_paste_block(
    state: &mut ChatState,
    path: String,
    width: u32,
    height: u32,
) {
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
    state.push_toast(
        &format!(
            "{} · {}×{}",
            crate::core::i18n::t1("paste.image"),
            width,
            height
        ),
        crate::ui::state::ToastKind::Info,
    );
}

/// 创建文件路径粘贴块
pub(crate) fn insert_file_paste_block(state: &mut ChatState, paths: Vec<String>) {
    state.input_intent = crate::types::InputIntent::Literal;
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
    state.current_status_line = Some(format!(
        "{} {} {}",
        crate::core::i18n::t1("paste.files"),
        count,
        crate::core::i18n::t1("paste.file_paths")
    ));
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
        state.textarea.move_cursor(tui_textarea::CursorMove::Down);
    }
}

pub(super) const PASTE_ENTER_GUARD_MS: u64 = 350;
pub(super) const RAPID_PASTE_KEY_INTERVAL_MS: u64 = 60;
/// Character count threshold — long single-line pastes also become paste blocks
/// (matching Claude Code's PASTE_THRESHOLD = 800 with margin)
pub(crate) const PASTE_CHAR_THRESHOLD: usize = 1000;

pub(super) fn reset_main_textarea(state: &mut ChatState) {
    // 清空或提交草稿后，旧剪贴板结果不能重新填回输入框。
    if state
        .pending_clipboard_read
        .as_ref()
        .is_some_and(|pending| {
            matches!(
                &pending.target,
                crate::ui::state::ClipboardTarget::Prompt { .. }
            )
        })
    {
        state.pending_clipboard_read = None;
        state.paste_in_progress = false;
        state.paste_end_time = None;
    }
    state.input_intent = crate::types::InputIntent::CommandEligible;
    state.textarea = TextArea::default();
    state.textarea.set_placeholder_text("Type a message...");
    state
        .textarea
        .set_cursor_line_style(ratatui::style::Style::default());
    state
        .textarea
        .set_cursor_render_mode(tui_textarea::CursorRenderMode::Hidden);
    state
        .textarea
        .set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);
    state.input.clear();
    state.input_line_count = 0;
    state.input_folded = false;
    state.paste_segments.clear();
}

pub(crate) fn sync_input_from_textarea(state: &mut ChatState) {
    state.input_line_count = state.textarea.lines().len();
    state.input = state.textarea.lines().join("\n");
    if state.input.is_empty() && state.input_intent == crate::types::InputIntent::Literal {
        // 完整删除正文时清掉 textarea 的 undo 历史，不能撤销后恢复为可执行命令。
        // 粘贴段本身仍保留：删除占位符只是取消本次引用，已有测试和后续粘贴
        // 逻辑需要它继续作为不可见的段记录存在。
        let segments = std::mem::take(&mut state.paste_segments);
        reset_main_textarea(state);
        state.paste_segments = segments;
    }
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

    if !normalized.is_empty() {
        state.input_intent = crate::types::InputIntent::Literal;
    }
    let is_large = line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES
        || normalized.len() >= PASTE_CHAR_THRESHOLD;
    if is_large {
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
            "{} #{}: {} {}",
            crate::core::i18n::t1("paste.block"),
            id + 1,
            line_count,
            crate::core::i18n::t1("paste.lines_with_hint")
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
    if !normalized.is_empty() {
        state.input_intent = crate::types::InputIntent::Literal;
    }
    let is_large = line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES
        || normalized.len() >= PASTE_CHAR_THRESHOLD;
    if is_large {
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
            "{} #{}: {} {}",
            crate::core::i18n::t1("paste.block"),
            id + 1,
            line_count,
            crate::core::i18n::t1("paste.lines_with_hint")
        ));
    } else {
        state.textarea.insert_str(&normalized);
    }
}
