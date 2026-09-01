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
        let correlation_id = Uuid::new_v4().to_string();
        let mut rx = self.subscribe();

        let mut request_msg = request.into();
        if let Message::ToolConfirmationRequest(ref mut req) = request_msg {
            req.correlation_id = correlation_id.clone();
        }

        self.publish(request_msg).await?;

        let timeout = tokio::time::timeout(tokio::time::Duration::from_millis(timeout_ms), async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Some(response) = TResponse::try_from_message(&msg, &correlation_id) {
                            return Ok::<TResponse, Box<dyn std::error::Error>>(response);
                        }
                    }
                    Err(_) => return Err("Channel closed".to_string().into()),
                }
            }
        })
        .await??;

        Ok(timeout)
    }
}

// TryFromMessage trait is defined in types.rs
