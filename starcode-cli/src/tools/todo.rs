use crate::core::config::Config;
use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::{TaskNode, TaskPriority, TaskStatus};
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};
use crate::types::ToolResult;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoUpdate {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Clone)]
pub struct TodoTool {
    workspace: PathBuf,
}

impl TodoTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            workspace: config.working_dir().clone(),
        }
    }

    fn task_path(&self) -> PathBuf {
        TaskManager::task_file_for_workspace(&self.workspace)
    }

    fn get_task_manager(&self) -> TaskManager {
        let path = self.task_path();
        TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new())
    }

    fn save_task_manager(&self, manager: &TaskManager) -> Result<(), String> {
        let path = self.task_path();
        manager.save_to_file(&path)
    }

    fn format_todo_list(&self) -> String {
        let manager = self.get_task_manager();
        let mut output = String::new();

        if manager.graph.nodes.is_empty() {
            return "No tasks found.".to_string();
        }

        fn recursive_render(manager: &TaskManager, id: &str, depth: usize, out: &mut String) {
            if let Some(node) = manager.graph.nodes.get(id) {
                let checkbox = match node.status {
                    TaskStatus::Completed => "●",
                    TaskStatus::InProgress => "◐",
                    TaskStatus::Pending => "○",
                    TaskStatus::Blocked => "✖",
                    TaskStatus::Skipped => "-",
                };

                let status_str = match node.status {
                    TaskStatus::Completed => "[Done]",
                    TaskStatus::InProgress => "[In Progress]",
                    TaskStatus::Pending => "[Pending]",
                    TaskStatus::Blocked => "[Blocked]",
                    TaskStatus::Skipped => "[Skipped]",
                };

                let indent = "  ".repeat(depth);
                out.push_str(&format!(
                    "{}{} {} (ID: {}) - {}\n",
                    indent, checkbox, node.title, node.id, status_str
                ));

                for child_id in &node.children {
                    recursive_render(manager, child_id, depth + 1, out);
                }
            }
        }

        for root_id in &manager.graph.root_ids {
            recursive_render(&manager, root_id, 0, &mut output);
        }

        output
    }

    pub fn create_todo_list(
        &mut self,
        todos: Vec<TodoItem>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut manager = self.get_task_manager();
        let mut created_count = 0;

        for todo in todos {
            let status = match todo.status.as_str() {
                "completed" => TaskStatus::Completed,
                "in_progress" => TaskStatus::InProgress,
                _ => TaskStatus::Pending,
            };

            let priority = match todo.priority.as_str() {
                "high" => TaskPriority::High,
                "low" => TaskPriority::Low,
                _ => TaskPriority::Medium,
            };

            let mut task = TaskNode::new(todo.content);
            if !todo.id.is_empty() {
                task.id = todo.id;
            }
            task.status = status;
            task.priority = priority;

            match manager.add_task_dedup(task) {
                Ok(crate::core::tasks::manager::AddTaskOutcome::Added(_)) => {
                    created_count += 1;
                }
                Ok(crate::core::tasks::manager::AddTaskOutcome::Existing(_)) => {}
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(format!("Failed to add task: {}", e)),
                        data: None,
                    });
                }
            }
        }

        if let Err(e) = self.save_task_manager(&manager) {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Failed to save tasks: {}", e)),
                data: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: Some(format!("Successfully created {} tasks.", created_count)),
            error: None,
            data: None,
        })
    }

    pub fn update_todo_list(
        &mut self,
        updates: Vec<TodoUpdate>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut manager = self.get_task_manager();
        let mut updated_count = 0;

        for update in updates {
            if let Some(mut task) = manager.graph.nodes.get(&update.id).cloned() {
                if let Some(status_str) = update.status {
                    task.status = match status_str.as_str() {
                        "completed" => TaskStatus::Completed,
                        "in_progress" => TaskStatus::InProgress,
                        "pending" => TaskStatus::Pending,
                        _ => task.status,
                    };
                }

                if let Some(content) = update.content {
                    task.title = content;
                }

                if let Some(priority_str) = update.priority {
                    task.priority = match priority_str.as_str() {
                        "high" => TaskPriority::High,
                        "low" => TaskPriority::Low,
                        "medium" => TaskPriority::Medium,
                        _ => task.priority,
                    };
                }

                if let Err(e) = manager.update_task(task) {
                    return Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(format!("Failed to update task {}: {}", update.id, e)),
                        data: None,
                    });
                }
                updated_count += 1;
            } else {
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!("Task with ID {} not found", update.id)),
                    data: None,
                });
            }
        }

        if let Err(e) = self.save_task_manager(&manager) {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Failed to save tasks: {}", e)),
                data: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: Some(format!("Successfully updated {} tasks.", updated_count)),
            error: None,
            data: None,
        })
    }

    pub fn delete_todo_list(
        &mut self,
        delete_ids: Vec<String>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut manager = self.get_task_manager();
        let mut deleted_count = 0;

        for id in delete_ids {
            if let Ok(_) = manager.delete_task(&id) {
                deleted_count += 1;
            }
        }

        if deleted_count > 0 {
            if let Err(e) = self.save_task_manager(&manager) {
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!("Failed to save tasks: {}", e)),
                    data: None,
                });
            }
        }

        Ok(ToolResult {
            success: true,
            output: Some(format!("Successfully deleted {} tasks.", deleted_count)),
            error: None,
            data: None,
        })
    }

    pub fn move_todo_item(
        &mut self,
        id: String,
        new_parent_id: Option<String>,
        after_id: Option<String>,
    ) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut manager = self.get_task_manager();

        if let Err(e) = manager.move_task(&id, new_parent_id, after_id) {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Failed to move task: {}", e)),
                data: None,
            });
        }

        if let Err(e) = self.save_task_manager(&manager) {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Failed to save tasks: {}", e)),
                data: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: Some(format!("Successfully moved task {}", id)),
            error: None,
            data: None,
        })
    }

    pub fn view_todo_list(&self) -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ToolResult {
            success: true,
            output: Some(self.format_todo_list()),
            error: None,
            data: None,
        })
    }
}

