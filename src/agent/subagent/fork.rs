//! SubAgent Fork 机制
//!
//! 对标 Claude Code 的 fork subagent：
//! - fork 子代理继承父上下文
//! - prompt cache 共享
//! - 独立执行环境

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// Fork 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkConfig {
    /// 是否继承父上下文
    pub inherit_context: bool,
    /// 是否共享 prompt cache
    pub share_cache: bool,
    /// 最大 fork 深度
    pub max_depth: usize,
    /// 子代理超时（秒）
    pub timeout_secs: u64,
    /// 是否使用精确工具集
    pub use_exact_tools: bool,
}

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
            inherit_context: true,
            share_cache: true,
            max_depth: 3,
            timeout_secs: 300,
            use_exact_tools: true,
        }
    }
}

/// Fork 会话
#[derive(Debug, Clone)]
pub struct ForkedSession {
    pub id: String,
    pub parent_id: Option<String>,
    pub depth: usize,
    pub config: ForkConfig,
    pub messages: Vec<Value>,
    pub status: ForkStatus,
    pub result: Option<ForkResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForkStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkResult {
    pub success: bool,
    pub output: String,
    pub tokens_used: usize,
    pub turns: usize,
    pub error: Option<String>,
}

/// Fork Manager
pub struct ForkManager {
    sessions: Vec<ForkedSession>,
    max_concurrent: usize,
}

impl ForkManager {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            sessions: Vec::new(),
            max_concurrent,
        }
    }

    /// 创建 fork 会话
    pub fn create_fork(
        &mut self,
        parent_messages: &[Value],
        config: ForkConfig,
        task: &str,
    ) -> Result<ForkedSession, String> {
        let running_count = self
            .sessions
            .iter()
            .filter(|s| s.status == ForkStatus::Running)
            .count();

        if running_count >= self.max_concurrent {
            return Err(format!(
                "Max concurrent forks reached ({})",
                self.max_concurrent
            ));
        }

        let parent_depth = self.sessions.last().map(|s| s.depth).unwrap_or(0);

        if parent_depth >= config.max_depth {
            return Err(format!("Max fork depth reached ({})", config.max_depth));
        }

        let session_id = uuid::Uuid::new_v4().to_string();

        // 构建 fork 消息
        let mut messages = Vec::new();

        if config.inherit_context {
            // 继承父上下文（截断到最近的消息）
            let max_inherit = 50; // 最多继承50条消息
            let start = parent_messages.len().saturating_sub(max_inherit);
            for msg in &parent_messages[start..] {
                messages.push(msg.clone());
            }
        }

        // 添加 fork 任务提示
        messages.push(json!({
            "role": "system",
            "content": format!(
                "You are a forked sub-agent. Task: {}\n\n\
                 Execute this task independently. When done, provide a clear summary of what was accomplished.",
                task
            )
        }));

        let session = ForkedSession {
            id: session_id,
            parent_id: self.sessions.last().map(|s| s.id.clone()),
            depth: parent_depth + 1,
            config,
            messages,
            status: ForkStatus::Pending,
            result: None,
        };

        self.sessions.push(session.clone());
        Ok(session)
    }

    /// 构建 forked 消息（共享 prompt cache）
    pub fn build_forked_messages(
        &self,
        parent_messages: &[Value],
        task: &str,
        tool_names: &[String],
    ) -> Vec<Value> {
        let mut messages = Vec::new();

        // 使用精确工具集
        let tool_desc = if !tool_names.is_empty() {
            format!(
                "\n\nAvailable tools for this fork: {}",
                tool_names.join(", ")
            )
        } else {
            String::new()
        };

        // 继承上下文
        let max_inherit = 30;
        let start = parent_messages.len().saturating_sub(max_inherit);
        for msg in &parent_messages[start..] {
            messages.push(msg.clone());
        }

        // 添加任务
        messages.push(json!({
            "role": "user",
            "content": format!("Fork task: {}{}", task, tool_desc)
        }));

        messages
    }

    /// 更新 fork 状态
    pub fn update_status(&mut self, session_id: &str, status: ForkStatus) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.status = status;
        }
    }

    /// 完成 fork
    pub fn complete_fork(&mut self, session_id: &str, result: ForkResult) {
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) {
            session.status = if result.success {
                ForkStatus::Completed
            } else {
                ForkStatus::Failed
            };
            session.result = Some(result);
        }
    }

    /// 获取 fork 结果
    pub fn get_result(&self, session_id: &str) -> Option<&ForkResult> {
        self.sessions
            .iter()
            .find(|s| s.id == session_id)
            .and_then(|s| s.result.as_ref())
    }

    /// 获取所有活跃 fork
    pub fn active_forks(&self) -> Vec<&ForkedSession> {
        self.sessions
            .iter()
            .filter(|s| s.status == ForkStatus::Running || s.status == ForkStatus::Pending)
            .collect()
    }

    /// 清理已完成的 fork
    pub fn cleanup_completed(&mut self) {
        self.sessions
            .retain(|s| s.status == ForkStatus::Running || s.status == ForkStatus::Pending);
    }
}
