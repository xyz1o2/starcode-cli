//! Computer Use 模块
//!
//! 对标 Claude Code 的 computer-use.md：
//! - 跨平台屏幕操控（Linux/macOS/Windows）
//! - 键鼠模拟
//! - 截图
//! - 应用管理

pub mod macos;
pub mod windows;

pub use macos::MacOSComputerAdapter;
pub use windows::WindowsComputerAdapter;

use serde::{Serialize, Deserialize};

/// 屏幕截图
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub format: ImageFormat,
}

#[derive(Debug, Clone)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Bmp,
}

/// 鼠标按钮
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 键盘修饰键
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Super,
}

/// Computer Use 操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputerAction {
    Click { x: i32, y: i32, button: MouseButton },
    DoubleClick { x: i32, y: i32 },
    RightClick { x: i32, y: i32 },
    MouseMove { x: i32, y: i32 },
    MouseDrag { from_x: i32, from_y: i32, to_x: i32, to_y: i32 },
    Type { text: String },
    KeyPress { key: String, modifiers: Vec<Modifier> },
    Screenshot,
    Scroll { x: i32, y: i32, delta_x: i32, delta_y: i32 },
    Wait { ms: u64 },
    GetScreenSize,
    GetActiveWindow,
    ListWindows,
    SwitchWindow { window_id: String },
}

/// Computer Use 结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputerResult {
    Success { message: String },
    ScreenshotData { width: u32, height: u32, base64: String },
    ScreenSize { width: u32, height: u32 },
    Windows(WindowList),
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowList(pub Vec<WindowInfo>);

/// 窗口信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub process_name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_focused: bool,
}

/// Computer Use 平台适配器 trait
pub trait ComputerAdapter: Send + Sync {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<(), String>;
    fn double_click(&self, x: i32, y: i32) -> Result<(), String>;
    fn right_click(&self, x: i32, y: i32) -> Result<(), String>;
    fn mouse_move(&self, x: i32, y: i32) -> Result<(), String>;
    fn mouse_drag(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<(), String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
    fn key_press(&self, key: &str, modifiers: &[Modifier]) -> Result<(), String>;
    fn screenshot(&self) -> Result<Screenshot, String>;
    fn scroll(&self, x: i32, y: i32, delta_x: i32, delta_y: i32) -> Result<(), String>;
    fn get_screen_size(&self) -> Result<(u32, u32), String>;
    fn get_active_window(&self) -> Result<WindowInfo, String>;
    fn list_windows(&self) -> Result<WindowList, String>;
    fn switch_window(&self, window_id: &str) -> Result<(), String>;
}

/// Linux X11/Wayland 适配器
pub struct LinuxComputerAdapter;

impl LinuxComputerAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ComputerAdapter for LinuxComputerAdapter {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<(), String> {
        let btn = match button {
            MouseButton::Left => "1",
            MouseButton::Right => "3",
            MouseButton::Middle => "2",
        };
        std::process::Command::new("xdotool")
            .args(["mousemove", &x.to_string(), &y.to_string(), "click", btn])
            .output()
            .map_err(|e| format!("xdotool click failed: {}", e))?;
        Ok(())
    }

    fn double_click(&self, x: i32, y: i32) -> Result<(), String> {
        std::process::Command::new("xdotool")
            .args(["mousemove", &x.to_string(), &y.to_string(), "click", "--repeat", "2", "1"])
            .output()
            .map_err(|e| format!("xdotool double click failed: {}", e))?;
        Ok(())
    }

    fn right_click(&self, x: i32, y: i32) -> Result<(), String> {
        self.click(x, y, MouseButton::Right)
    }

    fn mouse_move(&self, x: i32, y: i32) -> Result<(), String> {
        std::process::Command::new("xdotool")
            .args(["mousemove", &x.to_string(), &y.to_string()])
            .output()
            .map_err(|e| format!("xdotool move failed: {}", e))?;
        Ok(())
    }

