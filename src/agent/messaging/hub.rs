//! AgentRouter Hub：进程内 agent 间互通的寻址与邮箱（Phase 1 核心）。
//!
//! 对标 Claude Code 的 teammateMailbox + SendMessageTool：
//! - 每个身份（lead / 后台 worker / fork）一个 mailbox；
//! - `deliver` 三路寻址：name、agent_id、`"*"` 广播；
//! - 无死信：目标未注册时返回 [`DeliveryError::UnknownTarget`]，绝不静默成功。
//!
//! 消费侧（agent_loop 每轮 turn 开头 drain → user-role XML 注入）见
//! `docs/agent-messaging-design.zh-CN.md` §4/§5。接线状态同文档 §6。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

/// 单个 mailbox 的消息上限（对齐 swarm/mailbox.rs 的 MAX_RETAINED_MESSAGES 缩水版：
/// 进程内互通不需要 1000 条，顶住一篇长对话的开销即可）。
const MAX_MAILBOX_MESSAGES: usize = 64;

/// lead（主 agent）的固定身份键。
pub const LEAD_IDENTITY: &str = "lead";

// ── AgentMessage ─────────────────────────────────────────────────────────

/// 消息类型（与 `core::swarm::mailbox::MessageType` 对齐的子集：
/// v1 只接线“互通”需要的四个，其余等 swarm 工作流起来再扩）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HubMessageType {
    PlainText,
    TaskAssignment,
    PlanApprovalRequest,
    PermissionRequest,
}

impl Default for HubMessageType {
    fn default() -> Self {
        Self::PlainText
    }
}

impl HubMessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::TaskAssignment => "task_assignment",
            Self::PlanApprovalRequest => "plan_approval_request",
            Self::PermissionRequest => "permission_request",
        }
    }
}

/// 一条 agent → agent 的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    /// 发送者身份（worker name / agent_id / "lead"）
    pub from: String,
    /// 接收者身份（广播时与 mailbox 键一致）
    pub to: String,
    #[serde(default)]
    pub message_type: HubMessageType,
    pub content: String,
    /// UI 短摘要（对标 SendMessage 的 REQUIRED summary）
    pub summary: String,
    pub timestamp_ms: u128,
}

impl AgentMessage {
    pub fn new(
        from: impl Into<String>,
        content: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            from: from.into(),
            to: String::new(),
            message_type: HubMessageType::PlainText,
            content: content.into(),
            summary: summary.into(),
            timestamp_ms: now_ms(),
        }
    }

    /// 序列化为 user-role 注入用的 XML（对标 `<task-notification>` 风格）。
    pub fn to_xml(&self) -> String {
        format!(
            "<agent-message from=\"{}\" to=\"{}\" type=\"{}\" summary=\"{}\" id=\"{}\">\n{}\n</agent-message>",
            self.from, self.to, self.message_type.as_str(), self.summary, self.id, self.content
        )
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

// ── 注册表 ───────────────────────────────────────────────────────────────

/// 一个已注册 agent 的寻址信息。
#[derive(Debug, Clone)]
pub struct AgentEntry {
    /// 稳定 ID（如 "agent-a1b2c3"）
    pub agent_id: String,
    /// 可读名（可为空；fork 是 "fork-xxxx"，coordinator worker 可空）
    pub name: Option<String>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Completed,
}

/// 投递结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// 单播送达
    Delivered {
        message_id: String,
        mailbox_key: String,
    },
    /// 广播送达（返回收到消息的 agent 数与身份键列表）
    Broadcast { delivered_to: Vec<String> },
    /// 无此目标 —— 显式失败，绝不写死信
    UnknownTarget { to: String },
}

// ── AgentRouter ──────────────────────────────────────────────────────────

/// 进程级路由/邮箱管理器。外层套 `Arc<Mutex<>>` 使用
/// （与 `NotificationQueue` 的接线方式一致）。
#[derive(Default)]
pub struct AgentRouter {
    /// agent_id / name → 身份键（身份键 = name 优先，否则 agent_id）
    aliases: HashMap<String, String>,
    /// 身份键 → 注册信息
    entries: HashMap<String, AgentEntry>,
    /// 身份键 → mailbox
    mailboxes: HashMap<String, VecDeque<AgentMessage>>,
}

