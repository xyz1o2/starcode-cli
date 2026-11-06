use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentTerminateMode {
    #[serde(rename = "ERROR")]
    Error,
    #[serde(rename = "TIMEOUT")]
    Timeout,
    #[serde(rename = "GOAL")]
    Goal,
    #[serde(rename = "MAX_TURNS")]
    MaxTurns,
    #[serde(rename = "ABORTED")]
    Aborted,
    #[serde(rename = "ERROR_NO_COMPLETE_TASK_CALL")]
    ErrorNoCompleteTaskCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputObject {
    pub result: String,
    pub terminate_reason: AgentTerminateMode,
}

pub type AgentInputs = HashMap<String, serde_json::Value>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAgentInputs {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentActivityEvent {
    pub is_subagent_activity_event: bool,
    pub agent_name: String,
    pub event_type: SubagentEventType,
    pub data: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubagentEventType {
    #[serde(rename = "TOOL_CALL_START")]
    ToolCallStart,
    #[serde(rename = "TOOL_CALL_END")]
    ToolCallEnd,
    #[serde(rename = "THOUGHT_CHUNK")]
    ThoughtChunk,
    #[serde(rename = "ERROR")]
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseAgentDefinition {
    pub name: String,
    pub display_name: Option<String>,
    pub description: String,
    pub input_config: InputConfig,
    pub output_config: Option<OutputConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentDefinition {
    #[serde(flatten)]
    pub base: BaseAgentDefinition,
    pub kind: AgentKind,
    pub prompt_config: PromptConfig,
    pub model_config: ModelConfig,
    pub run_config: RunConfig,
    pub tool_config: Option<ToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAgentDefinition {
    #[serde(flatten)]
    pub base: BaseAgentDefinition,
    pub kind: AgentKind,
    pub agent_card_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "remote")]
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentDefinition {
    Local(LocalAgentDefinition),
    Remote(RemoteAgentDefinition),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    pub system_prompt: Option<String>,
    pub initial_messages: Option<Vec<Content>>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(TextContent),
    FunctionCall(FunctionCallContent),
    FunctionResponse(FunctionResponseContent),
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
    InlineData { inline_data: InlineData },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallContent {
    pub role: String,
    pub function_call: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponseContent {
    pub role: String,
    pub function_response: FunctionResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    pub name: String,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub tools: Vec<ToolReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolReference {
    String(String),
    Object(FunctionDeclaration),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub inputs: HashMap<String, InputField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputField {
    pub description: String,
    pub field_type: FieldType,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "string[]")]
    StringArray,
    #[serde(rename = "number[]")]
    NumberArray,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub output_name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub model: String,
    pub temp: f64,
    pub top_p: f64,
    pub thinking_budget: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub max_time_minutes: u64,
    pub max_turns: Option<i32>,
}

// ── 多 Agent 编排扩展类型 ────────────────────────────────────────────
// 对标 CCB sub-agents.mdx §AgentTool 输入参数 + coordinator-and-swarm.mdx

/// 命名 SubAgent 类型（对标 CCB 的 built-in agent definitions）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentType {
    GeneralPurpose,
    Explorer,
    Analyzer,
    Editor,
    CodeReviewer,
}

impl SubagentType {
    /// 序列化用的 slug（与 `#[serde(rename_all = "snake_case")]` 一致）
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GeneralPurpose => "general_purpose",
            Self::Explorer => "explorer",
            Self::Analyzer => "analyzer",
            Self::Editor => "editor",
            Self::CodeReviewer => "code_reviewer",
        }
    }

    /// 用户可见名称（对标 `AgentTool/UI.tsx::userFacingName`）
    ///
    /// 参考实现把内置的 general-purpose agent 显示为 "Agent"，
    /// 自定义类型则原样显示 slug。
    pub fn user_facing_name(&self) -> &'static str {
        match self {
            Self::GeneralPurpose => "Agent",
            Self::Explorer => "Explore",
            Self::Analyzer => "Analyze",
            Self::Editor => "Edit",
            Self::CodeReviewer => "Review",
        }
    }

    /// 是否为自定义（非内置 general-purpose）类型（对标 `isCustomSubagentType`）
    pub fn is_custom(&self) -> bool {
        !matches!(self, Self::GeneralPurpose)
    }

    /// 把模型 / skill frontmatter 给的任意写法解析成子代理类型。
    ///
    /// 全仓库唯一的别名归一入口：agent 参数归一（tool_executor）和 skill
    /// frontmatter `agent:` 字段（skill.rs）都走这里，各自不再维护别名表。
    /// 归一规则不枚举别名：小写并丢弃 `_`/`-`/空格后，与每个变体自身的 slug
    /// 和展示名比对 —— 精确相等或前缀命中（前缀至少 4 个字符，避免 "ana"
    /// 这类截断猜错方向）。
    /// 未识别返回 None：自定义 agent 的名字由调用方原样透传。
    pub fn from_name(raw: &str) -> Option<Self> {
        let canon = |s: &str| -> String {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect()
        };
        let target = canon(raw);
        if target.len() < 4 {
            return None;
        }
        for variant in [
            Self::GeneralPurpose,
            Self::Explorer,
            Self::Analyzer,
            Self::Editor,
            Self::CodeReviewer,
        ] {
            let slug = canon(variant.as_str());
            let display = canon(variant.user_facing_name());
            if slug == target
                || display == target
                || slug.starts_with(&target)
                || display.starts_with(&target)
                || target.starts_with(&slug)
                || target.starts_with(&display)
            {
                return Some(variant);
            }
        }
        None
    }
}

/// `Option<SubagentType>` 的用户可见标签：`None` 代表 fork（继承父上下文）
pub fn agent_type_label(subagent_type: Option<&SubagentType>) -> &'static str {
    match subagent_type {
        Some(t) => t.user_facing_name(),
        None => "Fork",
    }
}

/// Agent 隔离模式（对标 CCB isolation 参数）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentIsolation {
    None,
    Worktree,
}

/// Agent 执行模式（对标 CCB sync/async/fork 三条路径）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentExecutionMode {
    Sync,
    Async,
    Fork,
}

