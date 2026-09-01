//! Task management extension tools — merged from task_get + task_list + task_update + task_output

use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::TaskStatus;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

// ── TaskGet ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskGetTool {
    config: Arc<crate::core::config::Config>,
}

impl TaskGetTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskGetParams {
    pub task_id: String,
}

pub struct TaskGetInvocation {
    config: Arc<crate::core::config::Config>,
    params: TaskGetParams,
}

impl ToolInvocation for TaskGetInvocation {
    fn get_description(&self) -> String {
        format!("Get task details: {}", self.params.task_id)
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
        let task_id = self.params.task_id.clone();
        Box::pin(async move {
            let workspace = config.working_dir();
            let path = TaskManager::task_file_for_workspace(workspace);

            let manager = tokio::task::spawn_blocking(move || {
                TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new())
            })
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

            match manager.get_task(&task_id) {
                Some(task) => {
                    let output = serde_json::to_string_pretty(task)
                        .unwrap_or_else(|_| format!("{:?}", task));
                    Ok(ToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(format!("Task: {}", task.title)),
                        output,
                        error: None,
                        data: Some(serde_json::to_value(task).unwrap_or_default()),
                    })
                }
                None => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: format!("Task '{}' not found", task_id),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for TaskGetTool {
    fn name(&self) -> &str {
        "task_get"
    }

    fn display_name(&self) -> &str {
        "Task Get"
    }

    fn description(&self) -> &str {
        "获取指定任务的详细信息，包括状态、描述和子任务。(Get full details of a specific task including status, description, and subtasks.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任务ID (Task ID)"
                }
            },
            "required": ["task_id"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TaskGetParams = serde_json::from_value(params)?;
        Ok(Box::new(TaskGetInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

// ── TaskList ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskListTool {
    config: Arc<crate::core::config::Config>,
}

impl TaskListTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskListParams {
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_status() -> String {
    "all".to_string()
}

fn default_limit() -> usize {
    20
}

pub struct TaskListInvocation {
    config: Arc<crate::core::config::Config>,
    params: TaskListParams,
}

impl ToolInvocation for TaskListInvocation {
    fn get_description(&self) -> String {
        format!(
            "List tasks (status: {}, limit: {})",
            self.params.status, self.params.limit
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
        let status_label = params.status.clone();
        Box::pin(async move {
            let workspace = config.working_dir();
            let path = TaskManager::task_file_for_workspace(workspace);

            let result = tokio::task::spawn_blocking(move || {
                let manager =
                    TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());

                let status_filter = match params.status.as_str() {
                    "pending" => Some(TaskStatus::Pending),
                    "in_progress" => Some(TaskStatus::InProgress),
                    "completed" => Some(TaskStatus::Completed),
                    "blocked" => Some(TaskStatus::Blocked),
                    "skipped" => Some(TaskStatus::Skipped),
                    _ => None,
                };

                let mut tasks: Vec<_> = manager
                    .graph
                    .nodes
                    .values()
                    .filter(|t| {
                        if let Some(ref status) = status_filter {
                            std::mem::discriminant(&t.status) == std::mem::discriminant(status)
                        } else {
                            true
                        }
                    })
                    .take(params.limit)
                    .collect();

                tasks.sort_by(|a, b| a.id.cmp(&b.id));

                let lines: Vec<String> = tasks
                    .iter()
                    .map(|t| {
                        let status_str = match t.status {
                            TaskStatus::Pending => "[ ]",
                            TaskStatus::InProgress => "[>]",
                            TaskStatus::Completed => "[x]",
                            TaskStatus::Blocked => "[!]",
                            TaskStatus::Skipped => "[-]",
                        };
                        format!("{} {} (ID: {})", status_str, t.title, t.id)
                    })
                    .collect();

                (lines, tasks.len())
            })
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

            let (lines, total) = result;
            let output = if lines.is_empty() {
                format!("No tasks found (filter: {})", status_label)
            } else {
                format!("Tasks ({}):\n{}", total, lines.join("\n"))
            };

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(format!("{} tasks", total)),
                output,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn display_name(&self) -> &str {
        "Task List"
    }

    fn description(&self) -> &str {
        "列出任务，可按状态过滤。(List tasks with optional status filter.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["all", "pending", "in_progress", "completed", "blocked", "skipped"],
                    "description": "状态过滤 (Status filter, default: all)"
                },
                "limit": {
                    "type": "integer",
                    "description": "最大返回数量 (Max results, default: 20)"
                }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TaskListParams = serde_json::from_value(params)?;
        Ok(Box::new(TaskListInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

// ── TaskUpdate ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskUpdateTool {
    config: Arc<crate::core::config::Config>,
}

impl TaskUpdateTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskUpdateParams {
    pub task_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub struct TaskUpdateInvocation {
    config: Arc<crate::core::config::Config>,
    params: TaskUpdateParams,
}

impl ToolInvocation for TaskUpdateInvocation {
    fn get_description(&self) -> String {
        format!("Update task: {}", self.params.task_id)
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
            let workspace = config.working_dir().clone();
            let path = TaskManager::task_file_for_workspace(&workspace);

            let result = tokio::task::spawn_blocking(move || {
                let mut manager =
                    TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());

                if manager.get_task(&params.task_id).is_some() {
                    let mut changes = Vec::new();

                    if let Some(ref status_str) = params.status {
                        let new_status = match status_str.as_str() {
                            "pending" => TaskStatus::Pending,
                            "in_progress" => TaskStatus::InProgress,
                            "completed" => TaskStatus::Completed,
                            "blocked" => TaskStatus::Blocked,
                            "skipped" => TaskStatus::Skipped,
                            _ => return Err(format!("Invalid status: {}", status_str)),
                        };
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            task.status = new_status;
                        }
                        changes.push(format!("status={}", status_str));
                    }

                    if let Some(ref notes) = params.notes {
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            let existing = task.description.clone().unwrap_or_default();
                            if existing.is_empty() {
                                task.description = Some(notes.clone());
                            } else {
                                task.description = Some(format!("{}\n{}", existing, notes));
                            }
                        }
                        changes.push("notes added".to_string());
                    }

                    let title = manager
                        .get_task(&params.task_id)
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| params.task_id.clone());

                    manager
                        .save_to_file(&path)
                        .map_err(|e| format!("Failed to save: {}", e))?;

                    Ok(format!("Updated task '{}': {}", title, changes.join(", ")))
                } else {
                    Err(format!("Task '{}' not found", params.task_id))
                }
            })
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

            match result {
                Ok(msg) => Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some("Task updated".to_string()),
                    output: msg,
                    error: None,
                    data: None,
                }),
                Err(e) => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "task_error".to_string(),
                        message: e,
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for TaskUpdateTool {
    fn name(&self) -> &str {
        "task_update"
    }

    fn display_name(&self) -> &str {
        "Task Update"
    }

    fn description(&self) -> &str {
        "更新任务状态或添加备注。(Update task status or add notes to a task.)"
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任务ID (Task ID)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "blocked", "skipped"],
                    "description": "新状态 (New status)"
                },
                "notes": {
                    "type": "string",
                    "description": "备注内容 (Notes to append)"
                }
            },
            "required": ["task_id"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TaskUpdateParams = serde_json::from_value(params)?;
        Ok(Box::new(TaskUpdateInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}

// ── TaskOutput ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskOutputTool {
    config: Arc<crate::core::config::Config>,
}

impl TaskOutputTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TaskOutputParams {
    pub task_id: String,
}

pub struct TaskOutputInvocation {
    config: Arc<crate::core::config::Config>,
    params: TaskOutputParams,
}

impl ToolInvocation for TaskOutputInvocation {
    fn get_description(&self) -> String {
        format!("Get task output: {}", self.params.task_id)
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
        let task_id = self.params.task_id.clone();
        Box::pin(async move {
            let workspace = config.working_dir();
            let output_path = workspace
                .join(".star")
                .join("task_outputs")
                .join(format!("{}.json", task_id));

            if !output_path.exists() {
                let path = TaskManager::task_file_for_workspace(workspace);
                let manager = tokio::task::spawn_blocking(move || {
                    TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new())
                })
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

                if let Some(task) = manager.get_task(&task_id) {
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Task '{}' status: {:?}. No output stored yet.",
                            task.title, task.status
                        )),
                        return_display: Some("No output available".to_string()),
                        output: format!("Task '{}' has no stored output.", task.title),
                        error: None,
                        data: None,
                    });
                } else {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: String::new(),
                        error: Some(ToolError {
                            error_type: "not_found".to_string(),
                            message: format!("Task '{}' not found", task_id),
                        }),
                        data: None,
                    });
                }
            }

            let content = tokio::fs::read_to_string(&output_path)
                .await
                .map_err(|e| format!("Failed to read output: {}", e))?;

            let data: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or_else(|_| serde_json::json!({"raw": content}));

            let output_text =
                serde_json::to_string_pretty(&data).unwrap_or_else(|_| content.clone());

            Ok(ToolResult {
                llm_content: Some(output_text.clone()),
                return_display: Some(format!("Task {} output", task_id)),
                output: output_text,
                error: None,
                data: Some(data),
            })
        })
    }
}

impl BaseDeclarativeTool for TaskOutputTool {
    fn name(&self) -> &str {
        "task_output"
    }

    fn display_name(&self) -> &str {
        "Task Output"
    }

    fn description(&self) -> &str {
        "获取已完成任务的执行输出/结果。(Get the execution output/result of a completed task.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任务ID (Task ID)"
                }
            },
            "required": ["task_id"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TaskOutputParams = serde_json::from_value(params)?;
        Ok(Box::new(TaskOutputInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