impl AgentRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个 agent。`name` 非空时 name 与 agent_id 都解析到同一身份键。
    /// lead 用 `register_entry(LEAD_IDENTITY, …)` 直接以 "lead" 作键注册即可。
    pub fn register_entry(
        &mut self,
        identity: impl Into<String>,
        agent_id: impl Into<String>,
        name: Option<String>,
    ) {
        let identity = identity.into();
        let agent_id = agent_id.into();
        self.aliases.insert(agent_id.clone(), identity.clone());
        if let Some(n) = name
            .as_ref()
            .filter(|n| !n.trim().is_empty() && n.as_str() != agent_id)
        {
            self.aliases.insert(n.clone(), identity.clone());
        }
        let status = self
            .entries
            .get(&identity)
            .map(|e| e.status.clone())
            .unwrap_or(AgentStatus::Running);
        self.entries.insert(
            identity.clone(),
            AgentEntry {
                agent_id,
                name,
                status,
            },
        );
        self.mailboxes.entry(identity).or_default();
    }

    /// 注销（终态后调用；mailbox 保留，让迟到消息落进可见的空箱而不是复活一个幽灵）。
    pub fn unregister(&mut self, agent_id: &str) {
        if let Some(identity) = self.aliases.get(agent_id).cloned() {
            if let Some(entry) = self.entries.get_mut(&identity) {
                entry.status = AgentStatus::Completed;
            }
            self.aliases.retain(|_, v| v != &identity);
        }
    }

    /// 解析寻址串（name / agent_id / "lead"）→ 身份键。
    pub fn resolve(&self, to: &str) -> Option<String> {
        self.aliases.get(to).cloned()
    }

    /// 已注册身份键列表（broadcast 用）。
    pub fn registered_identities(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// 列出所有已注册的可读名（供 SendMessage 构建错误提示 / UI 展示）。
    pub fn registered_names(&self) -> Vec<String> {
        self.entries
            .values()
            .filter_map(|e| e.name.clone())
            .filter(|n| !n.is_empty())
            .collect()
    }

    /// 投递一条消息。`from_identity` 用于广播时排除发送者自己。
    pub fn deliver(
        &mut self,
        mut message: AgentMessage,
        to: &str,
        from_identity: &str,
    ) -> Delivery {
        // 广播
        if to == "*" {
            let mut delivered_to = Vec::new();
            let identities: Vec<String> = self.entries.keys().cloned().collect();
            for identity in &identities {
                if identity == from_identity {
                    continue;
                }
                let mut m = message.clone();
                m.to = identity.clone();
                self.push(identity.clone(), m);
                delivered_to.push(identity.clone());
            }
            return Delivery::Broadcast { delivered_to };
        }

        // 单播
        let Some(identity) = self.resolve(to) else {
            return Delivery::UnknownTarget { to: to.to_string() };
        };
        message.to = identity.clone();
        let message_id = message.id.clone();
        self.push(identity.clone(), message);
        Delivery::Delivered {
            message_id,
            mailbox_key: identity,
        }
    }

    /// 排出某身份的所有待处理消息（读后即清，与 NotificationQueue.drain 语义一致）。
    pub fn drain(&mut self, identity: &str) -> Vec<AgentMessage> {
        self.mailboxes
            .get_mut(identity)
            .map(|mailbox| std::mem::take(mailbox).into())
            .unwrap_or_default()
    }

    /// 某身份的未读数（UI 徽标 / 测试用）。
    pub fn pending_count(&self, identity: &str) -> usize {
        self.mailboxes.get(identity).map(|m| m.len()).unwrap_or(0)
    }

    fn push(&mut self, identity: String, message: AgentMessage) {
        let mailbox = self.mailboxes.entry(identity).or_default();
        mailbox.push_back(message);
        while mailbox.len() > MAX_MAILBOX_MESSAGES {
            mailbox.pop_front();
        }
    }
}

/// 线程安全句柄（接线时挂到 RuntimeServices / SendMessage 工具 / agent_loop）。
pub type SharedAgentRouter = Arc<Mutex<AgentRouter>>;

