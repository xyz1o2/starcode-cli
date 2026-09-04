/// UI Runtime — Main event loop and terminal management.
///
/// # Architecture
///
/// This module implements the core UI event loop with the following design:
///
/// ## Threading Model
/// - **Main thread (tokio)**: Runs the UI loop, renders frames, processes keyboard events
/// - **Key reader thread**: Dedicated `std::thread` reading from `/dev/tty` via crossterm
/// - **Agent worker**: Tokio task processing LLM requests
/// - **Watchdog**: Daemon thread monitoring UI responsiveness
///
/// ## Event Flow
/// ```text
/// /dev/tty → [Key Reader Thread] → mpsc channel → [UI Loop] → handle_key_event
/// [Agent Worker] → mpsc channel → [UI Loop] → handle_stream_update → terminal.draw
/// ```
///
/// ## Frame Budget
/// - Idle: ~30 FPS (33ms per frame)
/// - Streaming: Adaptive 20-30 FPS based on message rate
/// - Stream messages processed in 8ms batches to prevent input lag
///
/// ## Error Recovery
/// - Terminal initialization: 3 retries with exponential backoff
/// - Keyboard thread death: Detected via AtomicBool, shows error message
/// - Watchdog: Detects ANR (>5s unresponsive), logs warnings
///
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal as RatatuiTerminal;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::core::utils::watchdog::Watchdog;
use crate::runtime::messages::{AgentRequest, StreamMessage};
use crate::ui::events::clipboard_paste::{
    detect_file_paths, insert_file_paste_block, insert_image_paste_block, insert_paste_block,
    maybe_auto_fold_input, save_clipboard_image, sync_input_from_textarea,
};
use crate::ui::state::ChatState;

/// Result from clipboard operations (to avoid blocking async runtime)
enum ClipboardResult {
    Image(String, u32, u32),
    Text(String),
}
use std::sync::Arc;

/// Minimum interval between two paste events — skips the second one when
/// WSL2/Windows Terminal sends both Event::Paste and individual char events for a single paste action.
const PASTE_DEBOUNCE_MS: u64 = 200;

/// Prevent escape sequences from being split into independent key events due to crossterm's internal ESC timeout (~100ms).
/// On affected terminals (legacy Windows console, WSL2 PTY, high-latency SSH),
/// arrow key ESC [ A may be split into Esc + Char('[') + Char('A'),
/// where Char fragments get collected by the character batching logic into char_buf and inserted as garbage into the input box.
///
/// This guard uses timing heuristics to detect split sequences:
/// 1. CSI introducer ('[' or 'O') appearing within 250ms after Esc → enter absorb mode
/// 2. In absorb mode, all subsequent Char events are intercepted and collected into fragment_buf
/// 3. After 500ms the window closes and collected bytes are reconstructed into a correct KeyEvent
/// 4. If reconstruction succeeds, dispatch the reconstructed key event; otherwise discard the fragments
///
/// Safety assumption: A human cannot press a physical Esc key and then type '[' or 'O' within 250ms.
struct EscapeGuard {
    last_esc: Option<Instant>,
    absorbing_deadline: Option<Instant>,
    fragment_buf: String,
    /// Characters that were absorbed but don't form a valid escape sequence.
    /// These are pending normal characters that should be added to char_buf.
    pending_chars: Option<String>,
}

impl EscapeGuard {
    const CSI_INTRODUCERS: &[char] = &['[', 'O'];
    // Escape sequences arrive as a burst of bytes within a few ms, even over
    // high-latency connections (WSL2, SSH). Keep windows tight so standalone Esc
    // (cancel, dismiss) responds instantly without absorbing subsequent keystrokes.
    const TRIGGER_WINDOW_MS: u64 = 20;
    const ABSORB_DURATION_MS: u64 = 100;

    const fn new() -> Self {
        Self {
            last_esc: None,
            absorbing_deadline: None,
            fragment_buf: String::new(),
            pending_chars: None,
        }
    }

    /// Take any pending characters that were absorbed but not recognized as escape sequences.
    /// Returns the pending characters string, if any.
    fn take_pending_chars(&mut self) -> Option<String> {
        self.pending_chars.take()
    }

    fn on_esc(&mut self) {
        self.last_esc = Some(Instant::now());
    }

    /// Judge the received Char character. Returns:
    /// - false: normal character, should enter char_buf
    /// - true: escape sequence fragment, should be discarded
    fn feed_char(&mut self, c: char) -> bool {
        let now = Instant::now();

        // 1) Already within absorb window
        if let Some(deadline) = self.absorbing_deadline {
            if now < deadline {
                self.fragment_buf.push(c);
                return true;
            }
            // Window expired, clear state
            self.absorbing_deadline = None;
            self.fragment_buf.clear();
            self.last_esc = None;
            // Continue checking if current character opens a new sequence
        }

        // 2) Check Esc + CSI introducer pattern
        if let Some(esc_at) = self.last_esc {
            if esc_at.elapsed() < Duration::from_millis(Self::TRIGGER_WINDOW_MS)
                && Self::CSI_INTRODUCERS.contains(&c)
            {
                self.absorbing_deadline =
                    Some(now + Duration::from_millis(Self::ABSORB_DURATION_MS));
                self.last_esc = None;
                self.fragment_buf.clear();
                self.fragment_buf.push(c);
                return true;
            }
            self.last_esc = None;
        }

        false
    }

    /// Periodically call. If the absorb window expired and fragments were collected, try to reconstruct into KeyEvent.
    fn check_timeout(&mut self) -> Option<crossterm::event::KeyEvent> {
        if let Some(deadline) = self.absorbing_deadline {
            if Instant::now() >= deadline {
                // First try to reconstruct as an escape sequence
                if let Some(key) = Self::reconstruct(&self.fragment_buf) {
                    self.absorbing_deadline = None;
                    self.fragment_buf.clear();
                    self.last_esc = None;
                    return Some(key);
                }

                // EscapeGuard absorbed characters that don't form a valid escape sequence.
                // This happens when the user was typing in an input field and pressed
                // Esc (triggering the guard) followed by regular keys - the escaped chars
                // were intended as normal input, not escape sequences.
                // Store the fragments so runtime can treat them as pending input.
                let pending = self.fragment_buf.clone();
                self.fragment_buf.clear();
                self.pending_chars = Some(pending);
                self.absorbing_deadline = None;
                self.last_esc = None;
                return None;
            }
        }
        // Clear expired last_esc
        if let Some(esc_at) = self.last_esc {
            if esc_at.elapsed() >= Duration::from_millis(Self::TRIGGER_WINDOW_MS + 50) {
                self.last_esc = None;
            }
        }
        None
    }

