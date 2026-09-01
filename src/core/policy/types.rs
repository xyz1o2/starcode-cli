use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    #[serde(rename = "allow")]
    Allow,
    #[serde(rename = "deny")]
    Deny,
    #[serde(rename = "deny_with_reason")]
    DenyWithReason(String),
    #[serde(rename = "ask_user")]
    AskUser,
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

pub fn get_hook_source(input: &HashMap<String, serde_json::Value>) -> HookSource {
    if let Some(source) = input.get("hook_source") {
        if let Some(s) = source.as_str() {
            match s {
                "project" => return HookSource::Project,
                "user" => return HookSource::User,
                "system" => return HookSource::System,
                "extension" => return HookSource::Extension,
                _ => {}
            }
        }
    }
    HookSource::Project
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    #[serde(rename = "default")]
    Default, // Ask for permission
    #[serde(rename = "yolo")]
    Yolo, // Skip permission
    #[serde(rename = "plan")]
    Plan, // Plan mode (read-only)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedPathConfig {
    pub included_args: Option<Vec<String>>,
    pub excluded_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalCheckerConfig {
    #[serde(rename = "type")]
    pub checker_type: String,
    pub name: String,
    pub config: Option<serde_json::Value>,
    pub required_context: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InProcessCheckerType {
    #[serde(rename = "allowed-path")]
    AllowedPath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InProcessCheckerConfig {
    #[serde(rename = "type")]
    pub checker_type: String,
    pub name: InProcessCheckerType,
    pub config: Option<AllowedPathConfig>,
    pub required_context: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SafetyCheckerConfig {
    External(ExternalCheckerConfig),
    InProcess(InProcessCheckerConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub tool_name: Option<String>,
    pub args_pattern: Option<String>,
    pub decision: PolicyDecision,
    pub priority: Option<i32>,
    pub modes: Option<Vec<ApprovalMode>>,
    pub allow_redirection: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyCheckerRule {
    pub tool_name: Option<String>,
    pub args_pattern: Option<String>,
    pub priority: Option<i32>,
    pub checker: SafetyCheckerConfig,
    pub modes: Option<Vec<ApprovalMode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionContext {
    pub event_name: String,
    pub hook_source: Option<HookSource>,
    pub trusted_folder: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookCheckerRule {
    pub event_name: Option<String>,
    pub hook_source: Option<HookSource>,
    pub priority: Option<i32>,
    pub checker: SafetyCheckerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEngineConfig {
    pub rules: Option<Vec<PolicyRule>>,
    pub checkers: Option<Vec<SafetyCheckerRule>>,
    pub hook_checkers: Option<Vec<HookCheckerRule>>,
    pub default_decision: Option<PolicyDecision>,
    pub non_interactive: Option<bool>,
    pub allow_hooks: Option<bool>,
    pub approval_mode: Option<ApprovalMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySettings {
    pub mcp: Option<McpPolicy>,
    pub tools: Option<ToolsPolicy>,
    pub mcp_servers: Option<HashMap<String, McpServerTrust>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPolicy {
    pub excluded: Option<Vec<String>>,
    pub allowed: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsPolicy {
    pub exclude: Option<Vec<String>>,
    pub allowed: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerTrust {
    pub trust: Option<bool>,
}
