pub mod clipboard_paste;
/// UI Event Handling
///
/// # Event Types
/// - `input.rs`: Keyboard event handler (1592 lines) — handles all key bindings,
///   confirmation dialogs, palette navigation, modal input, etc.
/// - `keymap.rs`: Key mapping abstraction — defines `UiAction` enum
/// - `mouse.rs`: Mouse event handler — scroll, click, drag selection
/// - `clipboard_paste.rs`: Clipboard paste handling — image paste, file path detection,
///   text paste blocks with sentinel placeholders
///
/// # Event Flow
/// ```text
/// crossterm::Event → [Key Reader Thread] → mpsc → [UI Loop] → handle_key_event()
/// ```
///
/// # Key Binding Conventions
/// - Ctrl+C: Cancel streaming / double-press to exit
/// - Ctrl+P: Command palette
/// - Ctrl+B: Task panel toggle
/// - Shift+Tab: Plan/Build mode toggle
/// - Alt+P: Fold/unfold pasted input
/// - Esc: Dismiss overlays / cancel
///
pub mod input;
pub mod keymap;
pub mod mouse;
