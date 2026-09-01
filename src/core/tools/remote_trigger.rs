use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct RemoteTriggerTool;

impl RemoteTriggerTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RemoteTriggerParams {
    pub action: String,
    pub trigger_id: Option<String>,
    pub body: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct RemoteTriggerOutput {
    pub status: u16,
    pub json: String,
    pub audit_id: Option<String>,
}

pub struct RemoteTriggerInvocation {
    params: RemoteTriggerParams,
}

impl ToolInvocation for RemoteTriggerInvocation {
    fn get_description(&self) -> String {
        format!("Remote trigger: {}", self.params.action)
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
            let action = params.action.clone();

            match action.as_str() {
                "list" => {
                    // In a real implementation, this would list all triggers
                    Ok(ToolResult {
                        llm_content: Some("Listed remote triggers".to_string()),
                        return_display: Some("Triggers listed".to_string()),
                        output: serde_json::to_string(&RemoteTriggerOutput {
                            status: 200,
                            json: r#"{"triggers": []}"#.to_string(),
                            audit_id: None,
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "list"
                        })),
                    })
                }
                "get" => {
                    let trigger_id = params
                        .trigger_id
                        .ok_or("trigger_id is required for get action")?;

                    // In a real implementation, this would get the trigger details
                    Ok(ToolResult {
                        llm_content: Some(format!("Got trigger '{}'", trigger_id)),
                        return_display: Some(format!("Trigger '{}' retrieved", trigger_id)),
                        output: serde_json::to_string(&RemoteTriggerOutput {
                            status: 200,
                            json: format!(r#"{{"trigger_id": "{}"}}"#, trigger_id),
                            audit_id: None,
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "get",
                            "trigger_id": trigger_id
                        })),
                    })
                }
                "create" => {
                    let body = params.body.ok_or("body is required for create action")?;

                    // In a real implementation, this would create a new trigger
                    let trigger_id = format!("trigger_{}", uuid::Uuid::new_v4());

                    Ok(ToolResult {
                        llm_content: Some(format!("Created trigger '{}'", trigger_id)),
                        return_display: Some(format!("Trigger '{}' created", trigger_id)),
                        output: serde_json::to_string(&RemoteTriggerOutput {
                            status: 201,
                            json: format!(r#"{{"trigger_id": "{}"}}"#, trigger_id),
                            audit_id: Some(format!("audit_{}", uuid::Uuid::new_v4())),
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "create",
                            "trigger_id": trigger_id,
                            "body": body
                        })),
                    })
                }
                "update" => {
                    let trigger_id = params
                        .trigger_id
                        .ok_or("trigger_id is required for update action")?;
                    let body = params.body.ok_or("body is required for update action")?;

                    // In a real implementation, this would update the trigger
                    Ok(ToolResult {
                        llm_content: Some(format!("Updated trigger '{}'", trigger_id)),
                        return_display: Some(format!("Trigger '{}' updated", trigger_id)),
                        output: serde_json::to_string(&RemoteTriggerOutput {
                            status: 200,
                            json: format!(r#"{{"trigger_id": "{}"}}"#, trigger_id),
                            audit_id: Some(format!("audit_{}", uuid::Uuid::new_v4())),
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "update",
                            "trigger_id": trigger_id,
                            "body": body
                        })),
                    })
                }
                "run" => {
                    let trigger_id = params
                        .trigger_id
                        .ok_or("trigger_id is required for run action")?;

                    // In a real implementation, this would run the trigger
                    Ok(ToolResult {
                        llm_content: Some(format!("Ran trigger '{}'", trigger_id)),
                        return_display: Some(format!("Trigger '{}' executed", trigger_id)),
                        output: serde_json::to_string(&RemoteTriggerOutput {
                            status: 200,
                            json: format!(
                                r#"{{"trigger_id": "{}", "status": "executed"}}"#,
                                trigger_id
                            ),
                            audit_id: Some(format!("audit_{}", uuid::Uuid::new_v4())),
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "run",
                            "trigger_id": trigger_id
                        })),
                    })
                }
                _ => Ok(ToolResult {
                    llm_content: Some(format!("Unknown action: {}", action)),
                    return_display: Some(format!("Unknown action: {}", action)),
                    output: serde_json::to_string(&RemoteTriggerOutput {
                        status: 400,
                        json: format!(r#"{{"error": "Unknown action: {}"}}"#, action),
                        audit_id: None,
                    })?,
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: format!(
                            "Unknown action: {}. Use 'list', 'get', 'create', 'update', or 'run'",
                            action
                        ),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for RemoteTriggerTool {
    fn name(&self) -> &str {
        "remote_trigger"
    }

    fn display_name(&self) -> &str {
        "RemoteTrigger"
    }

    fn description(&self) -> &str {
        "管理远程代理的定时触发器（列出、获取、创建、更新、运行）。(Manage remote agent triggers - list, get, create, update, run.)"
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
                    "enum": ["list", "get", "create", "update", "run"],
                    "description": "要执行的操作 (Action to perform)"
                },
                "trigger_id": {
                    "type": "string",
                    "description": "触发器ID（get/update/run操作必填）(Trigger ID, required for get/update/run actions)"
                },
                "body": {
                    "type": "object",
                    "description": "触发器数据（create/update操作必填）(Trigger data, required for create/update actions)",
                    "additionalProperties": true
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: RemoteTriggerParams = serde_json::from_value(params)?;
        Ok(Box::new(RemoteTriggerInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}