/// AgentTool 完整输入参数（对标 CCB AgentTool fullInputSchema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolFullInput {
    /// 必填：3-5 词短描述，用于 UI/spinner/通知摘要
    pub description: String,
    /// 必填：完整任务说明
    pub prompt: String,
    /// 命名 Agent 类型（省略时默认 GeneralPurpose 或走 fork）
    pub subagent_type: Option<SubagentType>,
    /// agent 名称，供 SendMessage 按 name 寻址
    pub name: Option<String>,
    /// 隔离模式
    pub isolation: Option<AgentIsolation>,
    /// 模型覆盖
    pub model: Option<String>,
    /// 显式请求后台异步执行
    pub background: Option<bool>,
    /// 最大轮次数
    pub max_rounds: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部别名写法必须落到与旧硬编码表相同的变体 —— 两个调用方
    /// （tool_executor / skill.rs）删表前靠这套用例圈行为。
    #[test]
    fn from_name_covers_every_previous_alias_arm() {
        use SubagentType::*;
        for (raw, expected) in [
            ("general_purpose", GeneralPurpose),
            ("generalpurpose", GeneralPurpose),
            ("general", GeneralPurpose),
            ("general-purpose", GeneralPurpose),
            ("GeneralPurpose", GeneralPurpose),
            ("explorer", Explorer),
            ("explore", Explorer),
            ("Explore", Explorer),
            ("analyzer", Analyzer),
            ("analyze", Analyzer),
            ("editor", Editor),
            ("edit", Editor),
            ("code_reviewer", CodeReviewer),
            ("codereviewer", CodeReviewer),
            ("reviewer", CodeReviewer),
            ("review", CodeReviewer),
            ("Code Reviewer", CodeReviewer),
        ] {
            assert_eq!(
                SubagentType::from_name(raw),
                Some(expected.clone()),
                "{raw:?} 必须归一到 {expected:?}"
            );
        }
    }

    /// 未识别（自定义 agent）或过短截断必须返回 None，调用方才能透传。
    #[test]
    fn from_name_rejects_custom_names_and_short_prefixes() {
        assert_eq!(SubagentType::from_name("my_custom_agent"), None);
        assert_eq!(SubagentType::from_name("ana"), None);
        assert_eq!(SubagentType::from_name(""), None);
    }
}

/// SubAgent 权限模式（对标 CCB permissionMode）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubagentPermissionMode {
    AcceptEdits,
    Default,
    BypassPermissions,
    Bubble, // fork 专用：权限上浮到父级
    Plan,   // 只读模式
}

impl Default for SubagentPermissionMode {
    fn default() -> Self {
        Self::AcceptEdits
    }
}
