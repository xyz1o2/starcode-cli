#[cfg(windows)]
use std::io::IsTerminal;

#[cfg(windows)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
    FlushConsoleInputBuffer, GetConsoleMode, GetStdHandle, SetConsoleMode,
};

#[cfg(windows)]
const ENABLE_PROCESSED_INPUT: u32 = 0x0001;

#[cfg(windows)]
fn stdin_handle() -> Option<HANDLE> {
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        return None;
    }
    Some(handle)
}

#[cfg(windows)]
fn get_console_mode(handle: HANDLE) -> Option<u32> {
    let mut mode: u32 = 0;
    let ok = unsafe { GetConsoleMode(handle, &mut mode as *mut u32) };
    if ok == 0 {
        None
    } else {
        Some(mode)
    }
}

/// Set the console output code page to UTF-8 (65001).
/// cmd.exe defaults to CP936 (GBK) on Chinese Windows, which garbles UTF-8
/// encoded Chinese text. Returns the original code page so it can be restored.
#[cfg(windows)]
pub fn set_console_output_utf8() -> Option<u32> {
    use windows_sys::Win32::System::Console::{GetConsoleOutputCP, SetConsoleOutputCP};
    let original = unsafe { GetConsoleOutputCP() };
    if unsafe { SetConsoleOutputCP(65001) } == 0 {
        // Failed, return None but don't crash
        None
    } else {
        Some(original)
    }
}

/// Restore console output code page to a previously saved value.
#[cfg(windows)]
pub fn restore_console_output_cp(cp: u32) {
    use windows_sys::Win32::System::Console::SetConsoleOutputCP;
    unsafe {
        SetConsoleOutputCP(cp);
    }
}

#[cfg(not(windows))]
pub fn set_console_output_utf8() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn restore_console_output_cp(_cp: u32) {}

#[cfg(windows)]
fn set_console_mode(handle: HANDLE, mode: u32) {
    unsafe {
        let _ = SetConsoleMode(handle, mode);
    }
}

#[cfg(windows)]
pub fn disable_processed_input() {
    if !std::io::stdin().is_terminal() {
        return;
    }
    let Some(handle) = stdin_handle() else {
        return;
    };
    let Some(mode) = get_console_mode(handle) else {
        return;
    };
    if (mode & ENABLE_PROCESSED_INPUT) == 0 {
        return;
    }
    set_console_mode(handle, mode & !ENABLE_PROCESSED_INPUT);
}

#[cfg(not(windows))]
pub fn disable_processed_input() {}

#[cfg(windows)]
pub fn flush_input_buffer() {
    if !std::io::stdin().is_terminal() {
        return;
    }
    let Some(handle) = stdin_handle() else {
        return;
    };
    unsafe {
        let _ = FlushConsoleInputBuffer(handle);
    }
}

#[cfg(not(windows))]
pub fn flush_input_buffer() {}

#[cfg(windows)]
fn enforce_processed_input_off(handle: HANDLE) {
    let Some(mode) = get_console_mode(handle) else {
        return;
    };
    if (mode & ENABLE_PROCESSED_INPUT) == 0 {
        return;
    }
    set_console_mode(handle, mode & !ENABLE_PROCESSED_INPUT);
}

#[cfg(windows)]
pub struct CtrlCGuard {
    handle: HANDLE,
    original_mode: u32,
    original_output_cp: Option<u32>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl CtrlCGuard {
    pub fn install() -> Option<Self> {
        if !std::io::stdin().is_terminal() {
            return None;
        }
        let handle = stdin_handle()?;
        let original_mode = get_console_mode(handle)?;
        let original_output_cp = set_console_output_utf8();

        // Enforce immediately
        enforce_processed_input_off(handle);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                enforce_processed_input_off(handle);
                thread::sleep(Duration::from_millis(100));
            }
        });

        Some(Self {
            handle,
            original_mode,
            original_output_cp,
            stop,
            thread: Some(thread),
        })
    }

    pub fn enforce(&self) {
        enforce_processed_input_off(self.handle);
    }
}

#[cfg(windows)]
impl Drop for CtrlCGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        set_console_mode(self.handle, self.original_mode);
        if let Some(cp) = self.original_output_cp {
            restore_console_output_cp(cp);
        }
    }
}

#[cfg(not(windows))]
pub struct CtrlCGuard;

#[cfg(not(windows))]
impl CtrlCGuard {
    pub fn install() -> Option<Self> {
        None
    }

    pub fn enforce(&self) {}
}
