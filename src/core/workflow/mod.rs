//! Workflow 脚本模块
//!
//! 对标 Claude Code 的 workflow-scripts.md：
//! - .claude/workflows/ 脚本执行
//! - 工作流定义
//! - 步骤编排
//! - 进度追踪
//! - 持久化

pub mod engine;
pub mod persistence;
pub mod progress;
pub mod registry;

pub use engine::WorkflowEngine;
pub use persistence::WorkflowPersistence;
pub use progress::WorkflowProgress;
pub use registry::WorkflowRegistry;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 工作流定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    pub version: String,
    pub steps: Vec<WorkflowStep>,
    pub triggers: Vec<WorkflowTrigger>,
    pub env: std::collections::HashMap<String, String>,
}

/// 工作流步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub name: String,
    pub step_type: StepType,
    pub command: Option<String>,
    pub tool: Option<String>,
    pub params: Option<Value>,
    pub condition: Option<String>,
    pub on_failure: FailureAction,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepType {
    /// Shell 命令
    Shell,
    /// 工具调用
    Tool,
    /// LLM 提示
    Prompt,
    /// 条件判断
    Condition,
    /// 循环
    Loop,
    /// 并行
    Parallel,
    /// 子工作流
    SubWorkflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureAction {
    /// 停止
    Stop,
    /// 继续
    Continue,
    /// 重试
    Retry { max_attempts: u32 },
    /// 跳过
    Skip,
    /// 回退
    Rollback,
}

/// 工作流触发器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowTrigger {
    /// 手动触发
    Manual,
    /// 文件变更
    FileChange { pattern: String },
    /// 定时触发
    Schedule { cron: String },
    /// 事件触发
    Event { event_type: String },
}

