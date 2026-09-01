use crate::core::agents::{SharedSubAgentRunner, SubAgentRequest};
use crate::core::config::Config;
use crate::core::tasks::manager::{AddTaskOutcome, TaskManager};
use crate::core::tasks::models::{TaskNode, TaskPriority, TaskStatus};
use crate::core::tools::tools::{BaseDeclarativeTool, ToolInvocation, ToolLocation, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskToolParams {
    pub operation: TaskOperation,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TaskOperation {
    Add {
        title: String,
        parent_id: Option<String>,
        description: Option<String>,
        priority: Option<TaskPriority>,
    },
    Update {
        id: String,
        title: Option<String>,
        description: Option<String>,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    },
    Delete {
        id: String,
    },
    Move {
        id: String,
        new_parent_id: Option<String>,
        after_id: Option<String>,
    },
    Execute {
        id: String,
        agent_type: Option<String>,
        prompt: Option<String>,
    },
    Archive {
        status: Option<TaskStatus>,
    },
    List,
}

fn format_task_list(graph: &crate::core::tasks::models::TaskGraph) -> String {
    let mut output = String::new();

    // 统计任务状态
    let total = graph.nodes.len();
    let completed = graph
        .nodes
        .values()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let in_progress = graph
        .nodes
        .values()
        .filter(|t| t.status == TaskStatus::InProgress)
        .count();
    let pending = graph
        .nodes
        .values()
        .filter(|t| t.status == TaskStatus::Pending)
        .count();
    let blocked = graph
        .nodes
        .values()
        .filter(|t| t.status == TaskStatus::Blocked)
        .count();

    output.push_str(&format!(
        "Task List ({} total, {} completed, {} in progress, {} pending, {} blocked):\n",
        total, completed, in_progress, pending, blocked
    ));
    output.push_str("---\n");

    // Sort root IDs by creation time (implicitly by ID here for simplicity, or we could sort)
    for root_id in &graph.root_ids {
        format_task_recursive(graph, root_id, 0, &mut output);
    }

    if graph.root_ids.is_empty() {
        output.push_str("(No tasks found)\n");
    }

    output.push_str("---\n");
    output.push_str("Use 'update' with task ID to change status. Example: {\"operation\":{\"action\":\"update\",\"id\":\"1\",\"status\":\"InProgress\"}}\n");

    output
}

fn format_task_recursive(
    graph: &crate::core::tasks::models::TaskGraph,
    task_id: &str,
    depth: usize,
    output: &mut String,
) {
    if let Some(task) = graph.nodes.get(task_id) {
        let indent = "  ".repeat(depth);
        let status_symbol = match task.status {
            TaskStatus::Completed => "[x]",
            TaskStatus::InProgress => "[>]",
            TaskStatus::Pending => "[ ]",
            TaskStatus::Blocked => "[!]",
            TaskStatus::Skipped => "[-]",
        };

        let priority_mark = match task.priority {
            TaskPriority::High => " [HIGH]",
            TaskPriority::Medium => "",
            TaskPriority::Low => " [LOW]",
        };

        let agent_info = if let Some(agent) = &task.assigned_agent {
            format!(" @{}", agent)
        } else {
            String::new()
        };

        output.push_str(&format!(
            "{}#{} {} {}{}{}\n",
            indent, task.id, status_symbol, task.title, priority_mark, agent_info
        ));
        if let Some(desc) = &task.description {
            if !desc.is_empty() {
                output.push_str(&format!("{}    {}\n", indent, desc));
            }
        }

        // 显示依赖关系
        if !task.dependencies.is_empty() {
            let deps: Vec<String> = task
                .dependencies
                .iter()
                .map(|d| format!("#{}", d))
                .collect();
            output.push_str(&format!("{}    Blocked by: {}\n", indent, deps.join(", ")));
        }

        for child_id in &task.children {
            format_task_recursive(graph, child_id, depth + 1, output);
        }
    }
}

pub struct TaskTool {
    name: String,
    runner: SharedSubAgentRunner,
    config: Arc<Config>,
    session_id: Option<String>,
    team_name: Option<String>,
}

impl TaskTool {
    pub fn new(runner: SharedSubAgentRunner, config: Arc<Config>) -> Self {
        Self {
            name: "Todo".to_string(),
            runner,
            config,
            session_id: None,
            team_name: None,
        }
    }

    /// 设置会话ID以启用会话隔离
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// 设置团队名称以启用团队隔离
    pub fn with_team_name(mut self, team_name: String) -> Self {
        self.team_name = Some(team_name);
        self
    }
}

impl BaseDeclarativeTool for TaskTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> &str {
        "Task Manager"
    }

    fn description(&self) -> &str {
        "Manage the workspace-scoped coding task list. Use this only as a progress aid for multi-step coding work, not for simple one-shot fixes.
Lifecycle:
- For multi-step work, call list once first to inspect existing tasks for this workspace.
- Add at most one top-level task for the user's current request; prefer update when a matching task already exists.
- Update status after meaningful milestones: Pending -> InProgress -> Completed/Blocked/Skipped.
- Delete only when the user asks to remove a task. Archive completed tasks only when the active list is noisy.
Creation:
- Prefer {\"operation\":{\"action\":\"add\",\"title\":\"...\"}}.
- action='create' is accepted as an alias, but do not repeatedly create duplicate tasks.
- Never create a checklist by calling this tool repeatedly; one add/create call per assistant response is the limit."
    }

    fn kind(&self) -> crate::core::tools::tools::Kind {
        crate::core::tools::tools::Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": serde_json::to_value(all_accepted_action_strings()).unwrap_or_default(),
                            "description": "Task action. Use list to inspect existing tasks before adding."
                        },
                        "title": { "type": "string" },
                        "content": {},
                        "task": {},
                        "description": { "type": "string" },
                        "parent_id": { "type": "string" },
                        "priority": { "type": "string", "enum": ["High", "Medium", "Low", "high", "medium", "low"] },
                        "id": { "type": "string" },
                        "task_id": { "type": "string" },
                        "status": {
                            "type": "string",
                            "enum": ["Pending", "InProgress", "Completed", "Blocked", "Skipped", "pending", "in_progress", "completed", "blocked", "skipped"]
                        },
                        "new_parent_id": { "type": "string" },
                        "after_id": { "type": "string" },
                        "agent_type": { "type": "string" },
                        "prompt": { "type": "string" }
                    },
                    "description": "Task operation object. Prefer {\"action\":\"add\",\"title\":\"...\"}. Runtime still accepts legacy action strings.",
                },
                "action": {
                    "type": "string",
                    "enum": serde_json::to_value(all_accepted_action_strings()).unwrap_or_default(),
                    "description": "Top-level alias for operation.action, accepted for compatibility."
                },
                "title": {
                    "type": "string",
                    "description": "Top-level alias for operation.title when adding a task."
                },
                "content": {
                    "description": "Top-level alias for operation.title or a task object with title/content."
                },
                "task": {
                    "description": "Top-level alias for operation.title or a task object with title/content."
                },
                "id": {
                    "type": "string",
                    "description": "Top-level alias for operation.id when updating/deleting/moving/executing."
                },
                "description": {
                    "type": "string",
                    "description": "Top-level alias for operation.description."
                },
                "status": {
                    "type": "string",
                    "enum": ["Pending", "InProgress", "Completed", "Blocked", "Skipped", "pending", "in_progress", "completed", "blocked", "skipped"],
                    "description": "Top-level alias for operation.status."
                },
                "priority": {
                    "type": "string",
                    "enum": ["High", "Medium", "Low", "high", "medium", "low"],
                    "description": "Top-level alias for operation.priority."
                }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params = normalize_task_tool_params(params)
            .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e))?;
        Ok(Box::new(TaskToolInvocation {
            params,
            runner: self.runner.clone(),
            workspace: self.config.working_dir().clone(),
            session_id: self.session_id.clone(),
            team_name: self.team_name.clone(),
        }))
    }
}

