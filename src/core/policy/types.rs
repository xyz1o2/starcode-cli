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
    #[serde(rename = "acceptEdits")]
    AcceptEdits, // Accept file edits automatically
    // 对标 Claude Code：官方模式名是 `bypassPermissions`，serde rename 就是它。
    // `alias = "yolo"` 兼容本项目早期落盘的会话记录 / settings —— 读得到旧值，
    // 写出去的永远是官方名。
    #[serde(rename = "bypassPermissions", alias = "yolo")]
    BypassPermissions, // Skip permission
    #[serde(rename = "plan")]
    Plan, // Plan mode (read-only)
    // 对标 Claude Code：`dontAsk` —— 不弹任何确认，会弹的一律直接拒。
    // 给 CI / headless 用：比 bypassPermissions 收得紧（不是全放行，
    // 是「没人回答得了我，就别问」），allow 规则与只读操作仍然可用。
    #[serde(rename = "dontAsk")]
    DontAsk, // Auto-deny anything that would prompt
    #[serde(rename = "auto")]
    Auto, // AI classifier driven: allow safe ops without confirmation
}

impl ApprovalMode {
    /// 档位的**规范 slug** —— 与 serde 的 `rename` 一致，也是
    /// `approval_mode_from_str` 的目标形态。
    ///
    /// 这是档位名��**唯一真源**：settings 的 `defaultMode`、命令行
    /// `--permission-mode`、`/permissions <mode>` 的动词、落盘的会话记录
    /// 全用它。此前这些地方各写一份字面量，同一个 `Default` 在十处被叫成
    /// "Auto" / "Build" / "default" / "默认" 四种名字 —— 加上 `Auto` 档位
    /// 之后，"auto" 这个词同时指两个不同的档位。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::BypassPermissions => "bypassPermissions",
            Self::DontAsk => "dontAsk",
            Self::Auto => "auto",
        }
    }

    /// 档位的**显示标签**（面向用户）。
    ///
    /// 同样只此一份：`Default` 叫 "Default"，真正的 auto 档位叫 "Auto" ——
    /// 两个名字不再撞车。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AcceptEdits => "Accept edits",
            Self::Plan => "Plan",
            Self::BypassPermissions => "Bypass permissions",
            Self::DontAsk => "Don't ask",
            Self::Auto => "Auto",
        }
    }

    /// 中文标签（中文界面用）。
    ///
    /// 英文档位名保留在括号里 —— 用户在英文文档、命令行 `--permission-mode`
    /// 里看到的就是那些名字，中文标签不该把它们藏起来。
    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::Default => "默认 (default)",
            Self::AcceptEdits => "接受编辑 (acceptEdits)",
            Self::Plan => "Plan (计划模式)",
            Self::BypassPermissions => "Bypass (全部放行)",
            Self::DontAsk => "Don't ask (不问直接拒)",
            Self::Auto => "Auto (自动审批)",
        }
    }
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
