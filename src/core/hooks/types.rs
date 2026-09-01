use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigSource {
    #[serde(rename = "project")]
    Project,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "extensions")]
    Extensions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookEventName {
    #[serde(rename = "BeforeTool")]
    BeforeTool,
    #[serde(rename = "AfterTool")]
    AfterTool,
    #[serde(rename = "BeforeAgent")]
    BeforeAgent,
    #[serde(rename = "Notification")]
    Notification,
    #[serde(rename = "AfterAgent")]
    AfterAgent,
    #[serde(rename = "SessionStart")]
    SessionStart,
    #[serde(rename = "SessionEnd")]
    SessionEnd,
    #[serde(rename = "PreCompact", alias = "PreCompress")]
    PreCompact,
    #[serde(rename = "BeforeModel")]
    BeforeModel,
    #[serde(rename = "AfterModel")]
    AfterModel,
    #[serde(rename = "BeforeToolSelection")]
    BeforeToolSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    StopFailure,
    SessionStart,
    SessionEnd,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PostCompact,
    UserPromptSubmit,
    FileChanged,
    ConfigChange,
    PermissionRequest,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Stop => "Stop",
            Self::StopFailure => "StopFailure",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::FileChanged => "FileChanged",
            Self::ConfigChange => "ConfigChange",
            Self::PermissionRequest => "PermissionRequest",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHookConfig {
    #[serde(rename = "type")]
    pub hook_type: HookType,
    pub command: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub timeout: Option<u64>,
    pub source: Option<ConfigSource>,
}

pub type HookConfig = CommandHookConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub hook_type: EnhancedHookType,
    pub command: Option<String>,
    pub url: Option<String>,
    pub prompt: Option<String>,
    pub timeout_ms: u64,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyHookDefinition {
    pub matcher: Option<String>,
    pub sequential: Option<bool>,
    pub hooks: Vec<HookConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookType {
    #[serde(rename = "command")]
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnhancedHookType {
    #[serde(rename = "shell")]
    Shell,
    #[serde(rename = "http")]
    Http,
    #[serde(rename = "agent")]
    Agent,
    #[serde(rename = "function")]
    Function,
}

pub fn get_hook_key(hook: &HookConfig) -> String {
    let name = hook.name.as_deref().unwrap_or("");
    let command = hook.command.as_str();
    format!("{}:{}", name, command)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookDecision {
    Ask,
    Block,
    Deny,
    Approve,
    Allow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnhancedHookDecision {
    Allow,
    Deny,
    Block(String),
    Ask(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    pub decision: EnhancedHookDecision,
    pub updated_input: Option<serde_json::Value>,
    pub additional_context: Option<String>,
    pub message: Option<String>,
}

impl HookResult {
    pub fn allow() -> Self {
        Self {
            decision: EnhancedHookDecision::Allow,
            updated_input: None,
            additional_context: None,
            message: None,
        }
    }

    pub fn deny() -> Self {
        Self {
            decision: EnhancedHookDecision::Deny,
            updated_input: None,
            additional_context: None,
            message: None,
        }
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            decision: EnhancedHookDecision::Block(reason.into()),
            updated_input: None,
            additional_context: None,
            message: None,
        }
    }

    pub fn ask(prompt: impl Into<String>) -> Self {
        Self {
            decision: EnhancedHookDecision::Ask(prompt.into()),
            updated_input: None,
            additional_context: None,
            message: None,
        }
    }

    pub fn with_updated_input(mut self, input: serde_json::Value) -> Self {
        self.updated_input = Some(input);
        self
    }

    pub fn with_additional_context(mut self, ctx: impl Into<String>) -> Self {
        self.additional_context = Some(ctx.into());
        self
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    pub fn is_blocking(&self) -> bool {
        matches!(
            self.decision,
            EnhancedHookDecision::Deny | EnhancedHookDecision::Block(_)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub hook_event_name: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookOutput {
    pub continue_execution: Option<bool>,
    pub stop_reason: Option<String>,
    pub suppress_output: Option<bool>,
    pub system_message: Option<String>,
    pub decision: Option<HookDecision>,
    pub reason: Option<String>,
    pub hook_specific_output: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone)]
pub struct DefaultHookOutput {
    pub continue_execution: Option<bool>,
    pub stop_reason: Option<String>,
    pub suppress_output: Option<bool>,
    pub system_message: Option<String>,
    pub decision: Option<HookDecision>,
    pub reason: Option<String>,
    pub hook_specific_output: Option<HashMap<String, serde_json::Value>>,
}

impl DefaultHookOutput {
    pub fn new(data: Option<HashMap<String, serde_json::Value>>) -> Self {
        Self {
            continue_execution: data
                .as_ref()
                .and_then(|d| d.get("continue_execution").and_then(|v| v.as_bool())),
            stop_reason: data.as_ref().and_then(|d| {
                d.get("stopReason")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            }),
            suppress_output: data
                .as_ref()
                .and_then(|d| d.get("suppressOutput").and_then(|v| v.as_bool())),
            system_message: data.as_ref().and_then(|d| {
                d.get("systemMessage")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            }),
            decision: data
                .as_ref()
                .and_then(|d| d.get("decision").and_then(|v| Self::parse_decision(v))),
            reason: data.as_ref().and_then(|d| {
                d.get("reason")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            }),
            hook_specific_output: data.clone(),
        }
    }

    fn parse_decision(value: &serde_json::Value) -> Option<HookDecision> {
        match value.as_str() {
            Some("ask") => Some(HookDecision::Ask),
            Some("block") => Some(HookDecision::Block),
            Some("deny") => Some(HookDecision::Deny),
            Some("approve") => Some(HookDecision::Approve),
            Some("allow") => Some(HookDecision::Allow),
            _ => None,
        }
    }

    pub fn is_blocking_decision(&self) -> bool {
        matches!(
            self.decision,
            Some(HookDecision::Block) | Some(HookDecision::Deny)
        )
    }

    pub fn should_stop_execution(&self) -> bool {
        self.continue_execution == Some(false)
    }

    pub fn get_effective_reason(&self) -> String {
        self.stop_reason
            .as_ref()
            .or(self.reason.as_ref())
            .unwrap_or(&"No reason provided".to_string())
            .clone()
    }

    pub fn get_additional_context(&self) -> Option<String> {
        self.hook_specific_output
            .as_ref()
            .and_then(|map| map.get("additionalContext"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    pub fn get_blocking_error(&self) -> BlockingError {
        if self.is_blocking_decision() {
            BlockingError {
                blocked: true,
                reason: self.get_effective_reason(),
            }
        } else {
            BlockingError {
                blocked: false,
                reason: String::new(),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BlockingError {
    pub blocked: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub tool_name: String,
    pub tool_input: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterToolInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub tool_name: String,
    pub tool_input: HashMap<String, serde_json::Value>,
    pub tool_response: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeAgentInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationType {
    #[serde(rename = "ToolPermission")]
    ToolPermission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub notification_type: NotificationType,
    pub message: String,
    pub details: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationOutput {
    pub suppress_output: Option<bool>,
    pub system_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterAgentInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub prompt: String,
    pub prompt_response: String,
    pub stop_hook_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStartSource {
    #[serde(rename = "startup")]
    Startup,
    #[serde(rename = "resume")]
    Resume,
    #[serde(rename = "clear")]
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub source: SessionStartSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEndReason {
    #[serde(rename = "exit")]
    Exit,
    #[serde(rename = "clear")]
    Clear,
    #[serde(rename = "logout")]
    Logout,
    #[serde(rename = "prompt_input_exit")]
    PromptInputExit,
    #[serde(rename = "other")]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub reason: SessionEndReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreCompressTrigger {
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCompressInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub trigger: PreCompressTrigger,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCompressOutput {
    pub suppress_output: Option<bool>,
    pub system_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRequest {
    pub contents: Vec<Content>,
    pub generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(TextContent),
    FunctionCall(FunctionCallContent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextContent {
    pub role: String,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Text { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallContent {
    pub role: String,
    pub function_call: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub content: Content,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeModelInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub llm_request: LLMRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeModelOutput {
    #[serde(flatten)]
    pub base: HookOutput,
    pub hook_specific_output: Option<BeforeModelSpecificOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeModelSpecificOutput {
    pub hook_event_name: String,
    pub llm_request: Option<LLMRequest>,
    pub llm_response: Option<LLMResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterModelInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub llm_request: LLMRequest,
    pub llm_response: LLMResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterModelOutput {
    #[serde(flatten)]
    pub base: HookOutput,
    pub hook_specific_output: Option<AfterModelSpecificOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterModelSpecificOutput {
    pub hook_event_name: String,
    pub llm_response: Option<LLMResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolSelectionInput {
    #[serde(flatten)]
    pub base: HookInput,
    pub llm_request: LLMRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolSelectionOutput {
    #[serde(flatten)]
    pub base: HookOutput,
    pub hook_specific_output: Option<BeforeToolSelectionSpecificOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolSelectionSpecificOutput {
    pub hook_event_name: String,
    pub tool_config: Option<ToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub function_calling_config: Option<FunctionCallingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionResult {
    pub hook_config: HookConfig,
    pub event_name: HookEventName,
    pub success: bool,
    pub output: Option<HookOutput>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub exit_code: Option<i32>,
    pub duration: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionPlan {
    pub event_name: HookEventName,
    pub hook_configs: Vec<HookConfig>,
    pub sequential: bool,
}
