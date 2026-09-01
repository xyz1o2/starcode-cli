use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, BufReader};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerShellToolParams {
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Clone)]
pub struct PowerShellTool;

impl PowerShellTool {
    pub fn new() -> Self {
        Self
    }
}

pub struct PowerShellToolInvocation {
    params: PowerShellToolParams,
}

impl PowerShellToolInvocation {
    pub fn new(params: PowerShellToolParams) -> Self {
        Self { params }
    }
}

impl BaseDeclarativeTool for PowerShellTool {
    fn name(&self) -> &str {
        "powershell"
    }

    fn display_name(&self) -> &str {
        "PowerShell"
    }

    fn description(&self) -> &str {
        "Execute PowerShell commands on Windows systems. Use PowerShell syntax for system administration, automation, and Windows-specific tasks."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "PowerShell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30)",
                    "default": 30
                }
            },
            "required": ["command"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: PowerShellToolParams = serde_json::from_value(params)?;
        Ok(Box::new(PowerShellToolInvocation::new(params)))
    }
}

impl ToolInvocation for PowerShellToolInvocation {
    fn get_description(&self) -> String {
        format!("PowerShell: {}", self.params.command)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let command = self.params.command.clone();
        Box::pin(async move {
            let dangerous_patterns = [
                "Remove-Item -Recurse -Force C:\\",
                "Format-Volume",
                "Clear-Disk",
                "Remove-Partition",
            ];

            for pattern in dangerous_patterns {
                if command.contains(pattern) {
                    return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                        confirmation_type: crate::core::tools::tools::ConfirmationType::Danger,
                        title: "Dangerous PowerShell Command".to_string(),
                        prompt: format!(
                            "The command '{}' contains a dangerous pattern '{}'. Proceed with caution.",
                            command, pattern
                        ),
                        on_confirm: std::sync::Arc::new(|_| {}),
                    }));
                }
            }

            Ok(None)
        })
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        let signal = signal.cloned();

        Box::pin(async move {
            if let Some(signal) = &signal {
                if signal.is_cancelled() {
                    return Ok(ToolResult {
                        llm_content: Some("Command was cancelled by user.".to_string()),
                        return_display: Some("Command cancelled.".to_string()),
                        output: "Command cancelled by user.".to_string(),
                        error: None,
                        data: None,
                    });
                }
            }

            let timeout = Duration::from_secs(params.timeout);

            let mut cmd = tokio::process::Command::new("powershell.exe");
            cmd.arg("-NoProfile")
                .arg("-Command")
                .arg(format!(
                    "$OutputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {}",
                    params.command
                ))
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(e) => {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: String::new(),
                        error: Some(crate::core::tools::tools::ToolError {
                            error_type: "execution_error".to_string(),
                            message: format!("Failed to spawn PowerShell command: {}", e),
                        }),
                        data: None,
                    });
                }
            };

            let stdout = child.stdout.take().expect("Failed to open stdout");
            let stderr = child.stderr.take().expect("Failed to open stderr");

            let mut stdout_output = String::new();
            let mut stderr_output = String::new();

            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let tx_out = tx.clone();
            let tx_err = tx.clone();

            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = tx_out.send((true, s)).await;
                }
            });

            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = tx_err.send((false, s)).await;
                }
            });

            drop(tx);

            let result = tokio::time::timeout(timeout, async {
                while let Some((is_stdout, s)) = rx.recv().await {
                    if is_stdout {
                        stdout_output.push_str(&s);
                    } else {
                        stderr_output.push_str(&s);
                    }

                    if let Some(ref cb) = update_output {
                        cb(s);
                    }
                }

                child.wait().await
            })
            .await;

            match result {
                Ok(Ok(status)) => {
                    let llm_content = format!(
                        "PowerShell Command: {}\nOutput: {}\nError: {}\nExit Code: {}",
                        params.command,
                        if stdout_output.is_empty() {
                            "(empty)"
                        } else {
                            &stdout_output
                        },
                        if stderr_output.is_empty() {
                            "(none)"
                        } else {
                            &stderr_output
                        },
                        status.code().unwrap_or(-1)
                    );

                    Ok(ToolResult {
                        llm_content: Some(llm_content),
                        return_display: Some(if !stdout_output.is_empty() {
                            stdout_output.clone()
                        } else if !stderr_output.is_empty() {
                            format!("Command failed: {}", stderr_output)
                        } else {
                            String::new()
                        }),
                        output: stdout_output,
                        error: if !status.success() {
                            Some(crate::core::tools::tools::ToolError {
                                error_type: "execution_error".to_string(),
                                message: stderr_output,
                            })
                        } else {
                            None
                        },
                        data: None,
                    })
                }
                Ok(Err(e)) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(crate::core::tools::tools::ToolError {
                        error_type: "execution_error".to_string(),
                        message: format!("Failed to wait for PowerShell command: {}", e),
                    }),
                    data: None,
                }),
                Err(_) => {
                    let _ = child.kill().await;
                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "PowerShell command timed out after {}s: {}",
                            params.timeout, params.command
                        )),
                        return_display: Some(format!(
                            "Command timed out after {}s",
                            params.timeout
                        )),
                        output: stdout_output,
                        error: Some(crate::core::tools::tools::ToolError {
                            error_type: "timeout".to_string(),
                            message: format!("Command timed out after {}s", params.timeout),
                        }),
                        data: None,
                    })
                }
            }
        })
    }
}
