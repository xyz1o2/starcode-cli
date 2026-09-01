/// 工作流进度追踪

/// 工作流进度
pub struct WorkflowProgress {
    /// 工作流名称
    workflow_name: String,
    /// 总步骤数
    total_steps: usize,
    /// 当前步骤
    current_step: usize,
    /// 步骤状态
    step_statuses: Vec<StepProgress>,
    /// 开始时间
    started_at: u64,
}

/// 步骤进度
#[derive(Debug, Clone)]
pub struct StepProgress {
    /// 步骤名称
    pub name: String,
    /// 状态
    pub status: StepProgressStatus,
    /// 进度（0-100）
    pub progress: u8,
}

/// 步骤进度状态
#[derive(Debug, Clone, PartialEq)]
pub enum StepProgressStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl WorkflowProgress {
    /// 创建新的工作流进度
    pub fn new() -> Self {
        Self {
            workflow_name: String::new(),
            total_steps: 0,
            current_step: 0,
            step_statuses: Vec::new(),
            started_at: 0,
        }
    }

    /// 开始追踪
    pub fn start(&mut self, workflow_name: &str, total_steps: usize) {
        self.workflow_name = workflow_name.to_string();
        self.total_steps = total_steps;
        self.current_step = 0;
        self.step_statuses = (0..total_steps)
            .map(|i| StepProgress {
                name: format!("Step {}", i + 1),
                status: StepProgressStatus::Pending,
                progress: 0,
            })
            .collect();
        self.started_at = now_millis();
    }

    /// 更新步骤
    pub fn update_step(&mut self, step_index: usize, step_name: &str) {
        if step_index < self.step_statuses.len() {
            self.step_statuses[step_index].name = step_name.to_string();
            self.step_statuses[step_index].status = StepProgressStatus::Running;
            self.current_step = step_index;
        }
    }

    /// 完成步骤
    pub fn complete_step(&mut self, step_index: usize) {
        if step_index < self.step_statuses.len() {
            self.step_statuses[step_index].status = StepProgressStatus::Completed;
            self.step_statuses[step_index].progress = 100;
        }
    }

    /// 失败步骤
    pub fn fail_step(&mut self, step_index: usize, _error: &str) {
        if step_index < self.step_statuses.len() {
            self.step_statuses[step_index].status = StepProgressStatus::Failed;
        }
    }

    /// 完成工作流
    pub fn complete(&mut self) {
        // 所有步骤标记为完成
        for step in &mut self.step_statuses {
            if step.status == StepProgressStatus::Running {
                step.status = StepProgressStatus::Completed;
                step.progress = 100;
            }
        }
    }

    /// 获取整体进度
    pub fn get_overall_progress(&self) -> u8 {
        if self.total_steps == 0 {
            return 0;
        }

        let completed = self
            .step_statuses
            .iter()
            .filter(|s| s.status == StepProgressStatus::Completed)
            .count();

        (completed as f64 / self.total_steps as f64 * 100.0) as u8
    }

    /// 获取步骤状态
    pub fn get_step_statuses(&self) -> &[StepProgress] {
        &self.step_statuses
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
