use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static EARLY_INPUT_BUFFER: Lazy<Arc<Mutex<String>>> =
    Lazy::new(|| Arc::new(Mutex::new(String::new())));
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
                                if key.kind == crossterm::event::KeyEventKind::Press {
                                    handle_key(&buffer, key);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            crate::utils::logging::append_debug_log_line("[EARLY_INPUT] Capture thread stopped");
        })
        .expect("Failed to spawn early input capture thread");

    *THREAD_HANDLE.lock().unwrap() = Some(handle);
}

fn handle_key(buffer: &Arc<Mutex<String>>, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            stop_capturing();
            cleanup_terminal();
            std::process::exit(130);
        }
        KeyCode::Backspace => {
            let mut buf = buffer.lock().unwrap();
            buf.pop();
        }
        KeyCode::Enter => {
            let mut buf = buffer.lock().unwrap();
            buf.push('\n');
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            let mut buf = buffer.lock().unwrap();
            buf.push(c);
        }
        _ => {}
    }
}

pub fn stop_capturing() {
    CAPTURING.store(false, Ordering::SeqCst);
    if let Some(handle) = THREAD_HANDLE.lock().unwrap().take() {
        let _ = handle.join();
    }
}

pub fn consume_early_input() -> String {
    stop_capturing();
    let mut buf = EARLY_INPUT_BUFFER.lock().unwrap();
    std::mem::take(&mut *buf)
}

pub fn peek_early_input() -> String {
    let buf = EARLY_INPUT_BUFFER.lock().unwrap();
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