/// All field keys that can appear either at the top level or inside `operation`.
/// Defined once here — used by every normalization path below.
const TASK_FIELD_KEYS: &[&str] = &[
    "action",
    "title",
    "content",
    "task",
    "description",
    "parent_id",
    "parent",
    "priority",
    "id",
    "task_id",
    "status",
    "new_parent_id",
    "new_parent",
    "after_id",
    "after",
    "agent_type",
    "prompt",
];

/// Single source of truth for all accepted action strings.
///
/// Left column  = canonical name (must match `TaskOperation` serde variant name).
/// Right column = all accepted aliases, INCLUDING the canonical name itself.
///
/// `parameter_schema()` builds its enum list from this table, so the schema
/// and the normalization logic can never diverge.
const ACTION_ALIASES: &[(&str, &[&str])] = &[
    ("add", &["add", "create", "new", "add_task", "create_task"]),
    ("update", &["update", "set", "update_task"]),
    ("delete", &["delete", "remove", "delete_task"]),
    ("move", &["move", "reorder"]),
    ("execute", &["execute", "run"]),
    ("archive", &["archive"]),
    ("list", &["list", "show", "view", "unknown", ""]),
];

/// Collect every accepted action string for use in JSON schemas.
fn all_accepted_action_strings() -> Vec<&'static str> {
    ACTION_ALIASES
        .iter()
        .flat_map(|(_, aliases)| aliases.iter().copied())
        .filter(|s| !s.is_empty())
        .collect()
}

