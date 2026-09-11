use crate::types::{StarMessage, StarToolCall, StarUsage};
use serde_json::Value;

/// 缓存警告配置
#[derive(Debug, Clone)]
pub(crate) struct CacheWarningConfig {
    /// 是否启用缓存警告
    pub enabled: bool,
    /// 缓存创建token阈值（高于此值会警告）
    pub creation_token_threshold: u64,
}

impl Default for CacheWarningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            creation_token_threshold: 10000,
        }
    }
}

impl CacheWarningConfig {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_env() -> Self {
        let enabled = std::env::var("STAR_CACHE_WARNING_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let creation_token_threshold = std::env::var("STAR_CACHE_CREATION_TOKEN_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10000);

        Self {
            enabled,
            creation_token_threshold,
        }
    }
}

/// 缓存警告信息
#[derive(Debug, Clone)]
pub(crate) struct CacheWarningInfo {
    /// 警告类型
    pub warning_type: CacheWarningType,
    /// 警告消息
    pub message: String,
    /// 缓存创建tokens
    pub creation_tokens: u64,
}

/// 缓存警告类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CacheWarningType {
    /// 高创建成本
    HighCreationCost,
}

/// 缓存警告检测器
pub(crate) struct CacheWarningDetector {
    config: CacheWarningConfig,
}

impl CacheWarningDetector {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new() -> Self {
        let config = CacheWarningConfig::from_env();
        Self { config }
    }

    /// 仅在提供商明确上报缓存遥测时评估创建成本。缓存计数和提示 token 不共享可靠的分母，
    /// 因此不推导所谓“命中率”。
    pub(crate) fn warning_for_usage(&self, usage: &StarUsage) -> Option<CacheWarningInfo> {
        if !usage.cache_telemetry_reported {
            return None;
        }

        self.should_show_warning(usage.cache_creation_tokens as u64)
    }

    /// 检查是否需要显示缓存创建成本警告
    pub(crate) fn should_show_warning(&self, creation_tokens: u64) -> Option<CacheWarningInfo> {
        if !self.config.enabled || creation_tokens <= self.config.creation_token_threshold {
            return None;
        }

        Some(CacheWarningInfo {
            warning_type: CacheWarningType::HighCreationCost,
            message: format!(
                "High cache creation cost: {} tokens. This may impact performance on subsequent turns.",
                creation_tokens
            ),
            creation_tokens,
        })
    }
}

/// 缺失工具结果块生成器
pub struct MissingToolResultGenerator;

impl MissingToolResultGenerator {
    /// 为缺失的工具结果生成合成结果块
    pub fn yield_missing_tool_result_blocks(
        assistant_messages: &[StarMessage],
        error_message: &str,
    ) -> Vec<StarMessage> {
        let mut results = Vec::new();

        for msg in assistant_messages {
            if msg.role != "assistant" {
                continue;
            }

            if let Some(tool_calls) = &msg.tool_calls {
                for tool_call in tool_calls {
                    let error_content =
                        format!("[Tool result missing due to error: {}]", error_message);
                    results.push(StarMessage::tool(&tool_call.id, &error_content));
                }
            }
        }

        results
    }
}

/// 工具参数日志记录器
pub struct ToolParameterLogger;

