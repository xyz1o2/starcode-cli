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
        "TaskGet"
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

                // 必须按清单顺序（root_ids DFS）回传：`nodes` 是 HashMap，
                // 且 id 是随机 uuid——之前的 `sort_by(id)` 会把清单顺序完全打乱，
                // 导致模型不按面板上从上往下的顺序执行。
                let ordered: Vec<String> = manager.graph.ordered_ids();
                let mut tasks: Vec<_> = ordered
                    .iter()
                    .filter_map(|id| manager.graph.nodes.get(id).cloned())
                    .filter(|t| {
                        if let Some(ref status) = status_filter {
                            std::mem::discriminant(&t.status) == std::mem::discriminant(status)
                        } else {
                            true
                        }
                    })
                    .take(params.limit)
                    .collect();

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
        "TaskList"
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
    /// 重命名任务标题 (Rename task subject)
    #[serde(default)]
    pub subject: Option<String>,
    /// 设置 in_progress 时的动态描述 (Set activeForm for in_progress display)
    #[serde(default)]
    pub active_form: Option<String>,
    /// 本任务完成后解锁的任务 ID (Tasks unblocked when this completes)
    #[serde(default)]
    pub add_blocks: Option<Vec<String>>,
    /// 阻塞本任务的任务 ID (Tasks blocking this one)
    #[serde(default)]
    pub add_blocked_by: Option<Vec<String>>,
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
                        let is_terminal =
                            matches!(new_status, TaskStatus::Completed | TaskStatus::Skipped);
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            task.status = new_status;
                            // Set completed_at timestamp for 30s TTL
                            if is_terminal {
                                task.completed_at = Some(chrono::Utc::now());
                            } else {
                                task.completed_at = None;
                            }
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

                    if let Some(ref subject) = params.subject {
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            task.title = subject.clone();
                            task.updated_at = chrono::Utc::now();
                        }
                        changes.push(format!("subject={}", subject));
                    }

                    if let Some(ref active_form) = params.active_form {
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            task.active_form = Some(active_form.clone());
                        }
                        changes.push(format!("active_form={}", active_form));
                    }

                    if let Some(ref block_ids) = params.add_blocks {
                        let mut new_blocks = Vec::new();
                        if let Some(task) = manager.get_task(&params.task_id) {
                            for bid in block_ids {
                                if !task.blocks.contains(bid) {
                                    new_blocks.push(bid.clone());
                                }
                            }
                        }
                        // Apply: add to this task's blocks
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            for bid in &new_blocks {
                                task.blocks.push(bid.clone());
                            }
                        }
                        // Apply: add reverse dependency on blocked tasks
                        for bid in &new_blocks {
                            if let Some(blocked_task) = manager.get_task_mut(bid) {
                                if !blocked_task.dependencies.contains(&params.task_id) {
                                    blocked_task.dependencies.push(params.task_id.clone());
                                }
                            }
                        }
                        changes.push(format!("blocks=[{}]", block_ids.join(", ")));
                    }

                    if let Some(ref blocked_by_ids) = params.add_blocked_by {
                        let mut new_deps = Vec::new();
                        if let Some(task) = manager.get_task(&params.task_id) {
                            for bid in blocked_by_ids {
                                if !task.dependencies.contains(bid) {
                                    new_deps.push(bid.clone());
                                }
                            }
                        }
                        // Apply: add to this task's dependencies
                        if let Some(task) = manager.get_task_mut(&params.task_id) {
                            for bid in &new_deps {
                                task.dependencies.push(bid.clone());
                            }
                        }
                        // Apply: add reverse block on blocking tasks
                        for bid in &new_deps {
                            if let Some(blocker) = manager.get_task_mut(bid) {
                                if !blocker.blocks.contains(&params.task_id) {
                                    blocker.blocks.push(params.task_id.clone());
                                }
                            }
                        }
                        changes.push(format!("blocked_by=[{}]", blocked_by_ids.join(", ")));
                    }

                    // Auto-update status when marking completed: unblock dependents
                    if params.status.as_deref() == Some("completed") {
                        let task_id = params.task_id.clone();
                        let blocked_ids: Vec<String> = manager
                            .get_task(&task_id)
                            .map(|t| t.blocks.clone())
                            .unwrap_or_default();
                        // Collect which tasks need status change (avoid borrow conflict)
                        let mut unblock_ids = Vec::new();
                        for blocked_id in &blocked_ids {
                            if let Some(blocked_task) = manager.get_task_mut(blocked_id) {
                                blocked_task.dependencies.retain(|d| d != &task_id);
                                if blocked_task.dependencies.is_empty()
                                    && blocked_task.status
                                        == crate::core::tasks::models::TaskStatus::Blocked
                                {
                                    unblock_ids.push(blocked_id.clone());
                                }
                            }
                        }
                        for uid in &unblock_ids {
                            if let Some(task) = manager.get_task_mut(uid) {
                                task.status = crate::core::tasks::models::TaskStatus::Pending;
                            }
                        }
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
        "TaskUpdate"
    }

    fn display_name(&self) -> &str {
        "Task Update"
    }

    fn description(&self) -> &str {
        "更新任务状态、标题、依赖关系或备注。完成任务自动解锁被阻塞的依赖。(Update task status, subject, dependencies, or notes. Completing auto-unblocks dependents.)"
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
                    "description": "新状态 (New status). Completing auto-unblocks dependents."
                },
                "subject": {
                    "type": "string",
                    "description": "重命名任务标题 (Rename task subject)"
                },
                "notes": {
                    "type": "string",
                    "description": "备注内容 (Notes to append)"
                },
                "active_form": {
                    "type": "string",
                    "description": "进行时描述,如 'Running tests' (activeForm shown while in_progress)"
                },
                "add_blocks": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "本任务完成后解锁的任务 ID (Tasks unblocked when this completes)"
                },
                "add_blocked_by": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "阻塞本任务的任务 ID (Tasks blocking this one)"
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

