use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Skipped,
}

/// 任务变更事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskChangeEvent {
    /// 任务创建
    Created {
        task_id: String,
        title: String,
        status: TaskStatus,
    },
    /// 任务更新
    Updated {
        task_id: String,
        title: String,
        old_status: TaskStatus,
        new_status: TaskStatus,
        updated_fields: Vec<String>,
    },
    /// 任务删除
    Deleted { task_id: String, title: String },
    /// 任务列表重置
    Reset,
}

/// 任务变更通知器
#[derive(Clone)]
pub struct TaskNotifier {
    sender: broadcast::Sender<TaskChangeEvent>,
}

impl TaskNotifier {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(100);
        Self { sender }
    }

    /// 发送任务变更通知
    pub fn notify(&self, event: TaskChangeEvent) {
        // 忽略发送错误（如果没有接收者）
        let _ = self.sender.send(event);
    }

    /// 订阅任务变更
    pub fn subscribe(&self) -> broadcast::Receiver<TaskChangeEvent> {
        self.sender.subscribe()
    }
}

impl Default for TaskNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskPriority {
    High,
    Medium,
    Low,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub dependencies: Vec<String>,
    pub children: Vec<String>,
    pub assigned_agent: Option<String>,
    /// 进行时描述（对标 Claude Code 的 `activeForm`）：任务 in_progress 时面板/转圈
    /// 显示 "Running tests" 而不是祈使句 "Run tests"。旧文件没有该字段 → None。
    #[serde(default)]
    pub active_form: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TaskNode {
    pub fn new(title: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            title,
            description: None,
            status: TaskStatus::Pending,
            priority: TaskPriority::Medium,
            dependencies: Vec::new(),
            children: Vec::new(),
            assigned_agent: None,
            active_form: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskGraph {
    pub nodes: HashMap<String, TaskNode>,
    pub root_ids: Vec<String>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root_ids: Vec::new(),
        }
    }

    pub fn add_task(&mut self, mut task: TaskNode) {
        if task.parent_id.is_none() {
            if !self.root_ids.contains(&task.id) {
                self.root_ids.push(task.id.clone());
            }
        } else {
            // Ensure parent exists and add to parent's children
            if let Some(parent_id) = &task.parent_id {
                if let Some(parent) = self.nodes.get_mut(parent_id) {
                    if !parent.children.contains(&task.id) {
                        parent.children.push(task.id.clone());
                    }
                } else {
                    // Parent not found, treat as root? Or error?
                    // For now, force it to be root if parent missing
                    task.parent_id = None;
                    if !self.root_ids.contains(&task.id) {
                        self.root_ids.push(task.id.clone());
                    }
                }
            }
        }
        self.nodes.insert(task.id.clone(), task);
    }
}
