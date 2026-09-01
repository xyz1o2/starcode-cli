/// 任务框架
///
/// 对标claude-code-main的src/utils/task/framework.ts
/// 提供任务状态管理、注册、更新和驱逐功能
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    /// 等待中
    Pending,
    /// 运行中
    Running,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已停止
    Killed,
}

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskType {
    /// 本地Agent任务
    LocalAgent,
    /// 远程Agent任务
    RemoteAgent,
    /// Shell任务
    Shell,
    /// 工作流任务
    Workflow,
    /// Dream任务
    Dream,
    /// MCP监控任务
    MonitorMcp,
    /// 进程内队友任务
    InProcessTeammate,
}

/// 任务状态基础
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStateBase {
    /// 任务ID
    pub id: String,
    /// 任务类型
    pub task_type: TaskType,
    /// 描述
    pub description: String,
    /// 状态
    pub status: TaskStatus,
    /// 开始时间
    pub start_time: u64,
    /// 结束时间
    pub end_time: Option<u64>,
    /// 是否已通知
    pub notified: bool,
    /// 工具使用ID
    pub tool_use_id: Option<String>,
}

/// 任务附件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAttachment {
    /// 附件类型
    pub attachment_type: String,
    /// 任务ID
    pub task_id: String,
    /// 工具使用ID
    pub tool_use_id: Option<String>,
    /// 任务类型
    pub task_type: TaskType,
    /// 状态
    pub status: TaskStatus,
    /// 描述
    pub description: String,
    /// 增量摘要
    pub delta_summary: Option<String>,
}

/// 任务事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskEvent {
    /// 任务开始
    Started {
        task_id: String,
        task_type: TaskType,
        description: String,
    },
    /// 任务进度
    Progress {
        task_id: String,
        progress: f64,
        message: Option<String>,
    },
    /// 任务完成
    Completed {
        task_id: String,
        result: Option<String>,
    },
    /// 任务失败
    Failed { task_id: String, error: String },
    /// 任务停止
    Killed { task_id: String },
}

/// 任务框架
pub struct TaskFramework {
    /// 任务状态
    tasks: HashMap<String, TaskStateBase>,
    /// 最大显示时间（毫秒）
    max_display_ms: u64,
    /// 面板宽限期（毫秒）
    panel_grace_ms: u64,
}

impl TaskFramework {
    /// 创建新的任务框架
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            max_display_ms: 3000,
            panel_grace_ms: 30000,
        }
    }

    /// 注册任务
    pub fn register_task(&mut self, task: TaskStateBase) {
        self.tasks.insert(task.id.clone(), task);
    }

    /// 更新任务状态
    pub fn update_task_status(&mut self, task_id: &str, status: TaskStatus) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.status = status.clone();
            if status == TaskStatus::Completed
                || status == TaskStatus::Failed
                || status == TaskStatus::Killed
            {
                task.end_time = Some(Self::current_time_ms());
            }
        }
    }

    /// 获取任务
    pub fn get_task(&self, task_id: &str) -> Option<&TaskStateBase> {
        self.tasks.get(task_id)
    }

    /// 获取所有运行中的任务
    pub fn get_running_tasks(&self) -> Vec<&TaskStateBase> {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .collect()
    }

    /// 驱逐已完成的任务
    pub fn evict_completed_tasks(&mut self) {
        let now = Self::current_time_ms();
        self.tasks.retain(|_, task| {
            if task.status == TaskStatus::Completed
                || task.status == TaskStatus::Failed
                || task.status == TaskStatus::Killed
            {
                if let Some(end_time) = task.end_time {
                    now - end_time < self.max_display_ms
                } else {
                    false
                }
            } else {
                true
            }
        });
    }

    /// 生成任务附件
    pub fn generate_attachments(&self) -> Vec<TaskAttachment> {
        self.tasks
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .map(|t| TaskAttachment {
                attachment_type: "task_status".to_string(),
                task_id: t.id.clone(),
                tool_use_id: t.tool_use_id.clone(),
                task_type: t.task_type.clone(),
                status: t.status.clone(),
                description: t.description.clone(),
                delta_summary: None,
            })
            .collect()
    }

    /// 获取当前时间（毫秒）
    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
