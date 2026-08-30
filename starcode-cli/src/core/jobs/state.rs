/// 作业状态

use serde::{Deserialize, Serialize};

/// 作业状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    /// 总作业数
    pub total_jobs: u64,
    /// 活跃作业数
    pub active_jobs: u64,
    /// 完成作业数
    pub completed_jobs: u64,
    /// 失败作业数
    pub failed_jobs: u64,
    /// 取消作业数
    pub cancelled_jobs: u64,
    /// 最后更新时间
    pub last_updated: i64,
}

impl JobState {
    /// 创建新的作业状态
    pub fn new() -> Self {
        Self {
            total_jobs: 0,
            active_jobs: 0,
            completed_jobs: 0,
            failed_jobs: 0,
            cancelled_jobs: 0,
            last_updated: chrono::Utc::now().timestamp(),
        }
    }

    /// 更新状态
    pub fn update(&mut self, total: u64, active: u64, completed: u64, failed: u64, cancelled: u64) {
        self.total_jobs = total;
        self.active_jobs = active;
        self.completed_jobs = completed;
        self.failed_jobs = failed;
        self.cancelled_jobs = cancelled;
        self.last_updated = chrono::Utc::now().timestamp();
    }
}
