use crate::core::tools::tools::LocationType;
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolInvocation, ToolLocation,
};
use crate::types::ToolResult;
#[cfg(windows)]
use encoding_rs::GB18030;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{timeout, Duration};

#[cfg(windows)]
fn decode_cmd_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
        return s.trim_start_matches('\u{feff}').to_string();
    }

    let (s, _, _) = GB18030.decode(bytes);
    s.into_owned()
}

#[cfg(not(windows))]
fn decode_cmd_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(windows)]
fn split_shell_args(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;

    for ch in command.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(ch),
        }
    }

    if !cur.is_empty() {
        out.push(cur);
    }

    out
}

#[cfg(windows)]
fn try_fs_rename(command: &str) -> Option<ToolResult> {
    let args = split_shell_args(command);
    if args.is_empty() {
        return None;
    }
    let head = args[0].to_lowercase();
    if head != "mv" && head != "move" {
        return None;
    }
    if args.len() != 3 {
        return None;
    }
    let src = &args[1];
    let dst = &args[2];
    match std::fs::rename(src, dst) {
        Ok(()) => Some(ToolResult {
            success: true,
            output: Some(format!("Renamed: {} -> {}", src, dst)),
            error: None,
            data: None,
        }),
        Err(e) => Some(ToolResult {
            success: false,
            output: None,
            error: Some(format!("Command failed: {}", e)),
            data: None,
        }),
    }
}

