/// Daemon模式增强
///
/// 对标claude-code-main的src/daemon/
/// 长驻后台进程管理，支持Worker注册、状态监控和任务调度
pub mod worker_registry;

pub use worker_registry::{WorkerEntry, WorkerHealth, WorkerRegistry};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Daemon状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DaemonStatus {
    /// 停止
    Stopped,
    /// 运行中
    Running,
    /// 暂停
    Paused,
    /// 错误
    Error(String),
}

/// Daemon配置
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// 是否启用
    pub enabled: bool,
    /// PID文件路径
    pub pid_file: Option<String>,
    /// 日志文件路径
    pub log_file: Option<String>,
    /// 最大运行时间（秒）
    pub max_runtime_secs: u64,
    /// 健康检查间隔（秒）
    pub health_check_interval_secs: u64,
    /// 最大Worker数
    pub max_workers: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            pid_file: None,
            log_file: None,
            max_runtime_secs: 86400, // 24 hours
            health_check_interval_secs: 60,
            max_workers: 10,
        }
    }
}

impl DaemonConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_DAEMON_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let pid_file = std::env::var("STAR_DAEMON_PID_FILE").ok();
        let log_file = std::env::var("STAR_DAEMON_LOG_FILE").ok();

        let max_runtime_secs = std::env::var("STAR_DAEMON_MAX_RUNTIME")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(86400);

        let health_check_interval_secs = std::env::var("STAR_DAEMON_HEALTH_CHECK_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let max_workers = std::env::var("STAR_DAEMON_MAX_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        Self {
            enabled,
            pid_file,
            log_file,
            max_runtime_secs,
            health_check_interval_secs,
            max_workers,
        }
    }
}

/// Daemon任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonTask {
    /// 任务ID
    pub id: String,
    /// 任务名称
    pub name: String,
    /// 任务类型
    pub task_type: String,
    /// 状态
    pub status: DaemonStatus,
    /// 启动时间
    pub started_at: Option<i64>,
    /// 完成时间
    pub completed_at: Option<i64>,
    /// 结果
    pub result: Option<String>,
    /// Worker ID
    pub worker_id: Option<String>,
}

/// Daemon管理器
pub struct DaemonManager {
    config: DaemonConfig,
    status: DaemonStatus,
    tasks: HashMap<String, DaemonTask>,
    /// Worker注册表
    worker_registry: WorkerRegistry,
    /// 启动时间
    started_at: Option<i64>,
    /// 最后健康检查时间
    last_health_check: Option<i64>,
}

impl DaemonManager {
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            worker_registry: WorkerRegistry::new(config.max_workers),
            config,
            status: DaemonStatus::Stopped,
            tasks: HashMap::new(),
            started_at: None,
            last_health_check: None,
        }
    }

    /// 启动daemon
    pub fn start(&mut self) -> Result<(), String> {
        if !self.config.enabled {
            return Err("Daemon is not enabled".to_string());
        }

        self.status = DaemonStatus::Running;
        self.started_at = Some(chrono::Utc::now().timestamp());

        // 写入PID文件
        if let Some(pid_file) = &self.config.pid_file {
            let pid = std::process::id();
            std::fs::write(pid_file, pid.to_string())
                .map_err(|e| format!("Failed to write PID file: {}", e))?;
        }

        Ok(())
    }

    /// 停止daemon
    pub fn stop(&mut self) {
        self.status = DaemonStatus::Stopped;
        self.started_at = None;

        // 清理PID文件
        if let Some(pid_file) = &self.config.pid_file {
            let _ = std::fs::remove_file(pid_file);
        }

        // 停止所有Worker
        self.worker_registry.stop_all();
    }

    /// 注册Worker
    pub fn register_worker(&mut self, name: &str, worker_type: &str) -> String {
        self.worker_registry.register(name, worker_type)
    }

    /// 提交任务
    pub fn submit_task(&mut self, name: &str, task_type: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();

        let task = DaemonTask {
            id: id.clone(),
            name: name.to_string(),
            task_type: task_type.to_string(),
            status: DaemonStatus::Stopped,
            started_at: None,
            completed_at: None,
            result: None,
            worker_id: None,
        };

        self.tasks.insert(id.clone(), task);
        id
    }

    /// 分配任务给Worker
    pub fn assign_task(&mut self, task_id: &str, worker_id: &str) -> Result<(), String> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task not found: {}", task_id))?;

        let worker = self
            .worker_registry
            .get_worker(worker_id)
            .ok_or_else(|| format!("Worker not found: {}", worker_id))?;

        task.status = DaemonStatus::Running;
        task.started_at = Some(chrono::Utc::now().timestamp());
        task.worker_id = Some(worker_id.to_string());

        Ok(())
    }

    /// 完成任务
    pub fn complete_task(&mut self, task_id: &str, result: Option<String>) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.status = DaemonStatus::Stopped;
            task.completed_at = Some(chrono::Utc::now().timestamp());
            task.result = result;
        }
    }

    /// 执行健康检查
    pub fn health_check(&mut self) {
        let now = chrono::Utc::now().timestamp();
        self.last_health_check = Some(now);

        // 检查Worker健康状态
        self.worker_registry.health_check();
    }

    /// 获取任务状态
    pub fn get_task_status(&self, task_id: &str) -> Option<&DaemonTask> {
        self.tasks.get(task_id)
    }

    /// 获取daemon状态
    pub fn status(&self) -> &DaemonStatus {
        &self.status
    }

    /// 获取所有任务
    pub fn get_all_tasks(&self) -> Vec<&DaemonTask> {
        self.tasks.values().collect()
    }

    /// 获取Worker注册表
    pub fn worker_registry(&self) -> &WorkerRegistry {
        &self.worker_registry
    }

    /// 获取统计信息
    pub fn get_statistics(&self) -> DaemonStatistics {
        DaemonStatistics {
            status: self.status.clone(),
            uptime_seconds: self
                .started_at
                .map(|t| chrono::Utc::now().timestamp() - t)
                .unwrap_or(0),
            total_tasks: self.tasks.len() as u64,
            running_tasks: self
                .tasks
                .values()
                .filter(|t| t.status == DaemonStatus::Running)
                .count() as u64,
            total_workers: self.worker_registry.worker_count(),
            healthy_workers: self.worker_registry.healthy_worker_count(),
        }
    }
}

/// Daemon统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatistics {
    pub status: DaemonStatus,
    pub uptime_seconds: i64,
    pub total_tasks: u64,
    pub running_tasks: u64,
    pub total_workers: usize,
    pub healthy_workers: usize,
}
