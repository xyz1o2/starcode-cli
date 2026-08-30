use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

// ── CronCreateTool ──────────────────────────────────────────────

#[derive(Clone)]
pub struct CronCreateTool {
    config: Arc<crate::core::config::Config>,
}

impl CronCreateTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CronCreateParams {
    pub name: String,
    pub prompt: String,
    pub interval_minutes: u64,
}

pub struct CronCreateInvocation {
    config: Arc<crate::core::config::Config>,
    params: CronCreateParams,
}

impl ToolInvocation for CronCreateInvocation {
    fn get_description(&self) -> String {
        format!(
            "Create cron task '{}' (every {} min)",
            self.params.name, self.params.interval_minutes
        )
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
            match crate::core::loops::add_task(
                &config.project_root(),
                params.name.clone(),
                params.interval_minutes,
                params.prompt.clone(),
            )
            .await
            {
                Ok(task) => Ok(ToolResult {
                    llm_content: Some(format!(
                        "Loop task '{}' created: every {} min, next run at {}",
                        task.name, task.interval_minutes, task.next_run_at
                    )),
                    return_display: Some(format!("Loop task '{}' created", task.name)),
                    output: serde_json::to_string_pretty(&task).unwrap_or_else(|_| {
                        format!("Loop task '{}' created", task.name)
                    }),
                    error: None,
                    data: Some(serde_json::to_value(&task).unwrap_or_default()),
                }),
                Err(e) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "cron_error".to_string(),
                        message: e,
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for CronCreateTool {
    fn name(&self) -> &str {
        "cron_create"
    }

    fn display_name(&self) -> &str {
        "Cron Create"
    }

    fn description(&self) -> &str {
        "创建定时循环任务，按指定间隔重复执行提示。(Create a recurring loop task that executes a prompt at the specified interval.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "任务名称 (Task name)"
                },
                "prompt": {
                    "type": "string",
                    "description": "要执行的提示内容 (Prompt to execute)"
                },
                "interval_minutes": {
                    "type": "integer",
                    "description": "执行间隔分钟数 (Interval in minutes)"
                }
            },
            "required": ["name", "prompt", "interval_minutes"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: CronCreateParams = serde_json::from_value(params)?;
        Ok(Box::new(CronCreateInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}

// ── CronListTool ────────────────────────────────────────────────

#[derive(Clone)]
pub struct CronListTool {
    config: Arc<crate::core::config::Config>,
}

impl CronListTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CronListParams {}

pub struct CronListInvocation {
    config: Arc<crate::core::config::Config>,
}

impl ToolInvocation for CronListInvocation {
    fn get_description(&self) -> String {
        "List all cron/loop tasks".to_string()
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
        Box::pin(async move {
            match crate::core::loops::list_tasks(&config.project_root()).await {
                Ok(tasks) => {
                    if tasks.is_empty() {
                        Ok(ToolResult {
                            llm_content: Some("(no loop tasks)".to_string()),
                            return_display: Some("No loop tasks".to_string()),
                            output: "(no loop tasks)".to_string(),
                            error: None,
                            data: Some(serde_json::json!([])),
                        })
                    } else {
                        let text = tasks
                            .iter()
                            .map(|t| {
                                format!(
                                    "- **{}**: every {} min, enabled={}, next_run={}",
                                    t.name,
                                    t.interval_minutes,
                                    t.enabled,
                                    t.next_run_at
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult {
                            llm_content: Some(text.clone()),
                            return_display: Some(format!("{} loop tasks", tasks.len())),
                            output: text,
                            error: None,
                            data: Some(serde_json::to_value(&tasks).unwrap_or_default()),
                        })
                    }
                }
                Err(e) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "cron_error".to_string(),
                        message: e,
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for CronListTool {
    fn name(&self) -> &str {
        "cron_list"
    }

    fn display_name(&self) -> &str {
        "Cron List"
    }

    fn description(&self) -> &str {
        "列出所有已创建的定时循环任务。(List all scheduled loop tasks.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn create_invocation(
        &self,
        _params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(CronListInvocation {
            config: self.config.clone(),
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

// ── CronDeleteTool ──────────────────────────────────────────────

#[derive(Clone)]
pub struct CronDeleteTool {
    config: Arc<crate::core::config::Config>,
}

impl CronDeleteTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CronDeleteParams {
    pub name: String,
}

pub struct CronDeleteInvocation {
    config: Arc<crate::core::config::Config>,
    params: CronDeleteParams,
}

impl ToolInvocation for CronDeleteInvocation {
    fn get_description(&self) -> String {
        format!("Delete cron task '{}'", self.params.name)
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
        let name = self.params.name.clone();
        Box::pin(async move {
            match crate::core::loops::remove_task(&config.project_root(), &name).await {
                Ok(true) => Ok(ToolResult {
                    llm_content: Some(format!("Loop task '{}' deleted.", name)),
                    return_display: Some(format!("Loop task '{}' deleted", name)),
                    output: format!("Loop task '{}' deleted.", name),
                    error: None,
                    data: None,
                }),
                Ok(false) => Ok(ToolResult {
                    llm_content: Some(format!("Loop task '{}' not found.", name)),
                    return_display: Some(format!("Loop task '{}' not found", name)),
                    output: format!("Loop task '{}' not found.", name),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: format!("Loop task '{}' does not exist", name),
                    }),
                    data: None,
                }),
                Err(e) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "cron_error".to_string(),
                        message: e,
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for CronDeleteTool {
    fn name(&self) -> &str {
        "cron_delete"
    }

    fn display_name(&self) -> &str {
        "Cron Delete"
    }

    fn description(&self) -> &str {
        "删除已创建的定时循环任务。(Delete a scheduled loop task by name.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "要删除的任务名称 (Task name to delete)"
                }
            },
            "required": ["name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: CronDeleteParams = serde_json::from_value(params)?;
        Ok(Box::new(CronDeleteInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
