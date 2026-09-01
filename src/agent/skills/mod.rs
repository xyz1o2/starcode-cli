/// Agent Skills - Sub-Agent System
/// 参考 Star CLI 的实现
///
/// 实现任务分解和并行执行
///
/// 核心概念：
/// - SubAgent: 专门负责特定类型任务的子代理
/// - SubTask: 从主任务分解出的子任务
/// - SubTaskResult: 子任务执行结果
///
/// 使用场景：
/// 1. 复杂任务分解：将"重构模块"分解为"分析+编辑+测试"
/// 2. 并行执行：多个子任务同时执行，提升效率
/// 3. 专业化：每个 SubAgent 专注于自己擅长的领域
pub mod analyzer;
pub mod auto_fix;
pub mod custom;
pub mod editor;
pub mod loader;
pub mod navigator;
pub mod search;
pub mod verify;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub use analyzer::AnalyzerAgent;
pub use auto_fix::AutoFixAgent;
pub use custom::register_custom_subagents;
pub use editor::EditorAgent;
pub use navigator::NavigatorAgent;
pub use search::SearchAgent;

/// 子任务定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// 任务 ID
    pub id: String,

    /// 任务目标描述
    pub objective: String,

    /// 任务类型（analyze, edit, search, test）
    pub task_type: String,

    /// 目标对象（文件路径、模块名等）
    pub target: String,

    /// 最大执行步骤数
    pub max_steps: usize,

    /// 额外参数
    pub params: HashMap<String, serde_json::Value>,
}

impl SubTask {
    pub fn new(id: String, objective: String, task_type: String, target: String) -> Self {
        Self {
            id,
            objective,
            task_type,
            target,
            max_steps: 5,
            params: HashMap::new(),
        }
    }

    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_param(mut self, key: String, value: serde_json::Value) -> Self {
        self.params.insert(key, value);
        self
    }
}

/// 子任务执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTaskResult {
    /// 任务 ID
    pub task_id: String,

    /// 是否成功
    pub success: bool,

    /// 执行摘要
    pub summary: String,

    /// 详细结果
    pub details: Option<String>,

    /// 结构化数据
    pub data: Option<serde_json::Value>,

    /// 建议的下一步操作
    pub next_action: Option<String>,

    /// 错误信息
    pub error: Option<String>,
}

impl SubTaskResult {
    pub fn success(task_id: String, summary: String) -> Self {
        Self {
            task_id,
            success: true,
            summary,
            details: None,
            data: None,
            next_action: None,
            error: None,
        }
    }

    pub fn failure(task_id: String, error: String) -> Self {
        Self {
            task_id,
            success: false,
            summary: format!("Task failed: {}", error),
            details: None,
            data: None,
            next_action: None,
            error: Some(error),
        }
    }

    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn with_next_action(mut self, next_action: String) -> Self {
        self.next_action = Some(next_action);
        self
    }
}

/// SubAgent Trait
///
/// 所有子代理必须实现此 trait
#[async_trait]
pub trait SubAgent: Send + Sync {
    /// Sub-Agent 的唯一 ID
    fn id(&self) -> &str;

    /// Sub-Agent 的显示名称
    fn name(&self) -> &str;

    /// Sub-Agent 的能力描述
    fn capabilities(&self) -> Vec<String>;

    /// 检查是否可以处理此任务
    fn can_handle(&self, task: &SubTask) -> bool {
        self.capabilities()
            .iter()
            .any(|cap| task.objective.to_lowercase().contains(&cap.to_lowercase()))
    }

    /// 任务匹配分数（分数越高越优先）
    fn match_score(&self, task: &SubTask) -> i32 {
        let task_type = task.task_type.trim();
        if !task_type.is_empty() {
            if self.id().eq_ignore_ascii_case(task_type) {
                return 1000;
            }
            if self
                .capabilities()
                .iter()
                .any(|cap| cap.eq_ignore_ascii_case(task_type))
            {
                return 700;
            }
        }
        if self.can_handle(task) {
            100
        } else {
            0
        }
    }