impl ToolParameterLogger {
    /// 提取工具输入用于遥测（使用starcode-cli中的实际工具名称）
    pub fn extract_tool_input_for_telemetry(tool_name: &str, input: &Value) -> Option<Value> {
        if let Some(obj) = input.as_object() {
            let mut telemetry = serde_json::Map::new();

            match tool_name {
                "Bash" => {
                    if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
                        let parts: Vec<&str> = command.trim().split_whitespace().collect();
                        if let Some(first) = parts.first() {
                            telemetry.insert(
                                "bash_command".to_string(),
                                Value::String(first.to_string()),
                            );
                        }
                        telemetry.insert(
                            "full_command".to_string(),
                            Value::String(command.to_string()),
                        );
                    }
                    if let Some(timeout) = obj.get("timeout").and_then(|v| v.as_u64()) {
                        telemetry.insert("timeout".to_string(), Value::Number(timeout.into()));
                    }
                    if let Some(description) = obj.get("description").and_then(|v| v.as_str()) {
                        telemetry.insert(
                            "description".to_string(),
                            Value::String(description.to_string()),
                        );
                    }
                }
                "Read" => {
                    if let Some(path) = obj.get("file_path").and_then(|v| v.as_str()) {
                        telemetry.insert("file_path".to_string(), Value::String(path.to_string()));
                        if let Some(ext) = std::path::Path::new(path).extension() {
                            telemetry.insert(
                                "file_extension".to_string(),
                                Value::String(ext.to_string_lossy().to_string()),
                            );
                        }
                    }
                }
                "Edit" | "smart_edit" | "multi_edit" => {
                    if let Some(path) = obj.get("file_path").and_then(|v| v.as_str()) {
                        telemetry.insert("file_path".to_string(), Value::String(path.to_string()));
                        if let Some(ext) = std::path::Path::new(path).extension() {
                            telemetry.insert(
                                "file_extension".to_string(),
                                Value::String(ext.to_string_lossy().to_string()),
                            );
                        }
                    }
                }
                "Write" => {
                    if let Some(path) = obj.get("file_path").and_then(|v| v.as_str()) {
                        telemetry.insert("file_path".to_string(), Value::String(path.to_string()));
                        if let Some(ext) = std::path::Path::new(path).extension() {
                            telemetry.insert(
                                "file_extension".to_string(),
                                Value::String(ext.to_string_lossy().to_string()),
                            );
                        }
                    }
                    if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                        telemetry.insert(
                            "content_length".to_string(),
                            Value::Number(content.len().into()),
                        );
                    }
                }
                "Grep" => {
                    if let Some(query) = obj.get("query").and_then(|v| v.as_str()) {
                        telemetry.insert("query".to_string(), Value::String(query.to_string()));
                    }
                    if let Some(path) = obj
                        .get("path")
                        .or(obj.get("include_pattern"))
                        .and_then(|v| v.as_str())
                    {
                        telemetry
                            .insert("search_path".to_string(), Value::String(path.to_string()));
                    }
                }
                "Glob" => {
                    if let Some(pattern) = obj.get("pattern").and_then(|v| v.as_str()) {
                        telemetry.insert("pattern".to_string(), Value::String(pattern.to_string()));
                    }
                }
                _ => {
                    // 通用记录
                    for (key, value) in obj.iter().take(3) {
                        if let Some(s) = value.as_str() {
                            telemetry.insert(key.clone(), Value::String(s.to_string()));
                        }
                    }
                }
            }

            if !telemetry.is_empty() {
                return Some(Value::Object(telemetry));
            }
        }
        None
    }

    /// 提取MCP工具详情
    pub fn extract_mcp_tool_details(tool_name: &str) -> Option<McpToolDetails> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }

        let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
        if parts.len() >= 3 {
            Some(McpToolDetails {
                server_name: parts[1].to_string(),
                tool_name: parts[2].to_string(),
            })
        } else {
            None
        }
    }

    /// 提取技能名称
    pub fn extract_skill_name(tool_name: &str, input: &Value) -> Option<String> {
        if tool_name == "skill" || tool_name == "Skill" {
            if let Some(name) = input
                .get("name")
                .or(input.get("skill_name"))
                .and_then(|v| v.as_str())
            {
                return Some(name.to_string());
            }
        }
        None
    }

    /// 计算工具结果大小
    pub fn calculate_tool_result_size(content: &Value) -> usize {
        match content {
            Value::String(s) => s.len(),
            Value::Array(arr) => {
                let mut size = 0;
                for item in arr {
                    size += Self::calculate_tool_result_size(item);
                }
                size
            }
            Value::Object(obj) => {
                let mut size = 0;
                for (key, value) in obj {
                    size += key.len();
                    size += Self::calculate_tool_result_size(value);
                }
                size
            }
            _ => 0,
        }
    }

    /// 提取文件扩展名
    pub fn get_file_extension_for_analytics(path: &str) -> Option<String> {
        std::path::Path::new(path)
            .extension()
            .map(|ext| ext.to_string_lossy().to_string())
    }
}

