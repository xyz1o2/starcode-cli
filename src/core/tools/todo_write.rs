/// TodoWrite tool — simplified task management matching Claude Code's approach.
///
/// Claude Code uses a simple TodoWrite tool with:
/// - `content`: task description
/// - `status`: pending/in_progress/completed
/// - `activeForm`: what's currently happening
///
/// This tool provides the same interface, backed by the existing TaskManager.
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct TodoWriteTool {
    config: Arc<crate::core::config::Config>,
}

impl TodoWriteTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub active_form: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TodoWriteParams {
    pub todos: Vec<TodoItem>,
}

pub struct TodoWriteInvocation {
    params: TodoWriteParams,
}

impl ToolInvocation for TodoWriteInvocation {
    fn get_description(&self) -> String {
        format!("TodoWrite: {} items", self.params.todos.len())
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
        let todos = self.params.todos.clone();
        Box::pin(async move {
            // Update the task manager with the new todo list
            let workspace =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let path =
                crate::core::tasks::manager::TaskManager::task_file_for_workspace(&workspace);
            let mut manager = crate::core::tasks::manager::TaskManager::load_from_file(&path)
                .unwrap_or_else(|_| crate::core::tasks::manager::TaskManager::new());

            // Clear existing tasks and replace with new ones
            manager.graph.nodes.clear();

            for (i, todo) in todos.iter().enumerate() {
                let status = match todo.status.as_str() {
                    "completed" => crate::core::tasks::models::TaskStatus::Completed,
                    "in_progress" => crate::core::tasks::models::TaskStatus::InProgress,
                    _ => crate::core::tasks::models::TaskStatus::Pending,
                };

                let description = if todo.active_form.is_empty() {
                    None
                } else {
                    Some(todo.active_form.clone())
                };

                let node = crate::core::tasks::models::TaskNode {
                    id: format!("todo_{}", i),
                    title: todo.content.clone(),
                    description,
                    status,
                    priority: crate::core::tasks::models::TaskPriority::Medium,
                    parent_id: None,
                    children: Vec::new(),
                    dependencies: Vec::new(),
                    assigned_agent: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                manager.graph.nodes.insert(node.id.clone(), node);
            }

            let _ = manager.save_to_file(&path);

            // Format output
            let mut output = String::from("Todo list updated:\n");
            for todo in &todos {
                let icon = match todo.status.as_str() {
                    "completed" => "✅",
                    "in_progress" => "🔄",
                    _ => "⬜",
                };
                output.push_str(&format!("{} {}\n", icon, todo.content));
            }

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(output),
                output: String::new(),
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn display_name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        r#"Use this tool to create and manage a todo list for tracking progress on multi-step tasks.

When to use:
- Complex tasks with 3+ steps
- Tasks that require tracking progress
- When the user asks you to "track" or "organize" work

Usage:
- Call this tool with a list of todo items
- Each item has: content (task description), status (pending/in_progress/completed), activeForm (what's happening now)
- Update the entire list each time (not incremental)
- Mark items as completed as soon as they are done

Example:
{
  "todos": [
    {"content": "Read the codebase", "status": "completed", "activeForm": "Reading files"},
    {"content": "Implement the feature", "status": "in_progress", "activeForm": "Writing code"},
    {"content": "Run tests", "status": "pending", "activeForm": "Running tests"}
  ]
}"#
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Task status"
                            },
                            "activeForm": {
                                "type": "string",
                                "description": "What's currently happening (shown in UI)"
                            }
                        },
                        "required": ["content", "status"]
                    },
                    "description": "List of todo items"
                }
            },
            "required": ["todos"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TodoWriteParams = serde_json::from_value(params)?;
        Ok(Box::new(TodoWriteInvocation { params }))
    }
}