impl BaseDeclarativeTool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn display_name(&self) -> &str {
        "Todo List"
    }

    fn description(&self) -> &str {
        "Manage a todo list (create, update, view)"
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
                    "enum": ["create", "update", "view", "delete"],
                    "description": "Action to perform"
                },
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                            "priority": { "type": "string", "enum": ["high", "medium", "low"] }
                        },
                        "required": ["id", "content", "status", "priority"]
                    },
                    "description": "List of todos for 'create' action"
                },
                "updates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "status": { "type": "string" },
                            "content": { "type": "string" },
                            "priority": { "type": "string" }
                        },
                        "required": ["id"]
                    },
                    "description": "List of updates for 'update' action"
                },
                "delete_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of todo IDs to delete"
                },
                "move_params": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "new_parent_id": { "type": "string", "nullable": true },
                        "after_id": { "type": "string", "nullable": true }
                    },
                    "required": ["id"],
                    "description": "Parameters for 'move' action"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let action = params["action"].as_str().unwrap_or("view").to_string();
        Ok(Box::new(TodoToolInvocation {
            tool: self.clone(),
            action,
            params,
        }))
    }
}

struct TodoToolInvocation {
    tool: TodoTool,
    action: String,
    params: serde_json::Value,
}

impl ToolInvocation for TodoToolInvocation {
    fn get_description(&self) -> String {
        format!("Todo List Action: {}", self.action)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Pin<
        Box<
            dyn Future<
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
    ) -> Pin<Box<dyn Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let mut tool = self.tool.clone();
        let params = self.params.clone();
        let action = self.action.clone();

        Box::pin(async move {
            let result = tokio::task::spawn_blocking(move || match action.as_str() {
                "create" => {
                    let todos: Vec<TodoItem> = serde_json::from_value(params["todos"].clone())
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    tool.create_todo_list(todos)
                }
                "update" => {
                    let updates: Vec<TodoUpdate> =
                        serde_json::from_value(params["updates"].clone())
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    tool.update_todo_list(updates)
                }
                "delete" => {
                    let delete_ids: Vec<String> =
                        serde_json::from_value(params["delete_ids"].clone())
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    tool.delete_todo_list(delete_ids)
                }
                "move" => {
                    let move_params = params["move_params"].as_object().ok_or_else(|| {
                        Box::<dyn std::error::Error + Send + Sync>::from("Missing move_params")
                    })?;

                    let id = move_params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            Box::<dyn std::error::Error + Send + Sync>::from(
                                "Missing id in move_params",
                            )
                        })?
                        .to_string();

                    let new_parent_id = move_params
                        .get("new_parent_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let after_id = move_params
                        .get("after_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    tool.move_todo_item(id, new_parent_id, after_id)
                }
                "view" => tool.view_todo_list(),
                _ => {
                    return Err(Box::<dyn std::error::Error + Send + Sync>::from(format!(
                        "Unknown action: {}",
                        action
                    )))
                }
            })
            .await;

            let result = match result {
                Ok(res) => res.map_err(|e| e as Box<dyn std::error::Error>)?,
                Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
            };

            Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: result.output.unwrap_or_default(),
                error: result.error.map(|e| ToolError {
                    error_type: "execution_error".to_string(),
                    message: e,
                }),
                data: None,
            })
        })
    }
}