    fn reconstruct(fragment: &str) -> Option<crossterm::event::KeyEvent> {
        // First try matching sequences with modifiers, such as [1;2A (Shift+Up), [1;5D (Ctrl+Left)
        if let Some((code, mods)) = Self::reconstruct_with_modifiers(fragment) {
            return Some(crossterm::event::KeyEvent::new(code, mods));
        }

        let code = match fragment {
            // CSI sequences (ESC [ ...)
            "[A" => KeyCode::Up,
            "[B" => KeyCode::Down,
            "[C" => KeyCode::Right,
            "[D" => KeyCode::Left,
            "[H" => KeyCode::Home,
            "[F" => KeyCode::End,
            "[Z" => KeyCode::BackTab,
            "[3~" => KeyCode::Delete,
            "[5~" => KeyCode::PageUp,
            "[6~" => KeyCode::PageDown,
            "[2~" => KeyCode::Insert,
            "[1~" => KeyCode::Home,
            "[4~" => KeyCode::End,
            "[7~" => KeyCode::Home,
            "[8~" => KeyCode::End,
            // SS3 sequences (ESC O ...)
            "OP" => KeyCode::F(1),
            "OQ" => KeyCode::F(2),
            "OR" => KeyCode::F(3),
            "OS" => KeyCode::F(4),
            "OH" => KeyCode::Home,
            "OF" => KeyCode::End,
            _ => return None,
        };
        Some(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Parse CSI sequences with modifiers, e.g. [1;2A → (Up, SHIFT)
    fn reconstruct_with_modifiers(fragment: &str) -> Option<(KeyCode, KeyModifiers)> {
        // Format: [1;<mod>X where X is the terminator character
        let inner = fragment.strip_prefix('[')?;
        // Must contain ';' to be a modifier key sequence
        let (_num, rest) = inner.split_once(';')?;
        if rest.len() < 2 {
            return None;
        }
        let mod_byte = rest.as_bytes();
        // Parse modifier key parameter (1-based: 2=Shift, 3=Alt, 4=Shift+Alt, 5=Ctrl, 6=Ctrl+Shift, 7=Ctrl+Alt, 8=Ctrl+Alt+Shift)
        let mods;
        let mut idx = 0;
        while idx < mod_byte.len() && mod_byte[idx].is_ascii_digit() {
            idx += 1;
        }
        let mod_val: u8 = rest[..idx].parse().ok()?;
        match mod_val {
            2 => mods = KeyModifiers::SHIFT,
            3 => mods = KeyModifiers::ALT,
            4 => mods = KeyModifiers::SHIFT | KeyModifiers::ALT,
            5 => mods = KeyModifiers::CONTROL,
            6 => mods = KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            7 => mods = KeyModifiers::CONTROL | KeyModifiers::ALT,
            8 => mods = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
            _ => return None,
        }
        let final_byte = rest[idx..].as_bytes();
        let code = match final_byte {
            b"A" => KeyCode::Up,
            b"B" => KeyCode::Down,
            b"C" => KeyCode::Right,
            b"D" => KeyCode::Left,
            b"H" => KeyCode::Home,
            b"F" => KeyCode::End,
            b"Z" => KeyCode::BackTab,
            b"P" => KeyCode::F(1),
            b"Q" => KeyCode::F(2),
            b"R" => KeyCode::F(3),
            b"S" => KeyCode::F(4),
            _ => return None,
        };
        Some((code, mods))
    }
}

/// Determine if a key should allow repetition (key repeat/long press), should not be blocked by debouncing.
fn is_repeatable_key(code: &KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Tab
            | KeyCode::BackTab
    )
}
/// Normalize Shift+Tab (sent as Tab+SHIFT in some terminals) to BackTab so that
/// the debounce treats both variants as the same logical key.
#[inline]
fn normalize_key(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    if code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT) {
        (KeyCode::BackTab, KeyModifiers::NONE)
    } else {
        (code, mods)
    }
}

/// Flush character batch buffer as paste block (>= 8 lines) or direct text insertion.
/// Used for capturing large paste from terminals that don't support Bracketed Paste.
fn flush_input_batch(state: &mut ChatState, buf: &mut String) {
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
    if !state.show_input_modal && line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES {
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
    } else if state.show_input_modal {
        state.modal_textarea.insert_str(&text);
    } else {
        crate::ui::events::input::push_cursor_off_sentinel_pub(state);
        state.textarea.insert_str(&text);
        state.input_line_count = state.textarea.lines().len();
        state.input = state.textarea.lines().join("\n");
        crate::ui::components::command_suggestions::on_input_changed(state);
    }
}

#[allow(unused_assignments)]
pub async fn run_ui_loop(
    terminal: &mut RatatuiTerminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut ChatState,
    agent_tx: mpsc::Sender<AgentRequest>,
    mut rx: mpsc::Receiver<StreamMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut last_key: Option<(KeyCode, KeyModifiers, Instant)> = None;
    let mut last_processed_key_time: Option<Instant> = None;
    let mut last_paste_time: Option<Instant> = None;

    // Initialize Watchdog (check every 1s, alert after 5s)
    let watchdog = Watchdog::default();
    watchdog.spawn();

    // Spawn a dedicated thread for reading keyboard events.
    // crossterm::event::poll()/read() fails in release builds on WSL2 (stdin-based).
    // With "use-dev-tty" feature, crossterm uses /dev/tty directly which should work.
    let (key_tx, mut key_rx) = mpsc::channel::<crossterm::event::Event>(256);
    let key_thread_alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let key_thread_alive_clone = key_thread_alive.clone();
    std::thread::Builder::new()
        .name("crossterm-key-reader".into())
        .spawn(move || {
            crate::utils::logging::append_debug_log_line(
                "[KEY_THREAD] Started (crossterm + use-dev-tty)",
            );
            let mut ev_count: u64 = 0;
            loop {
                match crossterm::event::read() {
                    Ok(ev) => {
                        ev_count += 1;
                        match key_tx.blocking_send(ev) {
                            Ok(()) => {}
                            Err(_) => {
                                crate::utils::logging::append_debug_log_line(
                                    "[KEY_THREAD] send FAILED (receiver dropped)",
                                );
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[KEY_THREAD] read error: {}",
                            e
                        ));
                        // On some terminals, read() can fail transiently (e.g. SIGWINCH).
                        // Brief sleep before retrying to avoid tight error loop.
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
            key_thread_alive_clone.store(false, std::sync::atomic::Ordering::Relaxed);
            crate::utils::logging::append_debug_log_line("[KEY_THREAD] Exiting");
        })?;

    // Frame rate control (~30 FPS idle, ~24 FPS streaming)
    // Streaming at 24fps (42ms) for smoother text flow; adaptive when content changes rapidly.
    let target_framerate = Duration::from_millis(33);
    let target_framerate_streaming_base = Duration::from_millis(42); // 24fps base
    let target_framerate_streaming_fast = Duration::from_millis(33); // 30fps when rapid changes
    let target_framerate_streaming_slow = Duration::from_millis(50); // 20fps when idle
    let mut last_draw;
    let mut needs_redraw;
    let mut last_loop_tick = Instant::now();
    let mut last_remote_tick = Instant::now();
    let mut stream_msg_count: u64 = 0;
    let mut last_stream_rate_check = Instant::now();
    let mut stream_msgs_per_sec: f64 = 0.0;

    // Track layout changes for terminal clear (only on task panel toggle)
    let mut last_scroll = state.scroll;
    let mut last_task_panel_visible = state.task_panel.is_visible;

    // Initial load of configured providers and current model (non-blocking)
    let _ = agent_tx.try_send(AgentRequest::LoadConfiguredProviders);
    // Trigger async MCP server initialization — loads .star/mcp.json,
    // connects to servers, and registers tools in the background.
    let _ = agent_tx.try_send(AgentRequest::McpRefresh);
    let ui_cwd = crate::core::utils::paths::current_dir_cached().clone();

    // Initial draw
    terminal.draw(|f| super::draw_ui(f, state))?;
    last_draw = Instant::now();
    needs_redraw = false;

    crate::runtime::background::spawn_git_status_loop(agent_tx.clone(), ui_cwd.clone());

    crate::utils::logging::append_debug_log_line("[UI] Event loop started");
    loop {
        // Check exit flag
        if state.should_exit {
            break Ok(());
        }

        // Process ALL pending keyboard/mouse events FIRST (non-blocking).
        // This ensures keyboard input is responsive even if other operations are slow.
        // Drain up to 64 events per iteration to prevent infinite loops on rapid input.
        const MAX_KEY_EVENTS_PER_TICK: usize = 64;
        let mut key_event_count = 0;
        while key_event_count < MAX_KEY_EVENTS_PER_TICK {
            match key_rx.try_recv() {
                Ok(ev) => {
                    key_event_count += 1;
                    let ev_type = std::mem::discriminant(&ev);
                    match ev {
                        Event::Key(key) => {
                            if key.kind != KeyEventKind::Release {
                                let (norm_code, norm_mods) = normalize_key(key.code, key.modifiers);
                                let norm_key =
                                    crossterm::event::KeyEvent::new(norm_code, norm_mods);
                                crate::ui::events::input::handle_key_event(
                                    state,
                                    norm_key,
                                    &agent_tx,
                                    last_processed_key_time,
                                )
                                .await?;
                                last_processed_key_time = Some(Instant::now());
                                needs_redraw = true;
                            }
                        }
                        Event::Mouse(m) => {
                            // 鼠标捕获开启时 Moved（悬停）事件高频产生，
                            // 不处理也不触发重绘，避免重绘风暴
                            if !matches!(m.kind, crossterm::event::MouseEventKind::Moved) {
                                crate::ui::events::mouse::handle_mouse_event(state, m);
                                needs_redraw = true;
                            }
                        }
                        Event::Paste(pasted_text) => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[BRACKETED_PASTE] Received {} chars",
                                pasted_text.len()
                            ));
                            // Handle bracketed paste event from supported terminals
                            state.paste_in_progress = true;
                            state.paste_end_time = Some(Instant::now());

                            // Route paste to the correct target:
                            // 1. Input modal (provider key/base URL) → modal_textarea
                            // 2. Palette filter → palette_filter
                            // 3. Main input → main textarea
                            if state.show_input_modal {
                                let normalized =
                                    pasted_text.replace("\r\n", "\n").replace('\r', "\n");
                                state.modal_textarea.insert_str(&normalized);
                                state.input_modal_value =
                                    crate::ui::events::clipboard_paste::collect_modal_input(
                                        &state.modal_textarea,
                                    );
                            } else if state.is_palette_open() {
                                for c in pasted_text.chars() {
                                    if !c.is_control() {
                                        state.palette_filter.push(c);
                                    }
                                }
                                state.selected_palette_index = 0;
                            } else {
                                // Check if clipboard has image (priority over text paste)
                                let clipboard_result = tokio::task::spawn_blocking(|| {
                                    if let Some((path, w, h)) = save_clipboard_image() {
                                        return Ok(ClipboardResult::Image(path, w, h));
                                    }
                                    Err("No image in clipboard".to_string())
                                })
                                .await;

                                match clipboard_result {
                                    Ok(Ok(ClipboardResult::Image(path, w, h))) => {
                                        insert_image_paste_block(state, path, w, h);
                                        sync_input_from_textarea(state);
                                        crate::ui::components::command_suggestions::on_input_changed(state);
                                    }
                                    _ => {
                                        // No image, process as text paste
                                        if let Some(file_paths) = detect_file_paths(&pasted_text) {
                                            insert_file_paste_block(state, file_paths);
                                        } else {
                                            insert_paste_block(state, pasted_text);
                                            if state.paste_segments.is_empty() {
                                                maybe_auto_fold_input(state);
                                            }
                                        }
                                        sync_input_from_textarea(state);
                                        crate::ui::components::command_suggestions::on_input_changed(state);
                                    }
                                }
                            }
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    crate::utils::logging::append_debug_log_line("[KEY_RX] channel disconnected");
                    break;
                }
            }
        }

        // Only clear terminal on layout changes (task panel toggle), not on scroll.
        // The buffer clearing in render_page (cell.reset()) handles CJK ghosting.
        // terminal.clear() on scroll causes flickering.
        let task_panel_changed = state.task_panel.is_visible != last_task_panel_visible;
        if task_panel_changed {
            // Use try-clear to avoid crashing on terminal errors
            if let Err(e) = terminal.clear() {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[UI] terminal.clear() failed: {}",
                    e
                ));
            }
        }
        last_scroll = state.scroll;
        last_task_panel_visible = state.task_panel.is_visible;

        // Pet the watchdog at the start of each loop iteration
        watchdog.pet();

        // Check keyboard reader thread health — if it died, input is permanently broken.
        // This is a fatal condition since the user can't type anything.
        if !key_thread_alive.load(std::sync::atomic::Ordering::Relaxed) {
            crate::utils::logging::append_debug_log_line("[UI] Key reader thread died, exiting");
            state.current_status_line =
                Some("Input thread crashed. Press Ctrl+C to exit.".to_string());
            // Don't break immediately — let the user see the message and Ctrl+C.
            // The ctrl_c handler in the select below will catch it.
        }

        // Check and clear paste flag (delay a short time to ensure terminal has processed all characters and trailing enter)
        if state.paste_in_progress {
            if let Some(end_time) = state.paste_end_time {
                if end_time.elapsed() > Duration::from_millis(350) {
                    state.paste_in_progress = false;
                }
            }
        }

        // Safety: auto-reset is_streaming after cancelling grace period expires.
        // Without this, is_streaming can stay true indefinitely if Done arrived
        // during the grace window (which keeps is_streaming=true for the animation),
        // preventing Ctrl+C from ever exiting.
        if state.is_streaming
            && state
                .cancelling_since
                .map(|t| t.elapsed() > Duration::from_millis(1500))
                .unwrap_or(false)
        {
            state.is_streaming = false;
            state.is_processing = false;
            state.cancelling_since = None;
            state.current_tool_name = None;
            state.thinking_started_at = None;
            state.last_token_time = None;
            state.queued_messages_display.clear();
            // Clear confirmation state to unblock queued messages
            state.is_awaiting_confirmation = false;
            state.pending_confirmation_entry_idx = None;

            // Process queued messages after cancel completes
            if !state.is_awaiting_confirmation && !state.pending_user_messages.is_empty() {
                if let Some(next_input) = state.pending_user_messages.pop_front() {
                    let remaining = state.pending_user_messages.len();
                    if remaining > 0 {
                        state.current_status_line = Some(format!("\u{23f3} {} pending", remaining));
                    } else {
                        state.current_status_line = None;
                    }
                    let _ =
                        crate::ui::app::logic::enqueue_user_message(state, next_input, &agent_tx)
                            .await;
                }
            }
        }

        // 超时保护：如果 is_processing 为 true 但超过 30 秒没有新消息，自动清除
        // 这处理 Done 消息丢失或 agent 卡住的情况
        if state.is_processing {
            let stall_secs = state
                .last_token_time
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            if stall_secs > 30 {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[STALL] is_processing stuck for {}s, auto-clearing",
                    stall_secs
                ));
                state.is_processing = false;
                state.is_streaming = false;
                state.current_tool_name = None;
                state.thinking_started_at = None;
                state.current_status_line = Some("✓ Done (timeout)".to_string());
            }
        }

