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

            // hub 路径（Phase 4）：deliver 全走 AgentRouter —— 单播 /
            // 广播 / UnknownTarget 显式报错，绝不写死信文件。
            // 发件人身份：lead 的 config.agent_identity 是 None，固定 "lead"；
            // 子 agent 由 runner 写入自己的身份。
            if let Some(ref reg) = registry {
                let from_identity = config
                    .agent_identity
                    .clone()
                    .unwrap_or_else(|| crate::agent::messaging::LEAD_IDENTITY.to_string());

                let hub_type = match params.message_type {
                    MessageType::Broadcast => crate::agent::messaging::HubMessageType::PlainText,
                    MessageType::PlainText => crate::agent::messaging::HubMessageType::PlainText,
                    MessageType::ShutdownRequest => {
                        crate::agent::messaging::HubMessageType::TaskAssignment // 关停请求在 hub 侧暂归 plain_text 处理
                    }
                    MessageType::PlanApprovalRequest => {
                        crate::agent::messaging::HubMessageType::PlanApprovalRequest
                    }
                    MessageType::PermissionRequest => {
                        crate::agent::messaging::HubMessageType::PermissionRequest
                    }
                };
                let to = if params.message_type == MessageType::Broadcast {
                    "*".to_string()
                } else {
                    params.target_agent.clone()
                };

                return reg
                    .deliver_message(
                        &from_identity,
                        &to,
                        &params.message,
                        &params.summary,
                        hub_type,
                    )
                    .await;
            }

            let is_protocol = matches!(
                params.message_type,
                MessageType::ShutdownRequest
                    | MessageType::PlanApprovalRequest
                    | MessageType::PermissionRequest
            );

            // hub 未接线（registry None）：显式失败。旧兜底曾把消息写进
            // `.star/messages/` 文件然后报告成功 —— 全仓库没有消费者，消息
            // 发出去即消失（设计文档 §2 断点 1 的死信反模式）。宁可让模型
            // 看到 "not configured"，也不给它一个假成功。
            let _ = is_protocol;
            Err(
                "Messaging hub is not configured in this runtime (SendMessage registry missing). \
                 This session cannot deliver agent-to-agent messages."
                    .into(),
            )
        })
    }
}

impl BaseDeclarativeTool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
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
