use crate::core::tools::tools::{
    ToolError,
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct GoalTool;

impl GoalTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GoalParams {
    pub action: Option<String>,
    pub status: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct GoalOutput {
    pub success: bool,
    pub goal: Option<GoalSnapshot>,
    pub message: Option<String>,
    pub report: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct GoalSnapshot {
    pub objective: String,
    pub status: String,
    pub tokens_used: u64,
    pub token_budget: Option<u64>,
    pub elapsed: String,
    pub turns_executed: u32,
}

pub struct GoalInvocation {
    params: GoalParams,
}

impl ToolInvocation for GoalInvocation {
    fn get_description(&self) -> String {
        let action = self.params.action.clone().unwrap_or_else(|| {
            if self.params.status.is_some() {
                "update".to_string()
            } else {
                "get".to_string()
            }
        });
        match action.as_str() {
            "get" => "Get goal status".to_string(),
            "update" => format!(
                "Update goal: {}{}",
                self.params.status.as_deref().unwrap_or("unknown"),
                self.params.reason.as_deref().map(|r| format!(" — {}", r)).unwrap_or_default()
            ),
            _ => "Goal operation".to_string(),
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
            let action = params.action.unwrap_or_else(|| {
                if params.status.is_some() {
                    "update".to_string()
                } else {
                    "get".to_string()
                }
            });

            match action.as_str() {
                "get" => {
                    // In a real implementation, this would read from goal state
                    Ok(ToolResult {
                        llm_content: Some("No active goal. The user can set one with `/goal <objective>`.".to_string()),
                        return_display: Some("No active goal".to_string()),
                        output: serde_json::to_string(&GoalOutput {
                            success: true,
                            goal: None,
                            message: Some("No active goal. The user can set one with `/goal <objective>`.".to_string()),
                            report: None,
                            error: None,
                        })?,
                        error: None,
                        data: None,
                    })
                }
                "update" => {
                    let status = params.status.ok_or("status is required for update")?;
                    let reason = params.reason.unwrap_or_else(|| "unspecified".to_string());

                    match status.as_str() {
                        "complete" => {
                            // In a real implementation, this would mark goal as complete
                            Ok(ToolResult {
                                llm_content: Some("Goal marked as complete".to_string()),
                                return_display: Some("Goal completed".to_string()),
                                output: serde_json::to_string(&GoalOutput {
                                    success: true,
                                    goal: None,
                                    message: Some("Goal marked as complete".to_string()),
                                    report: Some("Goal achieved — usage report:\n  Token usage: 0\n  Active time: 0s\n  Continuation turns: 0".to_string()),
                                    error: None,
                                })?,
                                error: None,
                                data: None,
                            })
                        }
                        "blocked" => {
                            // In a real implementation, this would record blocked attempt
                            Ok(ToolResult {
                                llm_content: Some(format!("Goal marked as blocked. Reason: {}", reason)),
                                return_display: Some("Goal blocked".to_string()),
                                output: serde_json::to_string(&GoalOutput {
                                    success: true,
                                    goal: None,
                                    message: Some(format!("Goal marked as blocked. Reason: {}", reason)),
                                    report: None,
                                    error: None,
                                })?,
                                error: None,
                                data: None,
                            })
                        }
                        _ => Ok(ToolResult {
                            llm_content: Some(format!("Invalid status: {}. Use 'complete' or 'blocked'", status)),
                            return_display: Some(format!("Invalid status: {}", status)),
                            output: serde_json::to_string(&GoalOutput {
                                success: false,
                                goal: None,
                                message: None,
                                report: None,
                                error: Some(format!("Invalid status: {}. Use 'complete' or 'blocked'", status)),
                            })?,
                            error: Some(ToolError { error_type: "validation".to_string(), message: format!("Invalid status: {}", status) }),
                            data: None,
                        }),
                    }
                }
                _ => Ok(ToolResult {
                    llm_content: Some(format!("Unknown action: {}. Use 'get' or 'update'", action)),
                    return_display: Some(format!("Unknown action: {}", action)),
                    output: serde_json::to_string(&GoalOutput {
                        success: false,
                        goal: None,
                        message: None,
                        report: None,
                        error: Some(format!("Unknown action: {}. Use 'get' or 'update'", action)),
                    })?,
                    error: Some(ToolError { error_type: "validation".to_string(), message: format!("Unknown action: {}", action) }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for GoalTool {
    fn name(&self) -> &str {
        "goal"
    }

    fn display_name(&self) -> &str {
        "Goal"
    }

    fn description(&self) -> &str {
        "获取或更新当前活动目标的状态。用于跟踪任务进度和完成情况。(Get or update the status of the active goal. Used to track task progress and completion.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "update"],
                    "description": "要执行的操作：\"get\" 读取状态，\"update\" 标记完成或阻塞。默认如果有status则为update，否则为get。"
                },
                "status": {
                    "type": "string",
                    "enum": ["complete", "blocked"],
                    "description": "update操作必填。只接受 \"complete\" 或 \"blocked\"。"
                },
                "reason": {
                    "type": "string",
                    "description": "状态变更的原因说明。update操作时必填。"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GoalParams = serde_json::from_value(params)?;
        Ok(Box::new(GoalInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}