        // 超时保护：current_tool_name 超过 15 秒没更新就清除
        // 这处理工具已完成但 ToolResult 消息丢失的情况
        if state.current_tool_name.is_some() {
            let tool_stale = state
                .tool_started_at
                .values()
                .max()
                .map(|t| t.elapsed().as_secs() > 15)
                .unwrap_or(true);
            if tool_stale {
                state.current_tool_name = None;
            }
        }

        // Poll /loop scheduled tasks (once per second)
        // Use timeout to prevent slow filesystem (network drives) from blocking the UI
        if last_loop_tick.elapsed() >= Duration::from_secs(1) {
            last_loop_tick = Instant::now();
            match tokio::time::timeout(
                Duration::from_millis(500),
                crate::runtime::background::poll_loop_tasks(state, &agent_tx, &ui_cwd),
            )
            .await
            {
                Ok(Ok(redraw)) => {
                    needs_redraw |= redraw;
                }
                Ok(Err(err)) => {
                    state.current_status_line = Some(format!("/loop scheduling failed: {}", err));
                }
                Err(_) => {
                    crate::utils::logging::append_debug_log_line(
                        "[LOOP] poll_loop_tasks timed out (slow filesystem?)",
                    );
                }
            }
        }

        // Poll remote control inbox (once per second)
        // Use timeout to prevent slow filesystem (network drives) from blocking the UI
        if last_remote_tick.elapsed() >= Duration::from_secs(1) {
            last_remote_tick = Instant::now();
            match tokio::time::timeout(
                Duration::from_millis(500),
                crate::runtime::background::poll_remote_requests(state, &agent_tx, &ui_cwd),
            )
            .await
            {
                Ok(Ok(redraw)) => {
                    needs_redraw |= redraw;
                }
                Ok(Err(err)) => {
                    state.current_status_line =
                        Some(format!("/remote consumption failed: {}", err));
                }
                Err(_) => {
                    crate::utils::logging::append_debug_log_line(
                        "[REMOTE] poll_remote_requests timed out (slow filesystem?)",
                    );
                }
            }
        }

