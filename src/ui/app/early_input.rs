use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::types::{InputIntent, UserInput};

static EARLY_INPUT_BUFFER: Lazy<Arc<Mutex<UserInput>>> =
    Lazy::new(|| Arc::new(Mutex::new(UserInput::command_eligible(String::new()))));
static CAPTURING: AtomicBool = AtomicBool::new(false);
static THREAD_HANDLE: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

pub fn start_capturing() {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return;
    }

    let buffer = EARLY_INPUT_BUFFER.clone();

    let handle = std::thread::Builder::new()
        .name("early-key-capture".into())
        .spawn(move || {
            crate::utils::logging::append_debug_log_line("[EARLY_INPUT] Capture thread started");
            loop {
                if !CAPTURING.load(Ordering::Relaxed) {
                    break;
                }

                if crossterm::event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
                    if let Ok(event) = crossterm::event::read() {
                        match event {
                            crossterm::event::Event::Key(key) => {
                                if key.kind == crossterm::event::KeyEventKind::Press
                                    && handle_key(&buffer, key).is_break()
                                {
                                    // 捕获线程不能等待自己退出；终止进程前只发布停止状态。
                                    CAPTURING.store(false, Ordering::SeqCst);
                                    cleanup_terminal();
                                    std::process::exit(130);
                                }
                            }
                            crossterm::event::Event::Paste(text) => {
                                let mut input = buffer.lock().unwrap_or_else(|e| e.into_inner());
                                input.text.push_str(
                                    &text
                                        .replace("\r\n", "\n")
                                        .replace(['\r', '\u{2028}', '\u{2029}'], "\n"),
                                );
                                input.intent = InputIntent::Literal;
                            }
                            _ => {}
                        }
                    }
                }
            }
            crate::utils::logging::append_debug_log_line("[EARLY_INPUT] Capture thread stopped");
        })
        .expect("Failed to spawn early input capture thread");

    *THREAD_HANDLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
}

fn handle_key(
    buffer: &Arc<Mutex<UserInput>>,
    key: crossterm::event::KeyEvent,
) -> std::ops::ControlFlow<()> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return std::ops::ControlFlow::Break(());
        }
        KeyCode::Backspace => {
            let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
            buf.text.pop();
            if buf.text.is_empty() {
                buf.intent = InputIntent::CommandEligible;
            }
        }
        KeyCode::Enter => {
            let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
            buf.text.push('\n');
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());
            buf.text.push(c);
        }
        _ => {}
    }
    std::ops::ControlFlow::Continue(())
}

