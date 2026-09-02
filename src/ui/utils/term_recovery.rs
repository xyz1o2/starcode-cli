/// Terminal recovery utilities for graceful degradation.
///
/// Provides strategies for recovering from terminal errors without crashing.
/// Common scenarios:
/// - Cursor position timeout (DSR not responded)
/// - Raw mode failures
/// - Alternate screen failures
/// - Write errors to stdout
use std::io::Write;
use std::time::{Duration, Instant};

/// Terminal health state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermHealth {
    /// Terminal is functioning normally
    Healthy,
    /// Terminal has degraded capabilities (e.g. no alternate screen)
    Degraded,
    /// Terminal is unresponsive or broken
    Broken,
}

/// 是否被环境变量显式关闭了鼠标捕获（STARCODE_ENABLE_MOUSE=0/false/off）。
pub fn mouse_capture_disabled_by_env() -> bool {
    std::env::var("STARCODE_ENABLE_MOUSE")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
}

/// 按默认开启的策略（重新）启用鼠标捕获。
///
/// 恢复流程中 `disable_raw_mode` 会还原初始控制台模式，连带丢失 Windows
/// 控制台的鼠标输入标志——不补启用的话滚轮会再次被终端转成箭头键。
pub fn reenable_mouse_capture() {
    if mouse_capture_disabled_by_env() {
        return;
    }
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture
    );
}

/// Attempt to recover terminal to a usable state.
/// Returns the health status after recovery attempt.
pub fn attempt_recovery() -> TermHealth {
    use std::io::stdout;

    // Step 1: Try to reset terminal state
    let _ = stdout().write_all(b"\x1b[0m"); // reset attributes
    let _ = stdout().write_all(b"\x1b[?25h"); // show cursor
    let _ = crossterm::execute!(stdout(), crossterm::event::DisableBracketedPaste);
    let _ = stdout().flush();

    // Step 2: Check if raw mode is still functional
    let health = if crossterm::terminal::is_raw_mode_enabled().unwrap_or(false) {
        // Raw mode is on — try to disable and re-enable
        let _ = crossterm::terminal::disable_raw_mode();
        std::thread::sleep(Duration::from_millis(20));
        if crossterm::terminal::enable_raw_mode().is_ok() {
            TermHealth::Healthy
        } else {
            // Re-enable failed — terminal is degraded
            TermHealth::Degraded
        }
    } else {
        // Step 3: Try to enable raw mode
        if crossterm::terminal::enable_raw_mode().is_ok() {
            TermHealth::Healthy
        } else {
            TermHealth::Degraded
        }
    };

    reenable_mouse_capture();
    health
}

/// Check if the terminal can respond to a simple query within timeout.
pub fn check_terminal_responsive(timeout: Duration) -> bool {
    // Try to poll with zero timeout — if this fails, terminal is broken
    crossterm::event::poll(timeout).unwrap_or(false)
}

/// Safe terminal draw with error recovery.
/// If the draw fails, attempts recovery and retries once.
pub fn safe_draw<F>(
    terminal: &mut ratatui::Terminal<ratatui::prelude::CrosstermBackend<std::io::Stdout>>,
    mut draw_fn: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnMut(&mut ratatui::Frame<'_>),
{
    match terminal.draw(&mut draw_fn) {
        Ok(_) => Ok(()),
        Err(e) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[TERM_RECOVERY] Draw failed: {}, attempting recovery",
                e
            ));
            // Attempt recovery
            let health = attempt_recovery();
            match health {
                TermHealth::Healthy | TermHealth::Degraded => {
                    // Retry the draw
                    terminal.draw(&mut draw_fn)?;
                    Ok(())
                }
                TermHealth::Broken => Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Terminal is broken, draw failed: {}", e),
                ))),
            }
        }
    }
}

/// Duration since an instant, clamped to avoid overflow.
pub fn elapsed_clamped(since: Instant) -> Duration {
    since.elapsed()
}
