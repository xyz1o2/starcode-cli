use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkflowTool {
    config: Arc<crate::core::config::Config>,
}

impl WorkflowTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkflowParams {
    pub workflow: String,
    #[serde(default)]
    pub params: Option<HashMap<String, serde_json::Value>>,
}

pub struct WorkflowInvocation {
    config: Arc<crate::core::config::Config>,
    params: WorkflowParams,
}

impl ToolInvocation for WorkflowInvocation {
    fn get_description(&self) -> String {
        format!("Execute workflow: {}", self.params.workflow)
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
        let config = self.config.clone();
        let params = self.params.clone();
        Box::pin(async move {
            let workflows_dir = config.project_root().join(".star").join("workflows");
            let workflow_file = workflows_dir.join(format!("{}.json", params.workflow));

            if !workflow_file.exists() {
                let script_file = workflows_dir.join(format!("{}.sh", params.workflow));
                if script_file.exists() {
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg(&script_file);

                    if let Some(ref workflow_params) = params.params {
                        for (key, value) in workflow_params {
                            let val_str = match value {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            cmd.env(format!("WF_{}", key.to_uppercase()), val_str);
                        }
                    }

                    let output = cmd
                        .output()
                        .await
                        .map_err(|e| format!("Failed to execute workflow script: {}", e))?;

                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    if output.status.success() {
                        return Ok(ToolResult {
                            llm_content: Some(stdout.to_string()),
                            return_display: Some(format!(
                                "Workflow '{}' completed",
                                params.workflow
                            )),
                            output: stdout.to_string(),
                            error: None,
                            data: None,
                        });
                    } else {
                        return Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "workflow_error".to_string(),
                                message: format!(
                                    "Workflow failed (exit {}):\n{}",
                                    output.status.code().unwrap_or(-1),
                                    if stderr.is_empty() { &stdout } else { &stderr }
                                ),
                            }),
                            data: None,
                        });
                    }
                }

                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: format!("Workflow '{}' not found", params.workflow),
                    }),
                    data: None,
                });
            }

            let content = tokio::fs::read_to_string(&workflow_file)
                .await
                .map_err(|e| format!("Failed to read workflow: {}", e))?;

            let workflow_def: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse workflow JSON: {}", e))?;

            let steps = workflow_def
                .get("steps")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut results = Vec::new();
            for (i, step) in steps.iter().enumerate() {
                let default_name = format!("step_{}", i);
                let step_name = step
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_name);
                let step_cmd = step
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("echo 'no command'");

                let mut cmd = tokio::process::Command::new("bash");
                cmd.args(["-c", step_cmd]);

                if let Some(ref workflow_params) = params.params {
                    for (key, value) in workflow_params {
                        let val_str = match value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        cmd.env(format!("WF_{}", key.to_uppercase()), val_str);
                    }
                }

                let output = cmd
                    .output()
                    .await
                    .map_err(|e| format!("Step '{}' failed: {}", step_name, e))?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                results.push(format!(
                    "[{}] {}: {}",
                    step_name,
                    if output.status.success() {
                        "OK"
                    } else {
                        "FAIL"
                    },
                    stdout.trim()
                ));
            }

            Ok(ToolResult {
                llm_content: Some(results.join("\n")),
                return_display: Some(format!("Workflow '{}' executed", params.workflow)),
                output: results.join("\n"),
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for WorkflowTool {
    fn name(&self) -> &str {
        "workflow"
    }

    fn display_name(&self) -> &str {
        "Workflow"
    }

    fn description(&self) -> &str {
        "执行预定义的工作流脚本。工作流定义在 .star/workflows/ 目录中。(Execute a predefined workflow script. Workflows are defined in .star/workflows/ directory.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "workflow": {
                    "type": "string",
                    "description": "工作流名称 (Workflow name)"
                },
                "params": {
                    "type": "object",
                    "description": "工作流参数 (Workflow parameters as key-value pairs)"
                }
            },
            "required": ["workflow"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WorkflowParams = serde_json::from_value(params)?;
        Ok(Box::new(WorkflowInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
