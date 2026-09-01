/// Worker注册表
///
/// 对标claude-code-main的src/daemon/workerRegistry.ts
/// 管理后台Worker的注册、状态和健康检查
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Worker健康状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerHealth {
    /// 健康
    Healthy,
    /// 不健康
    Unhealthy,
    /// 未知
    Unknown,
}

/// Worker条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEntry {
    /// Worker ID
    pub id: String,
    /// Worker名称
    pub name: String,
    /// Worker类型
    pub worker_type: String,
    /// 状态
    pub status: WorkerStatus,
    /// 健康状态
    pub health: WorkerHealth,
    /// 注册时间
    pub registered_at: i64,
    /// 最后心跳时间
    pub last_heartbeat: i64,
    /// 处理的任务数
    pub tasks_processed: u64,
    /// 错误数
    pub errors: u64,
}

/// Worker状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerStatus {
    /// 空闲
    Idle,
    /// 忙碌
    Busy,
    /// 停止
    Stopped,
    /// 错误
    Error,
}

/// Worker注册表
pub struct WorkerRegistry {
    /// Worker映射
    workers: HashMap<String, WorkerEntry>,
    /// 最大Worker数
    max_workers: usize,
}

impl WorkerRegistry {
    /// 创建新的Worker注册表
    pub fn new(max_workers: usize) -> Self {
        Self {
            workers: HashMap::new(),
            max_workers,
        }
    }

    /// 注册Worker
    pub fn register(&mut self, name: &str, worker_type: &str) -> String {
        if self.workers.len() >= self.max_workers {
            // 移除最旧的空闲Worker
            if let Some(oldest_id) = self
                .workers
                .values()
                .filter(|w| w.status == WorkerStatus::Idle)
                .min_by_key(|w| w.last_heartbeat)
                .map(|w| w.id.clone())
            {
                self.workers.remove(&oldest_id);
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let entry = WorkerEntry {
            id: id.clone(),
            name: name.to_string(),
            worker_type: worker_type.to_string(),
            status: WorkerStatus::Idle,
            health: WorkerHealth::Healthy,
            registered_at: now,
            last_heartbeat: now,
            tasks_processed: 0,
            errors: 0,
        };

        self.workers.insert(id.clone(), entry);
        id
    }

    /// 获取Worker
    pub fn get_worker(&self, worker_id: &str) -> Option<&WorkerEntry> {
        self.workers.get(worker_id)
    }

    /// 获取所有Worker
    pub fn get_all_workers(&self) -> Vec<&WorkerEntry> {
        self.workers.values().collect()
    }

    /// 更新Worker状态
    pub fn update_status(&mut self, worker_id: &str, status: WorkerStatus) {
        if let Some(worker) = self.workers.get_mut(worker_id) {
            worker.status = status;
            worker.last_heartbeat = chrono::Utc::now().timestamp();
        }
    }

    /// 更新Worker心跳
    pub fn heartbeat(&mut self, worker_id: &str) {
        if let Some(worker) = self.workers.get_mut(worker_id) {
            worker.last_heartbeat = chrono::Utc::now().timestamp();
        }
    }

    /// 健康检查
    pub fn health_check(&mut self) {
        let now = chrono::Utc::now().timestamp();
        let timeout = 300; // 5分钟超时

        for worker in self.workers.values_mut() {
            if now - worker.last_heartbeat > timeout {
                worker.health = WorkerHealth::Unhealthy;
            } else {
                worker.health = WorkerHealth::Healthy;
            }
        }
    }

    /// 停止所有Worker
    pub fn stop_all(&mut self) {
        for worker in self.workers.values_mut() {
            worker.status = WorkerStatus::Stopped;
        }
    }

    /// Worker数量
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// 健康Worker数量
    pub fn healthy_worker_count(&self) -> usize {
        self.workers
            .values()
            .filter(|w| w.health == WorkerHealth::Healthy)
            .count()
    }
}
