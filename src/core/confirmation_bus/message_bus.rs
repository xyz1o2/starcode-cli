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
}

impl MessageBus {
    pub fn new(policy_engine: PolicyEngine, debug: bool) -> Self {
        let (sender, _) = broadcast::channel(100);
        Self {
            policy_engine: Arc::new(RwLock::new(policy_engine)),
            debug,
            sender,
        }
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
        !self.policy_engine.read().await.is_non_interactive()
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
    use std::time::Duration;

    fn asking_bus() -> MessageBus {
        // yolo 会让 publish 直接回 Allow，测不到"等人回答"这条路径
        MessageBus::new(
            PolicyEngine::new(crate::core::policy::PolicyEngineConfig::default()),
            false,
        )
    }
    fn request_for(name: &str) -> ToolConfirmationRequest {
        ToolConfirmationRequest {
            message_type: MessageBusType::ToolConfirmationRequest,
            tool_call: crate::core::policy::FunctionCall {
                name: name.to_string(),
                args: Some(serde_json::json!({"file_path": "a.rs"})),
            },
            correlation_id: String::new(),
            server_name: None,
            title: Some("t".to_string()),
            prompt: Some("p".to_string()),
            originating_tool_call_id: None,
        }
    }

    /// 把泛型实参钉死：几个测试都在同一个 request/response 对上。
    fn confirmation_wait<'a>(
        bus: &'a MessageBus,
        token: &'a tokio_util::sync::CancellationToken,
    ) -> impl std::future::Future<Output = Result<ToolConfirmationResponse, DecisionWaitError>> + Send + 'a
    {
        bus.await_tool_decision(request_for("Edit"), Some(token))
    }

    /// 回归（P0-2）：确认等待必须能被 abort 打断。
    ///
    /// 全部用 `join!` 而不是 `spawn`：等待 future 的生命周期绑在 `bus` 上，
    /// 而生产调用点也是就地 await。
    #[tokio::test]
    async fn confirmation_wait_unblocks_on_abort() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();

