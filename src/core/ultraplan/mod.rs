//! Ultraplan 高级规划模块
//!
//! 对标 Claude Code 的 ultraplan.md：
//! - 关键字检测触发
//! - CCR 远程会话
//! - ExitPlanModeScanner

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Ultraplan 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UltraplanConfig {
    pub enabled: bool,
    pub auto_trigger_keywords: Vec<String>,
    pub max_plan_depth: usize,
    pub enable_remote_planning: bool,
    pub remote_endpoint: Option<String>,
}

impl Default for UltraplanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_trigger_keywords: vec![
                "refactor".to_string(),
                "architecture".to_string(),
                "redesign".to_string(),
                "migrate".to_string(),
                "restructure".to_string(),
            ],
            max_plan_depth: 5,
            enable_remote_planning: false,
            remote_endpoint: None,
        }
    }
}

/// Ultraplan 计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ultraplan {
    pub id: String,
    pub title: String,
    pub description: String,
    pub phases: Vec<PlanPhase>,
    pub estimated_effort: EffortEstimate,
    pub risks: Vec<Risk>,
    pub dependencies: Vec<String>,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPhase {
    pub name: String,
    pub description: String,
    pub steps: Vec<PlanStep>,
    pub estimated_hours: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub description: String,
    pub files_affected: Vec<String>,
    pub tools_needed: Vec<String>,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffortEstimate {
    pub total_hours: f32,
    pub complexity: Complexity,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Complexity {
    Low,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Risk {
    pub description: String,
    pub severity: RiskSeverity,
    pub mitigation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Draft,
    Approved,
    InProgress,
    Completed,
    Cancelled,
}

/// Ultraplan 管理器
pub struct UltraplanManager {
    config: UltraplanConfig,
    plans: Vec<Ultraplan>,
}

impl UltraplanManager {
    pub fn new(config: UltraplanConfig) -> Self {
        Self {
            config,
            plans: Vec::new(),
        }
    }

    /// 检测输入是否需要 ultraplan
    pub fn should_trigger(&self, input: &str) -> bool {
        if !self.config.enabled {
            return false;
        }

        let input_lower = input.to_lowercase();
        self.config
            .auto_trigger_keywords
            .iter()
            .any(|kw| input_lower.contains(&kw.to_lowercase()))
    }

    /// 创建计划
    pub fn create_plan(
        &mut self,
        title: String,
        description: String,
        phases: Vec<PlanPhase>,
    ) -> Ultraplan {
        let plan = Ultraplan {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            title,
            description,
            phases,
            estimated_effort: EffortEstimate {
                total_hours: 0.0,
                complexity: Complexity::Medium,
                confidence: 0.5,
            },
            risks: Vec::new(),
            dependencies: Vec::new(),
            status: PlanStatus::Draft,
        };

        self.plans.push(plan.clone());
        plan
    }

    /// 获取计划
    pub fn get_plan(&self, plan_id: &str) -> Option<&Ultraplan> {
        self.plans.iter().find(|p| p.id == plan_id)
    }

    /// 列出所有计划
    pub fn list_plans(&self) -> &[Ultraplan] {
        &self.plans
    }

    /// 更新计划状态
    pub fn update_status(&mut self, plan_id: &str, status: PlanStatus) {
        if let Some(plan) = self.plans.iter_mut().find(|p| p.id == plan_id) {
            plan.status = status;
        }
    }

    /// 标记步骤完成
    pub fn complete_step(&mut self, plan_id: &str, phase_idx: usize, step_idx: usize) {
        if let Some(plan) = self.plans.iter_mut().find(|p| p.id == plan_id) {
            if let Some(phase) = plan.phases.get_mut(phase_idx) {
                if let Some(step) = phase.steps.get_mut(step_idx) {
                    step.completed = true;
                }
            }
        }
    }

    /// 检查计划是否完成
    pub fn is_plan_complete(&self, plan_id: &str) -> bool {
        self.plans
            .iter()
            .find(|p| p.id == plan_id)
            .map(|p| {
                p.phases
                    .iter()
                    .all(|phase| phase.steps.iter().all(|s| s.completed))
            })
            .unwrap_or(false)
    }

    /// 获取计划进度
    pub fn plan_progress(&self, plan_id: &str) -> (usize, usize) {
        self.plans
            .iter()
            .find(|p| p.id == plan_id)
            .map(|p| {
                let total = p.phases.iter().map(|ph| ph.steps.len()).sum();
                let completed = p
                    .phases
                    .iter()
                    .map(|ph| ph.steps.iter().filter(|s| s.completed).count())
                    .sum();
                (completed, total)
            })
            .unwrap_or((0, 0))
    }
}

/// ExitPlanModeScanner — 扫描计划完成状态
pub struct ExitPlanModeScanner;

impl ExitPlanModeScanner {
    /// 扫描是否所有任务都已完成
    pub fn scan(plan: &Ultraplan) -> ScanResult {
        let total_steps: usize = plan.phases.iter().map(|p| p.steps.len()).sum();
        let completed_steps: usize = plan
            .phases
            .iter()
            .map(|p| p.steps.iter().filter(|s| s.completed).count())
            .sum();

        let remaining_steps: Vec<String> = plan
            .phases
            .iter()
            .flat_map(|p| {
                p.steps
                    .iter()
                    .filter(|s| !s.completed)
                    .map(|s| format!("{}: {}", p.name, s.description))
                    .collect::<Vec<_>>()
            })
            .collect();

        ScanResult {
            all_complete: completed_steps == total_steps && total_steps > 0,
            total_steps,
            completed_steps,
            remaining_steps,
        }
    }
}

#[derive(Debug)]
pub struct ScanResult {
    pub all_complete: bool,
    pub total_steps: usize,
    pub completed_steps: usize,
    pub remaining_steps: Vec<String>,
}
