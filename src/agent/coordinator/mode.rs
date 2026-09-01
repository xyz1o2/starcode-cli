//! Coordinator Mode 五段状态机。
//!
//! 对标 CCB coordinator-and-swarm.mdx §Coordinator Mode 五段状态机：
//! ① 启用检测  →  ② 恢复对齐  →  ③ Prompt 注入  →  ④ Worker 生命周期  →  ⑤ 结果综合

use crate::agent::subagent::notification::TaskNotification;
use crate::agent::subagent::runner::AsyncSubagentRunner;
use crate::core::agents::SubAgentRequest as CoreSubAgentRequest;
use crate::core::config::Config;
use crate::types::ApprovalMode;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ── WorkerHandle ─────────────────────────────────────────────────────────

/// Worker 执行状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

/// Worker 运行时句柄
#[derive(Debug, Clone)]
pub struct WorkerHandle {
    pub agent_id: String,
    pub description: String,
    pub status: WorkerStatus,
    pub spawn_time: chrono::DateTime<Utc>,
}

// ── CoordinatorMode ──────────────────────────────────────────────────────

/// Coordinator Mode 状态
#[derive(Clone)]
pub struct CoordinatorMode {
    /// 当前是否处于协调者模式
    pub active: bool,
    /// 进入协调者模式前的权限模式（退出时恢复）
    pub previous_mode: Option<ApprovalMode>,
    /// Worker 运行时注册表（Arc<Mutex<>> 确保主循环读 + tokio::spawn 写安全）
    pub worker_registry: Arc<Mutex<HashMap<String, WorkerHandle>>>,
    /// 异步 SubAgent 执行器引用（spawn_worker 委托给它）
    pub async_runner: Option<Arc<AsyncSubagentRunner>>,
}

impl CoordinatorMode {
    pub fn new() -> Self {
        Self {
            active: false,
            previous_mode: None,
            worker_registry: Arc::new(Mutex::new(HashMap::new())),
            async_runner: None,
        }
    }

    // ── ① 启用检测 ──────────────────────────────────────────────────────

    /// 检查环境变量 STAR_COORDINATOR_MODE=1
    pub fn should_enter() -> bool {
        std::env::var("STAR_COORDINATOR_MODE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
    }

    /// 进入 Coordiantor Mode：记录当前模式 + 激活
    pub fn enter(&mut self, previous_mode: ApprovalMode) {
        let pm = previous_mode.clone();
        self.previous_mode = Some(previous_mode);
        self.active = true;
        tracing::info!("Entered Coordinator Mode (previous mode: {:?})", pm);
    }

    // ── ② 恢复对齐 ──────────────────────────────────────────────────────

    /// 从 session 恢复 Coordinator 状态
    pub fn resume_from_session(&mut self, session_mode: &str) {
        if session_mode == "coordinator" {
            self.active = true;
            tracing::info!("Resumed Coordinator Mode from session");
        }
    }

    /// 序列化当前 mode 到 session
    pub fn to_session_mode(&self) -> &str {
        if self.active {
            "coordinator"
        } else {
            "normal"
        }
    }

    // ── ③ Prompt ────────────────────────────────────────────────────────

    /// 当前是否应该使用 Coordinator prompt
    pub fn is_active(&self) -> bool {
        self.active
    }

    // ── ④ Worker 生命周期 ───────────────────────────────────────────────

    /// 派发 Worker（强制异步），返回 WorkerHandle
    ///
    /// 委托 `AsyncSubagentRunner::spawn_background()` 执行。
    pub async fn spawn_worker(
        &mut self,
        prompt: &str,
        description: &str,
    ) -> Result<WorkerHandle, String> {
        let runner = self
            .async_runner
            .as_ref()
            .ok_or_else(|| "AsyncSubagentRunner not set".to_string())?;

        let request = CoreSubAgentRequest::new(prompt);
        let launch = runner.spawn_background(request, None, description.to_string());

        let handle = WorkerHandle {
            agent_id: launch.agent_id.clone(),
            description: description.to_string(),
            status: WorkerStatus::Running,
            spawn_time: Utc::now(),
        };

        let mut registry = self.worker_registry.lock().await;
        registry.insert(launch.agent_id.clone(), handle.clone());

        Ok(handle)
    }

    /// 更新 Worker 状态
    pub async fn update_worker_status(&self, agent_id: &str, status: WorkerStatus) {
        let mut registry = self.worker_registry.lock().await;
        if let Some(handle) = registry.get_mut(agent_id) {
            handle.status = status;
        }
    }

    // ── ⑤ 结果综合 ──────────────────────────────────────────────────────

    /// 收集 Worker 完成通知，生成综合回复提示
    pub fn synthesize_results(&self, notifications: &[TaskNotification]) -> String {
        if notifications.is_empty() {
            return String::new();
        }

        let mut summary = String::from("## Worker Results Summary\n\n");
        for n in notifications {
            summary.push_str(&format!(
                "- **{}** ({:?}): {}\n",
                n.summary,
                n.status,
                n.result.lines().next().unwrap_or("(no output)")
            ));
        }
        summary.push_str("\nSynthesize these results into a coherent response for the user.");
        summary
    }

    // ── 退出 ─────────────────────────────────────────────────────────────

    /// 退出 Coordinator Mode，返回之前的权限模式
    pub fn exit(&mut self) -> Option<ApprovalMode> {
        self.active = false;
        tracing::info!("Exited Coordinator Mode");
        self.previous_mode.take()
    }
}

impl Default for CoordinatorMode {
    fn default() -> Self {
        Self::new()
    }
}
