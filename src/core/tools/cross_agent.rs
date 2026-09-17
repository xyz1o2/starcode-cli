//! Cross-agent messaging tools — merged from push_notification + send_message + send_user_file

use crate::agent::subagent::runner::AsyncSubagentRunner;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── SendMessage ──────────────────────────────────────────────────────

/// 消息类型（对标 CCB 结构化协议消息）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
    #[serde(rename = "plain_text")]
    PlainText,
    #[serde(rename = "broadcast")]
    Broadcast,
    #[serde(rename = "shutdown_request")]
    ShutdownRequest,
    #[serde(rename = "plan_approval_request")]
    PlanApprovalRequest,
    #[serde(rename = "permission_request")]
    PermissionRequest,
}

impl Default for MessageType {
    fn default() -> Self {
        Self::PlainText
    }
}

fn default_message_type() -> MessageType {
    MessageType::PlainText
}

#[derive(Clone)]
pub struct SendMessageTool {
    config: Arc<crate::core::config::Config>,
    agent_registry: Option<Arc<AsyncSubagentRunner>>,
}

impl SendMessageTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self {
            config,
            agent_registry: None,
        }
    }

    /// 注入 agent 注册表（启用 broadcast/protocol 消息功能）
    pub fn with_agent_registry(mut self, registry: Arc<AsyncSubagentRunner>) -> Self {
        self.agent_registry = Some(registry);
        self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SendMessageParams {
    pub target_agent: String,
    pub message: String,
    pub summary: String,
    #[serde(default = "default_message_type")]
    pub message_type: MessageType,
    #[serde(default = "default_priority")]
    pub priority: String,
}

fn default_priority() -> String {
    "normal".to_string()
}

pub struct SendMessageInvocation {
    config: Arc<crate::core::config::Config>,
    params: SendMessageParams,
    agent_registry: Option<Arc<AsyncSubagentRunner>>,
}

impl ToolInvocation for SendMessageInvocation {
    fn get_description(&self) -> String {
        format!(
            "Send [{}] to agent '{}' (priority: {})",
            self.params.summary, self.params.target_agent, self.params.priority
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let registry = self.agent_registry.clone();

        Box::pin(async move {
            if params.summary.trim().is_empty() {
                return Err("SendMessage requires a non-empty 'summary' field".into());
            }

            // Broadcast 路径：写所有已注册 agent
            if params.target_agent == "*" || params.message_type == MessageType::Broadcast {
                if let Some(ref reg) = registry {
                    let names = reg.list_agent_names().await;
                    let count = names.len();
                    for name in &names {
                        let _ = reg
                            .deliver_message(name, &params.message, &params.summary, false)
                            .await;
                    }
                    return Ok(ToolResult {
                        llm_content: Some(format!("Broadcast sent to {} agents", count)),
                        return_display: Some(format!("Broadcast to {} agents", count)),
                        output: format!("Broadcast to {} agents", count),
                        error: None,
                        data: Some(serde_json::json!({
                            "status": "broadcast_sent", "targets": names,
                        })),
                    });
                }
            }

            // 协议消息：通过 AsyncSubagentRunner 投递
            let is_protocol = matches!(
                params.message_type,
                MessageType::ShutdownRequest
                    | MessageType::PlanApprovalRequest
                    | MessageType::PermissionRequest
            );

            if let Some(ref reg) = registry {
                return reg
                    .deliver_message(
                        &params.target_agent,
                        &params.message,
                        &params.summary,
                        is_protocol,
                    )
                    .await;
            }

            // 兜底：原有文件 inbox 方式
            let root = config.project_root();
            let inbox_dir = root.join(".star").join("messages");
            if !inbox_dir.exists() {
                tokio::fs::create_dir_all(&inbox_dir)
                    .await
                    .map_err(|e| format!("Failed to create messages dir: {}", e))?;
            }

            let timestamp = chrono::Utc::now().timestamp_millis();
            let filename = format!("{}_{}.json", params.target_agent, timestamp);
            let msg_path = inbox_dir.join(&filename);

            let msg = serde_json::json!({
                "from": "agent",
                "to": params.target_agent,
                "message": params.message,
                "summary": params.summary,
                "message_type": params.message_type,
                "is_protocol": is_protocol,
                "priority": params.priority,
                "timestamp": timestamp,
            });

            let content = serde_json::to_string_pretty(&msg)
                .map_err(|e| format!("Failed to serialize message: {}", e))?;
            tokio::fs::write(&msg_path, content)
                .await
                .map_err(|e| format!("Failed to write message: {}", e))?;

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Message '{}' sent to agent '{}'.",
                    params.summary, params.target_agent
                )),
                return_display: Some(format!("Message sent to {}", params.target_agent)),
                output: format!("Message queued for agent '{}'", params.target_agent),
                error: None,
                data: Some(serde_json::json!({
                    "status": "sent",
                    "target": params.target_agent,
                    "summary": params.summary,
                    "is_protocol": is_protocol,
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn display_name(&self) -> &str {
        "Send Message"
    }

    fn description(&self) -> &str {
        "Send a message to another agent. Use target_agent='*' for broadcast. \
         Use message_type for structured protocol messages (shutdown_request/plan_approval_request/permission_request). \
         summary is REQUIRED (3-10 word short description)."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target_agent": {
                    "type": "string",
                    "description": "Target agent name, or '*' to broadcast to all team members."
                },
                "message": {
                    "type": "string",
                    "description": "Full message content."
                },
                "summary": {
                    "type": "string",
                    "description": "A short 3-10 word summary of the message. REQUIRED."
                },
                "message_type": {
                    "type": "string",
                    "enum": ["plain_text", "broadcast", "shutdown_request", "plan_approval_request", "permission_request"],
                    "description": "Type of message. Default: plain_text."
                },
                "priority": {
                    "type": "string",
                    "enum": ["normal", "high"],
                    "description": "Message priority (default: normal)"
                }
            },
            "required": ["target_agent", "message", "summary"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SendMessageParams = serde_json::from_value(params)?;
        Ok(Box::new(SendMessageInvocation {
            config: self.config.clone(),
            params,
            agent_registry: self.agent_registry.clone(),
        }))
    }
}
