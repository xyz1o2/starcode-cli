use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct BackgroundTaskTool {
    config: Arc<crate::core::config::Config>,
}

impl BackgroundTaskTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BackgroundTaskParams {
    pub task_description: String,
    pub task_prompt: String,
    /// If true, mark task as one-shot and skip re-execution after completion
    #[serde(default)]
    pub once: Option<bool>,
}

pub struct BackgroundTaskInvocation {
    config: Arc<crate::core::config::Config>,
    params: BackgroundTaskParams,
}

impl ToolInvocation for BackgroundTaskInvocation {
    fn get_description(&self) -> String {
        format!("Background task: {}", self.params.task_description)
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
            // Check if this is a one-shot task that was already completed
            if params.once.unwrap_or(false) {
                let completion_file = config.project_root()
                    .join(".star")
                    .join("completed_tasks")
                    .join(format!("{}.done", sanitize_filename(&params.task_description)));
                if completion_file.exists() {
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Task '{}' was already completed. Skipping re-execution.",
                            params.task_description
                        )),
                        return_display: Some("Task already completed".to_string()),
                        output: format!(
                            "Task '{}' already completed. Use once=false to force re-execution.",
                            params.task_description
                        ),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "skipped",
                            "reason": "already_completed",
                            "description": params.task_description,
                        })),
                    });
                }
            }

            // Queue via remote infrastructure; background poller picks it up
            let message = format!(
                "[BackgroundTask: {}]\n{}",
                params.task_description, params.task_prompt
            );
            match crate::core::remote::queue_message(
                &config.project_root(),
                message,
                Some("background_task".to_string()),
            )
            .await
            {
                Ok(()) => {
                    // If one-shot, create completion marker after successful queue
                    if params.once.unwrap_or(false) {
                        let completion_dir = config.project_root()
                            .join(".star")
                            .join("completed_tasks");
                        let _ = tokio::fs::create_dir_all(&completion_dir).await;
                        let completion_file = completion_dir
                            .join(format!("{}.done", sanitize_filename(&params.task_description)));
                        let _ = tokio::fs::write(&completion_file, chrono::Utc::now().to_string()).await;
                    }

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Background task '{}' queued for execution.",
                            params.task_description
                        )),
                        return_display: Some("Background task submitted".to_string()),
                        output: format!(
                            "Task '{}' queued for background execution.",
                            params.task_description
                        ),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "queued",
                            "description": params.task_description,
                            "once": params.once.unwrap_or(false),
                        })),
                    })
                }
                Err(e) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "background_task_error".to_string(),
                        message: e,
                    }),
                    data: None,
                }),
            }
        })
    }
}

/// Sanitize a string for use as a filename
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .chars()
        .take(50)
        .collect()
}

impl BaseDeclarativeTool for BackgroundTaskTool {
    fn name(&self) -> &str {
        "background_task"
    }

    fn display_name(&self) -> &str {
        "Background Task"
    }

    fn description(&self) -> &str {
        "提交后台任务，任务将在后台轮询循环中执行。(Submit a background task that will be executed in the background polling loop.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_description": {
                    "type": "string",
                    "description": "任务简述 (Short description of the task)"
                },
                "task_prompt": {
                    "type": "string",
                    "description": "任务提示内容 (Prompt content for the task execution)"
                },
                "once": {
                    "type": "boolean",
                    "description": "If true, skip re-execution after first completion. Default: false."
                }
            },
            "required": ["task_description", "task_prompt"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: BackgroundTaskParams = serde_json::from_value(params)?;
        Ok(Box::new(BackgroundTaskInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
