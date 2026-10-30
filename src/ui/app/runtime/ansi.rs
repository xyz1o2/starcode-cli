//! 原始字节到 `Event` 的解析：给不认 crossterm 的终端兜底。
//!
//! crossterm 在部分终端（legacy conhost、某些 SSH 客户端）上会把一个方向键
//! 拆成三次事件，或者在 Windows 上根本读不到。这里按字节自己解一遍。

use super::*;

/// Parse ANSI escape sequences from raw stdin bytes into crossterm Events.
/// Returns (bytes_consumed, Option<Event>). If 0 consumed, need more bytes.
pub(super) fn parse_ansi_event(buf: &[u8]) -> (usize, Option<Event>) {
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
pub(super) fn parse_csi_modifiers(seq: &[u8]) -> KeyModifiers {
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
pub(super) fn byte_to_keycode(b: u8, alt: bool) -> (KeyCode, KeyModifiers) {
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
pub(super) fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}