        // Only draw if needed or enough time passed.
        // Adaptive framerate during streaming: faster when content changes rapidly.
        let framerate = if state.is_streaming {
            if stream_msgs_per_sec > 20.0 {
                target_framerate_streaming_fast // 30fps — rapid content changes
            } else if stream_msgs_per_sec > 5.0 {
                target_framerate_streaming_base // 24fps — moderate streaming
            } else {
                target_framerate_streaming_slow // 20fps — slow/idle streaming
            }
        } else {
            target_framerate
        };
        if needs_redraw || last_draw.elapsed() > framerate {
            // Ctrl+L clear screen request
            if state.request_clear_screen {
                terminal.clear()?;
                state.request_clear_screen = false;
            }
            terminal.draw(|f| super::draw_ui(f, state))?;
            last_draw = Instant::now();
            needs_redraw = false;
        }

        // ── Non-blocking stream message processing ──
        // Process stream messages in batches with a time budget to prevent
        // blocking keyboard input. Each batch has a max duration of 8ms.
        // This ensures the UI remains responsive even during heavy streaming.
        let stream_budget = Duration::from_millis(8);
        let stream_start = Instant::now();
        let mut stream_batch_count = 0u32;
        const MAX_STREAM_BATCH: u32 = 10;

        while stream_batch_count < MAX_STREAM_BATCH && stream_start.elapsed() < stream_budget {
            match rx.try_recv() {
                Ok(msg) => {
                    crate::ui::services::stream::handle_stream_update(state, msg, &agent_tx)
                        .await?;
                    needs_redraw = true;
                    stream_msg_count += 1;
                    stream_batch_count += 1;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    crate::utils::logging::append_debug_log_line("[STREAM] channel disconnected");
                    // Show error to user when worker disconnects unexpectedly
                    if state.is_streaming || state.is_processing {
                        state.current_status_line =
                            Some("Worker disconnected — press Enter to retry".to_string());
                        state.is_streaming = false;
                        state.is_processing = false;
                        state.thinking_started_at = None;
                        needs_redraw = true;
                    }
                    break;
                }
            }
        }

