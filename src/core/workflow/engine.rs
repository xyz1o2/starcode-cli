/// 工作流引擎
use super::{StepType, Workflow, WorkflowContext, WorkflowStep};

/// 工作流引擎
pub struct WorkflowEngine {
    /// 执行历史
    execution_history: Vec<WorkflowExecution>,
}

/// 工作流执行记录
#[derive(Debug, Clone)]
pub struct WorkflowExecution {
    /// 执行ID
    pub id: String,
    /// 工作流名称
    pub workflow_name: String,
    /// 开始时间
    pub started_at: u64,
    /// 结束时间
    pub completed_at: Option<u64>,
    /// 状态
    pub status: ExecutionStatus,
    /// 步骤结果
    pub step_results: Vec<StepResult>,
}

/// 执行状态
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// 步骤结果
#[derive(Debug, Clone)]
pub struct StepResult {
    /// 步骤名称
    pub step_name: String,
    /// 状态
    pub status: StepStatus,
    /// 输出
    pub output: Option<String>,
    /// 错误
    pub error: Option<String>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
}

/// 步骤状态
#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl WorkflowEngine {
    /// 创建新的工作流引擎
    pub fn new() -> Self {
        Self {
            execution_history: Vec::new(),
        }
    }

    /// 获取执行历史
    pub fn get_execution_history(&self) -> &[WorkflowExecution] {
        &self.execution_history
    }

    /// 获取最近的执行
    pub fn get_recent_execution(&self, workflow_name: &str) -> Option<&WorkflowExecution> {
        self.execution_history
            .iter()
            .filter(|e| e.workflow_name == workflow_name)
            .last()
    }
}
