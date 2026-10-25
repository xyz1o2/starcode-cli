use crate::core::confirmation_bus::types::*;
use crate::core::policy::{get_hook_source, PolicyDecision, PolicyEngine};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct MessageBus {
    policy_engine: Arc<RwLock<PolicyEngine>>,
    debug: bool,
    sender: broadcast::Sender<Message>,
    /// 有外部应答者（headless + 外部 harness）。
    ///
    /// headless 默认 `non_interactive`，策略层把 `AskUser` 翻成 `Deny`，
    /// 工具自己提出的确认要求则在等待层被 `NoOneToAsk` 挡掉 —— 因为"没有人
    /// 能回答"。
    ///
    /// 但 headless 挂上外部 harness 之后**确实有人能回答**：inbox 里的
    /// `approve_tool` 会被翻成 `ToolConfirmationResponse` 发回这条总线。
    /// 这个开关就是让等待层知道这一点的。
    ///
    /// 用 `AtomicBool` 而不是 `bool`：总线是 `Arc` 共享的，而
    /// `Arc::get_mut` 要求**没有其他强引用** —— 实际上永远不满足（config
    /// 与 agent 各持一份）。内部可变性绕开这个限制。
    external_responder: std::sync::atomic::AtomicBool,
}

impl MessageBus {
    pub fn new(policy_engine: PolicyEngine, debug: bool) -> Self {
        let (sender, _) = broadcast::channel(100);
        Self {
            policy_engine: Arc::new(RwLock::new(policy_engine)),
            debug,
            sender,
            external_responder: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 声明"有人能从外部回答确认请求"。
    ///
    /// 由 headless + 外部 harness 的组合在启动时调用。没有它，`await_tool_decision`
    /// 会因为"没有订阅者"直接返回 `NoOneToAsk` —— 外部 harness 根本来不及答复。
    pub fn set_external_responder(&self, enabled: bool) {
        self.external_responder
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    /// 当前是否有外部应答者。
    pub fn has_external_responder(&self) -> bool {
        self.external_responder
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_valid_message(&self, message: &Message) -> bool {
        match message {
            Message::ToolConfirmationRequest(req) => !req.correlation_id.is_empty(),
            _ => true,
        }
    }

    async fn emit_message(&self, message: Message) {
        let _ = self.sender.send(message);
    }

    pub async fn allow_tool_for_session(&self, tool_name: String) {
        let mut policy_engine = self.policy_engine.write().await;
        policy_engine.allow_tool_for_session(tool_name);
    }

    pub async fn set_approval_mode(&self, mode: crate::core::policy::types::ApprovalMode) {
        let mut policy_engine = self.policy_engine.write().await;
        policy_engine.set_approval_mode(mode);
    }

    /// 重新从磁盘装载权限规则。PolicyEngine 只活在这里，UI 改完 settings 必须走这一趟。
    pub async fn reload_permission_rules(&self, cwd: &std::path::Path) -> usize {
        let mut policy_engine = self.policy_engine.write().await;
        policy_engine.load_project_permissions(cwd);
        policy_engine.settings_permissions().rule_count()
    }

    /// 当前生效的 settings 权限规则快照，给 `/permissions list` 用。
    pub async fn settings_permissions(&self) -> crate::core::policy::SettingsPermissions {
        self.policy_engine
            .read()
            .await
            .settings_permissions()
            .clone()
    }

    pub async fn check_tool_policy(
        &self,
        tool_call: &crate::core::policy::FunctionCall,
        server_name: Option<&str>,
    ) -> crate::core::policy::CheckResult {
        let policy_engine = self.policy_engine.read().await;
        policy_engine.check(tool_call, server_name).await
    }

    /// 现在有没有人能够回答确认卡片。
    ///
    /// `-p` headless 没有 UI 订阅总线，策略层已经把 `AskUser` 翻成了 `Deny`；
    /// 但工具自己提出的确认要求（未信任目录之类）不经过策略，于是仍可能走到
    /// 等待那一层。等待没有超时，进去就出不来了 —— 所以这里要在**开始等之前**
    /// 先问一句有没有人。
    pub async fn has_a_human_on_the_other_end(&self) -> bool {
        if self.has_external_responder() {
            return true;
        }
        !self.policy_engine.read().await.is_non_interactive()
    }

    /// 往总线上发一条消息 —— 给外部应答者用。
    ///
    /// 外部 harness 批准工具时走这里：`ToolConfirmationResponse` 的
    /// `correlation_id` 就是确认卡上挂着的那个 id（等待层生成，UI 与外部
    /// harness 看到的是同一个）。
    pub async fn respond_external(&self, message: Message) {
        self.emit_message(message).await;
    }

    pub async fn publish(&self, message: Message) -> Result<(), Box<dyn std::error::Error>> {
        if self.debug {
            crate::utils::logging::append_debug_log_line(&format!(
                "[MESSAGE_BUS] publish: {:?}",
                serde_json::to_string(&message)
            ));
        }

        if !self.is_valid_message(&message) {
            return Err(format!("Invalid message structure: {:?}", message).into());
        }

        match message {
            Message::ToolConfirmationRequest(req) => {
                let original_message = Message::ToolConfirmationRequest(req.clone());
                let policy_engine = self.policy_engine.read().await;
                let tool_call = crate::core::policy::FunctionCall {
                    name: req.tool_call.name.clone(),
                    args: req.tool_call.args.clone(),
                };
                let check_result = policy_engine
                    .check(&tool_call, req.server_name.as_deref())
                    .await;

                match check_result.decision {
                    // 这里**不能**因为复查结果是 Allow 就自己回一条 `confirmed: true`。
                    //
                    // 能走到 publish 的请求，调用方（ToolExecutor）已经判过"要问用户"
                    // 才把它发出来的；在 publish 里再判一次并且**只允许它抬高**判定，
                    // 等于开了一条"没有任何人点过同意就执行"的旁路：两次判定之间只要
                    // 状态动过（`/permissions` 重载规则、审批模式被切到 yolo、
                    // 子代理并发），命令就会在 UI 一张卡都没画的情况下跑掉。
                    //
                    // publish 这一层的职责是**收紧**（deny 仍然生效），不是放宽。
                    PolicyDecision::Allow => {
                        self.emit_message(original_message).await;
                    }
                    PolicyDecision::Deny => {
                        self.emit_message(Message::ToolPolicyRejection(ToolPolicyRejection {
                            message_type: MessageBusType::ToolPolicyRejection,
                            tool_call: req.tool_call,
                        }))
                        .await;

                        self.emit_message(Message::ToolConfirmationResponse(
                            ToolConfirmationResponse {
                                message_type: MessageBusType::ToolConfirmationResponse,
                                correlation_id: req.correlation_id,
                                confirmed: false,
                                requires_user_confirmation: None,
                                outcome: None,
                                feedback: None,
                            },
                        ))
                        .await;
                    }
                    PolicyDecision::DenyWithReason(_reason) => {
                        self.emit_message(Message::ToolPolicyRejection(ToolPolicyRejection {
                            message_type: MessageBusType::ToolPolicyRejection,
                            tool_call: req.tool_call,
                        }))
                        .await;

                        self.emit_message(Message::ToolConfirmationResponse(
                            ToolConfirmationResponse {
                                message_type: MessageBusType::ToolConfirmationResponse,
                                correlation_id: req.correlation_id,
                                confirmed: false,
                                requires_user_confirmation: None,
                                outcome: Some(crate::types::ToolConfirmationOutcome::Cancel),
                                feedback: None,
                            },
                        ))
                        .await;
                    }
                    PolicyDecision::AskUser => {
                        self.emit_message(original_message).await;
                    }
                }
            }
            Message::HookExecutionRequest(req) => {
                let original_message = Message::HookExecutionRequest(req.clone());
                let policy_engine = self.policy_engine.read().await;
                let hook_source_policy = get_hook_source(&req.input);
                let hook_source = match hook_source_policy {
                    crate::core::policy::types::HookSource::Project => HookSource::Project,
                    crate::core::policy::types::HookSource::User => HookSource::User,
                    crate::core::policy::types::HookSource::System => HookSource::System,
                    crate::core::policy::types::HookSource::Extension => HookSource::Extension,
                };
                let decision = policy_engine
                    .check_hook(&crate::core::policy::types::HookExecutionContext {
                        event_name: req.event_name.clone(),
                        hook_source: Some(hook_source_policy),
                        trusted_folder: req.input.get("trusted_folder").and_then(|v| v.as_bool()),
                    })
                    .await;

                let effective_decision = if decision == PolicyDecision::Allow {
                    HookDecision::Allow
                } else {
                    HookDecision::Deny
                };

                self.emit_message(Message::HookPolicyDecision(HookPolicyDecision {
                    message_type: MessageBusType::HookPolicyDecision,
                    event_name: req.event_name.clone(),
                    hook_source,
                    decision: effective_decision.clone(),
                    reason: if decision != PolicyDecision::Allow {
                        Some("Hook execution denied by policy".to_string())
                    } else {
                        None
                    },
                }))
                .await;

                if decision == PolicyDecision::Allow {
                    self.emit_message(original_message).await;
                } else {
                    self.emit_message(Message::HookExecutionResponse(HookExecutionResponse {
                        message_type: MessageBusType::HookExecutionResponse,
                        correlation_id: req.correlation_id,
                        success: false,
                        output: None,
                        error: Some("Hook execution denied by policy".to_string()),
                    }))
                    .await;
                }
            }
            _ => {
                self.emit_message(message).await;
            }
        }

        Ok(())
    }

    /// 把"这条请求已经作废"广播出去，让屏幕上那张卡跟着消失。
    ///
    /// 只丢本地 future 是不够的 —— UI 上的确认卡是按 correlation_id 等应答的，
    /// 没有这条广播它就永远停在"Allow once / Deny"上，而这次调用的等待方早就
    /// 没了。用户随后按下的 1/4 发给一个没有接收者的 id：卡片还在、命令却已经
    /// 按超时结束，谁也说不清刚才那次批准算不算数 —— 表现上就是"门控拦不住"。
    async fn dismiss_request(&self, correlation_id: String) {
        self.emit_message(Message::ToolConfirmationResponse(
            ToolConfirmationResponse {
                message_type: MessageBusType::ToolConfirmationResponse,
                correlation_id,
                confirmed: false,
                requires_user_confirmation: None,
                outcome: Some(crate::types::ToolConfirmationOutcome::Cancel),
                feedback: None,
            },
        ))
        .await;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.sender.subscribe()
    }

    /// 挂起一次工具调用，一直等到**用户**给出决定。
    ///
    /// # 这里没有超时，而且不能有
    ///
    /// 门禁的语义是"没批准就不许动"，不是"等太久就当没批准"。曾经这里是
    /// `tokio::time::timeout(600s, …)`，于是用户只是去倒了杯水，回来就看到
    /// `Confirmation timeout or error: Confirmation timed out` —— 一次还没被否决的
    /// 写文件请求，在用户眼前自己变成了失败，模型据此认定 `Write` 不可用并放弃。
    /// 计时器在这里能表达的只有"用户还没回答"，却把答案写成了"用户拒绝了"。
    ///
    /// 能结束这次等待的结局只有这些，**没有"超时"**：
    ///
    /// | 结局 | 谁触发的 | 语义 |
    /// |------|----------|------|
    /// | 收到应答 | 用户按了 1–4 / ESC | 用户做了决定，尊重它 |
    /// | [`DecisionWaitError::Aborted`] | 用户中断了这一轮（ESC / Ctrl+C） | 用户主动放弃 |
    /// | [`DecisionWaitError::NoOneToAsk`] | `-p` headless | 没有人能回答，压根别开始等 |
    /// | [`DecisionWaitError::BusClosed`] | 订阅者全部走了 | 没人能回答，不再空等 |
    ///
    /// `-p` headless 有两道防线：策略层在 `non_interactive` 下把 `AskUser` 直接判成
    /// `Deny`；工具自己提出的确认要求（未信任目录之类）不经过策略，所以这里还要
    /// 再问一句有没有人 —— 没有超时的等待，进去就出不来了。
    ///
    /// `abort` 传 `None` 表示"这一轮没有中断通道"，此时等待真的会一直挂下去。
    async fn await_tool_decision_inner<TResponse: TryFromMessage>(
        &self,
        request: Message,
        abort: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<TResponse, DecisionWaitError> {
        let correlation_id = Uuid::new_v4().to_string();
        let mut rx = self.subscribe();

        // 没有人能回答就不要开始等。这条等待没有超时，进去就出不来了。
        if !self.has_a_human_on_the_other_end().await {
            crate::utils::logging::append_debug_log_line(
                "[MESSAGE_BUS] refusing to wait for a decision: nobody can answer",
            );
            return Err(DecisionWaitError::NoOneToAsk);
        }

        let mut request_msg = request;
        if let Message::ToolConfirmationRequest(ref mut req) = request_msg {
            req.correlation_id = correlation_id.clone();
        }

        if let Err(e) = self.publish(request_msg).await {
            crate::utils::logging::append_debug_log_line(&format!(
                "[MESSAGE_BUS] confirmation request never reached the user: {e}"
            ));
            return Err(DecisionWaitError::NotPublished);
        }

        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Some(response) = TResponse::try_from_message(&msg, &correlation_id) {
                            return Ok::<TResponse, DecisionWaitError>(response);
                        }
                    }
                    // 慢消费者只说明缓冲里被丢掉了几条**旧**消息，决定本身还在路上。
                    // 把它当成"没人应答"去结束等待，等于让一次无关的消费压力
                    // 否决掉用户的批准 —— 这正是原实现 `Err(_) => "Channel closed"`
                    // 干的事。
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(DecisionWaitError::BusClosed);
                    }
                }
            }
        };

        let Some(token) = abort else {
            return wait.await;
        };

        tokio::select! {
            // `biased` 且应答排在前面：用户按下的那一刻就已经做了决定，
            // 而中断往往是"确认完顺手把这一轮收了"。两者同时就绪时丢掉
            // 已经做出的决定，比丢掉中断更让人无法解释。
            biased;
            result = wait => result,
            _ = token.cancelled() => {
                self.dismiss_request(correlation_id).await;
                Err(DecisionWaitError::Aborted)
            }
        }
    }