        // Update streaming message rate
        if stream_batch_count > 0 {
            let rate_elapsed = last_stream_rate_check.elapsed().as_secs_f64();
            if rate_elapsed >= 0.5 {
                stream_msgs_per_sec = stream_msg_count as f64 / rate_elapsed;
                stream_msg_count = 0;
                last_stream_rate_check = Instant::now();
            }
        }

        // ── Frame budget sleep ──
        // Sleep for the remaining frame budget. This yields CPU to the tokio
        // runtime and the keyboard reader thread, preventing busy-waiting.
        let elapsed_since_draw = last_draw.elapsed();
        if elapsed_since_draw < framerate {
            let sleep_duration = framerate - elapsed_since_draw;
            // Use tokio::time::sleep so the runtime can schedule other tasks
            tokio::time::sleep(sleep_duration).await;
        }
    }
}

fn sanitize_session_id(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "autosave".to_string()
    } else {
        trimmed.to_string()
    }
}

async fn persist_session_on_exit(
    config: &Arc<crate::core::config::Config>,
    state: &ChatState,
) -> Option<String> {
    // Exclude the transient welcome header entry from saved history
    let history: Vec<_> = state
        .chat_history
        .iter()
        .filter(|e| !e.is_welcome)
        .cloned()
        .collect();

    if history.is_empty() {
        return None;
    }

    let id = format!("auto-{}", sanitize_session_id(config.session_id()));
    if let Err(e) = crate::utils::session_manager::save_session(&id, &history).await {
        crate::utils::logging::append_debug_log_line(&format!(
            "[SESSION] Failed to auto-save session on exit: {}",
            e
        ));
        None
    } else {
        crate::utils::logging::append_debug_log_line(&format!(
            "[SESSION] Auto-saved session on exit: {}",
            id
        ));
        Some(id)
    }
}

/// Initialize terminal with robust error recovery.
///
/// The cursor position timeout ("could not be read within a normal duration") occurs when
/// `RatatuiTerminal::new()` sends ANSI DSR (Device Status Report) queries but the terminal
/// doesn't respond in time. This happens on:
/// - WSL2/Windows Terminal with slow PTY
/// - SSH connections with high latency
/// - tmux/screen sessions intercepting DSR
/// - Non-interactive or redirected stdin/stdout
///
/// This function implements a multi-stage initialization with fallbacks:
/// 1. Clean up any residual terminal state
/// 2. Enable raw mode and enter alternate screen
/// 3. Flush pending input bytes
/// 4. Wait for terminal to stabilize
/// 5. Create Terminal with retry + exponential backoff
fn init_terminal() -> Result<
    ratatui::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::prelude::CrosstermBackend;
    use ratatui::Terminal as RatatuiTerminal;
    use std::io::{stdout, Write};

    // Stage 1: Clean slate — disable raw mode and leave alternate screen
    // This handles the case where a previous session crashed without cleanup.
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);

    // Reset terminal state: disable bracketed paste, switch to main screen, show cursor
    let _ = crossterm::execute!(stdout(), crossterm::event::DisableBracketedPaste);
    let _ = stdout().write_all(b"\x1b[?1049l"); // switch to main screen
    let _ = stdout().write_all(b"\x1b[?25h"); // show cursor
    let _ = stdout().write_all(b"\x1b[0m"); // reset attributes
    let _ = stdout().flush();

    // Stage 2: Enable raw mode
    crate::utils::logging::append_debug_log_line("[TERM] calling enable_raw_mode...");
    enable_raw_mode().map_err(|e| {
        crate::utils::logging::append_debug_log_line(&format!(
            "[TERM] enable_raw_mode() FAILED: {}",
            e
        ));
        e
    })?;
    crate::utils::logging::append_debug_log_line("[TERM] enable_raw_mode() succeeded");

    // Stage 3: Enter alternate screen, enable bracketed paste and mouse capture
    execute!(stdout(), EnterAlternateScreen)?;
    let _ = crossterm::execute!(stdout(), crossterm::event::EnableBracketedPaste);
    // 默认启用鼠标捕获：滚轮以真实鼠标事件到达，直接滚动聊天区，
    // 不再经终端转成 Up/Down 箭头键污染输入历史。
    // 应用自带拖选复制（events/mouse.rs），Shift+点击仍可用终端原生选择。
    // 若终端鼠标支持异常，可设 STARCODE_ENABLE_MOUSE=0 关闭——此时滚轮会被
    // 终端转成箭头键，由 input.rs 的时间启发式尽力区分（不保证可靠）。
    if !crate::ui::utils::term_recovery::mouse_capture_disabled_by_env() {
        let _ = crossterm::execute!(stdout(), crossterm::event::EnableMouseCapture);
    }
    let _ = stdout().flush();

    // Stage 4: Drain any pending input bytes.
    // Stale bytes in the input buffer (e.g. from a previous crashed session or
    // fast pasted input) can confuse the DSR response parser and cause timeouts.
    crate::utils::logging::append_debug_log_line("[TERM] draining pending input...");
    while crossterm::event::poll(std::time::Duration::from_millis(0))? {
        let _ = crossterm::event::read();
    }

    // Stage 5: Wait for terminal to stabilize.
    // The delay allows the terminal emulator to process the mode switches
    // and be ready to respond to DSR queries. 80ms is conservative enough
    // for most terminals including WSL2 and SSH.
    std::thread::sleep(std::time::Duration::from_millis(80));

    // Stage 6: Create Terminal with retry.
    // RatatuiTerminal::new() internally sends DSR (ESC [ 6 n) and waits for CPR response.
    // If it fails, retry with increasing delays.
    let max_retries = 3;
    let mut last_err = None;
    for attempt in 0..max_retries {
        let backend = CrosstermBackend::new(stdout());
        match RatatuiTerminal::new(backend) {
            Ok(t) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[TERM] Terminal::new succeeded on attempt {}",
                    attempt + 1
                ));
                return Ok(t);
            }
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[TERM] Terminal::new failed attempt {}/{}: {}",
                    attempt + 1,
                    max_retries,
                    e
                ));
                last_err = Some(e);
                // Exponential backoff: 100ms, 200ms, 400ms
                let delay = std::time::Duration::from_millis(100 * (1 << attempt));
                std::thread::sleep(delay);

                // Re-drain input before retry
                while crossterm::event::poll(std::time::Duration::from_millis(0))? {
                    let _ = crossterm::event::read();
                }
            }
        }
    }

    // All retries failed — try one last time with raw stdin approach
    crate::utils::logging::append_debug_log_line(
        "[TERM] All retries failed, attempting fallback...",
    );
    Err(Box::new(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            "Terminal initialization failed after all retries",
        )
    })))
}

