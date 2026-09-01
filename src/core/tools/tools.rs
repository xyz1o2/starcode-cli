use crate::core::confirmation_bus::MessageBus;
use crate::types::ToolConfirmationOutcome;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolLocation {
    pub path: PathBuf,
    pub location_type: LocationType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationType {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Read,
    Edit,
    Search,
    Execute,
    Think,
    Other,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffStat {
    pub model_added_lines: usize,
    pub model_removed_lines: usize,
    pub model_added_chars: usize,
    pub model_removed_chars: usize,
    pub user_added_lines: usize,
    pub user_removed_lines: usize,
    pub user_added_chars: usize,
    pub user_removed_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub file_diff: String,
    pub file_name: String,
    pub original_content: Option<String>,
    pub new_content: String,
    pub diff_stat: Option<DiffStat>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    pub llm_content: Option<String>,
    pub return_display: Option<String>,
    pub output: String,
    pub error: Option<ToolError>,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ToolError {
    pub error_type: String,
    pub message: String,
}

#[derive(Clone)]
pub struct ToolCallConfirmationDetails {
    pub confirmation_type: ConfirmationType,
    pub title: String,
    pub prompt: String,
    pub on_confirm: Arc<dyn Fn(ToolConfirmationOutcome) + Send + Sync>,
}

impl std::fmt::Debug for ToolCallConfirmationDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCallConfirmationDetails")
            .field("confirmation_type", &self.confirmation_type)
            .field("title", &self.title)
            .field("prompt", &self.prompt)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationType {
    Info,
    Warning,
    Danger,
    Ask,
}

pub trait ToolInvocation: Send + Sync {
    fn get_description(&self) -> String;
    fn tool_locations(&self) -> Vec<ToolLocation>;
    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }
    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    >;
}

pub trait BaseDeclarativeTool: Send + Sync {
    fn name(&self) -> &str;
    fn display_name(&self) -> &str;
    fn description(&self) -> &str;
    fn kind(&self) -> Kind;
    fn parameter_schema(&self) -> serde_json::Value;

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>>;

    fn is_output_markdown(&self) -> bool {
        false
    }

    fn can_update_output(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn permission_cache_identity(&self) -> Option<String> {
        None
    }

    fn normalize_confirmation_outcome(
        &self,
        outcome: crate::types::ToolConfirmationOutcome,
    ) -> crate::types::ToolConfirmationOutcome {
        outcome
    }

    /// 工具结果的最大字符数限制（对标 Claude Code 的 maxResultSizeChars）
    ///
    /// 超出限制的结果会被截断，完整内容持久化到磁盘。
    /// 返回 None 表示使用默认限制。
    /// 返回 Some(usize::MAX) 表示不限制（如 FileReadTool）。
    fn max_result_size_chars(&self) -> Option<usize> {
        None // 使用全局默认值
    }
}

pub struct BaseToolInvocation {
    pub params: serde_json::Value,
    pub message_bus: Option<MessageBus>,
    pub tool_name: Option<String>,
    pub tool_display_name: Option<String>,
    pub server_name: Option<String>,
}

impl BaseToolInvocation {
    pub fn new(
        params: serde_json::Value,
        message_bus: Option<MessageBus>,
        tool_name: Option<String>,
        tool_display_name: Option<String>,
        server_name: Option<String>,
    ) -> Self {
        Self {
            params,
            message_bus,
            tool_name,
            tool_display_name,
            server_name,
        }
    }

    pub fn get_tool_name(&self) -> &str {
        self.tool_name.as_deref().unwrap_or("unknown")
    }

    pub fn get_display_name(&self) -> &str {
        self.tool_display_name
            .as_deref()
            .unwrap_or_else(|| self.get_tool_name())
    }

    pub fn get_server_name(&self) -> Option<&str> {
        self.server_name.as_deref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(rename = "parametersJsonSchema")]
    pub parameters_json_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub text: Option<String>,
    pub match_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    pub path: String,
    pub name: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResult {
    #[serde(rename = "type")]
    pub result_type: String, // "text" or "file"
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<u32>,
}