    /// 执行子任务
    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>>;
}

/// SubAgent Manager
///
/// 管理所有 SubAgent，负责任务分配和结果聚合。
///
/// 支持共享上下文缓存：设置后，子代理复用父级已加载的项目上下文，
/// 避免每个子代理独立执行昂贵的 ContextEngine 索引。
#[derive(Clone)]
pub struct SubAgentManager {
    agents: Vec<Arc<dyn SubAgent>>,
    /// 共享项目上下文缓存——从父级 Agent 的 ContextEngine 获取
    shared_context: Option<String>,
    /// 项目根目录
    project_root: Option<String>,
}

impl SubAgentManager {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            shared_context: None,
            project_root: None,
        }
    }

    /// 设置共享上下文（由父级 Agent 在创建子代理前调用）
    pub fn set_shared_context(&mut self, context: String, project_root: String) {
        self.shared_context = Some(context);
        self.project_root = Some(project_root);
    }

    /// 获取共享上下文（供子代理在执行时使用）
    pub fn get_shared_context(&self) -> Option<&str> {
        self.shared_context.as_deref()
    }

    /// 获取项目根目录
    pub fn get_project_root(&self) -> Option<&str> {
        self.project_root.as_deref()
    }

    /// 为子任务注入共享上下文，避免子代理重复加载
    pub fn enrich_task_with_context(&self, task: &mut SubTask) {
        if let Some(ctx) = &self.shared_context {
            let ctx_summary = if ctx.chars().count() > 3000 {
                format!("{}...[truncated]", ctx.chars().take(3000).collect::<String>())
            } else {
                ctx.clone()
            };
            task.params.insert(
                "_shared_context".to_string(),
                serde_json::json!(ctx_summary),
            );
        }
        if let Some(root) = &self.project_root {
            task.params.insert(
                "_project_root".to_string(),
                serde_json::json!(root),
            );
        }
    }

    /// 注册一个 SubAgent
    pub fn register(&mut self, agent: Box<dyn SubAgent>) {
        self.agents.push(Arc::from(agent));
    }

    /// 根据 ID 获取 SubAgent
    pub fn get_agent(&self, id: &str) -> Option<Arc<dyn SubAgent>> {
        self.agents
            .iter()
            .rev()
            .find(|agent| agent.id() == id)
            .map(Arc::clone)
    }

    pub fn agent_ids(&self) -> Vec<String> {
        self.agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect()
    }

    /// 根据任务类型选择合适的 SubAgent
    pub fn select_agent(&self, task: &SubTask) -> Option<Arc<dyn SubAgent>> {
        let mut best: Option<(i32, Arc<dyn SubAgent>)> = None;
        for agent in &self.agents {
            let score = agent.match_score(task);
            if score <= 0 {
                continue;
            }
            match &best {
                Some((best_score, _)) if *best_score >= score => {}
                _ => best = Some((score, Arc::clone(agent))),
            }
        }
        best.map(|(_, agent)| agent)
    }

    /// 并行执行多个子任务（自动注入共享上下文）
    pub async fn execute_parallel(
        &self,
        tasks: Vec<SubTask>,
    ) -> Vec<Result<SubTaskResult, String>> {
        let mut handles = Vec::new();

        for mut task in tasks {
            // 注入共享上下文，避免每个子代理重复加载
            self.enrich_task_with_context(&mut task);

            let manager = self.clone();
            let handle = tokio::spawn(async move {
                if let Some(agent) = manager.select_agent(&task) {
                    agent.execute(task).await.map_err(|e| e.to_string())
                } else {
                    Err(format!("No agent found for task: {}", task.objective))
                }
            });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(res) => results.push(res),
                Err(e) => results.push(Err(format!("Task execution panicked: {}", e))),
            }
        }

        results
    }
}
