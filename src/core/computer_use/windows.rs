/// Windows Computer Use适配器
///
/// 对标claude-code-main的packages/@ant/computer-use-input/
/// 使用PowerShell和Windows API进行Windows屏幕操控
use super::{
    ComputerAdapter, ImageFormat, Modifier, MouseButton, Screenshot, WindowInfo, WindowList,
};

/// Windows Computer Use适配器
pub struct WindowsComputerAdapter;

impl WindowsComputerAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ComputerAdapter for WindowsComputerAdapter {
    fn click(&self, x: i32, y: i32, button: MouseButton) -> Result<(), String> {
        let button_str = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };

        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})
            [System.Windows.Forms.SendInput]::Click("{}")"#,
            x, y, button_str
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows click failed: {}", e))?;

        Ok(())
    }

    fn double_click(&self, x: i32, y: i32) -> Result<(), String> {
        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})
            [System.Windows.Forms.SendInput]::Click("left")
            Start-Sleep -Milliseconds 100
            [System.Windows.Forms.SendInput]::Click("left")"#,
            x, y
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows double click failed: {}", e))?;

        Ok(())
    }

    fn right_click(&self, x: i32, y: i32) -> Result<(), String> {
        self.click(x, y, MouseButton::Right)
    }

    fn mouse_move(&self, x: i32, y: i32) -> Result<(), String> {
        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})"#,
            x, y
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows mouse move failed: {}", e))?;

        Ok(())
    }

    fn mouse_drag(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<(), String> {
        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})
            [System.Windows.Forms.SendInput]::MouseDown("left")
            Start-Sleep -Milliseconds 100
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})
            [System.Windows.Forms.SendInput]::MouseUp("left")"#,
            from_x, from_y, to_x, to_y
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows mouse drag failed: {}", e))?;

        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.SendKeys]::SendWait("{}")"#,
            text.replace('"', "\\\"")
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows type failed: {}", e))?;

        Ok(())
    }

    fn key_press(&self, key: &str, modifiers: &[Modifier]) -> Result<(), String> {
        let mut modifier_str = String::new();
        for m in modifiers {
            match m {
                Modifier::Ctrl => modifier_str.push('^'),
                Modifier::Alt => modifier_str.push('%'),
                Modifier::Shift => modifier_str.push('+'),
                Modifier::Super => modifier_str.push('#'),
            }
        }

        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.SendKeys]::SendWait("{}{}")"#,
            modifier_str, key
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows key press failed: {}", e))?;

        Ok(())
    }

    fn screenshot(&self) -> Result<Screenshot, String> {
        let script = r#"Add-Type -AssemblyName System.Windows.Forms
        Add-Type -AssemblyName System.Drawing
        $screen = [System.Windows.Forms.Screen]::PrimaryScreen
        $bitmap = New-Object System.Drawing.Bitmap($screen.Bounds.Width, $screen.Bounds.Height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($screen.Bounds.Location, [System.Drawing.Point]::Empty, $screen.Bounds.Size)
        $bitmap.Save("C:\temp\starcode_screenshot.png", [System.Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose()
        $bitmap.Dispose()"#;

        std::process::Command::new("powershell")
            .args(["-Command", script])
            .output()
            .map_err(|e| format!("Windows screenshot failed: {}", e))?;

        let data = std::fs::read("C:\\temp\\starcode_screenshot.png")
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
            r#"Add-Type -AssemblyName System.Windows.Forms
            [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point({}, {})
            [System.Windows.Forms.SendInput]::Scroll({})"#,
            x, y, delta_y
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows scroll failed: {}", e))?;

        Ok(())
    }

    fn get_screen_size(&self) -> Result<(u32, u32), String> {
        let script = r#"Add-Type -AssemblyName System.Windows.Forms
        $screen = [System.Windows.Forms.Screen]::PrimaryScreen
        Write-Output "$($screen.Bounds.Width) $($screen.Bounds.Height)""#;

        let output = std::process::Command::new("powershell")
            .args(["-Command", script])
            .output()
            .map_err(|e| format!("Windows screen size failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split_whitespace().collect();
        if parts.len() >= 2 {
            let w: u32 = parts[0].parse().unwrap_or(0);
            let h: u32 = parts[1].parse().unwrap_or(0);
            return Ok((w, h));
        }

        Err("Failed to parse screen size".to_string())
    }

    fn get_active_window(&self) -> Result<WindowInfo, String> {
        let script = r#"Add-Type -AssemblyName Microsoft.VisualBasic
        $window = [Microsoft.VisualBasic.Interaction]::AppActivate((Get-Process | Where-Object {$_.MainWindowTitle -ne ""} | Select-Object -First 1).Id)
        Write-Output $window"#;

        let output = std::process::Command::new("powershell")
            .args(["-Command", script])
            .output()
            .map_err(|e| format!("Windows active window failed: {}", e))?;

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
        let script = r#"Get-Process | Where-Object {$_.MainWindowTitle -ne ""} | Select-Object Id, MainWindowTitle, ProcessName"#;

        let output = std::process::Command::new("powershell")
            .args(["-Command", script])
            .output()
            .map_err(|e| format!("Windows list windows failed: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let windows: Vec<WindowInfo> = stdout
            .lines()
            .skip(3) // Skip header
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    Some(WindowInfo {
                        id: parts[0].to_string(),
                        title: parts[1..parts.len() - 1].join(" "),
                        process_name: parts[parts.len() - 1].to_string(),
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                        is_focused: false,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(WindowList(windows))
    }

    fn switch_window(&self, window_id: &str) -> Result<(), String> {
        let script = format!(
            r#"Add-Type -AssemblyName Microsoft.VisualBasic
            [Microsoft.VisualBasic.Interaction]::AppActivate({})"#,
            window_id
        );

        std::process::Command::new("powershell")
            .args(["-Command", &script])
            .output()
            .map_err(|e| format!("Windows switch window failed: {}", e))?;

        Ok(())
    }
}