/// 工作流执行上下文
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    pub workflow_name: String,
    pub variables: std::collections::HashMap<String, Value>,
    pub current_step: usize,
    pub started_at: u64,
    pub log: Vec<WorkflowLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowLogEntry {
    pub step_name: String,
    pub status: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Workflow 管理器
pub struct WorkflowManager {
    workflows_dir: PathBuf,
    workflows: Vec<Workflow>,
    /// 工作流引擎
    engine: WorkflowEngine,
    /// 进度追踪器
    progress: WorkflowProgress,
    /// 持久化管理器
    persistence: WorkflowPersistence,
    /// 注册表
    registry: WorkflowRegistry,
}

impl WorkflowManager {
    pub fn new(project_root: &Path) -> Self {
        Self {
            workflows_dir: project_root.join(".claude").join("workflows"),
            workflows: Vec::new(),
            engine: WorkflowEngine::new(),
            progress: WorkflowProgress::new(),
            persistence: WorkflowPersistence::new(project_root),
            registry: WorkflowRegistry::new(),
        }
    }

    /// 加载工作流定义
    pub fn load_workflows(&mut self) -> Result<usize, String> {
        if !self.workflows_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        for entry in std::fs::read_dir(&self.workflows_dir)
            .map_err(|e| format!("Failed to read workflows dir: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(workflow) = serde_json::from_str::<Workflow>(&content) {
                        self.workflows.push(workflow);
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// 获取工作流
    pub fn get_workflow(&self, name: &str) -> Option<&Workflow> {
        self.workflows.iter().find(|w| w.name == name)
    }

    /// 列出所有工作流
    pub fn list_workflows(&self) -> &[Workflow] {
        &self.workflows
    }

    /// 执行工作流
    pub async fn execute(
        &mut self,
        workflow_name: &str,
        variables: std::collections::HashMap<String, Value>,
    ) -> Result<WorkflowContext, String> {
        let workflow = self
            .get_workflow(workflow_name)
            .ok_or_else(|| format!("Workflow '{}' not found", workflow_name))?
            .clone();

        let mut context = WorkflowContext {
            workflow_name: workflow.name.clone(),
            variables,
            current_step: 0,
            started_at: now_secs(),
            log: Vec::new(),
        };

        // 开始进度追踪
        self.progress.start(&workflow.name, workflow.steps.len());

        for (i, step) in workflow.steps.iter().enumerate() {
            context.current_step = i;

            // 更新进度
            self.progress.update_step(i, &step.name);

            // 检查条件
            if let Some(condition) = &step.condition {
                if !evaluate_condition(condition, &context) {
                    context.log.push(WorkflowLogEntry {
                        step_name: step.name.clone(),
                        status: "skipped".to_string(),
                        output: Some("Condition not met".to_string()),
                        error: None,
                        duration_ms: 0,
                    });
                    continue;
                }
            }

            let start = std::time::Instant::now();
            let result = execute_step(step, &mut context).await;
            let duration = start.elapsed().as_millis() as u64;

            match result {
                Ok(output) => {
                    context.log.push(WorkflowLogEntry {
                        step_name: step.name.clone(),
                        status: "success".to_string(),
                        output: Some(output),
                        error: None,
                        duration_ms: duration,
                    });
                    self.progress.complete_step(i);
                }
                Err(error) => {
                    context.log.push(WorkflowLogEntry {
                        step_name: step.name.clone(),
                        status: "failed".to_string(),
                        output: None,
                        error: Some(error.clone()),
                        duration_ms: duration,
                    });

                    match &step.on_failure {
                        FailureAction::Stop => {
                            self.progress.fail_step(i, &error);
                            return Err(error);
                        }
                        FailureAction::Continue => continue,
                        FailureAction::Retry { max_attempts } => {
                            // 重试逻辑
                            let mut attempts = 0;
                            while attempts < *max_attempts {
                                attempts += 1;
                                // 重试...
                            }
                        }
                        FailureAction::Skip => continue,
                        FailureAction::Rollback => {
                            // 回退逻辑
                            self.progress.fail_step(i, &error);
                            return Err(format!("Rollback required: {}", error));
                        }
                    }
                }
            }
        }

        // 完成进度追踪
        self.progress.complete();

        // 持久化执行结果
        self.persistence.save_execution(&context)?;

        Ok(context)
    }

    /// 创建默认工作流模板
    pub fn create_template(name: &str) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: format!("Workflow: {}", name),
            version: "1.0".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "run_tests".to_string(),
                    step_type: StepType::Shell,
                    command: Some("cargo test".to_string()),
                    tool: None,
                    params: None,
                    condition: None,
                    on_failure: FailureAction::Stop,
                    timeout_secs: Some(300),
                },
                WorkflowStep {
                    name: "run_lint".to_string(),
                    step_type: StepType::Shell,
                    command: Some("cargo clippy".to_string()),
                    tool: None,
                    params: None,
                    condition: None,
                    on_failure: FailureAction::Continue,
                    timeout_secs: Some(120),
                },
            ],
            triggers: vec![WorkflowTrigger::Manual],
            env: std::collections::HashMap::new(),
        }
    }
}

fn evaluate_condition(condition: &str, context: &WorkflowContext) -> bool {
    // 简单条件评估
    !condition.is_empty()
}

async fn execute_step(
    step: &WorkflowStep,
    context: &mut WorkflowContext,
) -> Result<String, String> {
    match step.step_type {
        StepType::Shell => {
            if let Some(cmd) = &step.command {
                let output = std::process::Command::new("sh")
                    .args(["-c", cmd])
                    .output()
                    .map_err(|e| format!("Shell execution failed: {}", e))?;

                if output.status.success() {
                    Ok(String::from_utf8_lossy(&output.stdout).to_string())
                } else {
                    Err(String::from_utf8_lossy(&output.stderr).to_string())
                }
            } else {
                Err("No command specified for shell step".to_string())
            }
        }
        StepType::Tool => {
            // 工具调用需要集成到 Agent 系统
            Ok("Tool step executed (placeholder)".to_string())
        }
        StepType::Prompt => Ok("Prompt step executed (placeholder)".to_string()),
        _ => Ok(format!(
            "Step '{}' completed (type not implemented)",
            step.name
        )),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
