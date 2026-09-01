/// macOS Computer Use适配器
///
/// 对标claude-code-main的packages/@ant/computer-use-swift/
/// 使用AppleScript和osascript进行macOS屏幕操控
use super::{
    ComputerAdapter, ImageFormat, Modifier, MouseButton, Screenshot, WindowInfo, WindowList,
};

/// macOS Computer Use适配器
pub struct MacOSComputerAdapter;

impl MacOSComputerAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ComputerAdapter for MacOSComputerAdapter {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<(), String> {
        let button_str = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };

        let script = format!(
            r#"tell application "System Events"
                click at {{{}, {}}} with button "{}"
            end tell"#,
            x, y, button_str
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS click failed: {}", e))?;

        Ok(())
    }

    fn double_click(&self, x: i32, y: i32) -> Result<(), String> {
        let script = format!(
            r#"tell application "System Events"
                click at {{{}, {}}} with button "left"
                delay 0.1
                click at {{{}, {}}} with button "left"
            end tell"#,
            x, y, x, y
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS double click failed: {}", e))?;

        Ok(())
    }

    fn right_click(&self, x: i32, y: i32) -> Result<(), String> {
        self.click(x, y, MouseButton::Right)
    }

    fn mouse_move(&self, x: i32, y: i32) -> Result<(), String> {
        let script = format!(
            r#"tell application "System Events"
                set position of mouse to {{{}, {}}}
            end tell"#,
            x, y
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS mouse move failed: {}", e))?;

        Ok(())
    }

    fn mouse_drag(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<(), String> {
        let script = format!(
            r#"tell application "System Events"
                set position of mouse to {{{}, {}}}
                mouse down
                delay 0.1
                set position of mouse to {{{}, {}}}
                mouse up
            end tell"#,
            from_x, from_y, to_x, to_y
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS mouse drag failed: {}", e))?;

        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        let script = format!(
            r#"tell application "System Events"
                keystroke "{}"
            end tell"#,
            text.replace('"', "\\\"")
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS type failed: {}", e))?;

        Ok(())
    }

    fn key_press(&self, key: &str, modifiers: &[Modifier]) -> Result<(), String> {
        let mut modifier_str = String::new();
        for m in modifiers {
            match m {
                Modifier::Ctrl => modifier_str.push_str("control down, "),
                Modifier::Alt => modifier_str.push_str("option down, "),
                Modifier::Shift => modifier_str.push_str("shift down, "),
                Modifier::Super => modifier_str.push_str("command down, "),
            }
        }

        let script = format!(
            r#"tell application "System Events"
                key code {} using {{{}}}
            end tell"#,
            key,
            modifier_str.trim_end_matches(", ")
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS key press failed: {}", e))?;

        Ok(())
    }

    fn screenshot(&self) -> Result<Screenshot, String> {
        std::process::Command::new("screencapture")
            .args(["-x", "/tmp/starcode_screenshot.png"])
            .output()
            .map_err(|e| format!("macOS screenshot failed: {}", e))?;

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
        let script = format!(
            r#"tell application "System Events"
                set position of mouse to {{{}, {}}}
                scroll {} {} 
            end tell"#,
            x, y, delta_x, delta_y
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS scroll failed: {}", e))?;

        Ok(())
    }

    fn get_screen_size(&self) -> Result<(u32, u32), String> {
        let output = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"Finder\" to get bounds of window of desktop",
            ])
            .output()
            .map_err(|e| format!("macOS screen size failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split(", ").collect();
        if parts.len() >= 4 {
            let w: u32 = parts[2].parse().unwrap_or(0);
            let h: u32 = parts[3].parse().unwrap_or(0);
            return Ok((w, h));
        }

        Err("Failed to parse screen size".to_string())
    }

    fn get_active_window(&self) -> Result<WindowInfo, String> {
        let output = std::process::Command::new("osascript")
            .args(["-e", "tell application \"System Events\" to get name of first application process whose frontmost is true"])
            .output()
            .map_err(|e| format!("macOS active window failed: {}", e))?;

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
        let output = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"System Events\" to get name of every application process",
            ])
            .output()
            .map_err(|e| format!("macOS list windows failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let windows: Vec<WindowInfo> = stdout
            .trim()
            .split(", ")
            .enumerate()
            .map(|(i, name)| WindowInfo {
                id: i.to_string(),
                title: name.to_string(),
                process_name: name.to_string(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                is_focused: false,
            })
            .collect();

        Ok(WindowList(windows))
    }

    fn switch_window(&self, window_id: &str) -> Result<(), String> {
        let script = format!(
            r#"tell application "System Events"
                set frontmost of process "{}" to true
            end tell"#,
            window_id
        );

        std::process::Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("macOS switch window failed: {}", e))?;

        Ok(())
    }
}
