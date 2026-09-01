use crate::core::policy::FunctionCall;
use crate::types::ToolConfirmationOutcome;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageBusType {
    #[serde(rename = "tool-confirmation-request")]
    ToolConfirmationRequest,
    #[serde(rename = "tool-confirmation-response")]
    ToolConfirmationResponse,
    #[serde(rename = "tool-policy-rejection")]
    ToolPolicyRejection,
    #[serde(rename = "tool-execution-success")]
    ToolExecutionSuccess,
    #[serde(rename = "tool-execution-failure")]
    ToolExecutionFailure,
    #[serde(rename = "update-policy")]
    UpdatePolicy,
    #[serde(rename = "hook-execution-request")]
    HookExecutionRequest,
    #[serde(rename = "hook-execution-response")]
    HookExecutionResponse,
    #[serde(rename = "hook-policy-decision")]
    HookPolicyDecision,
    #[serde(rename = "tool-started")]
    ToolStarted,
    #[serde(rename = "tool-finished")]
    ToolFinished,
    #[serde(rename = "context-updated")]
    ContextUpdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStarted {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call_id: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFinished {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call_id: String,
    pub tool_name: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextUpdated {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub new_token_count: usize,
    pub messages_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfirmationRequest {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call: FunctionCall,
    pub correlation_id: String,
    pub server_name: Option<String>,
    pub title: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfirmationResponse {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub correlation_id: String,
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ToolConfirmationOutcome>,
    pub requires_user_confirmation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePolicy {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_name: String,
    pub persist: Option<bool>,
    pub args_pattern: Option<String>,
    pub command_prefix: Option<CommandPrefix>,
    pub mcp_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandPrefix {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicyRejection {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionSuccess<T = serde_json::Value> {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call: FunctionCall,
    pub result: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionFailure<E = String> {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub tool_call: FunctionCall,
    pub error: E,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionRequest {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub event_name: String,
    pub input: HashMap<String, serde_json::Value>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionResponse {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub correlation_id: String,
    pub success: bool,
    pub output: Option<HashMap<String, serde_json::Value>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPolicyDecision {
    #[serde(rename = "type")]
    pub message_type: MessageBusType,
    pub event_name: String,
    pub hook_source: HookSource,
    pub decision: HookDecision,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookSource {
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "extension")]
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookDecision {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "deny")]
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    ToolConfirmationRequest(ToolConfirmationRequest),
    ToolConfirmationResponse(ToolConfirmationResponse),
    ToolPolicyRejection(ToolPolicyRejection),
    ToolExecutionSuccess(ToolExecutionSuccess),
    ToolExecutionFailure(ToolExecutionFailure),
    UpdatePolicy(UpdatePolicy),
    HookExecutionRequest(HookExecutionRequest),
    HookExecutionResponse(HookExecutionResponse),
    HookPolicyDecision(HookPolicyDecision),
    ToolStarted(ToolStarted),
    ToolFinished(ToolFinished),
    ContextUpdated(ContextUpdated),
}

pub trait TryFromMessage: Sized {
    fn try_from_message(msg: &Message, correlation_id: &str) -> Option<Self>;
}

impl TryFromMessage for ToolConfirmationResponse {
    fn try_from_message(msg: &Message, correlation_id: &str) -> Option<Self> {
        if let Message::ToolConfirmationResponse(resp) = msg {
            if resp.correlation_id == correlation_id {
                return Some(resp.clone());
            }
        }
        None
    }
}

impl From<ToolConfirmationRequest> for Message {
    fn from(req: ToolConfirmationRequest) -> Self {
        Message::ToolConfirmationRequest(req)
    }
}

impl From<ToolConfirmationResponse> for Message {
    fn from(resp: ToolConfirmationResponse) -> Self {
        Message::ToolConfirmationResponse(resp)
    }
}

impl From<ToolPolicyRejection> for Message {
    fn from(msg: ToolPolicyRejection) -> Self {
        Message::ToolPolicyRejection(msg)
    }
}

impl From<ToolExecutionSuccess> for Message {
    fn from(msg: ToolExecutionSuccess) -> Self {
        Message::ToolExecutionSuccess(msg)
    }
}

impl From<ToolExecutionFailure> for Message {
    fn from(msg: ToolExecutionFailure) -> Self {
        Message::ToolExecutionFailure(msg)
    }
}

impl From<UpdatePolicy> for Message {
    fn from(msg: UpdatePolicy) -> Self {
        Message::UpdatePolicy(msg)
    }
}

impl From<HookExecutionRequest> for Message {
    fn from(msg: HookExecutionRequest) -> Self {
        Message::HookExecutionRequest(msg)
    }
}

impl From<HookExecutionResponse> for Message {
    fn from(msg: HookExecutionResponse) -> Self {
        Message::HookExecutionResponse(msg)
    }
}

impl From<HookPolicyDecision> for Message {
    fn from(msg: HookPolicyDecision) -> Self {
        Message::HookPolicyDecision(msg)
    }
}

impl From<ToolStarted> for Message {
    fn from(msg: ToolStarted) -> Self {
        Message::ToolStarted(msg)
    }
}

impl From<ToolFinished> for Message {
    fn from(msg: ToolFinished) -> Self {
        Message::ToolFinished(msg)
    }
}

impl From<ContextUpdated> for Message {
    fn from(msg: ContextUpdated) -> Self {
        Message::ContextUpdated(msg)
    }
}