#[cfg(windows)]
fn normalize_needle(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[bytes.len() - 1] as char;
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

#[cfg(windows)]
fn parse_exa_level(left: &str) -> Option<usize> {
    let args = split_shell_args(left);
    for (i, a) in args.iter().enumerate() {
        let al = a.to_lowercase();
        if let Some(v) = al.strip_prefix("--level=") {
            if let Ok(n) = v.parse::<usize>() {
                return Some(n);
            }
        }
        if al == "--level" {
            if let Some(v) = args.get(i + 1) {
                if let Ok(n) = v.parse::<usize>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn try_windows_find_files_from_exa_grep(command: &str, base_dir: &str) -> Option<ToolResult> {
    use walkdir::WalkDir;

    let cmd_trim = command.trim_start();
    let head_lower = cmd_trim
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    if head_lower != "exa" && head_lower != "ls" && head_lower != "dir" {
        return None;
    }

    let parts: Vec<&str> = command.splitn(2, '|').collect();
    if parts.len() != 2 {
        return None;
    }
    let left = parts[0];
    let right = parts[1];
    if !right.to_lowercase().contains("grep") {
        return None;
    }

    let right_args = split_shell_args(right);
    let mut grep_pos: Option<usize> = None;
    for (i, a) in right_args.iter().enumerate() {
        if a.to_lowercase() == "grep" {
            grep_pos = Some(i);
            break;
        }
    }
    let Some(grep_pos) = grep_pos else {
        return None;
    };

    let grep_args = &right_args[grep_pos + 1..];
    let mut case_insensitive = false;
    let mut needle: Option<String> = None;
    for a in grep_args {
        let al = a.to_lowercase();
        if al == "-i" || al == "--ignore-case" {
            case_insensitive = true;
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        needle = Some(normalize_needle(a));
    }
    let Some(needle) = needle else {
        return None;
    };

    let level = parse_exa_level(left).unwrap_or(256);
    let needle_cmp = if case_insensitive {
        needle.to_lowercase()
    } else {
        needle.clone()
    };

    let mut hits: Vec<String> = Vec::new();
    for entry in WalkDir::new(base_dir)
        .follow_links(true)
        .max_depth(level)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if hits.len() >= 80 {
            break;
        }
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        let name_cmp = if case_insensitive {
            name.to_lowercase()
        } else {
            name.to_string()
        };
        if name_cmp.contains(&needle_cmp) {
            hits.push(p.to_string_lossy().to_string());
        }
    }

    if hits.is_empty() {
        return Some(ToolResult {
            success: true,
            output: Some(format!("No results found for \"{}\"", needle)),
            error: None,
            data: None,
        });
    }

    let mut out = String::new();
    out.push_str(&format!("Found {} file(s):\n", hits.len()));
    for h in hits {
        out.push_str("  ");
        out.push_str(&h);
        out.push('\n');
    }

    Some(ToolResult {
        success: true,
        output: Some(out.trim_end().to_string()),
        error: None,
        data: None,
    })
}

#[derive(Clone)]
pub struct BashTool {
    current_directory: String,
    sandbox_manager: Option<Arc<crate::core::sandbox::SandboxManager>>,
    sandbox_enabled: bool,
}

impl BashTool {
    pub fn new() -> Self {
        let sandbox_available = crate::core::sandbox::SandboxManager::is_available();
        Self {
            current_directory: std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            sandbox_manager: None,
            sandbox_enabled: sandbox_available,
        }
    }

    pub fn with_sandbox(config: crate::core::sandbox::SandboxConfig) -> Result<Self, String> {
        let manager = crate::core::sandbox::SandboxManager::new(config)
            .map_err(|e| format!("Failed to create sandbox: {}", e))?;
        Ok(Self {
            current_directory: std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            sandbox_manager: Some(Arc::new(manager)),
            sandbox_enabled: true,
        })
    }

    pub fn set_sandbox_enabled(&mut self, enabled: bool) {
        self.sandbox_enabled = enabled;
    }

    pub fn is_sandbox_active(&self) -> bool {
        self.sandbox_enabled && self.sandbox_manager.is_some()
    }

    pub fn sandbox_status(&self) -> &'static str {
        if self.sandbox_enabled && self.sandbox_manager.is_some() {
            "enabled"
        } else if self.sandbox_manager.is_some() {
            "disabled"
        } else {
            "unavailable"
        }
    }

    fn non_interactive_enabled() -> bool {
        std::env::var("STAR_ENABLE_NON_INTERACTIVE_ENV")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    }

    fn apply_non_interactive_env(cmd: &mut Command) {
        if !Self::non_interactive_enabled() {
            return;
        }

        cmd.env("PAGER", "cat");
        cmd.env("GIT_PAGER", "cat");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("CI", "1");
    }

    fn detect_interactive_command(command: &str) -> Option<&'static str> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Some("empty command");
        }

        let first = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let token_count = trimmed.split_whitespace().count();

        if [
            "vim", "vi", "nano", "emacs", "top", "htop", "less", "more", "man", "watch",
        ]
        .contains(&first.as_str())
        {
            return Some("interactive TUI command");
        }

        if [
            "python", "python3", "node", "irb", "lua", "bash", "sh", "zsh", "fish",
        ]
        .contains(&first.as_str())
            && token_count == 1
        {
            return Some("interactive interpreter");
        }

        if trimmed.contains(" -i ") || trimmed.ends_with(" -i") {
            return Some("interactive option (-i)");
        }

        None
    }

    fn resolve_working_directory(&self, raw: &str) -> std::path::PathBuf {
        let trimmed = raw.trim();
        let expanded = if trimmed == "~" || trimmed.starts_with("~/") {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| "~".to_string());
            if trimmed == "~" {
                home
            } else {
                format!("{}/{}", home, trimmed.trim_start_matches("~/"))
            }
        } else {
            trimmed.to_string()
        };

        let candidate = std::path::PathBuf::from(expanded);
        if candidate.is_absolute() {
            candidate
        } else {
            std::path::PathBuf::from(&self.current_directory).join(candidate)
        }
    }

    pub async fn execute(
        &mut self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let command_owned = command.replace('\r', "");
        let command = command_owned.trim();

        if let Some(new_dir) = command.strip_prefix("cd ") {
            let new_dir = new_dir.trim();
            let target = self.resolve_working_directory(new_dir);
            match std::fs::canonicalize(&target) {
                Ok(path) if path.is_dir() => {
                    self.current_directory = path.to_string_lossy().to_string();
                    Ok(ToolResult {
                        success: true,
                        output: Some(format!("Changed directory to: {}", self.current_directory)),
                        error: None,
                        data: None,
                    })
                }
                Ok(path) => Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Cannot change directory: not a directory ({})",
                        path.display()
                    )),
                    data: None,
                }),
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!("Cannot change directory: {}", e)),
                    data: None,
                }),
            }
        } else {
            if let Some(reason) = Self::detect_interactive_command(command) {
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Blocked interactive command ({}). Use non-interactive command or add explicit timeout.",
                        reason
                    )),
                    data: None,
                });
            }

            if self.is_sandbox_active() {
                return self.execute_in_sandbox(command, timeout_secs).await;
            }

            #[cfg(windows)]
            if let Some(result) = try_fs_rename(command) {
                return Ok(result);
            }

            #[cfg(windows)]
            {
                let base_dir = self.current_directory.clone();
                if let Some(result) = try_windows_find_files_from_exa_grep(command, &base_dir) {
                    return Ok(result);
                }
            }

            let effective_timeout_secs = timeout_secs
                .or_else(|| {
                    std::env::var("STAR_BASH_TIMEOUT_SECS")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .unwrap_or(180);

            #[cfg(unix)]
            let output = {
                let mut cmd = Command::new("sh");
                cmd.arg("-c").arg(command);
                cmd.current_dir(&self.current_directory);
                cmd.kill_on_drop(true);
                Self::apply_non_interactive_env(&mut cmd);
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());

                let mut child = cmd.spawn()?;
                let mut child_stdout = child.stdout.take();
                let mut child_stderr = child.stderr.take();

                let stdout_handle = tokio::spawn(async move {
                    let mut buf: Vec<u8> = Vec::new();
                    if let Some(out) = child_stdout.as_mut() {
                        let _ = out.read_to_end(&mut buf).await;
                    }
                    buf
                });
                let stderr_handle = tokio::spawn(async move {
                    let mut buf: Vec<u8> = Vec::new();
                    if let Some(out) = child_stderr.as_mut() {
                        let _ = out.read_to_end(&mut buf).await;
                    }
                    buf
                });

                let status = if effective_timeout_secs == 0 {
                    child.wait().await?
                } else {
                    match timeout(Duration::from_secs(effective_timeout_secs), child.wait()).await {
                        Ok(res) => res?,
                        Err(_) => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            let _ = stdout_handle.await;
                            let _ = stderr_handle.await;
                            return Ok(ToolResult {
                                success: false,
                                output: None,
                                error: Some(format!("command timed out: {}s", effective_timeout_secs)),
                                data: None,
                            });
                        }
                    }
                };

                let stdout = stdout_handle.await.unwrap_or_default();
                let stderr = stderr_handle.await.unwrap_or_default();
                std::process::Output {
                    status,
                    stdout,
                    stderr,
                }
            };

            #[cfg(windows)]
            let output = {
                let mut cmd = Command::new("powershell.exe");
                cmd.arg("-NoProfile").arg("-Command").arg(command);
                cmd.current_dir(&self.current_directory);
                cmd.kill_on_drop(true);

                Self::apply_non_interactive_env(&mut cmd);
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());

                let mut child = cmd.spawn()?;
                let mut child_stdout = child.stdout.take();
                let mut child_stderr = child.stderr.take();

                let stdout_handle = tokio::spawn(async move {
                    let mut buf: Vec<u8> = Vec::new();
                    if let Some(out) = child_stdout.as_mut() {
                        let _ = out.read_to_end(&mut buf).await;
                    }
                    buf
                });
                let stderr_handle = tokio::spawn(async move {
                    let mut buf: Vec<u8> = Vec::new();
                    if let Some(out) = child_stderr.as_mut() {
                        let _ = out.read_to_end(&mut buf).await;
                    }
                    buf
                });

                let status = if effective_timeout_secs == 0 {
                    child.wait().await?
                } else {
                    match timeout(Duration::from_secs(effective_timeout_secs), child.wait()).await {
                        Ok(res) => res?,
                        Err(_) => {
                            if let Some(pid) = child.id() {
                                let _ = Command::new("taskkill")
                                    .arg("/PID")
                                    .arg(pid.to_string())
                                    .arg("/T")
                                    .arg("/F")
                                    .output()
                                    .await;
                            }
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            let _ = stdout_handle.await;
                            let _ = stderr_handle.await;
                            return Ok(ToolResult {
                                success: false,
                                output: None,
                                error: Some(format!("command timed out: {}s", effective_timeout_secs)),
                                data: None,
                            });
                        }
                    }
                };

                let stdout = stdout_handle.await.unwrap_or_default();
                let stderr = stderr_handle.await.unwrap_or_default();
                std::process::Output {
                    status,
                    stdout,
                    stderr,
                }
            };

            let mut stdout = decode_cmd_output(&output.stdout);
            let mut stderr = decode_cmd_output(&output.stderr);
            stdout = stdout.replace("\r\n", "\n");
            stderr = stderr.replace("\r\n", "\n");

            if output.status.success() {
                let full_output = if !stderr.is_empty() {
                    format!("{}\n{}", stdout, stderr)
                } else {
                    stdout
                };

                Ok(ToolResult {
                    success: true,
                    output: Some(full_output.trim().to_string()),
                    error: None,
                    data: None,
                })
            } else {
                let err_text = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    stdout.trim().to_string()
                };
                Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!("Command failed: {}", err_text)),
                    data: None,
                })
            }
        }
    }

    async fn execute_in_sandbox(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(manager) = &self.sandbox_manager {
            let result = manager
                .execute(command, timeout_secs)
                .await
                .map_err(|e| format!("Sandbox error: {}", e))?;

            if !result.success && Self::is_sandbox_restriction_error(&result.stderr) {
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!(
                        "{}\n\n⚠️ Sandbox restriction failed, retrying automatically...\nUse /sandbox to manage restrictions",
                        result.stderr
                    )),
                    data: Some(serde_json::json!({
                        "sandbox_restriction": true,
                        "should_retry_without_sandbox": true
                    })),
                });
            }

            Ok(ToolResult {
                success: result.success,
                output: if result.stdout.is_empty() {
                    None
                } else {
                    Some(result.stdout)
                },
                error: if result.stderr.is_empty() {
                    None
                } else {
                    Some(result.stderr)
                },
                data: None,
            })
        } else {
            Err("Sandbox manager not initialized".into())
        }
    }

    fn is_sandbox_restriction_error(stderr: &str) -> bool {
        let lower = stderr.to_lowercase();

        lower.contains("operation not permitted")
            || lower.contains("permission denied")
            || lower.contains("access denied")
            || lower.contains("connection refused")
            || lower.contains("network is unreachable")
            || lower.contains("cannot connect")
            || lower.contains("socket")
                && (lower.contains("permission") || lower.contains("denied"))
            || lower.contains("eperm")
            || lower.contains("eacces")
            || lower.contains("sandbox")
            || lower.contains("isolated")
            || lower.contains("restricted")
    }

    pub async fn execute_without_sandbox(
        &mut self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let was_enabled = self.sandbox_enabled;
        self.sandbox_enabled = false;
        let result = self.execute(command, timeout_secs).await;
        self.sandbox_enabled = was_enabled;
        result
    }

    pub async fn execute_streaming(
        &mut self,
        command: &str,
        timeout_secs: Option<u64>,
        progress_tx: UnboundedSender<String>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let command_owned = command.replace('\r', "");
        let command = command_owned.trim();

        if command.starts_with("cd ") {
            return self.execute(command, timeout_secs).await;
        }

        if let Some(reason) = Self::detect_interactive_command(command) {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!(
                    "拒绝执行可能阻塞的交互式命令（{}）。请改为非交互命令或显式加超时。",
                    reason
                )),
                data: None,
            });
        }

        #[cfg(windows)]
        if let Some(result) = try_fs_rename(command) {
            return Ok(result);
        }

        #[cfg(windows)]
        {
            let base_dir = self.current_directory.clone();
            if let Some(result) = try_windows_find_files_from_exa_grep(command, &base_dir) {
                return Ok(result);
            }
        }

        let effective_timeout_secs = timeout_secs
            .or_else(|| {
                std::env::var("STAR_BASH_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(180);

        #[cfg(unix)]
        let output = {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);
            cmd.current_dir(&self.current_directory);
            cmd.kill_on_drop(true);
            Self::apply_non_interactive_env(&mut cmd);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            let mut child = cmd.spawn()?;
            let child_stdout = child.stdout.take();
            let child_stderr = child.stderr.take();

            let tx_out = progress_tx.clone();
            let stdout_handle = tokio::spawn(async move {
                let mut acc: Vec<u8> = Vec::new();
                if let Some(out) = child_stdout {
                    let mut reader = BufReader::new(out);
                    let mut buf: Vec<u8> = Vec::new();
                    loop {
                        buf.clear();
                        let n = reader.read_until(b'\n', &mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        acc.extend_from_slice(&buf);
                        let text = String::from_utf8_lossy(&buf);
                        let text = truncate_for_output_streaming(&text, 2000);
                        let _ = tx_out.send(text);
                    }
                }
                acc
            });

            let tx_err = progress_tx.clone();
            let stderr_handle = tokio::spawn(async move {
                let mut acc: Vec<u8> = Vec::new();
                if let Some(out) = child_stderr {
                    let mut reader = BufReader::new(out);
                    let mut buf: Vec<u8> = Vec::new();
                    loop {
                        buf.clear();
                        let n = reader.read_until(b'\n', &mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        acc.extend_from_slice(&buf);
                        let text = String::from_utf8_lossy(&buf);
                        let text = truncate_for_output_streaming(&text, 2000);
                        let _ = tx_err.send(format!("[stderr] {}", text));
                    }
                }
                acc
            });

            let status = if effective_timeout_secs == 0 {
                child.wait().await?
            } else {
                match timeout(Duration::from_secs(effective_timeout_secs), child.wait()).await {
                    Ok(res) => res?,
                    Err(_) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = stdout_handle.await;
                        let _ = stderr_handle.await;
                        return Ok(ToolResult {
                            success: false,
                            output: None,
                            error: Some(format!("命令超时：{}s", effective_timeout_secs)),
                            data: None,
                        });
                    }
                }
            };

            let stdout = stdout_handle.await.unwrap_or_default();
            let stderr = stderr_handle.await.unwrap_or_default();
            std::process::Output {
                status,
                stdout,
                stderr,
            }
        };

        #[cfg(windows)]
        let output = {
            let cmd_trim = command.trim_start();
            let lower = cmd_trim.to_lowercase();
            let mut cmd = if lower == "cargo" || lower.starts_with("cargo ") {
                let args = split_shell_args(command);
                if args.is_empty() {
                    let mut c = Command::new("cmd");
                    c.arg("/C").arg(command);
                    c
                } else {
                    let mut c = Command::new(&args[0]);
                    for a in args.iter().skip(1) {
                        c.arg(a);
                    }
                    c
                }
            } else {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(command);
                c
            };

            cmd.current_dir(&self.current_directory);
            cmd.kill_on_drop(true);
            Self::apply_non_interactive_env(&mut cmd);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            let mut child = cmd.spawn()?;
            let pid = child.id();
            let child_stdout = child.stdout.take();
            let child_stderr = child.stderr.take();

            let tx_out = progress_tx.clone();
            let stdout_handle = tokio::spawn(async move {
                let mut acc: Vec<u8> = Vec::new();
                if let Some(out) = child_stdout {
                    let mut reader = BufReader::new(out);
                    let mut buf: Vec<u8> = Vec::new();
                    loop {
                        buf.clear();
                        let n = reader.read_until(b'\n', &mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        acc.extend_from_slice(&buf);
                        let text = String::from_utf8_lossy(&buf);
                        let text = truncate_for_output_streaming(&text, 2000);
                        let _ = tx_out.send(text);
                    }
                }
                acc
            });

            let tx_err = progress_tx.clone();
            let stderr_handle = tokio::spawn(async move {
                let mut acc: Vec<u8> = Vec::new();
                if let Some(out) = child_stderr {
                    let mut reader = BufReader::new(out);
                    let mut buf: Vec<u8> = Vec::new();
                    loop {
                        buf.clear();
                        let n = reader.read_until(b'\n', &mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        acc.extend_from_slice(&buf);
                        let text = String::from_utf8_lossy(&buf);
                        let text = truncate_for_output_streaming(&text, 2000);
                        let _ = tx_err.send(format!("[stderr] {}", text));
                    }
                }
                acc
            });

            let status = if effective_timeout_secs == 0 {
                child.wait().await?
            } else {
                match timeout(Duration::from_secs(effective_timeout_secs), child.wait()).await {
                    Ok(res) => res?,
                    Err(_) => {
                        if let Some(pid) = pid {
                            let _ = Command::new("taskkill")
                                .arg("/PID")
                                .arg(pid.to_string())
                                .arg("/T")
                                .arg("/F")
                                .output()
                                .await;
                        }
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = stdout_handle.await;
                        let _ = stderr_handle.await;
                        return Ok(ToolResult {
                            success: false,
                            output: None,
                            error: Some(format!("命令超时：{}s", effective_timeout_secs)),
                            data: None,
                        });
                    }
                }
            };

            let stdout = stdout_handle.await.unwrap_or_default();
            let stderr = stderr_handle.await.unwrap_or_default();
            std::process::Output {
                status,
                stdout,
                stderr,
            }
        };

        let mut stdout = decode_cmd_output(&output.stdout);
        let mut stderr = decode_cmd_output(&output.stderr);
        stdout = stdout.replace("\r\n", "\n");
        stderr = stderr.replace("\r\n", "\n");

        if output.status.success() {
            let full_output = if !stderr.is_empty() {
                format!("{}\n{}", stdout, stderr)
            } else {
                stdout
            };
            Ok(ToolResult {
                success: true,
                output: Some(full_output.trim().to_string()),
                error: None,
                data: None,
            })
        } else {
            let err_text = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Command failed: {}", err_text)),
                data: None,
            })
        }
    }

    pub fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
}

fn truncate_for_output_streaming(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

struct BashToolInvocation {
    tool: BashTool,
    command: String,
    timeout_secs: Option<u64>,
}

impl ToolInvocation for BashToolInvocation {
    fn get_description(&self) -> String {
        format!("Bash: {}", self.command)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: std::path::PathBuf::from("."),
            location_type: LocationType::Execute,
        }]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::core::tools::tools::ToolResult,
                        Box<dyn std::error::Error>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let mut tool = self.tool.clone();
        let command = self.command.clone();
        let timeout = self.timeout_secs;

        Box::pin(async move {
            let result = tool.execute(&command, timeout).await;

            let result = match result {
                Ok(r) => r,
                Err(e) => return Err(e as Box<dyn std::error::Error>),
            };

            if result.success {
                let out = result.output.unwrap_or_default();
                Ok(crate::core::tools::tools::ToolResult {
                    llm_content: Some(out.clone()),
                    return_display: Some(out.clone()),
                    output: out,
                    error: None,
                    data: None,
                })
            } else {
                let err_msg = result.error.unwrap_or_else(|| "Unknown error".to_string());
                Ok(crate::core::tools::tools::ToolResult {
                    llm_content: Some(format!("Error: {}", err_msg)),
                    return_display: Some(format!("Error: {}", err_msg)),
                    output: String::new(),
                    error: Some(crate::core::tools::tools::ToolError {
                        error_type: "execution_error".to_string(),
                        message: err_msg,
                    }),
                    data: None,
                })
            }
        })
    }
}

impl BaseDeclarativeTool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn display_name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in seconds (optional)"
                }
            },
            "required": ["command"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("Missing command parameter")?
            .to_string();

        let timeout_secs = params.get("timeout").and_then(|v| v.as_u64());

        Ok(Box::new(BashToolInvocation {
            tool: self.clone(),
            command,
            timeout_secs,
        }))
    }
}