/// 把输入框内容整体重设为 `text`（而不是追加）。
///
/// 初始化等待期间要把 early_input 的快照反复镜像进真正的输入框，
/// 追加语义会越写越长，所以这里按 repo 既有做法重建一个 TextArea 并补回样式。
fn sync_textarea(state: &mut ChatState, text: &str) {
    use tui_textarea::TextArea;

    let mut textarea = TextArea::default();
    textarea.set_placeholder_text(crate::ui::utils::text::input_placeholder_text());
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );
    if !text.is_empty() {
        textarea.insert_str(text);
    }
    state.textarea = textarea;
    state.input = text.to_string();
    state.input_line_count = text.lines().count().max(1);
}

pub async fn run_app(
    init_rx: tokio::sync::oneshot::Receiver<
        Result<
            (
                crate::agent::StarAgent,
                std::sync::Arc<crate::core::config::Config>,
            ),
            String,
        >,
    >,
    initial_message: String,
    initial_history: Vec<crate::types::ChatEntry>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::ui::services::worker::agent_worker;
    use crossterm::execute;
    use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    use std::io::{stdout, Write};
    use std::panic;

    // ── 终端初始化（在独立线程执行，避免阻塞 tokio 运行时）──
    let terminal_result = tokio::task::spawn_blocking(|| init_terminal()).await?;

    let mut terminal = match terminal_result {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(stdout(), LeaveAlternateScreen);
            eprintln!("❌ Terminal initialization failed: {}", e);
            eprintln!("   This usually means the terminal does not support DSR queries.");
            eprintln!("   Try using a modern terminal (iTerm2, Windows Terminal, Alacritty).");
            std::process::exit(1);
        }
    };

    // ── 直接进主界面 ──
    //
    // 以前这里是 splash 启动画面 + 两块手搓的 "Loading..." 文本页。现在
    // `ChatState::new()` 提前到初始化之前构造（它不依赖 Config，只读历史文件），
    // 于是等待期间画的就是真正的主界面：抬头、空的对话区、真的输入框。
    // 初始化进度改为走状态行，而不是另起一个页面。
    let mut state = ChatState::new();
    state.auto_follow = true;
    state.virtual_list.mark_all_dirty();
    state.current_status_line = Some(
        crate::core::i18n::t(
            "ui.init.loading",
            "正在加载配置和工具…",
            "Loading configuration and tools…",
        )
        .to_string(),
    );
    terminal.draw(|f| super::draw_ui(f, &mut state))?;

    // ── 启动早期输入捕获 ──
    super::early_input::start_capturing();

    // ── 等待后台初始化完成（同时响应键盘输入和 Ctrl+C）──
    let mut init_rx = init_rx;
    let mut last_preview = String::new();
    let early_input_text;
    let (agent, config) = loop {
        tokio::select! {
            result = &mut init_rx => {
                match result {
                    Ok(Ok(pair)) => {
                        early_input_text = super::early_input::consume_early_input();
                        break pair;
                    }
                    Ok(Err(e)) => {
                        super::early_input::stop_capturing();
                        cleanup_terminal();
                        eprintln!("❌ {}", e);
                        std::process::exit(1);
                    }
                    Err(_) => {
                        super::early_input::stop_capturing();
                        cleanup_terminal();
                        eprintln!("❌ Init task crashed");
                        std::process::exit(1);
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                super::early_input::stop_capturing();
                cleanup_terminal();
                std::process::exit(0);
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                let preview = super::early_input::peek_early_input();
                if preview != last_preview {
                    // 把早期输入镜像进真正的输入框，用户看到的就是自己在主界面里打字
                    sync_textarea(&mut state, &preview);
                    last_preview = preview;
                }
                terminal.draw(|f| super::draw_ui(f, &mut state))?;
            }
        }
    };

    // ── 初始化完成，进入事件循环 ──

    #[cfg(windows)]
    let win32_guard = crate::ui::win32::CtrlCGuard::install();

    /// Robust terminal cleanup — safe to call multiple times.
    /// Handles partial initialization states (e.g. raw mode enabled but alt screen not entered).
    fn cleanup_terminal() {
        use std::io::Write;
        // Best-effort: ignore individual errors since we're cleaning up
        let _ = crossterm::execute!(stdout(), crossterm::event::DisableMouseCapture);
        let _ = crossterm::execute!(stdout(), crossterm::event::DisableBracketedPaste);
        let _ = stdout().write_all(b"\x1b[?25h"); // show cursor
        let _ = stdout().write_all(b"\x1b[0m"); // reset attributes
        let _ = stdout().flush();
        // Leave alternate screen first, then disable raw mode
        // (disabling raw mode first can cause issues if alt screen is still active)
        let _ = execute!(stdout(), LeaveAlternateScreen);
        if crossterm::terminal::is_raw_mode_enabled().unwrap_or(false) {
            let _ = disable_raw_mode();
        }
        let _ = execute!(stdout(), crossterm::cursor::Show);
    }

    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        cleanup_terminal();
        original_hook(panic_info);
    }));

    let (agent_tx, agent_rx) = mpsc::channel::<AgentRequest>(100);
    let (ui_tx, ui_rx) = mpsc::channel::<StreamMessage>(100);

    // 初始化完成：清掉加载提示，把解析出来的模型名填进抬头
    state.current_status_line = None;
    if state.current_model.is_empty() {
        state.current_model = config.model().to_string();
    }

    // Restore draft from previous session
    state.restore_draft();

    // Initialize context engine and start background indexing
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut context_engine = crate::core::context::integration::ContextEngine::new(cwd.clone());
    state.context_engine = Some(context_engine);

    // Spawn background indexing task
    let index_tx = agent_tx.clone();
    tokio::spawn(async move {
        let mut engine = crate::core::context::integration::ContextEngine::new(cwd);
        match engine.index_project().await {
            Ok(result) => {
                let _ = index_tx
                    .send(AgentRequest::EmitStatus(format!("Indexed: {}", result)))
                    .await;
            }
            Err(e) => {
                let _ = index_tx
                    .send(AgentRequest::EmitStatus(format!("Index failed: {}", e)))
                    .await;
            }
        }
    });

    // 抬头由 `ChatState::new()` 建的那条 is_welcome 条目承载，渲染期现算
    // （见 ui::components::welcome_header）。这里不再另塞一条，否则主界面
    // 顶部会出现两个欢迎块。
    state.virtual_list.mark_all_dirty();

    // Load thinking_effort from user settings
    if let Ok(settings_manager) = crate::core::config::settings_manager::SettingsManager::new() {
        if let Ok(settings) = settings_manager.load_user_settings().await {
            if let Some(ref effort_str) = settings.thinking_effort {
                state.thinking_effort = match effort_str.to_lowercase().as_str() {
                    "off" => crate::types::ThinkingEffort::Off,
                    "low" => crate::types::ThinkingEffort::Low,
                    "medium" => crate::types::ThinkingEffort::Medium,
                    "high" => crate::types::ThinkingEffort::High,
                    _ => crate::types::ThinkingEffort::Off,
                };
            }
            if let Some(ctx) = settings.context_window {
                state.context_window_override = Some(ctx);
            }
        }
    }

    // Load previous session history if provided
    if !initial_history.is_empty() {
        state.chat_history.extend(initial_history);
        state.auto_follow = true;
        let len = state.chat_history.len();
        state.chat_list_state.select(Some(len.saturating_sub(1)));
    }

    // 等待期间早期输入已镜像进 textarea，这里统一按最终文本重设一次，
    // 不能再 insert_str —— 那会把已经显示出来的内容再追加一遍
    if !initial_message.is_empty() {
        sync_textarea(&mut state, &initial_message);
    } else if !early_input_text.is_empty() {
        sync_textarea(&mut state, &early_input_text);
    }

    // Start worker
    tokio::spawn(async move {
        agent_worker(agent, agent_rx, ui_tx).await;
    });

    // Run UI loop
    let res = run_ui_loop(&mut terminal, &mut state, agent_tx, ui_rx).await;

    // Persist chat history for resume after graceful exits (including Ctrl+C key handling).
    let saved_session_id = persist_session_on_exit(&config, &state).await;

    // Cleanup
    #[cfg(windows)]
    drop(win32_guard);
    cleanup_terminal();

    // Print resume hint after terminal is restored
    if let Some(id) = saved_session_id {
        println!("\nSession saved. To resume:");
        // 用本次实际启动用的命令名（sc / starcode / starcode-cli 是同一个二进制）
        println!(
            "  {} --resume {}",
            crate::utils::invocation::program_name(),
            id
        );
    }

    res
}

