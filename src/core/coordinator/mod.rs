/// 协调器模式
/// 
/// 对标claude-code-main的src/coordinator/
/// 多Agent协调和worker管理，支持任务调度、负载均衡和故障恢复

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Worker状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerStatus {
    /// 空闲
    Idle,
    /// 运行中
    Running,
    /// 完成
    Completed,
    /// 失败
    Failed,
    /// 暂停
    Paused,
}

/// Worker信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    /// Worker ID
    pub id: String,
    /// Worker类型
    pub worker_type: String,
    /// 状态
    pub status: WorkerStatus,
    /// 当前任务
    pub current_task: Option<String>,
    /// 启动时间
    pub started_at: i64,
    /// 完成任务数
    pub completed_tasks: u32,
    /// 失败任务数
    pub failed_tasks: u32,
    /// 平均执行时间（毫秒）
    pub avg_execution_time_ms: u64,
    /// 最后活动时间
    pub last_activity: i64,
}

/// 任务定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// 任务ID
    pub id: String,
    /// 任务名称
    pub name: String,
    /// 任务类型
    pub task_type: String,
    /// 优先级
    pub priority: TaskPriority,
    /// 输入参数
    pub input: serde_json::Value,
    /// 创建时间
    pub created_at: i64,
    /// 超时时间（秒）
    pub timeout_secs: u64,
    /// 依赖任务
    pub dependencies: Vec<String>,
}

/// 任务优先级
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// 任务结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// 任务ID
    pub task_id: String,
    /// Worker ID
    pub worker_id: String,
    /// 是否成功
    pub success: bool,
    /// 输出
    pub output: Option<serde_json::Value>,
    /// 错误信息
    pub error: Option<String>,
    /// 执行时间（毫秒）
    pub execution_time_ms: u64,
}

/// 协调器配置
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// 最大worker数
    pub max_workers: usize,
    /// 任务超时（秒）
    pub task_timeout_secs: u64,
    /// 是否启用并行执行
    pub parallel_execution: bool,
    /// 负载均衡策略
    pub load_balancing_strategy: LoadBalancingStrategy,
    /// 是否启用故障恢复
    pub fault_recovery: bool,
}

/// 负载均衡策略
#[derive(Debug, Clone)]
pub enum LoadBalancingStrategy {
    /// 轮询
    RoundRobin,
    /// 最少连接
    LeastConnections,
    /// 加权
    Weighted,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            max_workers: 5,
            task_timeout_secs: 300,
            parallel_execution: true,
            load_balancing_strategy: LoadBalancingStrategy::LeastConnections,
            fault_recovery: true,
        }
    }
}

/// 协调器
pub struct Coordinator {
    config: CoordinatorConfig,
    workers: HashMap<String, WorkerInfo>,
    task_queue: Vec<Task>,
    task_results: Vec<TaskResult>,
    /// 当前轮询索引
    round_robin_index: usize,
}

impl Coordinator {
    pub fn new(config: CoordinatorConfig) -> Self {
        Self {
            config,
            workers: HashMap::new(),
            task_queue: Vec::new(),
            task_results: Vec::new(),
            round_robin_index: 0,
        }
    }

    /// 注册worker
    pub fn register_worker(&mut self, worker_type: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        
        let worker = WorkerInfo {
            id: id.clone(),
            worker_type: worker_type.to_string(),
            status: WorkerStatus::Idle,
            current_task: None,
            started_at: now,
            completed_tasks: 0,
            failed_tasks: 0,
            avg_execution_time_ms: 0,
            last_activity: now,
        };

        self.workers.insert(id.clone(), worker);
        id
    }

    /// 提交任务
    pub fn submit_task(&mut self, task: Task) {
        self.task_queue.push(task);
    }

    /// 分配任务
    pub fn assign_task(&mut self, task_name: &str) -> Option<String> {
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            name: task_name.to_string(),
            task_type: "default".to_string(),
            priority: TaskPriority::Normal,
            input: serde_json::json!({}),
            created_at: chrono::Utc::now().timestamp(),
            timeout_secs: self.config.task_timeout_secs,
            dependencies: Vec::new(),
        };

