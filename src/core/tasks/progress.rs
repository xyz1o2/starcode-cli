/// 任务进度追踪
///
/// 对标claude-code-main的进度追踪功能
use serde::{Deserialize, Serialize};

/// 进度更新
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    /// 任务ID
    pub task_id: String,
    /// 工具使用次数
    pub tool_use_count: u32,
    /// Token数量
    pub token_count: u32,
    /// 最后活动
    pub last_activity: Option<String>,
    /// 摘要
    pub summary: Option<String>,
}

/// 任务进度追踪器
pub struct TaskProgressTracker {
    /// 进度记录
    progress: HashMap<String, ProgressRecord>,
}

/// 进度记录
#[derive(Debug, Clone)]
struct ProgressRecord {
    /// 工具使用次数
    tool_use_count: u32,
    /// Token数量
    token_count: u32,
    /// 最后活动
    last_activity: Option<String>,
    /// 摘要
    summary: Option<String>,
    /// 最后更新时间
    last_updated: u64,
}

impl TaskProgressTracker {
    /// 创建新的任务进度追踪器
    pub fn new() -> Self {
        Self {
            progress: HashMap::new(),
        }
    }

    /// 更新进度
    pub fn update_progress(&mut self, update: ProgressUpdate) {
        let record = ProgressRecord {
            tool_use_count: update.tool_use_count,
            token_count: update.token_count,
            last_activity: update.last_activity,
            summary: update.summary,
            last_updated: Self::current_time_ms(),
        };
        self.progress.insert(update.task_id, record);
    }

    /// 获取进度
    pub fn get_progress(&self, task_id: &str) -> Option<ProgressUpdate> {
        self.progress.get(task_id).map(|r| ProgressUpdate {
            task_id: task_id.to_string(),
            tool_use_count: r.tool_use_count,
            token_count: r.token_count,
            last_activity: r.last_activity.clone(),
            summary: r.summary.clone(),
        })
    }

    /// 获取当前时间（毫秒）
    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

use std::collections::HashMap;