fn normalize_task_tool_params(mut params: Value) -> Result<TaskToolParams, String> {
    if params.get("operation").is_none() {
        if params.get("action").is_some() {
            let mut operation = serde_json::Map::new();
            for key in TASK_FIELD_KEYS {
                move_if_present(&mut params, &mut operation, key);
            }
            params["operation"] = Value::Object(operation);
        } else {
            params["operation"] = json!({ "action": "list" });
        }
    }

    let operation_action = params
        .get("operation")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    if let Some(action) = operation_action {
        let mut operation_obj = serde_json::Map::new();
        operation_obj.insert("action".to_string(), Value::String(action));
        for key in TASK_FIELD_KEYS {
            if let Some(value) = params.get(key).cloned() {
                operation_obj.insert(key.to_string(), value);
            }
        }
        params["operation"] = Value::Object(operation_obj);
    }

    merge_top_level_aliases_into_operation(&mut params);

    let Some(operation) = params.get_mut("operation") else {
        return Err("Todo requires an operation object, for example {\"operation\":{\"action\":\"add\",\"title\":\"...\"}}".to_string());
    };

    normalize_operation_object(operation)?;
    serde_json::from_value(params).map_err(|e| format!("invalid Todo parameters: {}", e))
}

fn move_if_present(src: &mut Value, dst: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(value) = src.as_object_mut().and_then(|obj| obj.remove(key)) {
        dst.insert(key.to_string(), value);
    }
}

fn merge_top_level_aliases_into_operation(params: &mut Value) {
    let top_level_values = TASK_FIELD_KEYS
        .iter()
        .filter_map(|key| params.get(*key).cloned().map(|value| (*key, value)))
        .collect::<Vec<_>>();

    let Some(operation) = params.get_mut("operation").and_then(Value::as_object_mut) else {
        return;
    };
    for (key, value) in top_level_values {
        operation.entry(key.to_string()).or_insert(value);
    }
}

fn normalize_operation_object(operation: &mut Value) -> Result<(), String> {
    let Some(obj) = operation.as_object_mut() else {
        return Err("Todo operation must be an object or action string".to_string());
    };

    let action = obj
        .get("action")
        .or_else(|| obj.get("operation"))
        .or_else(|| obj.get("type"))
        .or_else(|| obj.get("op"))
        .or_else(|| obj.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();
    let action = ACTION_ALIASES
        .iter()
        .find(|(_, aliases)| aliases.contains(&action.as_str()))
        .map(|(canonical, _)| *canonical)
        .unwrap_or("list");
    obj.insert("action".to_string(), Value::String(action.to_string()));

    copy_alias_if_missing(obj, "parent_id", &["parent"]);
    copy_alias_if_missing(obj, "id", &["task_id"]);
    copy_alias_if_missing(obj, "new_parent_id", &["new_parent"]);
    copy_alias_if_missing(obj, "after_id", &["after"]);

    if action == "add" {
        if obj.get("title").and_then(Value::as_str).is_none() {
            let title = ["content", "task", "todo", "item", "name"]
                .iter()
                .find_map(|key| obj.get(*key).and_then(extract_task_title))
                .or_else(|| {
                    ["tasks", "todos", "items"].iter().find_map(|key| {
                        obj.get(*key)
                            .and_then(Value::as_array)
                            .and_then(|items| items.first())
                            .and_then(extract_task_title)
                    })
                })
                .ok_or_else(|| "Todo add/create requires title".to_string())?;
            obj.insert("title".to_string(), Value::String(title));
        }

        if obj.get("description").and_then(Value::as_str).is_none() {
            if let Some(description) = ["content", "task", "todo", "item"]
                .iter()
                .find_map(|key| obj.get(*key).and_then(extract_task_description))
            {
                obj.insert("description".to_string(), Value::String(description));
            }
        }
    }

    normalize_enum_string(obj, "priority", &["High", "Medium", "Low"]);
    normalize_enum_string(
        obj,
        "status",
        &["Pending", "InProgress", "Completed", "Blocked", "Skipped"],
    );

    Ok(())
}

fn extract_task_title(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return non_empty_string(text);
    }

    let object = value.as_object()?;
    ["title", "content", "task", "name", "summary"]
        .iter()
        .find_map(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .and_then(non_empty_string)
        })
}

