//! 按键去抖与 EscapeGuard 的归属决策：同一个键该不该被当成新的一次按键。
//!
//! 这里只有判断，不碰终端、不碰状态机。

use super::*;

/// 尝试重组已被终端事件解析器拆开的 ESC 序列，避免片段进入正文。
///
/// Esc 后在 TRIGGER_WINDOW_MS 内出现 '[' / 'O' 才开始吸收，
/// ABSORB_DURATION_MS 后按收集片段重建按键。当前窗口为 20 / 100ms；
/// 这是兼顾独立 Esc 与正常文本的启发式，不保证恢复任意慢速拆分序列。
/// 未识别片段按下方逻辑归还文本，不能假设用户不可能快速输入 Esc 后接 '['。
pub(super) struct EscapeGuard {
    last_esc: Option<Instant>,
    absorbing_deadline: Option<Instant>,
    fragment_buf: String,
    /// Characters that were absorbed but don't form a valid escape sequence.
    /// These are pending normal characters that should be added to char_buf.
    pending_chars: Option<String>,
}

impl EscapeGuard {
    const CSI_INTRODUCERS: &[char] = &['[', 'O'];
    // 窗口保持较短以减少误吞正常输入；跨终端的最佳值需实测，不能仅凭网络延迟推断。
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
pub(super) fn is_repeatable_key(code: &KeyCode) -> bool {
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
pub(super) fn normalize_key(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    if code == KeyCode::Tab && mods.contains(KeyModifiers::SHIFT) {
        (KeyCode::BackTab, KeyModifiers::NONE)
    } else {
        (code, mods)
    }
}
