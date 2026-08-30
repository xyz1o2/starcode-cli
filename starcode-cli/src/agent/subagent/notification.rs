//! 异步 SubAgent 完成通知。
//!
//! 对标 CCB 的 `<task-notification>` XML 机制。
//! 后台 Agent 完成后，TaskNotification 被序列化为 user-role 消息注入主对话。

use crate::types::StarMessage;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

// ── TaskNotification ─────────────────────────────────────────────────────

/// 后台 Agent 完成后的结构化通知
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNotification {
    /// 唯一标识（如 "agent-a1b2c3"）
    pub task_id: String,
    /// 对应的 tool_use id
    pub tool_use_id: Option<String>,
    /// 完成状态
    pub status: NotificationStatus,
    /// 简短摘要（3-5 词，用于 spinner/日志）
    pub summary: String,
    /// 完整输出内容
    pub result: String,
    /// token + 工具调用统计
    pub usage: NotificationUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationStatus {
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationUsage {
    pub total_tokens: u64,
    pub tool_uses: u64,
    pub duration_ms: u64,
}

impl TaskNotification {
    /// 序列化为 XML（兼容 CCB 标准格式）
    pub fn to_xml(&self) -> String {
        format!(
            r#"<task-notification>
  <task-id>{task_id}</task-id>
  <status>{status}</status>
  <summary>{summary}</summary>
  <result>{result}</result>
  <usage>
    <total_tokens>{tokens}</total_tokens>
    <tool_uses>{tools}</tool_uses>
    <duration_ms>{dur}</duration_ms>
  </usage>
</task-notification>"#,
            task_id = self.task_id,
            status = self.status_str(),
            summary = self.summary,
            result = self.result,
            tokens = self.usage.total_tokens,
            tools = self.usage.tool_uses,
            dur = self.usage.duration_ms,
        )
    }

    /// 转为 StarMessage，role="user"，作为系统注入消息
    pub fn to_message(&self) -> StarMessage {
        StarMessage {
            role: "user".to_string(),
            content: Some(self.to_xml()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            cache_control: None,
        }
    }

    fn status_str(&self) -> &str {
        match self.status {
            NotificationStatus::Completed => "completed",
            NotificationStatus::Failed => "failed",
            NotificationStatus::Killed => "killed",
        }
    }
}

// ── NotificationQueue ────────────────────────────────────────────────────

/// 全局通知队列，主循环每轮 turn 开始时消费
pub struct NotificationQueue {
    pending: VecDeque<TaskNotification>,
    /// 已注入的 task_id 集合，防止同一通知重复注入
    notified_ids: HashSet<String>,
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            notified_ids: HashSet::new(),
        }
    }

    /// 入队一条通知（去重：已通知过的 id 会被跳过）
    pub fn enqueue(&mut self, notification: TaskNotification) {
        if self.notified_ids.contains(&notification.task_id) {
            return;
        }
        self.pending.push_back(notification);
    }

    /// 排出本轮到期的所有通知，标记为已通知
    pub fn drain_for_next_turn(&mut self) -> Vec<TaskNotification> {
        let drained: Vec<TaskNotification> = self.pending.drain(..).collect();
        for n in &drained {
            self.notified_ids.insert(n.task_id.clone());
        }
        drained
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl Default for NotificationQueue {
    fn default() -> Self {
        Self::new()
    }
}