        let wait = confirmation_wait(&bus, &token);
        let canceller = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        };

        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(wait, canceller)
        })
        .await
        .expect("abort 后必须立刻返回");

        assert_eq!(
            result.err(),
            Some(DecisionWaitError::Aborted),
            "被中断的等待必须如实报中断，而不是超时或批准"
        );
    }

    /// 回归：等待**没有**超时。用户没回答就不是结局，门禁必须一直挂着。
    ///
    /// 修复前这里是 `timeout(600s, …)`：用户离开十分钟再回来，看到的是
    /// `Confirmation timeout or error` —— 一次没人否决的写文件请求在用户眼前
    /// 自己变成了失败，模型据此宣布该工具不可用。
    #[tokio::test]
    async fn a_confirmation_waits_as_long_as_the_user_takes() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();
        let mut rx = bus.subscribe();

        // 挂上等待：第一次 poll 才会把请求真正发出去。
        let mut wait = Box::pin(bus.await_tool_decision(request_for("Edit"), Some(&token)));
        let requested_id = loop {
            tokio::select! {
                biased;
                msg = rx.recv() => match msg.expect("应收到确认请求") {
                    Message::ToolConfirmationRequest(req) => break req.correlation_id,
                    _ => continue,
                },
                early = &mut wait => panic!("请求还没发出去，等待就结束了：{early:?}"),
            }
        };

        // 没有人回答：等待必须继续挂着，而不是变成一次失败。
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            early = &mut wait => panic!("没人回答，等待就结束了：{early:?}"),
        }

        // 决定什么时候给都算数，晚了不作废。
        bus.publish(Message::ToolConfirmationResponse(
            ToolConfirmationResponse::from_user_decision(
                requested_id,
                crate::types::ToolConfirmationOutcome::ProceedOnce,
                None,
            ),
        ))
        .await
        .expect("publish decision");

        let response = tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("批准之后必须立刻返回")
            .expect("等待应当成功");
        assert!(response.confirmed, "用户点了允许却拿到取消");
    }

    /// 放弃等待时要补发取消应答：UI 那边的确认卡按 correlation_id 匹配，
    /// 只把本地 future 丢掉的话它会一直挂在屏幕上。
    #[tokio::test]
    async fn abort_broadcasts_a_cancel_response_for_the_same_correlation_id() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();
        let mut rx = bus.subscribe();
        // 独立订阅者：取消应答是在 `await_tool_decision` 内部发的，
        // broadcast 会为已订阅但没轮询的接收者缓冲。
        let mut observer = bus.subscribe();

        let wait = confirmation_wait(&bus, &token);
        let driver = async {
            let id = loop {
                match rx.recv().await.expect("应收到确认请求") {
                    Message::ToolConfirmationRequest(req) => break req.correlation_id,
                    _ => continue,
                }
            };
            token.cancel();
            id
        };

        let (_, requested_id) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(wait, driver) })
                .await
                .expect("abort 后必须立刻返回");

        let cancel = loop {
            match tokio::time::timeout(Duration::from_secs(2), observer.recv())
                .await
                .expect("abort 后应补发一条应答")
                .expect("通道不应关闭")
            {
                Message::ToolConfirmationResponse(resp) if resp.correlation_id == requested_id => {
                    break resp;
                }
                _ => continue,
            }
        };
        assert!(!cancel.confirmed);
        assert_eq!(
            cancel.outcome,
            Some(crate::types::ToolConfirmationOutcome::Cancel)
        );
    }

    /// 回归：确认请求**不能**在 publish 里被策略的第二次判定自动放行。
    ///
    /// 能走到 `publish` 的请求，调用方已经判过"要问用户"。这里再判一次、而且只
    /// 允许它抬高判定，于是两次判定之间只要状态动过（切 yolo、`/permissions`
    /// 重载规则、子代理并发），命令就会在 UI 一张卡都没画的情况下跑掉 ——
    /// 也就是"还没同意就执行了"。
    ///
    /// 这条用 yolo 构造最容易复现：策略此刻说 Allow，而请求本身已经带着
    /// "要问用户"的决定走到这里。
    #[tokio::test]
    async fn publish_never_self_approves_a_confirmation_request() {
        let bus = MessageBus::new(
            PolicyEngine::new(crate::core::policy::PolicyEngineConfig {
                approval_mode: Some(crate::core::policy::types::ApprovalMode::Yolo),
                ..Default::default()
            }),
            false,
        );
        let mut rx = bus.subscribe();

        let mut request = request_for("Bash");
        request.correlation_id = "corr-auto-approve".to_string();
        bus.publish(Message::ToolConfirmationRequest(request))
            .await
            .expect("publish confirmation request");

        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("应收到广播")
            .expect("通道不应关闭");
        match msg {
            Message::ToolConfirmationRequest(req) => {
                assert_eq!(req.correlation_id, "corr-auto-approve");
            }
            Message::ToolConfirmationResponse(resp) => panic!(
                "没有任何人点过同意就回了应答（confirmed={}）",
                resp.confirmed
            ),
            other => panic!("期望确认请求，实际 {other:?}"),
        }

        // 请求之后不该再有第二条消息 —— 特别是那条自动生成的 confirmed=true。
        if let Ok(extra) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            match extra {
                Ok(Message::ToolConfirmationResponse(resp)) => panic!(
                    "没有任何人点过同意就回了应答（confirmed={}）",
                    resp.confirmed
                ),
                Ok(_) | Err(_) => {}
            }
        }
    }

    /// 回归：`-p` headless 里没有人能回答，就**不能开始等**。
    ///
    /// 等待没有超时，一旦进去就出不来了。策略层已经把 `AskUser` 翻成 `Deny`，
    /// 但工具自己提出的确认要求（未信任目录之类）不经过策略 —— 没有这道闸门，
    /// `starcode -p` 会永远挂在那里。
    #[tokio::test]
    async fn a_run_with_nobody_to_ask_never_starts_waiting() {
        let bus = MessageBus::new(
            PolicyEngine::new(crate::core::policy::PolicyEngineConfig {
                non_interactive: Some(true),
                ..Default::default()
            }),
            false,
        );
        let mut rx = bus.subscribe();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            bus.await_tool_decision(request_for("Edit"), None),
        )
        .await
        .expect("没有人能回答时必须立刻返回，不能挂住");

        assert_eq!(
            result.err(),
            Some(DecisionWaitError::NoOneToAsk),
            "headless 下必须直接拒绝等待，而不是等一个不会来的回答"
        );
        // 连卡片都不该画出去：没有任何 UI 能处理它。
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "没有人能回答时不该广播确认请求"
        );
    }

    /// 回归：用户按 ESC 是**取消**，不是失败，也不能被当成超时。
    ///
    /// 取消走的是一条普通的应答（`confirmed=false` + `outcome=Cancel`），等待方
    /// 拿到的是 `Ok`，因此上层能区分"用户拒绝"和"这一轮被中断"——后者必须报
    /// 中断，否则模型会以为换个写法重试就能过。
    #[tokio::test]
    async fn an_explicit_cancel_arrives_as_a_decision_not_as_a_wait_error() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();
        let mut rx = bus.subscribe();

        let wait = confirmation_wait(&bus, &token);
        let canceller = async {
            let id = loop {
                match rx.recv().await.expect("应收到确认请求") {
                    Message::ToolConfirmationRequest(req) => break req.correlation_id,
                    _ => continue,
                }
            };
            bus.publish(Message::ToolConfirmationResponse(
                ToolConfirmationResponse::from_user_decision(
                    id,
                    crate::types::ToolConfirmationOutcome::Cancel,
                    None,
                ),
            ))
            .await
            .ok();
        };

        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(wait, canceller)
        })
        .await
        .expect("取消后必须立刻返回");

        let response = result.expect("用户取消是一次决定，不是等待失败");
        assert!(!response.confirmed, "取消不能被算成批准");
        assert_eq!(
            response.outcome,
            Some(crate::types::ToolConfirmationOutcome::Cancel)
        );
    }

    /// 回归：正常回答仍然按 correlation_id 匹配回来（`select!` 没把应答吃掉）。
    #[tokio::test]
    async fn confirmation_still_resolves_when_answered() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();
        let mut rx = bus.subscribe();

        let wait = confirmation_wait(&bus, &token);
        let responder = async {
            let id = loop {
                match rx.recv().await.expect("应收到确认请求") {
                    Message::ToolConfirmationRequest(req) => break req.correlation_id,
                    _ => continue,
                }
            };
            bus.publish(Message::ToolConfirmationResponse(
                ToolConfirmationResponse {
                    message_type: MessageBusType::ToolConfirmationResponse,
                    correlation_id: id,
                    confirmed: true,
                    requires_user_confirmation: None,
                    outcome: Some(crate::types::ToolConfirmationOutcome::ProceedOnce),
                    feedback: None,
                },
            ))
            .await
            .ok();
        };

        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(wait, responder)
        })
        .await
        .expect("应答后必须返回");
        assert!(
            result.expect("应答应被匹配上").confirmed,
            "用户点了允许却拿到取消"
        );
    }
}
