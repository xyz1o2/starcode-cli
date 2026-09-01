use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;

#[derive(Clone)]
pub struct MonitorTool;

impl MonitorTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MonitorParams {
    pub action: String,
    #[serde(default)]
    pub pid: Option<u32>,
}

pub struct MonitorInvocation {
    params: MonitorParams,
}

impl ToolInvocation for MonitorInvocation {
    fn get_description(&self) -> String {
        match self.params.action.as_str() {
            "list" => "List running processes".to_string(),
            "status" => format!(
                "Check process status (PID: {})",
                self.params.pid.unwrap_or(0)
            ),
            "kill" => format!(
                "Kill process (PID: {})",
                self.params.pid.unwrap_or(0)
            ),
            _ => format!("Monitor action: {}", self.params.action),
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            match params.action.as_str() {
                "list" => {
                    let output = tokio::process::Command::new("ps")
                        .args(["aux", "--sort=-pid"])
                        .output()
                        .await
                        .map_err(|e| format!("Failed to list processes: {}", e))?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let lines: Vec<&str> = stdout.lines().take(30).collect();
                    let result = lines.join("\n");

                    Ok(ToolResult {
                        llm_content: Some(result.clone()),
                        return_display: Some("Process list".to_string()),
                        output: result,
                        error: None,
                        data: None,
                    })
                }
                "status" => {
                    let pid = match params.pid {
                        Some(p) => p,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "pid is required for status action".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };

                    let output = tokio::process::Command::new("ps")
                        .args(["-p", &pid.to_string(), "-o", "pid,stat,etime,cmd"])
                        .output()
                        .await
                        .map_err(|e| format!("Failed to check process: {}", e))?;

                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        Ok(ToolResult {
                            llm_content: Some(stdout.to_string()),
                            return_display: Some(format!("Process {} is running", pid)),
                            output: stdout.to_string(),
                            error: None,
                            data: None,
                        })
                    } else {
                        Ok(ToolResult {
                            llm_content: Some(format!("Process {} not found", pid)),
                            return_display: Some(format!("Process {} not running", pid)),
                            output: format!("Process {} is not running", pid),
                            error: None,
                            data: None,
                        })
                    }
                }
                "kill" => {
                    let pid = match params.pid {
                        Some(p) => p,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "pid is required for kill action".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };

                    let output = tokio::process::Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .output()
                        .await
                        .map_err(|e| format!("Failed to kill process: {}", e))?;

                    if output.status.success() {
                        Ok(ToolResult {
                            llm_content: Some(format!("Process {} terminated", pid)),
                            return_display: Some(format!("Killed process {}", pid)),
                            output: format!("Sent SIGTERM to process {}", pid),
                            error: None,
                            data: None,
                        })
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "kill_error".to_string(),
                                message: format!("Failed to kill process {}: {}", pid, stderr),
                            }),
                            data: None,
                        })
                    }
                }
                _ => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "invalid_action".to_string(),
                        message: format!(
                            "Unknown action '{}'. Valid actions: list, status, kill",
                            params.action
                        ),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for MonitorTool {
    fn name(&self) -> &str {
        "monitor"
    }

    fn display_name(&self) -> &str {
        "Process Monitor"
    }

    fn description(&self) -> &str {
        "监控代理启动的进程：列出、查看状态或终止。(Monitor processes started by the agent: list, check status, or kill.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "status", "kill"],
                    "description": "操作类型 (Action: list, status, or kill)"
                },
                "pid": {
                    "type": "integer",
                    "description": "进程ID，status/kill 时必需 (Process ID, required for status/kill)"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: MonitorParams = serde_json::from_value(params)?;
        Ok(Box::new(MonitorInvocation { params }))
    }
}