/// hub 的到信唤醒信号（IdleNotification 联动，设计文档 §6 Phase 6）。
///
/// 背景：`agent_loop` 只在 turn 开头 drain mailbox —— lead 正**空闲**时，
/// worker / 远端进程发来的消息要等到下一次用户输入才会被消费。这个信号让
/// UI 事件循环（或外部 harness 的轮询层）在空收到信时主动唤醒 agent。
///
/// 用法：`notify` 由 [`AgentMailbox::deliver_unwrapped`]（及进程内一切
/// `router.lock().deliver(...)` 的封装路径）触发；消费方 `notified().await`。
/// 语义对齐 tokio `Notify`：多次投递只留一次唤醒 —— 消费方醒来后应该
/// `pending_count` 实查，而不是假设有一条。
#[derive(Clone, Default)]
pub struct MailWakeupSignal {
    inner: Arc<tokio::sync::Notify>,
}

impl MailWakeupSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// 有信落入寄存时触发。
    pub fn notify_mail(&self) {
        self.inner.notify_waiters();
    }

    /// 等下一次到信。
    pub async fn wait_mail(&self) {
        self.inner.notified().await;
    }
}

/// hub + 唤醒信号的成对封装：`AgentMessage` 投递方（SendMessage 工具、
/// RemoteBridge 入站）走它的 `deliver`，投进裸 router 之后顺手 notify。
#[derive(Clone)]
pub struct AgentMailbox {
    pub router: SharedAgentRouter,
    pub wakeup: MailWakeupSignal,
}

impl AgentMailbox {
    pub fn new() -> Self {
        Self {
            router: new_shared_router(),
            wakeup: MailWakeupSignal::new(),
        }
    }

    pub fn from_router(router: SharedAgentRouter) -> Self {
        Self {
            router,
            wakeup: MailWakeupSignal::new(),
        }
    }

    /// 投递并按需唤醒（目标/广播含 lead 时）。
    pub async fn deliver(&self, msg: AgentMessage, to: &str, from: &str) -> Delivery {
        let delivery = self.router.lock().await.deliver(msg, to, from);
        // 只有投给 lead 才需要唤醒 UI —— worker 在自己的 turn 循环里，
        // 活跃时 turn 开头就会 drain；空闲 worker 的消费由后续阶段接。
        let should_wake = matches!(
            delivery,
            Delivery::Delivered { ref mailbox_key, .. }
                if mailbox_key == LEAD_IDENTITY
        ) || matches!(delivery, Delivery::Broadcast { .. });
        if should_wake {
            self.wakeup.notify_mail();
        }
        delivery
    }
}

impl Default for AgentMailbox {
    fn default() -> Self {
        Self::new()
    }
}

