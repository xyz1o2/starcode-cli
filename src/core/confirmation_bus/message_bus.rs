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
                    PolicyDecision::Allow => {
                        self.emit_message(Message::ToolConfirmationResponse(
                            ToolConfirmationResponse {
                                message_type: MessageBusType::ToolConfirmationResponse,
                                correlation_id: req.correlation_id,
                                confirmed: true,
                                requires_user_confirmation: None,
                                outcome: None,
                                feedback: None,
                            },
                        ))
                        .await;
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

    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.sender.subscribe()
    }

    pub async fn request<TRequest, TResponse>(
        &self,
        request: TRequest,
        _response_type: MessageBusType,
        timeout_ms: u64,
    ) -> Result<TResponse, Box<dyn std::error::Error>>
    where
        TRequest: Into<Message>,
        TResponse: TryFromMessage,
    {
        self.request_cancellable(request, timeout_ms, None).await
    }

    /// 与 [`Self::request`] 相同，但可以被 abort 打断。
    ///
    /// 原实现的等待是 `tokio::time::timeout(600s, …)` 且完全不看 abort 信号，于是
    /// "用户在某个确认上按了 ESC" 与 "这一轮被中断" 是两件互不相干的事：工具执行层
    /// 一直挂到超时才返回（默认 10 分钟无响应），而等待方是子代理发起的确认时，
    /// UI 上根本没有对应的卡片可按。
    ///
    /// 放弃等待前必须**补发一条取消应答**：请求本身是广播出去的，UI 与其它等待方都
    /// 按 correlation_id 匹配应答，只把本地 future 丢掉会让它们继续挂着。
    pub async fn request_cancellable<TRequest, TResponse>(
        &self,
        request: TRequest,
        timeout_ms: u64,
        abort: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<TResponse, Box<dyn std::error::Error>>
    where
        TRequest: Into<Message>,
        TResponse: TryFromMessage,
    {
        let correlation_id = Uuid::new_v4().to_string();
        let mut rx = self.subscribe();

        let mut request_msg = request.into();
        if let Message::ToolConfirmationRequest(ref mut req) = request_msg {
            req.correlation_id = correlation_id.clone();
        }

        self.publish(request_msg).await?;

        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Some(response) = TResponse::try_from_message(&msg, &correlation_id) {
                            return Ok::<TResponse, &'static str>(response);
                        }
                    }
                    Err(_) => return Err("Channel closed"),
                }
            }
        };

        let timeout = tokio::time::timeout(tokio::time::Duration::from_millis(timeout_ms), wait);

        // 内部错误用 `&'static str` 而不是 `Box<dyn Error>`：等待 future 会被
        // `select!` 跨线程 await，对外那个非 Send 的错误类型进不了这个 future。
        let timeout_err = || -> Box<dyn std::error::Error> { "Confirmation timed out".into() };

        match abort {
            None => match timeout.await {
                Ok(result) => result.map_err(|e| -> Box<dyn std::error::Error> { e.into() }),
                Err(_) => Err(timeout_err()),
            },
            Some(token) => {
                tokio::select! {
                    biased;
                    result = timeout => match result {
                        Ok(result) => {
                            result.map_err(|e| -> Box<dyn std::error::Error> { e.into() })
                        }
                        Err(_) => Err(timeout_err()),
                    },
                    _ = token.cancelled() => {
                        // 用同一个 correlation_id 广播取消，别让 UI 的确认卡一直挂着
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
                        Err("Request cancelled".to_string().into())
                    }
                }
            }
        }
    }
}

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
        }
    }

    /// 把泛型实参钉死：三个测试都在同一个 request/response 对上。
    fn confirmation_wait<'a>(
        bus: &'a MessageBus,
        token: &'a tokio_util::sync::CancellationToken,
    ) -> impl std::future::Future<
        Output = Result<ToolConfirmationResponse, Box<dyn std::error::Error>>,
    > + Send
    + 'a {
        bus.request_cancellable(request_for("Edit"), 600_000, Some(token))
    }

    /// 回归（P0-2）：确认等待必须能被 abort 打断。
    ///
    /// 修复前这里是纯 `timeout(600s, …)`：用户中断这一轮之后，工具执行层仍然挂着，
    /// 最长十分钟没有任何反馈；而等待方是子代理发起的确认时，UI 上根本没有可按的卡片。
    ///
    /// 全部用 `join!` 而不是 `spawn`：对外的错误类型是 `Box<dyn Error>`（非 Send），
    /// 跨 spawn 边界过不去，而生产调用点是就地 await。
    #[tokio::test]
    async fn confirmation_wait_unblocks_on_abort() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();

        let wait = confirmation_wait(&bus, &token);
        let canceller = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        };

        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(wait, canceller) })
                .await
                .expect("abort 后必须立刻返回，不能等 600 秒超时");

        assert!(result.is_err(), "被中断的等待必须返回错误而不是确认通过");
    }

    /// 放弃等待时要补发取消应答：UI 那边的确认卡按 correlation_id 匹配，
    /// 只把本地 future 丢掉的话它会一直挂在屏幕上。
    #[tokio::test]
    async fn abort_broadcasts_a_cancel_response_for_the_same_correlation_id() {
        let bus = asking_bus();
        let token = tokio_util::sync::CancellationToken::new();
        let mut rx = bus.subscribe();
        // 独立订阅者：取消应答是在 `request_cancellable` 内部发的，
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

    /// 反向保证：正常回答仍然按 correlation_id 匹配回来（`select!` 没把应答吃掉）。
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
            bus.publish(Message::ToolConfirmationResponse(ToolConfirmationResponse {
                message_type: MessageBusType::ToolConfirmationResponse,
                correlation_id: id,
                confirmed: true,
                requires_user_confirmation: None,
                outcome: Some(crate::types::ToolConfirmationOutcome::ProceedOnce),
                feedback: None,
            }))
            .await
            .ok();
        };

        let (result, ()) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(wait, responder) })
                .await
                .expect("应答后必须返回");
        assert!(
            result.expect("应答应被匹配上").confirmed,
            "用户点了允许却拿到取消"
        );
    }
}