fn extract_task_description(value: &Value) -> Option<String> {
    value.as_object().and_then(|object| {
        ["description", "details", "body", "note"]
            .iter()
            .find_map(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
            })
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn copy_alias_if_missing(
    obj: &mut serde_json::Map<String, Value>,
    canonical_key: &str,
    aliases: &[&str],
) {
    if obj.get(canonical_key).is_some() {
        return;
    }

    if let Some(value) = aliases.iter().find_map(|alias| obj.get(*alias).cloned()) {
        obj.insert(canonical_key.to_string(), value);
    }
}

fn normalize_enum_string(obj: &mut serde_json::Map<String, Value>, key: &str, allowed: &[&str]) {
    let Some(value) = obj.get_mut(key) else {
        return;
    };
    let Some(raw) = value.as_str() else {
        return;
    };
    let normalized = raw.replace(['-', '_', ' '], "").to_ascii_lowercase();
    if let Some(canonical) = allowed
        .iter()
        .find(|item| item.replace(['-', '_', ' '], "").to_ascii_lowercase() == normalized)
    {
        *value = Value::String((*canonical).to_string());
    }
}

pub struct TaskToolInvocation {
    params: TaskToolParams,
    runner: SharedSubAgentRunner,
    workspace: std::path::PathBuf,
    session_id: Option<String>,
    team_name: Option<String>,
}

impl ToolInvocation for TaskToolInvocation {
    fn get_description(&self) -> String {
        match &self.params.operation {
            TaskOperation::Add { title, .. } => format!("Add task: {}", title),
            TaskOperation::Update { id, .. } => format!("Update task: {}", id),
            TaskOperation::Delete { id } => format!("Delete task: {}", id),
            TaskOperation::Move { id, .. } => format!("Move task: {}", id),
            TaskOperation::Execute { id, .. } => format!("Execute task: {}", id),
            TaskOperation::Archive { .. } => "Archive tasks".to_string(),
            TaskOperation::List => "List all tasks".to_string(),
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let op = self.params.operation.clone();
        let runner = self.runner.clone();
        let workspace = self.workspace.clone();
        let session_id = self.session_id.clone();
        let team_name = self.team_name.clone();

        Box::pin(async move {
            // 根据session_id或team_name选择正确的任务文件路径
            let path = if let Some(team) = &team_name {
                TaskManager::task_file_for_team(&workspace, team)
            } else if let Some(session) = &session_id {
                TaskManager::task_file_for_session(&workspace, session)
            } else {
                TaskManager::task_file_for_workspace(&workspace)
            };

            // Handle Execute separately because it involves async operations
            if let TaskOperation::Execute {
                id,
                agent_type: _,
                prompt,
            } = &op
            {
                // Load manager in blocking thread to get task details
                let path_clone = path.clone();
                let manager = tokio::task::spawn_blocking(move || {
                    TaskManager::load_from_file(&path_clone).unwrap_or_else(|_| TaskManager::new())
                })
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

                let task_opt = manager.get_task(id);
                if let Some(task) = task_opt {
                    let task_prompt = if let Some(p) = prompt {
                        format!("Task: {}\nDescription: {}\nInstructions: {}\n\n<system-reminder>You are a SubAgent executed by the TaskTool. Focus on completing this specific task. Do not use the TaskTool to spawn more agents recursively unless absolutely necessary for complex sub-problems.</system-reminder>", task.title, task.description.clone().unwrap_or_default(), p)
                    } else {
                        format!("Task: {}\nDescription: {}\n\n<system-reminder>You are a SubAgent executed by the TaskTool. Focus on completing this specific task. Do not use the TaskTool to spawn more agents recursively unless absolutely necessary for complex sub-problems.</system-reminder>", task.title, task.description.clone().unwrap_or_default())
                    };

                    let result = runner
                        .run(SubAgentRequest::new(task_prompt).with_max_rounds(50))
                        .await
                        .map_err(|e| {
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                e.to_string(),
                            )) as Box<dyn std::error::Error>
                        })?;

                    let final_output = format!("Task executed. Output:\n{}", result.output);

                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: final_output,
                        error: None,
                        data: None,
                    });
                } else {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: format!("Task {} not found", id),
                        error: None,
                        data: None,
                    });
                }
            }

            // For all other operations, run in blocking thread
            let result = tokio::task::spawn_blocking(move || {
                let mut manager =
                    TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());

                let output = match op {
                    TaskOperation::Add {
                        title,
                        parent_id,
                        description,
                        priority,
                    } => {
                        let mut task = TaskNode::new(title.clone());
                        task.parent_id = parent_id.clone();
                        task.description = description.clone();
                        if let Some(ref p) = priority {
                            task.priority = p.clone();
                        }

                        match manager.add_task_dedup(task) {
                            Ok(AddTaskOutcome::Added(id)) => {
                                manager
                                    .save_to_file(&path)
                                    .map_err(|e| format!("Failed to save: {}", e))?;
                                let task_info = manager.get_task(&id);
                                let title = task_info.map(|t| t.title.as_str()).unwrap_or(&id);
                                let status = task_info.map(|t| format!("{:?}", t.status)).unwrap_or_else(|| "Pending".to_string());
                                format!("Task #{} created successfully: {}\nStatus: {}\nWorkspace: {}", id, title, status, workspace.display())
                            }
                            Ok(AddTaskOutcome::Existing(id)) => {
                                let task_info = manager.get_task(&id);
                                let title = task_info.map(|t| t.title.as_str()).unwrap_or(&id);
                                let status = task_info.map(|t| format!("{:?}", t.status)).unwrap_or_else(|| "Unknown".to_string());
                                format!("Task #{} already exists: {}\nStatus: {}\nUse 'update' to modify existing tasks.", id, title, status)
                            }
                            Err(e) => format!("Error adding task: {}", e),
                        }
                    }
                    TaskOperation::Update {
                        id,
                        title,
                        description,
                        status,
                        priority,
                    } => {
                        if let Some(task) = manager.get_task_mut(&id) {
                            let mut updated_fields = Vec::new();
                            
                            if let Some(t) = title {
                                task.title = t.clone();
                                updated_fields.push("title");
                            }
                            if let Some(d) = description {
                                task.description = Some(d.clone());
                                updated_fields.push("description");
                            }
                            if let Some(s) = status {
                                task.status = s.clone();
                                updated_fields.push("status");
                            }
                            if let Some(p) = priority {
                                task.priority = p.clone();
                                updated_fields.push("priority");
                            }

                            let title = task.title.clone();
                            let status = format!("{:?}", task.status);
                            
                            manager
                                .save_to_file(&path)
                                .map_err(|e| format!("Failed to save: {}", e))?;
                            
                            format!("Updated task #{}: {}\nStatus: {}\nUpdated fields: {}", id, title, status, updated_fields.join(", "))
                        } else {
                            format!("Task #{} not found", id)
                        }
                    }
                    TaskOperation::Delete { id } => {
                        let title = { manager.get_task(&id).map(|t| t.title.clone()) };
                        match manager.delete_task(&id) {
                            Ok(_) => {
                                manager.save_to_file(&path).map_err(|e| format!("Failed to save: {}", e))?;
                                format!("Deleted task #{}: {}", id, title.as_deref().unwrap_or(&id))
                            }
                            Err(e) => format!("Error deleting task #{}: {}", id, e),
                        }
                    },
                    TaskOperation::Move {
                        id,
                        new_parent_id,
                        after_id,
                    } => match manager.move_task(&id, new_parent_id.clone(), after_id.clone()) {
                        Ok(_) => {
                            manager
                                .save_to_file(&path)
                                .map_err(|e| format!("Failed to save: {}", e))?;
                            format!("Task {} moved", id)
                        }
                        Err(e) => format!("Error moving task: {}", e),
                    },
                    TaskOperation::List => {
                        format!(
                            "Workspace: {}\n{}",
                            workspace.display(),
                            format_task_list(&manager.graph)
                        )
                    }
                    TaskOperation::Archive { status } => {
                        // 根据session_id或team_name选择正确的归档路径
                        let archive_path = if let Some(team) = &team_name {
                            TaskManager::archive_file_for_team(&workspace, team)
                        } else if let Some(session) = &session_id {
                            TaskManager::archive_file_for_session(&workspace, session)
                        } else {
                            TaskManager::archive_file_for_workspace(&workspace)
                        };
                        
                        match manager.archive_completed_tasks(&archive_path, status) {
                            Ok(count) => {
                                manager
                                    .save_to_file(&path)
                                    .map_err(|e| format!("Failed to save: {}", e))?;
                                format!("Archived {} tasks to {}", count, archive_path.display())
                            }
                            Err(e) => format!("Error archiving tasks: {}", e),
                        }
                    }
                    TaskOperation::Execute { .. } => unreachable!("Handled separately"),
                };

                Ok(output)
            })
            .await;

            let output = match result {
                Ok(res) => res.map_err(|e: String| {
                    Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
                        as Box<dyn std::error::Error>
                })?,
                Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
            };

            Ok(ToolResult {
                llm_content: None,
                return_display: None,
                output,
                error: None,
                data: None,
            })
        })
    }
}