pub fn new_shared_router() -> SharedAgentRouter {
    Arc::new(Mutex::new(AgentRouter::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicast_by_name_and_agent_id_reach_the_same_mailbox() {
        let mut router = AgentRouter::new();
        router.register_entry("researcher", "agent-aaa111", Some("researcher".to_string()));

        let msg = AgentMessage::new(LEAD_IDENTITY, "换用 tokio::spawn", "调度建议");
        assert!(matches!(
            router.deliver(msg.clone(), "researcher", LEAD_IDENTITY),
            Delivery::Delivered { .. }
        ));
        assert!(matches!(
            router.deliver(msg, "agent-aaa111", LEAD_IDENTITY),
            Delivery::Delivered { .. }
        ));
        assert_eq!(router.pending_count("researcher"), 2);

        let drained = router.drain("researcher");
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].from, LEAD_IDENTITY);
        assert_eq!(router.pending_count("researcher"), 0);
    }

    #[test]
    fn unknown_target_is_explicit_not_a_dead_letter() {
        let mut router = AgentRouter::new();
        let msg = AgentMessage::new(LEAD_IDENTITY, "在吗", "寻呼");
        assert_eq!(
            router.deliver(msg, "ghost", LEAD_IDENTITY),
            Delivery::UnknownTarget {
                to: "ghost".to_string()
            }
        );
    }

    #[test]
    fn broadcast_reaches_registered_agents_but_not_sender() {
        let mut router = AgentRouter::new();
        router.register_entry(
            LEAD_IDENTITY,
            "lead-session",
            Some(LEAD_IDENTITY.to_string()),
        );
        router.register_entry("w1", "agent-111111", Some("w1".to_string()));
        router.register_entry("w2", "agent-222222", Some("w2".to_string()));

        let msg = AgentMessage::new("w1", "我这边完成了", "w1 完成");
        match router.deliver(msg, "*", "w1") {
            Delivery::Broadcast { delivered_to } => {
                // registry 是 HashMap，广播顺序不保证 —— 断集合不断序。
                let mut got = delivered_to.clone();
                got.sort();
                assert_eq!(got, vec![LEAD_IDENTITY.to_string(), "w2".to_string()]);
            }
            other => panic!("expected broadcast, got {:?}", other),
        }
        assert_eq!(router.pending_count("w1"), 0); // 不回环发给自己
        assert_eq!(router.pending_count("w2"), 1);
        assert_eq!(router.pending_count(LEAD_IDENTITY), 1);
    }

    #[test]
    fn unregister_marks_completed_but_keeps_mailbox() {
        let mut router = AgentRouter::new();
        router.register_entry("w1", "agent-111111", Some("w1".to_string()));
        router.deliver(
            AgentMessage::new(LEAD_IDENTITY, "迟到的消息", "补发"),
            "w1",
            LEAD_IDENTITY,
        );

        router.unregister("agent-111111");
        // 旧 agent_id 不再解析
        assert_eq!(
            router.deliver(
                AgentMessage::new(LEAD_IDENTITY, "再发", "重试"),
                "agent-111111",
                LEAD_IDENTITY
            ),
            Delivery::UnknownTarget {
                to: "agent-111111".to_string()
            }
        );
        // 已投递的消息仍在箱内，drain 不丢
        assert_eq!(router.pending_count("w1"), 1);
    }

    #[test]
    fn mailbox_cap_trims_oldest() {
        let mut router = AgentRouter::new();
        router.register_entry("w1", "agent-111111", Some("w1".to_string()));
        for i in 0..(MAX_MAILBOX_MESSAGES + 10) {
            router.deliver(
                AgentMessage::new(LEAD_IDENTITY, format!("m{}", i), "灌水"),
                "w1",
                LEAD_IDENTITY,
            );
        }
        assert_eq!(router.pending_count("w1"), MAX_MAILBOX_MESSAGES);
        let drained = router.drain("w1");
        assert_eq!(drained.first().unwrap().content, "m10"); // 最早的 10 条被裁掉
    }

    #[test]
    fn message_xml_has_task_notification_style() {
        let msg = AgentMessage::new("researcher", "正文", "摘要");
        let xml = msg.to_xml();
        assert!(xml.starts_with("<agent-message from=\"researcher\""));
        assert!(xml.contains("type=\"plain_text\""));
        assert!(xml.ends_with("</agent-message>"));
    }

    /// Phase 6 IdleNotification 联动：投给 lead / 广播触发唤醒信号，
    /// 投给普通 worker 不触发（worker 在自己的 turn 循环里，活跃时
    /// turn 开头自然 drain；空闲 worker 的唤醒是后续阶段的事）。
    #[tokio::test]
    async fn mailbox_wakeup_fires_for_lead_and_broadcast_only() {
        let mailbox = AgentMailbox::new();
        mailbox.router.lock().await.register_entry(
            LEAD_IDENTITY,
            "lead-s",
            Some(LEAD_IDENTITY.to_string()),
        );
        mailbox
            .router
            .lock()
            .await
            .register_entry("w1", "agent-111", Some("w1".to_string()));

        // 投给 w1 → 不唤醒
        let msg = AgentMessage::new(LEAD_IDENTITY, "给 worker 的活", "派工");
        mailbox.deliver(msg, "w1", LEAD_IDENTITY).await;
        let woke = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            mailbox.wakeup.wait_mail(),
        )
        .await;
        assert!(woke.is_err(), "投给普通 worker 不该触发 lead 的唤醒信号");

        // 投给 lead → 唤醒。先订阅再投递（Notify 的 notified() 若在 notify_waiters
        // 之后才挂靠会错过上一次通知），等待带超时防挂死。
        let waiter = mailbox.wakeup.clone();
        let waiter_task = tokio::spawn(async move {
            let _guard = waiter.inner.notified();
            _guard.await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let msg = AgentMessage::new("w1", "worker 回话", "汇报");
        mailbox.deliver(msg, LEAD_IDENTITY, "w1").await;
        let woke = tokio::time::timeout(std::time::Duration::from_millis(500), waiter_task).await;
        assert!(woke.is_ok(), "投给 lead 必须触发唤醒信号");
        mailbox.router.lock().await.drain(LEAD_IDENTITY);
        assert_eq!(mailbox.router.lock().await.pending_count(LEAD_IDENTITY), 0);
    }
}