fn join_capture_thread(slot: &Mutex<Option<std::thread::JoinHandle<()>>>) {
    // 先释放句柄锁，再等待线程；线程收尾可以安全访问生命周期状态。
    let handle = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

pub fn stop_capturing() {
    CAPTURING.store(false, Ordering::SeqCst);
    join_capture_thread(&THREAD_HANDLE);
}

pub fn consume_early_input() -> UserInput {
    stop_capturing();
    let mut buf = EARLY_INPUT_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::replace(&mut *buf, UserInput::command_eligible(String::new()))
}

pub fn peek_early_input() -> UserInput {
    let buf = EARLY_INPUT_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    buf.clone()
}

fn cleanup_terminal() {
    use crossterm::execute;
    use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    use std::io::{stdout, Write};

    let _ = crossterm::execute!(stdout(), crossterm::event::DisableMouseCapture);
    let _ = stdout().write_all(b"\x1b[?2004l");
    let _ = stdout().write_all(b"\x1b[?25h");
    let _ = stdout().write_all(b"\x1b[0m");
    let _ = stdout().flush();
    let _ = execute!(stdout(), LeaveAlternateScreen);
    if crossterm::terminal::is_raw_mode_enabled().unwrap_or(false) {
        let _ = disable_raw_mode();
    }
    let _ = execute!(stdout(), crossterm::cursor::Show);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[cfg(target_os = "linux")]
    #[test]
    fn ctrl_c_exits_capture_process_and_restores_terminal() {
        run_capture_pty_test("ctrl_c_exits_capture_process_and_restores_terminal", false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bracketed_paste_is_preserved_during_startup() {
        run_capture_pty_test("bracketed_paste_is_preserved_during_startup", true);
    }

    #[cfg(target_os = "linux")]
    fn run_capture_pty_test(test_name: &str, paste: bool) {
        const CHILD_FLAG: &str = "SC_TEST_EARLY_INPUT_PTY_CHILD";
        if std::env::var(CHILD_FLAG).as_deref() == Ok(test_name) {
            use std::io::Write;
            crossterm::terminal::enable_raw_mode().expect("enable child raw mode");
            crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::EnterAlternateScreen,
                crossterm::event::EnableBracketedPaste
            )
            .expect("enter child alternate screen");
            start_capturing();
            println!("EARLY_PTY_READY");
            std::io::stdout().flush().expect("flush ready marker");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while peek_early_input().text != "q" {
                assert!(
                    std::time::Instant::now() < deadline,
                    "capture handshake timed out"
                );
                std::thread::yield_now();
            }
            println!("EARLY_PTY_CAPTURED");
            std::io::stdout().flush().expect("flush capture marker");
            if paste {
                while peek_early_input().text != "q/help\n检查\u{3}" {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "bracketed paste was lost"
                    );
                    std::thread::yield_now();
                }
                assert_eq!(peek_early_input().intent, InputIntent::Literal);
                println!("EARLY_PTY_PASTED");
                std::io::stdout().flush().expect("flush paste marker");
            }
            // 父进程在握手后发送 Ctrl+C，只有真实捕获线程能完成退出。
            std::thread::park();
            panic!("capture child unexpectedly resumed");
        }
        let output = std::process::Command::new("python3")
            .arg("-I")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/test-early-input-pty.py"
            ))
            .arg(std::env::current_exe().expect("test executable"))
            .arg(format!("ui::app::early_input::tests::{test_name}"))
            .arg(if paste { "paste" } else { "ctrl-c" })
            .env(CHILD_FLAG, test_name)
            .env("STAR_LOG_ENABLED", "0")
            .output()
            .expect("run Python PTY driver");
        assert!(
            output.status.success(),
            "PTY driver failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn ctrl_c_requests_exit_without_joining_or_touching_input() {
        let buffer = Arc::new(Mutex::new(UserInput::command_eligible(
            "unsent input".into(),
        )));
        let result = handle_key(
            &buffer,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(
            result.is_break(),
            "Ctrl+C must request exit, not wait for the capture thread"
        );
        assert_eq!(buffer.lock().expect("buffer lock").text, "unsent input");
    }

    #[test]
    fn capture_thread_can_access_handle_slot_while_being_joined() {
        let slot = Arc::new(Mutex::new(None));
        let worker_slot = slot.clone();
        let (start_tx, start_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            start_rx.recv().expect("start capture teardown");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if let Ok(slot) = worker_slot.try_lock() {
                    if slot.is_none() {
                        result_tx.send(true).expect("report released slot");
                        return;
                    }
                }
                if std::time::Instant::now() >= deadline {
                    result_tx.send(false).expect("report held slot");
                    return;
                }
                std::thread::yield_now();
            }
        });
        *slot.lock().expect("install handle") = Some(handle);
        start_tx.send(()).expect("release capture teardown");
        join_capture_thread(&slot);
        assert!(
            result_rx.recv().expect("teardown result"),
            "join must not hold the handle lock"
        );
        assert!(slot.lock().expect("empty handle slot").is_none());
    }

    #[test]
    fn capture_key_actions_preserve_unicode_and_ignore_shortcuts() {
        let buffer = Arc::new(Mutex::new(UserInput::command_eligible(String::new())));
        for code in [
            KeyCode::Char('检'),
            KeyCode::Char('查'),
            KeyCode::Enter,
            KeyCode::Char('🦀'),
            KeyCode::Backspace,
        ] {
            assert!(handle_key(&buffer, KeyEvent::new(code, KeyModifiers::NONE)).is_continue());
        }
        assert!(handle_key(
            &buffer,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
        )
        .is_continue());
        assert_eq!(buffer.lock().expect("buffer lock").text, "检查\n");
    }
}