    fn mouse_drag(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<(), String> {
        std::process::Command::new("xdotool")
            .args([
                "mousemove", &from_x.to_string(), &from_y.to_string(),
                "mousedown", "1",
                "mousemove", &to_x.to_string(), &to_y.to_string(),
                "mouseup", "1",
            ])
            .output()
            .map_err(|e| format!("xdotool drag failed: {}", e))?;
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        std::process::Command::new("xdotool")
            .args(["type", "--clearmodifiers", text])
            .output()
            .map_err(|e| format!("xdotool type failed: {}", e))?;
        Ok(())
    }

    fn key_press(&self, key: &str, modifiers: &[Modifier]) -> Result<(), String> {
        let mut args = Vec::new();
        args.push("key".to_string());
        for m in modifiers {
            let prefix = match m {
                Modifier::Ctrl => "ctrl+",
                Modifier::Alt => "alt+",
                Modifier::Shift => "shift+",
                Modifier::Super => "super+",
            };
            args.push(prefix.to_string());
        }
        args.push(key.to_string());
        std::process::Command::new("xdotool")
            .args(args.as_slice())
            .output()
            .map_err(|e| format!("xdotool key failed: {}", e))?;
        Ok(())
    }

    fn screenshot(&self) -> Result<Screenshot, String> {
        std::process::Command::new("scrot")
            .args(["-o", "/tmp/starcode_screenshot.png"])
            .output()
            .map_err(|e| format!("screenshot failed: {}", e))?;

        let data = std::fs::read("/tmp/starcode_screenshot.png")
            .map_err(|e| format!("failed to read screenshot: {}", e))?;

        Ok(Screenshot {
            width: 0,
            height: 0,
            data,
            format: ImageFormat::Png,
        })
    }

    fn scroll(&self, x: i32, y: i32, delta_x: i32, delta_y: i32) -> Result<(), String> {
        let btn = if delta_y > 0 { "4" } else { "5" };
        std::process::Command::new("xdotool")
            .args(["mousemove", &x.to_string(), &y.to_string(), "click", btn])
            .output()
            .map_err(|e| format!("xdotool scroll failed: {}", e))?;
        Ok(())
    }

    fn get_screen_size(&self) -> Result<(u32, u32), String> {
        let output = std::process::Command::new("xdpyinfo")
            .output()
            .map_err(|e| format!("xdpyinfo failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains("dimensions:") {
                if let Some(dim) = line.split_whitespace().nth(1) {
                    let parts: Vec<&str> = dim.split('x').collect();
                    if parts.len() == 2 {
                        let w: u32 = parts[0].parse().unwrap_or(0);
                        let h: u32 = parts[1].parse().unwrap_or(0);
                        return Ok((w, h));
                    }
                }
            }
        }
        Err("Failed to parse screen size".to_string())
    }

    fn get_active_window(&self) -> Result<WindowInfo, String> {
        let output = std::process::Command::new("xdotool")
            .args(["getactivewindow", "getwindowname"])
            .output()
            .map_err(|e| format!("xdotool failed: {}", e))?;

        let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(WindowInfo {
            id: "active".to_string(),
            title,
            process_name: String::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            is_focused: true,
        })
    }

    fn list_windows(&self) -> Result<WindowList, String> {
        let output = std::process::Command::new("wmctrl")
            .args(["-l"])
            .output()
            .map_err(|e| format!("wmctrl failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut windows = Vec::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                windows.push(WindowInfo {
                    id: parts[0].to_string(),
                    title: parts[3].to_string(),
                    process_name: String::new(),
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    is_focused: false,
                });
            }
        }
        Ok(WindowList(windows))
    }

    fn switch_window(&self, window_id: &str) -> Result<(), String> {
        std::process::Command::new("xdotool")
            .args(["windowactivate", window_id])
            .output()
            .map_err(|e| format!("xdotool switch failed: {}", e))?;
        Ok(())
    }
}

/// Computer Use 管理器
pub struct ComputerUseManager {
    adapter: Box<dyn ComputerAdapter>,
}

impl ComputerUseManager {
    pub fn new() -> Self {
        let adapter = Box::new(LinuxComputerAdapter::new());
        Self { adapter }
    }

    pub fn execute(&self, action: ComputerAction) -> ComputerResult {
        match action {
            ComputerAction::Click { x, y, button } => {
                self.adapter.click(x, y, button)
                    .map(|()| ComputerResult::Success { message: format!("Clicked at ({}, {})", x, y) })
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            ComputerAction::DoubleClick { x, y } => {
                self.adapter.double_click(x, y)
                    .map(|()| ComputerResult::Success { message: format!("Double-clicked at ({}, {})", x, y) })
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            ComputerAction::Type { text } => {
                self.adapter.type_text(&text)
                    .map(|()| ComputerResult::Success { message: format!("Typed {} chars", text.len()) })
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            ComputerAction::Screenshot => {
                self.adapter.screenshot()
                    .map(|s| ComputerResult::ScreenshotData {
                        width: s.width,
                        height: s.height,
                        base64: base64_encode(&s.data),
                    })
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            ComputerAction::GetScreenSize => {
                self.adapter.get_screen_size()
                    .map(|(w, h)| ComputerResult::ScreenSize { width: w, height: h })
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            ComputerAction::ListWindows => {
                self.adapter.list_windows()
                    .map(ComputerResult::Windows)
                    .unwrap_or_else(|e| ComputerResult::Error { message: e })
            }
            _ => ComputerResult::Error {
                message: "Action not yet implemented".to_string(),
            },
        }
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