        self.submit_task(task);
        self.schedule_next()
    }

    /// 调度下一个任务
    pub fn schedule_next(&mut self) -> Option<String> {
        // 按优先级排序
        self.task_queue.sort_by(|a, b| b.priority.cmp(&a.priority));

        let task = self.task_queue.first()?.clone();
        
        // 根据负载均衡策略选择worker
        let worker_id = match self.config.load_balancing_strategy {
            LoadBalancingStrategy::RoundRobin => self.select_round_robin(),
            LoadBalancingStrategy::LeastConnections => self.select_least_connections(),
            LoadBalancingStrategy::Weighted => self.select_weighted(),
        }?;

        if let Some(worker) = self.workers.get_mut(&worker_id) {
            worker.status = WorkerStatus::Running;
            worker.current_task = Some(task.name.clone());
            worker.last_activity = chrono::Utc::now().timestamp();
            
            self.task_queue.remove(0);
            Some(worker_id)
        } else {
            None
        }
    }

    /// 轮询选择
    fn select_round_robin(&mut self) -> Option<String> {
        let idle_workers: Vec<&WorkerInfo> = self.workers.values()
            .filter(|w| w.status == WorkerStatus::Idle)
            .collect();

        if idle_workers.is_empty() {
            return None;
        }

        let index = self.round_robin_index % idle_workers.len();
        self.round_robin_index += 1;
        
        Some(idle_workers[index].id.clone())
    }

    /// 最少连接选择
    fn select_least_connections(&self) -> Option<String> {
        self.workers.values()
            .filter(|w| w.status == WorkerStatus::Idle)
            .min_by_key(|w| w.completed_tasks + w.failed_tasks)
            .map(|w| w.id.clone())
    }

    /// 加权选择
    fn select_weighted(&self) -> Option<String> {
        self.workers.values()
            .filter(|w| w.status == WorkerStatus::Idle)
            .min_by_key(|w| w.avg_execution_time_ms)
            .map(|w| w.id.clone())
    }

    /// 完成任务
    pub fn complete_task(&mut self, worker_id: &str, success: bool, output: Option<serde_json::Value>, error: Option<String>) {
        if let Some(worker) = self.workers.get_mut(worker_id) {
            let execution_time = chrono::Utc::now().timestamp() - worker.last_activity;
            
            worker.status = WorkerStatus::Idle;
            worker.current_task = None;
            worker.last_activity = chrono::Utc::now().timestamp();
            
            if success {
                worker.completed_tasks += 1;
            } else {
                worker.failed_tasks += 1;
            }

            // 更新平均执行时间
            let total_tasks = worker.completed_tasks + worker.failed_tasks;
            if total_tasks > 0 {
                worker.avg_execution_time_ms = 
                    (worker.avg_execution_time_ms * (total_tasks - 1) as u64 + execution_time as u64 * 1000) / total_tasks as u64;
            }

            // 记录结果
            let result = TaskResult {
                task_id: worker.current_task.clone().unwrap_or_default(),
                worker_id: worker_id.to_string(),
                success,
                output,
                error,
                execution_time_ms: execution_time as u64 * 1000,
            };
            self.task_results.push(result);
        }

        // 尝试调度下一个任务
        self.schedule_next();
    }

    /// 获取worker信息
    pub fn get_worker(&self, worker_id: &str) -> Option<&WorkerInfo> {
        self.workers.get(worker_id)
    }

    /// 获取所有worker
    pub fn get_all_workers(&self) -> Vec<&WorkerInfo> {
        self.workers.values().collect()
    }

    /// 获取队列长度
    pub fn queue_length(&self) -> usize {
        self.task_queue.len()
    }

    /// 获取任务结果
    pub fn get_task_results(&self) -> &[TaskResult] {
        &self.task_results
    }

    /// 获取统计信息
    pub fn get_statistics(&self) -> CoordinatorStatistics {
        let total_workers = self.workers.len();
        let idle_workers = self.workers.values().filter(|w| w.status == WorkerStatus::Idle).count();
        let running_workers = self.workers.values().filter(|w| w.status == WorkerStatus::Running).count();
        let total_completed: u32 = self.workers.values().map(|w| w.completed_tasks).sum();
        let total_failed: u32 = self.workers.values().map(|w| w.failed_tasks).sum();

        CoordinatorStatistics {
            total_workers,
            idle_workers,
            running_workers,
            pending_tasks: self.task_queue.len(),
            completed_tasks: total_completed,
            failed_tasks: total_failed,
        }
    }
}

/// 协调器统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorStatistics {
    pub total_workers: usize,
    pub idle_workers: usize,
    pub running_workers: usize,
    pub pending_tasks: usize,
    pub completed_tasks: u32,
    pub failed_tasks: u32,
}