    /// 等用户对这一次工具调用拍板。见 [`Self::await_tool_decision_inner`]。
    pub async fn await_tool_decision(
        &self,
        request: ToolConfirmationRequest,
        abort: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<ToolConfirmationResponse, DecisionWaitError> {
        self.await_tool_decision_inner::<ToolConfirmationResponse>(request.into(), abort)
            .await
    }
}

/// 一次门禁等待没能等到"用户的决定"就结束了。
///
/// 注意这里**没有** `TimedOut`：等不到决定不是结局之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionWaitError {
    /// 这一轮被中断（ESC / Ctrl+C）。UI 上那张确认卡已被一并收掉。
    Aborted,
    /// 总线关闭：没有任何订阅者了，不会再有人应答。
    BusClosed,
    /// 请求没能发出去（比如消息体非法），等同无人应答。
    NotPublished,
    /// 运行环境里根本没有人能回答（`-p` headless）：压根不该开始等。
    NoOneToAsk,
}

impl std::fmt::Display for DecisionWaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionWaitError::Aborted => {
                write!(f, "the turn was interrupted before a decision was made")
            }
            DecisionWaitError::BusClosed => {
                write!(f, "the message bus closed before a decision was made")
            }
            DecisionWaitError::NotPublished => {
                write!(f, "the confirmation request never reached the user")
            }
            DecisionWaitError::NoOneToAsk => write!(
                f,
                "this run has nobody who can answer an approval prompt (headless); \
                 run interactively, or set an approval mode that does not ask"
            ),
        }
    }
}