#[cfg(test)]
mod output_tests {
    use super::*;
    use crate::agent::subagent::notification::NotificationStatus;

    #[tokio::test]
    async fn background_output_is_shared_and_survives_completion() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::core::config::Config::new(crate::core::config::ConfigParameters {
            target_dir: directory.path().to_owned(),
            ..Default::default()
        });
        let bus = Arc::new(crate::core::confirmation_bus::MessageBus::new(
            crate::core::policy::policy_engine::PolicyEngine::new(Default::default()),
            false,
        ));
        let services = crate::core::config::RuntimeServices::new(
            None,
            bus,
            None,
            Arc::new(crate::core::state::GlobalState::new()),
        );
        let tasks = services.subagent_tasks();
        config.install_runtime_services(services.clone());
        assert!(Arc::ptr_eq(
            &tasks,
            &config.clone().runtime_subagent_tasks().unwrap()
        ));
        let config = Arc::new(config);
        let tool = TaskOutputTool::new(config.clone());
        let path = config
            .storage()
            .project_subagent_outputs_dir()
            .join("agent-result.txt");
        tasks
            .lock()
            .await
            .register("agent-result", "inspect".into(), path.clone());
        let invocation = tool
            .create_invocation(serde_json::json!({"task_id": "agent-result"}))
            .unwrap();
        let result = invocation.execute(None, None).await.unwrap();
        assert_eq!(result.data.unwrap()["status"], "running");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "final result").await.unwrap();
        tasks
            .lock()
            .await
            .finish("agent-result", NotificationStatus::Completed, None);
        tasks
            .lock()
            .await
            .finish("agent-result", NotificationStatus::Failed, None);
        for _ in 0..2 {
            let result = invocation.execute(None, None).await.unwrap();
            assert_eq!(result.output, "final result");
            assert_eq!(result.data.unwrap()["status"], "completed");
        }
        tasks
            .lock()
            .await
            .register("agent-write-error", "inspect".into(), path);
        tasks.lock().await.finish(
            "agent-write-error",
            NotificationStatus::Failed,
            Some("disk full".into()),
        );
        let error = tool
            .create_invocation(serde_json::json!({"task_id": "agent-write-error"}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disk full"));
    }

    #[tokio::test]
    async fn final_outputs_survive_a_fresh_runtime_and_corruption_is_explicit() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::core::config::Config::new(crate::core::config::ConfigParameters {
            target_dir: directory.path().to_owned(),
            ..Default::default()
        });
        config.initialize().await.unwrap();
        let tasks = config.runtime_subagent_tasks().unwrap();
        let output_dir = config.storage().project_subagent_outputs_dir();
        tokio::fs::create_dir_all(&output_dir).await.unwrap();
        for (id, status) in [
            ("done", NotificationStatus::Completed),
            ("failed", NotificationStatus::Failed),
            ("stopped", NotificationStatus::Killed),
        ] {
            let path = output_dir.join(format!("{id}.txt"));
            tokio::fs::write(&path, format!("result {id}"))
                .await
                .unwrap();
            let snapshot = {
                let mut tasks = tasks.lock().await;
                tasks.register(id, format!("description {id}"), path);
                tasks.finish(id, status, None);
                tasks.get(id).unwrap()
            };
            snapshot.persist_final().await.unwrap();
        }
        let mut fresh = crate::core::config::Config::new(crate::core::config::ConfigParameters {
            target_dir: directory.path().to_owned(),
            ..Default::default()
        });
        fresh.initialize().await.unwrap();
        assert!(!Arc::ptr_eq(
            &tasks,
            &fresh.runtime_subagent_tasks().unwrap()
        ));
        assert!(fresh
            .runtime_subagent_tasks()
            .unwrap()
            .lock()
            .await
            .get("done")
            .is_none());
        let tool = TaskOutputTool::new(Arc::new(fresh));
        for (id, status) in [
            ("done", "completed"),
            ("failed", "failed"),
            ("stopped", "killed"),
        ] {
            let result = tool
                .create_invocation(serde_json::json!({"task_id": id}))
                .unwrap()
                .execute(None, None)
                .await
                .unwrap();
            assert_eq!(result.output, format!("result {id}"));
            assert_eq!(result.data.unwrap()["status"], status);
        }
        let child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("core::tools::task_management::output_tests::stored_output_reader_in_another_process")
            .arg("--test-threads=1")
            .arg("--ignored")
            .env("STAR_TEST_TASK_OUTPUT_PROJECT", directory.path())
            .kill_on_drop(true)
            .output();
        let child = tokio::time::timeout(std::time::Duration::from_secs(15), child)
            .await
            .unwrap()
            .unwrap();
        assert!(
            child.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        for content in [
            "{broken",
            r#"{"version":2,"task_id":"done","description":"x","status":"Completed","output_error":null}"#,
            r#"{"version":1,"task_id":"other","description":"x","status":"Completed","output_error":null}"#,
            r#"{"version":1,"task_id":"done","description":"x","status":"Running","output_error":null}"#,
        ] {
            tokio::fs::write(output_dir.join("done.json"), content)
                .await
                .unwrap();
            let error = tool
                .create_invocation(serde_json::json!({"task_id":"done"}))
                .unwrap()
                .execute(None, None)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("Invalid task record"));
        }
        tokio::fs::write(output_dir.join("done.json"), r#"{"version":1,"task_id":"done","description":"x","status":"Failed","output_error":"disk full"}"#).await.unwrap();
        let error = tool
            .create_invocation(serde_json::json!({"task_id":"done"}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disk full"));
        tokio::fs::remove_file(output_dir.join("failed.txt"))
            .await
            .unwrap();
        let error = tool
            .create_invocation(serde_json::json!({"task_id":"failed"}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Failed to read output"));
    }

    #[tokio::test]
    #[ignore = "spawned by the persistence regression with its temporary project"]
    async fn stored_output_reader_in_another_process() {
        let directory = std::env::var_os("STAR_TEST_TASK_OUTPUT_PROJECT").expect("parent project");
        let config = crate::core::config::Config::new(crate::core::config::ConfigParameters {
            target_dir: directory.into(),
            ..Default::default()
        });
        assert!(config.runtime_subagent_tasks().is_none());
        let tool = TaskOutputTool::new(Arc::new(config));
        for (id, status) in [
            ("done", "completed"),
            ("failed", "failed"),
            ("stopped", "killed"),
        ] {
            let result = tool
                .create_invocation(serde_json::json!({"task_id": id}))
                .unwrap()
                .execute(None, None)
                .await
                .unwrap();
            assert_eq!(result.output, format!("result {id}"));
            assert_eq!(result.data.unwrap()["status"], status);
        }
    }

    #[tokio::test]
    async fn task_output_preserves_json_results_and_rejects_path_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let config = Arc::new(crate::core::config::Config::new(
            crate::core::config::ConfigParameters {
                target_dir: directory.path().to_owned(),
                ..Default::default()
            },
        ));
        let path = config.storage().project_task_outputs_dir();
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(path.join("old-task.json"), r#"{"result":"legacy"}"#)
            .await
            .unwrap();
        let tool = TaskOutputTool::new(config);
        let result = tool
            .create_invocation(serde_json::json!({"task_id":"old-task"}))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();
        assert_eq!(result.data.unwrap()["result"], "legacy");
        for id in ["", "../outside", "a/b", "a\\b", ".", ".."] {
            assert!(tool
                .create_invocation(serde_json::json!({"task_id": id}))
                .is_err());
        }
    }
}

// ── TaskStop ─────────────────────────────────────────────────────────

pub struct TaskStopTool {
    config: Arc<crate::core::config::Config>,
}

impl TaskStopTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

struct TaskStopInvocation {
    config: Arc<crate::core::config::Config>,
    task_id: String,
}

impl ToolInvocation for TaskStopInvocation {
    fn get_description(&self) -> String {
        format!("Request stop: {}", self.task_id)
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
        Box::pin(async move {
            let tasks = self
                .config
                .runtime_subagent_tasks()
                .ok_or("Background task registry unavailable")?;
            let requested = tasks.lock().await.request_stop(&self.task_id)?;
            let message = if requested {
                format!(
                    "Stop requested for {}. Wait for terminal status before claiming it stopped.",
                    self.task_id
                )
            } else {
                format!(
                    "Task {} has finished execution or is already finalizing.",
                    self.task_id
                )
            };
            Ok(ToolResult {
                llm_content: Some(message.clone()),
                return_display: Some(message.clone()),
                output: message,
                error: None,
                data: Some(
                    serde_json::json!({"task_id": self.task_id, "stop_requested": requested}),
                ),
            })
        })
    }
}

impl BaseDeclarativeTool for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }
    fn display_name(&self) -> &str {
        "Stop Task"
    }
    fn description(&self) -> &str {
        "Request cancellation of a running background agent by task ID."
    }
    fn kind(&self) -> Kind {
        Kind::Execute
    }
    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object", "properties":{"task_id":{"type":"string"}}, "required":["task_id"]})
    }
    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TaskOutputParams = serde_json::from_value(params)?;
        Ok(Box::new(TaskStopInvocation {
            config: self.config.clone(),
            task_id: params.task_id,
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
            let live_task = if let Some(tasks) = config.runtime_subagent_tasks() {
                tasks.lock().await.get(&task_id)
            } else {
                None
            };
            let task = match live_task {
                Some(task) => Some(task),
                None => {
                    crate::agent::subagent::tasks::SubagentTask::load_final(
                        &config.storage().project_subagent_outputs_dir(),
                        &task_id,
                    )
                    .await?
                }
            };
            if let Some(task) = task {
                let status = match task.status.as_ref() {
                    None => "running",
                    Some(crate::agent::subagent::notification::NotificationStatus::Completed) => {
                        "completed"
                    }
                    Some(crate::agent::subagent::notification::NotificationStatus::Failed) => {
                        "failed"
                    }
                    Some(crate::agent::subagent::notification::NotificationStatus::Killed) => {
                        "killed"
                    }
                };
                if let Some(error) = task.output_error {
                    return Err(
                        format!("Failed to store output for task {task_id}: {error}").into(),
                    );
                }
                let output = if task.status.is_some() {
                    tokio::fs::read_to_string(&task.output_file)
                        .await
                        .map_err(|error| {
                            format!("Failed to read output for task {task_id}: {error}")
                        })?
                } else {
                    format!("Task '{task_id}' is running. No final output stored yet.")
                };
                let data = serde_json::json!({
                    "task_id": task.task_id, "description": task.description,
                    "status": status, "output": output, "output_file": task.output_file,
                });
                return Ok(ToolResult {
                    llm_content: Some(serde_json::to_string_pretty(&data)?),
                    return_display: Some(format!("Task {task_id}: {status}")),
                    output,
                    error: None,
                    data: Some(data),
                });
            }
            let output_path = config
                .storage()
                .project_task_outputs_dir()
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
        "TaskOutput"
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
        if params.task_id.is_empty()
            || params.task_id.len() > 128
            || !params
                .task_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(
                "task_id must contain only ASCII letters, digits, hyphens, or underscores".into(),
            );
        }
        Ok(Box::new(TaskOutputInvocation {
            config: self.config.clone(),
            params,
        }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
