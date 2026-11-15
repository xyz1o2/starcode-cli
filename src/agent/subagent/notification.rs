//! 异步 SubAgent 完成通知。
//!
//! 对标 CCB 的 `<task-notification>` XML 机制。
//! 后台 Agent 完成后，TaskNotification 被序列化为 user-role 消息注入主对话。

use crate::types::StarMessage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

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
    /// 执行过程中产生的所有条目（包括 ToolCall、Assistant 消息等）
    #[serde(default)]
    pub entries: Vec<crate::types::ChatEntry>,
    /// 用户可见的 Agent 类型标签（对标 `userFacingName`），用于 UI 回显
    #[serde(default)]
    pub agent_type: Option<String>,
    /// teammate 自定义名称（`@name`）
    #[serde(default)]
    pub name: Option<String>,
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
            images: Vec::new(),
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
    /// 已 spawn 但尚未收到终态通知的 task_id → 登记时刻。
    ///
    /// `pending` 是「结果已就绪等注入」，这个集合是「活儿还在跑」。主 agent
    /// 的循环退出判定读后者：模型说 Done 但后台还有人没回来时，不能就此
    /// 收工 —— 否则 `<task-notification>` 永远等不到消费者（本次事故）。
    ///
    /// 存时刻而不是纯集合：任务被 abort / panic 时终态通知可能永远不会
    /// enqueue，没有时刻就无从判断它是不是「早就该回来了却没回来」——
    /// 那种条目会永久卡住主 agent（只能等超时兜底）。
    inflight_ids: HashMap<String, Instant>,
}

/// in-flight 条目的最长存活时间。
///
/// 超过这个时长还没收到终态通知，就当它已经失效：主 agent 不再为它
/// 无限等待。子代理跑满 50 轮 + 工具超时，正常的终态会在几分钟内回来；
/// 10 分钟是「肯定出事了」的量级。
const INFLIGHT_STALE_SECS: u64 = 600;