/// Parse ANSI escape sequences from raw stdin bytes into crossterm Events.
/// Returns (bytes_consumed, Option<Event>). If 0 consumed, need more bytes.
fn parse_ansi_event(buf: &[u8]) -> (usize, Option<Event>) {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    if buf.is_empty() {
        return (0, None);
    }

    match buf[0] {
        // ESC sequence
        0x1B => {
            if buf.len() == 1 {
                // Lone ESC - might be Escape key, or start of sequence.
                // Wait for more bytes briefly (will be caught by timeout).
                return (0, None);
            }
            match buf[1] {
                b'[' => {
                    // CSI sequence: ESC [ ...
                    if buf.len() < 3 {
                        return (0, None);
                    }
                    match buf[2] {
                        b'A' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))),
                        ),
                        b'B' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))),
                        ),
                        b'C' => (
                            3,
                            Some(Event::Key(KeyEvent::new(
                                KeyCode::Right,
                                KeyModifiers::NONE,
                            ))),
                        ),
                        b'D' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))),
                        ),
                        b'Z' => (
                            3,
                            Some(Event::Key(KeyEvent::new(
                                KeyCode::BackTab,
                                KeyModifiers::SHIFT,
                            ))),
                        ),
                        b'H' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))),
                        ),
                        b'F' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))),
                        ),
                        b'3' => {
                            if buf.len() >= 4 && buf[3] == b'~' {
                                (
                                    4,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::Delete,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            } else {
                                (
                                    2,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::Esc,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            }
                        }
                        b'5' => {
                            if buf.len() >= 4 && buf[3] == b'~' {
                                (
                                    4,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::PageUp,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            } else {
                                (
                                    2,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::Esc,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            }
                        }
                        b'6' => {
                            if buf.len() >= 4 && buf[3] == b'~' {
                                (
                                    4,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::PageDown,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            } else {
                                (
                                    2,
                                    Some(Event::Key(KeyEvent::new(
                                        KeyCode::Esc,
                                        KeyModifiers::NONE,
                                    ))),
                                )
                            }
                        }
                        b'1' => {
                            // ESC[1; modifier sequences like ESC[1;2A (Shift+Up)
                            // Find the end of the sequence
                            if let Some(end) = buf[3..].iter().position(|&b| b >= 0x40 && b < 0x80)
                            {
                                let total = 3 + end + 1;
                                let seq = &buf[3..3 + end];
                                let final_byte = buf[3 + end];
                                let mods = parse_csi_modifiers(seq);
                                let key = match final_byte {
                                    b'A' => KeyCode::Up,
                                    b'B' => KeyCode::Down,
                                    b'C' => KeyCode::Right,
                                    b'D' => KeyCode::Left,
                                    _ => {
                                        return (
                                            total,
                                            Some(Event::Key(KeyEvent::new(
                                                KeyCode::Esc,
                                                KeyModifiers::NONE,
                                            ))),
                                        );
                                    }
                                };
                                (total, Some(Event::Key(KeyEvent::new(key, mods))))
                            } else {
                                (0, None) // need more bytes
                            }
                        }
                        _ => {
                            // Unknown CSI - consume ESC[ and the byte
                            (
                                3,
                                Some(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
                            )
                        }
                    }
                }
                b'O' => {
                    // SS3 sequence: ESC O ...
                    if buf.len() < 3 {
                        return (0, None);
                    }
                    match buf[2] {
                        b'P' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))),
                        ),
                        b'Q' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE))),
                        ),
                        b'R' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE))),
                        ),
                        b'S' => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE))),
                        ),
                        _ => (
                            3,
                            Some(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
                        ),
                    }
                }
                _ => {
                    // ESC + regular char = Alt+char
                    let ch = buf[1];
                    let (code, mods) = byte_to_keycode(ch, true);
                    (2, Some(Event::Key(KeyEvent::new(code, mods))))
                }
            }
        }
        // Ctrl+C
        0x03 => (
            1,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
        ),
        // Ctrl+D
        0x04 => (
            1,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
            ))),
        ),
        // Ctrl+Z
        0x1A => (
            1,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ))),
        ),
        // Tab
        0x09 => (
            1,
            Some(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))),
        ),
        // Enter (CR or LF)
        0x0D | 0x0A => (
            1,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))),
        ),
        // Backspace
        0x7F => (
            1,
            Some(Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            ))),
        ),
        // Other control chars 0x01-0x1A (Ctrl+A through Ctrl+Z)
        0x01..=0x08 | 0x0B..=0x0C | 0x0E..=0x1A => {
            let ch = (buf[0] + b'a' - 1) as char;
            (
                1,
                Some(Event::Key(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::CONTROL,
                ))),
            )
        }
        // FS/GS/RS/US (0x1C-0x1F) - treat as escape or skip
        0x1C..=0x1F => (1, None),
        // NUL
        0x00 => (1, None),
        // Regular printable ASCII
        0x20..=0x7E => {
            let ch = buf[0] as char;
            (
                1,
                Some(Event::Key(KeyEvent::new(
                    KeyCode::Char(ch),
                    KeyModifiers::NONE,
                ))),
            )
        }
        // UTF-8 multi-byte
        0x80..=0xFF => {
            // Try to decode a UTF-8 character
            let len = utf8_char_len(buf[0]);
            if buf.len() < len {
                return (0, None); // need more bytes
            }
            if let Ok(s) = std::str::from_utf8(&buf[..len]) {
                if let Some(ch) = s.chars().next() {
                    return (
                        len,
                        Some(Event::Key(KeyEvent::new(
                            KeyCode::Char(ch),
                            KeyModifiers::NONE,
                        ))),
                    );
                }
            }
            // Invalid UTF-8, skip byte
            (
                1,
                Some(Event::Key(KeyEvent::new(
                    KeyCode::Char('\u{FFFD}'),
                    KeyModifiers::NONE,
                ))),
            )
        }
    }
}

