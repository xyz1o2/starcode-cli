/// 作业系统
/// 
/// 对标claude-code-main的src/jobs/
/// 后台作业调度和管理

pub mod classifier;
pub mod scheduler;
pub mod state;
pub mod templates;

pub use classifier::JobClassifier;
pub use scheduler::JobScheduler;
pub use state::JobState;
pub use templates::JobTemplates;

use serde::{Deserialize, Serialize};

/// 作业类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobType {
    /// Dream任务
    Dream,
    /// 本地Agent任务
    LocalAgent,
    /// 远程Agent任务
    RemoteAgent,
    /// 进程内队友任务
    InProcessTeammate,
    /// 本地Shell任务
    LocalShell,
    /// 本地工作流任务
    LocalWorkflow,
    /// MCP监控任务
    MonitorMcp,
    /// 自定义任务
    Custom(String),
}

/// 作业状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    /// 等待中
    Pending,
    /// 运行中
    Running,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
    /// 暂停
    Paused,
}

/// 作业优先级
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    /// 低
    Low,
    /// 正常
    Normal,
    /// 高
    High,
    /// 紧急
    Critical,
}

/// 作业定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// 作业ID
    pub id: String,
    /// 作业名称
    pub name: String,
    /// 作业类型
    pub job_type: JobType,
    /// 作业状态
    pub status: JobStatus,
    /// 优先级
    pub priority: JobPriority,
    /// 创建时间
    pub created_at: i64,
    /// 开始时间
    pub started_at: Option<i64>,
    /// 完成时间
    pub completed_at: Option<i64>,
    /// 输入参数
    pub input: serde_json::Value,
    /// 输出结果
    pub output: Option<serde_json::Value>,
    /// 错误信息
    pub error: Option<String>,
    /// 进度（0-100）
    pub progress: u8,
    /// 标签
    pub tags: Vec<String>,
    /// 元数据
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// 作业管理器
pub struct JobManager {
    /// 作业存储
    jobs: std::collections::HashMap<String, Job>,
    /// 调度器
    scheduler: JobScheduler,
    /// 分类器
    classifier: JobClassifier,
    /// 最大并发作业数
    max_concurrent: usize,
}

impl JobManager {
    /// 创建新的作业管理器
    pub fn new() -> Self {
        Self {
            jobs: std::collections::HashMap::new(),
            scheduler: JobScheduler::new(),
            classifier: JobClassifier::new(),
            max_concurrent: 5,
        }
    }

    /// 提交作业
    pub fn submit(&mut self, name: &str, job_type: JobType, input: serde_json::Value) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let job = Job {
            id: id.clone(),
            name: name.to_string(),
            job_type,
            status: JobStatus::Pending,
            priority: JobPriority::Normal,
            created_at: now,
            started_at: None,
            completed_at: None,
            input,
            output: None,
            error: None,
            progress: 0,
            tags: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        self.jobs.insert(id.clone(), job);
        self.scheduler.schedule(&id);
        
        id
    }

    /// 获取作业
    pub fn get_job(&self, job_id: &str) -> Option<&Job> {
        self.jobs.get(job_id)
    }

    /// 获取所有作业
    pub fn get_all_jobs(&self) -> Vec<&Job> {
        self.jobs.values().collect()
    }

    /// 获取活跃作业
    pub fn get_active_jobs(&self) -> Vec<&Job> {
        self.jobs.values()
            .filter(|j| j.status == JobStatus::Running || j.status == JobStatus::Pending)
            .collect()
    }

    /// 更新作业状态
    pub fn update_status(&mut self, job_id: &str, status: JobStatus) {
        if let Some(job) = self.jobs.get_mut(job_id) {
            job.status = status.clone();
            
            if status == JobStatus::Running && job.started_at.is_none() {
                job.started_at = Some(chrono::Utc::now().timestamp());
            }
            
            if status == JobStatus::Completed || status == JobStatus::Failed {
                job.completed_at = Some(chrono::Utc::now().timestamp());
            }
        }
    }

    /// 更新作业进度
    pub fn update_progress(&mut self, job_id: &str, progress: u8) {
        if let Some(job) = self.jobs.get_mut(job_id) {
            job.progress = progress.min(100);
        }
    }

    /// 取消作业
    pub fn cancel(&mut self, job_id: &str) {
        self.update_status(job_id, JobStatus::Cancelled);
    }

    /// 删除作业
    pub fn delete(&mut self, job_id: &str) {
        self.jobs.remove(job_id);
    }

    /// 清理已完成的作业
    pub fn cleanup_completed(&mut self) {
        self.jobs.retain(|_, job| {
            job.status != JobStatus::Completed && job.status != JobStatus::Failed
        });
    }
}