impl std::error::Error for DecisionWaitError {}

// TryFromMessage trait is defined in types.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::policy::types::PolicyEngineConfig;

    /// 造一条 headless 语境的总线：策略层是 non_interactive（没有人能按键）。
    fn headless_bus() -> MessageBus {
        let policy = PolicyEngine::new(PolicyEngineConfig {
            non_interactive: Some(true),
            ..Default::default()
        });
        MessageBus::new(policy, false)
    }

    /// headless + 外部 harness 时，必须认为"有人能回答"——否则等待层会直接
    /// 拒绝工具确认，外部 harness 的批准永远送不到。
    #[tokio::test]
    async fn an_external_responder_counts_as_someone_who_can_answer() {
        let bus = headless_bus();
        // 策略层是 non_interactive（headless 的默认）。
        assert!(!bus.has_a_human_on_the_other_end().await);

        bus.set_external_responder(true);
        assert!(
            bus.has_a_human_on_the_other_end().await,
            "挂了外部应答者之后，等待层必须愿意等"
        );
        assert!(bus.has_external_responder());
    }

    /// 没有外部应答者时，行为必须和以前一样 —— 别让这个开关顺带改了
    /// 交互模式的语义。
    #[tokio::test]
    async fn without_an_external_responder_nothing_changes() {
        let bus = headless_bus();
        assert!(!bus.has_external_responder());
        assert!(!bus.has_a_human_on_the_other_end().await);
    }

    /// 外部应答者标志是**共享**的 —— 总线是 `Arc`，用内部可变性才能在
    /// 拿到 `&Arc` 的情况下设置它（`Arc::get_mut` 要求没有其他强引用，
    /// 实际上永远不满足）。
    #[tokio::test]
    async fn the_external_responder_flag_is_shared_through_an_arc() {
        let bus = std::sync::Arc::new(headless_bus());
        let clone = bus.clone();
        clone.set_external_responder(true);
        assert!(
            bus.has_external_responder(),
            "通过 Arc 克隆设置的标志必须对原句柄可见"
        );
    }
}