/// Parse CSI modifier parameters from bytes like "1;2" (Shift), "1;5" (Ctrl), "1;3" (Alt)
fn parse_csi_modifiers(seq: &[u8]) -> KeyModifiers {
    use crossterm::event::KeyModifiers;
    let s = match std::str::from_utf8(seq) {
        Ok(s) => s,
        _ => return KeyModifiers::NONE,
    };
    // Format is typically "1;mod" where mod: 2=Shift, 3=Alt, 4=Shift+Alt, 5=Ctrl, etc.
    let parts: Vec<&str> = s.split(';').collect();
    if parts.len() >= 2 {
        if let Ok(mod_val) = parts[1].parse::<u8>() {
            let mut mods = KeyModifiers::NONE;
            if mod_val & 1 != 0 {
                mods |= KeyModifiers::SHIFT;
            }
            if mod_val & 2 != 0 {
                mods |= KeyModifiers::ALT;
            } // actually Alt in crossterm is 0x04, but this is csi modifier
            if mod_val & 4 != 0 {
                mods |= KeyModifiers::CONTROL;
            }
            return mods;
        }
    }
    KeyModifiers::NONE
}

/// Map a byte to (KeyCode, KeyModifiers) for Alt+key combinations
fn byte_to_keycode(b: u8, alt: bool) -> (KeyCode, KeyModifiers) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut mods = if alt {
        KeyModifiers::ALT
    } else {
        KeyModifiers::NONE
    };
    match b {
        b'a'..=b'z' => {
            if b >= b'a' && b <= b'z' {
                (KeyCode::Char(b as char), mods)
            } else {
                (KeyCode::Char(b as char), mods)
            }
        }
        b'A'..=b'Z' => {
            mods |= KeyModifiers::SHIFT;
            (KeyCode::Char((b - b'A' + b'a') as char), mods)
        }
        b'0'..=b'9' => (KeyCode::Char(b as char), mods),
        _ => (KeyCode::Char(b as char), mods),
    }
}

/// Get UTF-8 character length from first byte
fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}