impl NotificationQueue {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            notified_ids: HashSet::new(),
            inflight_ids: HashMap::new(),
        }
    }

    /// 标记一个后台子代理开始运行（spawn 时调用）。
    ///
    /// 主 agent 的循环在收尾时用它区分「真的没事了」和「后台还有人没回来」：
    /// in-flight 集合非空时收尾被推迟，终态通知入队时移除（enqueue）。
    pub fn mark_spawned(&mut self, task_id: &str) {
        self.inflight_ids
            .insert(task_id.to_string(), Instant::now());
    }

    /// 是否还有已 spawn 但未通知的后台子代理。
    ///
    /// 顺带清掉过期条目：任务被 abort / panic 时终态通知不会到达，
    /// 那个条目会永久卡住主 agent（只能等外层超时兜底）。
    pub fn has_inflight(&mut self) -> bool {
        self.has_inflight_within(INFLIGHT_STALE_SECS)
    }

    /// 同 [`Self::has_inflight`]，但过期阈值可指定（测试用 0 表示「立刻过期」）。
    fn has_inflight_within(&mut self, stale_secs: u64) -> bool {
        self.inflight_ids
            .retain(|_, started| started.elapsed().as_secs() < stale_secs);
        !self.inflight_ids.is_empty()
    }

    /// in-flight 条目数（含过期条目，读的时候不清理）。
    pub fn inflight_count(&self) -> usize {
        self.inflight_ids.len()
    }

    /// 放弃所有 in-flight 条目 —— 等待超时后调用。
    ///
    /// 不清的话会死循环：等待超时 → 收工 → 下一轮 `has_inflight()` 仍为
    /// true → 又等 → 又超时。而等待超时阈值（`STAR_BG_WAIT_TIMEOUT_SECS`，
    /// 默认 600s）可以和 in-flight 过期阈值（同样 600s）不一致 —— 谁小
    /// 谁先到，超时后条目还活着，下一轮又认为「还有人在跑」。
    ///
    /// 放弃只是让 lead 不再等它；子代理真跑完了，终态通知仍会入队，
    /// 下一次 drain 照样能拿到。
    pub fn abandon_inflight(&mut self) {
        self.inflight_ids.clear();
    }

    /// 入队一条通知（去重：已通知过的 id 会被跳过）
    pub fn enqueue(&mut self, notification: TaskNotification) {
        // 终态到达 → 该任务不再 in-flight。即使通知本身被去重跳过
        // （已注入过一次，理论不该发生），in-flight 也必须移除，否则
        // 主 agent 会为一个永远等不到第二次通知的任务一直续命。
        self.inflight_ids.remove(&notification.task_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(id: &str) -> TaskNotification {
        TaskNotification {
            task_id: id.to_string(),
            tool_use_id: None,
            status: NotificationStatus::Completed,
            summary: "done".to_string(),
            result: "result".to_string(),
            usage: NotificationUsage {
                total_tokens: 1,
                tool_uses: 1,
                duration_ms: 1,
            },
            entries: Vec::new(),
            agent_type: None,
            name: None,
        }
    }

    #[test]
    fn mark_spawned_then_enqueue_clears_inflight() {
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-a");
        q.mark_spawned("agent-b");
        assert!(q.has_inflight());

        // 第一个终态入队 → 其 in-flight 移除、通知进 pending
        q.enqueue(completed("agent-a"));
        assert!(q.has_inflight(), "agent-b still running");
        assert_eq!(q.pending_count(), 1);

        // 第二个终态入队 → in-flight 全清
        q.enqueue(completed("agent-b"));
        assert!(!q.has_inflight());
        assert_eq!(q.pending_count(), 2);
    }

    #[test]
    fn duplicate_enqueue_still_clears_inflight() {
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-a");
        q.enqueue(completed("agent-a"));
        let drained = q.drain_for_next_turn();
        assert_eq!(drained.len(), 1);

        // 同一通知再次入队被去重 —— 但 in-flight 必须照样移除，
        // 否则主 agent 会为一个永远等不到第二次通知的任务续命。
        q.mark_spawned("agent-a");
        q.enqueue(completed("agent-a"));
        assert!(!q.has_inflight());
        assert_eq!(q.pending_count(), 0, "deduplicated");
    }

    #[test]
    fn drain_does_not_touch_inflight() {
        // drain 只消费已就绪的通知；还在跑的任务保持 in-flight，
        // 主 agent 的 Done 分支等的是它，不能被 drain 误清。
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-live");
        q.mark_spawned("agent-done");
        q.enqueue(completed("agent-done"));
        let drained = q.drain_for_next_turn();
        assert_eq!(drained.len(), 1);
        assert!(q.has_inflight(), "agent-live still running");
    }

    #[test]
    fn a_stale_inflight_entry_does_not_block_the_lead_forever() {
        // 任务被 abort / panic → 终态通知永远不会 enqueue。那个 in-flight
        // 条目不能永久卡住主 agent（它只能等外层的 600s 超时兜底）。
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-zombie");
        assert_eq!(q.inflight_count(), 1);

        // 阈值 0 = 立刻过期。
        assert!(
            !q.has_inflight_within(0),
            "a stale entry must not keep the lead waiting"
        );
        assert_eq!(q.inflight_count(), 0, "and it must be pruned");
    }

    #[test]
    fn a_fresh_inflight_entry_still_blocks_the_lead() {
        // 正常在跑的任务不能被误清 —— 那会让主 agent 在子代理没回来时收工。
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-live");
        assert!(q.has_inflight_within(600));
        assert_eq!(q.inflight_count(), 1);
    }

    #[test]
    fn abandoning_inflight_ends_the_wait_cycle() {
        // 死循环闸门：等待超时后放弃 in-flight，下一轮就不会再认为
        // 「还有人在跑」→ 又等 → 又超时。
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-zombie");
        assert!(q.has_inflight_within(600));

        q.abandon_inflight();
        assert!(!q.has_inflight_within(600));
        assert_eq!(q.inflight_count(), 0);
    }

    #[test]
    fn abandoning_inflight_does_not_discard_a_late_notification() {
        // 放弃只是让 lead 不再等它 —— 子代理真跑完了，终态通知仍会入队，
        // 下一次 drain 照样能拿到。
        let mut q = NotificationQueue::new();
        q.mark_spawned("agent-late");
        q.abandon_inflight();

        q.enqueue(completed("agent-late"));
        assert_eq!(q.pending_count(), 1);
        assert_eq!(q.drain_for_next_turn().len(), 1);
    }
}
