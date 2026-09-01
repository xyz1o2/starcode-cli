use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

pub const SCROLLBACK_LIMIT: usize = 300_000;

#[derive(Debug, Clone)]
pub struct ShellExecutionResult {
    pub raw_output: Vec<u8>,
    pub output: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub error: Option<String>,
    pub aborted: bool,
    pub pid: Option<u32>,
    pub execution_method: ExecutionMethod,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMethod {
    LydellNodePty,
    NodePty,
    ChildProcess,
    None,
}

#[derive(Debug)]
pub struct ShellExecutionHandle {
    pub pid: Option<u32>,
    pub result: tokio::task::JoinHandle<ShellExecutionResult>,
}

#[derive(Debug, Clone)]
pub struct ShellExecutionConfig {
    pub terminal_width: Option<u16>,
    pub terminal_height: Option<u16>,
    pub pager: Option<String>,
    pub show_color: Option<bool>,
    pub default_fg: Option<String>,
    pub default_bg: Option<String>,
    pub sanitization_config: EnvironmentSanitizationConfig,
    pub disable_dynamic_line_trimming: Option<bool>,
    pub scrollback: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct EnvironmentSanitizationConfig {
    pub allowed_environment_variables: Vec<String>,
    pub blocked_environment_variables: Vec<String>,
    pub enable_environment_variable_redaction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ShellOutputEvent {
    Data { chunk: String },
    BinaryDetected,
    BinaryProgress { bytes_received: usize },
}

pub struct ShellExecutionService;

impl ShellExecutionService {
    pub async fn execute(
        command: &str,
        cwd: &str,
        _on_output_event: impl Fn(ShellOutputEvent) + Send + Sync,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
        _config: ShellExecutionConfig,
    ) -> Result<ShellExecutionHandle, Box<dyn std::error::Error>> {
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("powershell.exe");
            c.arg("-NoProfile").arg("-Command").arg(command);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(command);
            c
        };

        cmd.current_dir(cwd);
        cmd.env("PAGER", "cat");
        cmd.env("GIT_PAGER", "cat");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("CI", "1");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id();
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

        let timeout_secs = std::env::var("STAR_DIRECT_SHELL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                std::env::var("STAR_SHELL_TIMEOUT")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .unwrap_or(120);

        let status = if timeout_secs == 0 {
            child.wait().await?
        } else {
            match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
                Ok(res) => res?,
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let stdout = stdout_handle.await.unwrap_or_default();
                    let stderr = stderr_handle.await.unwrap_or_default();
                    let output = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr)
                    );
                    let result = ShellExecutionResult {
                        raw_output: stdout,
                        output,
                        exit_code: None,
                        signal: None,
                        error: Some(format!("Command timed out after {}s", timeout_secs)),
                        aborted: true,
                        pid,
                        execution_method: ExecutionMethod::ChildProcess,
                    };

                    let handle = ShellExecutionHandle {
                        pid,
                        result: tokio::task::spawn(async move { result }),
                    };
                    return Ok(handle);
                }
            }
        };

        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();

        let stdout = if cfg!(windows) {
            let (cow, _, had_errors) = encoding_rs::GB18030.decode(&stdout_bytes);
            if had_errors {
                String::from_utf8_lossy(&stdout_bytes).to_string()
            } else {
                cow.to_string()
            }
        } else {
            String::from_utf8_lossy(&stdout_bytes).to_string()
        };

        let stderr = if cfg!(windows) {
            let (cow, _, had_errors) = encoding_rs::GB18030.decode(&stderr_bytes);
            if had_errors {
                String::from_utf8_lossy(&stderr_bytes).to_string()
            } else {
                cow.to_string()
            }
        } else {
            String::from_utf8_lossy(&stderr_bytes).to_string()
        };

        let result = ShellExecutionResult {
            raw_output: stdout_bytes,
            output: format!("{}\n{}", stdout, stderr),
            exit_code: status.code(),
            signal: None,
            error: if status.success() {
                None
            } else {
                Some(format!("Exit code {:?}: {}", status.code(), stderr.trim()))
            },
            aborted: false,
            pid,
            execution_method: ExecutionMethod::ChildProcess,
        };

        let handle = ShellExecutionHandle {
            pid,
            result: tokio::task::spawn(async move { result }),
        };

        Ok(handle)
    }
}