/// MCP工具详情
#[derive(Debug, Clone)]
pub struct McpToolDetails {
    pub server_name: String,
    pub tool_name: String,
}

/// 消息归一化器
pub struct MessageNormalizer;

impl MessageNormalizer {
    /// 归一化消息列表
    /// 将多内容块消息拆分为单内容块消息
    pub fn normalize_messages(messages: &[StarMessage]) -> Vec<NormalizedMessage> {
        let mut result = Vec::new();
        let mut is_new_chain = false;

        for msg in messages {
            match msg.role.as_str() {
                "assistant" => {
                    if let Some(tool_calls) = &msg.tool_calls {
                        is_new_chain = is_new_chain || tool_calls.len() > 1;
                        for (index, tool_call) in tool_calls.iter().enumerate() {
                            let uuid = if is_new_chain {
                                Self::derive_uuid(
                                    &msg.tool_call_id.clone().unwrap_or_default(),
                                    index,
                                )
                            } else {
                                msg.tool_call_id.clone().unwrap_or_default()
                            };
                            result.push(NormalizedMessage {
                                message_type: NormalizedMessageType::Assistant,
                                uuid,
                                content: msg.content.clone(),
                                tool_calls: Some(vec![tool_call.clone()]),
                                tool_call_id: None,
                            });
                        }
                    } else {
                        let uuid = if is_new_chain {
                            Self::derive_uuid(&msg.tool_call_id.clone().unwrap_or_default(), 0)
                        } else {
                            msg.tool_call_id.clone().unwrap_or_default()
                        };
                        result.push(NormalizedMessage {
                            message_type: NormalizedMessageType::Assistant,
                            uuid,
                            content: msg.content.clone(),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
                "user" => {
                    let uuid = if is_new_chain {
                        Self::derive_uuid(&msg.tool_call_id.clone().unwrap_or_default(), 0)
                    } else {
                        msg.tool_call_id.clone().unwrap_or_default()
                    };
                    result.push(NormalizedMessage {
                        message_type: NormalizedMessageType::User,
                        uuid,
                        content: msg.content.clone(),
                        tool_calls: None,
                        tool_call_id: msg.tool_call_id.clone(),
                    });
                }
                "tool" => {
                    result.push(NormalizedMessage {
                        message_type: NormalizedMessageType::Tool,
                        uuid: msg.tool_call_id.clone().unwrap_or_default(),
                        content: msg.content.clone(),
                        tool_calls: None,
                        tool_call_id: msg.tool_call_id.clone(),
                    });
                }
                _ => {
                    result.push(NormalizedMessage {
                        message_type: NormalizedMessageType::System,
                        uuid: msg.tool_call_id.clone().unwrap_or_default(),
                        content: msg.content.clone(),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        result
    }

    /// 派生UUID
    pub fn derive_uuid(parent_uuid: &str, index: usize) -> String {
        let hex = format!("{:012x}", index);
        if parent_uuid.len() >= 24 {
            format!("{}{}", &parent_uuid[..24], hex)
        } else {
            format!("{:0<24}{}", parent_uuid, hex)
        }
    }
}

/// 归一化消息
#[derive(Debug, Clone)]
pub struct NormalizedMessage {
    pub message_type: NormalizedMessageType,
    pub uuid: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<StarToolCall>>,
    pub tool_call_id: Option<String>,
}

/// 归一化消息类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedMessageType {
    Assistant,
    User,
    Tool,
    System,
}

/// 权限拒绝重试管理器
pub struct PermissionDenialRetryManager {
    /// 最大重试次数
    max_retries: usize,
    /// 当前重试次数
    retry_count: usize,
}

impl PermissionDenialRetryManager {
    pub fn new() -> Self {
        Self {
            max_retries: 1,
            retry_count: 0,
        }
    }

    /// 检查是否可以重试
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    /// 记录重试
    pub fn record_retry(&mut self) {
        self.retry_count += 1;
    }

    /// 重置重试计数
    pub fn reset(&mut self) {
        self.retry_count = 0;
    }

    /// 创建重试消息
    pub fn create_retry_message(&self) -> StarMessage {
        StarMessage::user(
            "The PermissionDenied hook indicated this command is now approved. You may retry it if you would like."
        )
    }
}

/// 工具执行时间跟踪器
pub struct ToolDurationTracker {
    /// 总工具执行时间（毫秒）
    total_duration_ms: u64,
    /// 工具执行次数
    tool_count: u64,
}

impl ToolDurationTracker {
    pub fn new() -> Self {
        Self {
            total_duration_ms: 0,
            tool_count: 0,
        }
    }

    /// 记录工具执行时间
    pub fn add_duration(&mut self, duration_ms: u64) {
        self.total_duration_ms += duration_ms;
        self.tool_count += 1;
    }

    /// 获取平均执行时间
    pub fn average_duration(&self) -> u64 {
        if self.tool_count == 0 {
            return 0;
        }
        self.total_duration_ms / self.tool_count
    }

    /// 获取总执行时间
    pub fn total_duration(&self) -> u64 {
        self.total_duration_ms
    }

    /// 获取工具执行次数
    pub fn tool_count(&self) -> u64 {
        self.tool_count
    }
}

/// 代码编辑工具检测器
pub struct CodeEditToolDetector;

impl CodeEditToolDetector {
    /// 代码编辑工具名称列表
    const CODE_EDIT_TOOLS: &'static [&'static str] = &[
        "Edit",
        "edit_file",
        "FileEdit",
        "Write",
        "create_file",
        "FileWrite",
        "NotebookEdit",
    ];

    /// 检查是否是代码编辑工具
    pub fn is_code_editing_tool(tool_name: &str) -> bool {
        Self::CODE_EDIT_TOOLS.contains(&tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_warning_requires_reported_telemetry_and_never_warns_for_zeroes() {
        let detector = CacheWarningDetector {
            config: CacheWarningConfig::default(),
        };
        let base_usage = StarUsage {
            prompt_tokens: 1_000,
            completion_tokens: 100,
            total_tokens: 1_100,
            ..Default::default()
        };

        assert!(detector.warning_for_usage(&base_usage).is_none());
        assert!(detector
            .warning_for_usage(&StarUsage {
                cache_telemetry_reported: true,
                ..base_usage.clone()
            })
            .is_none());
    }

    #[test]
    fn cache_warning_respects_thresholds_and_enabled_flag() {
        let usage = StarUsage {
            prompt_tokens: 900,
            completion_tokens: 100,
            total_tokens: 1_000,
            cache_read_tokens: 100,
            cache_creation_tokens: 20_000,
            cache_telemetry_reported: true,
        };
        let detector = CacheWarningDetector {
            config: CacheWarningConfig {
                enabled: true,
                creation_token_threshold: 10_000,
            },
        };
        let warning = detector
            .warning_for_usage(&usage)
            .expect("high creation cost warning");
        assert_eq!(warning.warning_type, CacheWarningType::HighCreationCost);
        assert_eq!(warning.creation_tokens, 20_000);
        assert!(warning.message.contains("20,000") || warning.message.contains("20000"));

        let disabled = CacheWarningDetector {
            config: CacheWarningConfig {
                enabled: false,
                ..CacheWarningConfig::default()
            },
        };
        assert!(disabled.warning_for_usage(&usage).is_none());
    }

    #[test]
    fn test_uuid_derivation() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let derived = MessageNormalizer::derive_uuid(uuid, 42);
        assert!(derived.starts_with(&uuid[..24]));
    }

    #[test]
    fn test_code_edit_tool_detection() {
        assert!(CodeEditToolDetector::is_code_editing_tool("Edit"));
        assert!(CodeEditToolDetector::is_code_editing_tool("FileWrite"));
        assert!(!CodeEditToolDetector::is_code_editing_tool("Read"));
    }

    #[test]
    fn test_mcp_tool_details() {
        let details = ToolParameterLogger::extract_mcp_tool_details("mcp__server__tool");
        assert!(details.is_some());
        let details = details.unwrap();
        assert_eq!(details.server_name, "server");
        assert_eq!(details.tool_name, "tool");
    }
}
