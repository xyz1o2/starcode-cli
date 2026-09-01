use crate::types::{StarMessage, StarToolCall};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// MCP服务器类型 - 对标claude-code的McpServerType
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerType {
    /// 标准输入输出
    Stdio,
    /// 服务器发送事件
    Sse,
    /// HTTP
    Http,
    /// WebSocket
    Ws,
    /// SDK
    Sdk,
    /// SSE IDE
    SseIde,
    /// WebSocket IDE
    WsIde,
    /// Claude AI代理
    ClaudeAiProxy,
}

impl McpServerType {
    /// 从字符串解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "stdio" => Some(Self::Stdio),
            "sse" => Some(Self::Sse),
            "http" => Some(Self::Http),
            "ws" => Some(Self::Ws),
            "sdk" => Some(Self::Sdk),
            "sse-ide" => Some(Self::SseIde),
            "ws-ide" => Some(Self::WsIde),
            "claudeai-proxy" => Some(Self::ClaudeAiProxy),
            _ => None,
        }
    }

    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
            Self::Http => "http",
            Self::Ws => "ws",
            Self::Sdk => "sdk",
            Self::SseIde => "sse-ide",
            Self::WsIde => "ws-ide",
            Self::ClaudeAiProxy => "claudeai-proxy",
        }
    }
}

/// MCP服务器连接信息
#[derive(Debug, Clone)]
pub struct McpServerConnection {
    /// 服务器名称
    pub name: String,
    /// 服务器类型
    pub server_type: McpServerType,
    /// 基础URL
    pub base_url: Option<String>,
    /// 是否已连接
    pub is_connected: bool,
}

/// MCP服务器管理器 - 对标claude-code的MCP服务器管理
pub struct McpServerManager {
    /// 服务器连接映射
    connections: HashMap<String, McpServerConnection>,
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }

    /// 添加服务器连接
    pub fn add_connection(&mut self, connection: McpServerConnection) {
        self.connections.insert(connection.name.clone(), connection);
    }

    /// 获取服务器类型
    pub fn get_server_type(&self, tool_name: &str) -> Option<McpServerType> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }

        // 从工具名称中提取服务器名称
        let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
        if parts.len() < 2 {
            return None;
        }

        let server_name = parts[1];
        self.connections
            .get(server_name)
            .map(|c| c.server_type.clone())
    }

    /// 获取服务器基础URL
    pub fn get_server_base_url(&self, tool_name: &str) -> Option<String> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }

        // 从工具名称中提取服务器名称
        let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
        if parts.len() < 2 {
            return None;
        }

        let server_name = parts[1];
        self.connections
            .get(server_name)
            .and_then(|c| c.base_url.clone())
    }

    /// 检查是否是MCP工具
    pub fn is_mcp_tool(tool_name: &str) -> bool {
        tool_name.starts_with("mcp__")
    }

    /// 从工具名称中提取MCP信息
    pub fn extract_mcp_info(tool_name: &str) -> Option<McpToolInfo> {
        if !tool_name.starts_with("mcp__") {
            return None;
        }

        let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
        if parts.len() >= 3 {
            Some(McpToolInfo {
                server_name: parts[1].to_string(),
                tool_name: parts[2].to_string(),
            })
        } else {
            None
        }
    }
}

/// MCP工具信息
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_name: String,
    pub tool_name: String,
}

/// 权限决策来源映射器 - 对标claude-code的decisionReasonToOTelSource
pub struct PermissionDecisionMapper;

impl PermissionDecisionMapper {
    /// 将权限决策来源映射到OTel源
    pub fn decision_reason_to_otel_source(
        reason: &PermissionDecisionReason,
        behavior: PermissionBehavior,
    ) -> &'static str {
        match reason {
            PermissionDecisionReason::PermissionPromptTool { .. } => match behavior {
                PermissionBehavior::Allow => "user_temporary",
                PermissionBehavior::Deny => "user_reject",
            },
            PermissionDecisionReason::Rule { source } => {
                Self::rule_source_to_otel_source(source, behavior)
            }
            PermissionDecisionReason::Hook { .. } => "hook",
            PermissionDecisionReason::Mode
            | PermissionDecisionReason::Classifier
            | PermissionDecisionReason::SubcommandResults
            | PermissionDecisionReason::AsyncAgent
            | PermissionDecisionReason::SandboxOverride
            | PermissionDecisionReason::WorkingDir
            | PermissionDecisionReason::SafetyCheck
            | PermissionDecisionReason::Other => "config",
        }
    }

    /// 将规则来源映射到OTel源
    fn rule_source_to_otel_source(source: &str, behavior: PermissionBehavior) -> &'static str {
        match source {
            "session" => match behavior {
                PermissionBehavior::Allow => "user_temporary",
                PermissionBehavior::Deny => "user_reject",
            },
            "localSettings" | "userSettings" => match behavior {
                PermissionBehavior::Allow => "user_permanent",
                PermissionBehavior::Deny => "user_reject",
            },
            _ => "config",
        }
    }
}

/// 权限决策原因
#[derive(Debug, Clone)]
pub enum PermissionDecisionReason {
    /// 权限提示工具
    PermissionPromptTool { tool_result: Option<Value> },
    /// 规则
    Rule { source: String },
    /// Hook
    Hook { hook_name: String },
    /// 模式
    Mode,
    /// 分类器
    Classifier,
    /// 子命令结果
    SubcommandResults,
    /// 异步Agent
    AsyncAgent,
    /// 沙箱覆盖
    SandboxOverride,
    /// 工作目录
    WorkingDir,
    /// 安全检查
    SafetyCheck,
    /// 其他
    Other,
}

/// 权限行为
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionBehavior {
    Allow,
    Deny,
}

/// 图片粘贴ID管理器 - 对标claude-code的getNextImagePasteId
pub struct ImagePasteIdManager {
    /// 当前最大ID
    max_id: usize,
}

impl ImagePasteIdManager {
    pub fn new() -> Self {
        Self { max_id: 0 }
    }

    /// 获取下一个图片粘贴ID
    pub fn get_next_id(&mut self, messages: &[StarMessage]) -> usize {
        // 从消息中查找最大的图片ID
        for msg in messages {
            if msg.role == "user" {
                // 简化处理：假设图片ID在内容中
                // 实际应该从imagePasteIds字段获取
            }
        }
        self.max_id += 1;
        self.max_id
    }

    /// 批量获取图片粘贴ID
    pub fn get_next_ids(&mut self, count: usize, messages: &[StarMessage]) -> Vec<usize> {
        let start_id = self.get_next_id(messages);
        (start_id..start_id + count).collect()
    }
}

/// 工具错误分类器 - 对标claude-code的classifyToolError
pub struct ToolErrorClassifier;

impl ToolErrorClassifier {
    /// 分类工具错误
    pub fn classify_error(error: &str) -> String {
        let error_lower = error.to_lowercase();

        // 检查是否是文件系统错误
        if error_lower.contains("enoent") || error_lower.contains("not found") {
            return "Error:ENOENT".to_string();
        }
        if error_lower.contains("eacces") || error_lower.contains("permission denied") {
            return "Error:EACCES".to_string();
        }
        if error_lower.contains("eexist") || error_lower.contains("already exists") {
            return "Error:EEXIST".to_string();
        }

        // 检查是否是Shell错误
        if error_lower.contains("command not found") || error_lower.contains("no such file") {
            return "ShellError".to_string();
        }

        // 检查是否是超时错误
        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            return "TimeoutError".to_string();
        }

        // 检查是否是网络错误
        if error_lower.contains("connection refused") || error_lower.contains("network") {
            return "NetworkError".to_string();
        }

        // 检查是否是MCP错误
        if error_lower.contains("mcp") || error_lower.contains("tool_use_error") {
            return "McpError".to_string();
        }

        // 默认返回Error
        "Error".to_string()
    }

    /// 检查是否是可重试的错误
    pub fn is_retryable_error(error: &str) -> bool {
        let error_lower = error.to_lowercase();
        error_lower.contains("timeout")
            || error_lower.contains("connection")
            || error_lower.contains("temporary")
            || error_lower.contains("retry")
    }
}

/// 消息安全清理器
pub struct MessageSanitizer;

impl MessageSanitizer {
    /// 清理工具名称用于分析
    pub fn sanitize_tool_name_for_analytics(tool_name: &str) -> String {
        // 移除MCP前缀
        if tool_name.starts_with("mcp__") {
            let parts: Vec<&str> = tool_name.splitn(3, "__").collect();
            if parts.len() >= 3 {
                return parts[2].to_string();
            }
        }
        tool_name.to_string()
    }

    /// 提取工具输入用于遥测
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
                    }
                }
                "Read" => {
                    if let Some(path) = obj.get("file_path").and_then(|v| v.as_str()) {
                        telemetry.insert("file_path".to_string(), Value::String(path.to_string()));
                    }
                }
                "Edit" | "Write" => {
                    if let Some(path) = obj.get("file_path").and_then(|v| v.as_str()) {
                        telemetry.insert("file_path".to_string(), Value::String(path.to_string()));
                    }
                }
                "Grep" => {
                    if let Some(query) = obj.get("query").and_then(|v| v.as_str()) {
                        telemetry.insert("query".to_string(), Value::String(query.to_string()));
                    }
                }
                _ => {}
            }

            if !telemetry.is_empty() {
                return Some(Value::Object(telemetry));
            }
        }
        None
    }

    /// 提取文件扩展名用于分析
    pub fn get_file_extension_for_analytics(path: &str) -> Option<String> {
        std::path::Path::new(path)
            .extension()
            .map(|ext| ext.to_string_lossy().to_string())
    }

    /// 从bash命令中提取文件扩展名
    pub fn get_file_extensions_from_bash_command(command: &str) -> Vec<String> {
        let mut extensions = Vec::new();
        let words: Vec<&str> = command.split_whitespace().collect();

        for word in words {
            // 检查是否是文件路径
            if word.contains('.') && !word.starts_with('-') {
                if let Some(ext) = std::path::Path::new(word).extension() {
                    extensions.push(ext.to_string_lossy().to_string());
                }
            }
        }

        extensions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_type() {
        assert_eq!(McpServerType::from_str("stdio"), Some(McpServerType::Stdio));
        assert_eq!(McpServerType::from_str("sse"), Some(McpServerType::Sse));
        assert_eq!(McpServerType::from_str("http"), Some(McpServerType::Http));
        assert_eq!(McpServerType::from_str("ws"), Some(McpServerType::Ws));
        assert_eq!(McpServerType::from_str("unknown"), None);
    }

    #[test]
    fn test_mcp_server_manager() {
        let mut manager = McpServerManager::new();

        manager.add_connection(McpServerConnection {
            name: "test-server".to_string(),
            server_type: McpServerType::Stdio,
            base_url: None,
            is_connected: true,
        });

        assert_eq!(
            manager.get_server_type("mcp__test-server__tool"),
            Some(McpServerType::Stdio)
        );
        assert!(McpServerManager::is_mcp_tool("mcp__test-server__tool"));
        assert!(!McpServerManager::is_mcp_tool("Bash"));
    }

    #[test]
    fn test_tool_error_classifier() {
        assert_eq!(
            ToolErrorClassifier::classify_error("No such file or directory (os error 2)"),
            "Error:ENOENT"
        );
        assert_eq!(
            ToolErrorClassifier::classify_error("Permission denied"),
            "Error:EACCES"
        );
        assert_eq!(
            ToolErrorClassifier::classify_error("Command timed out"),
            "TimeoutError"
        );
    }

    #[test]
    fn test_permission_decision_mapper() {
        let reason = PermissionDecisionReason::Rule {
            source: "session".to_string(),
        };
        assert_eq!(
            PermissionDecisionMapper::decision_reason_to_otel_source(
                &reason,
                PermissionBehavior::Allow
            ),
            "user_temporary"
        );
    }

    #[test]
    fn test_message_sanitizer() {
        assert_eq!(
            MessageSanitizer::sanitize_tool_name_for_analytics("mcp__server__tool"),
            "tool"
        );
        assert_eq!(
            MessageSanitizer::sanitize_tool_name_for_analytics("Bash"),
            "Bash"
        );
    }
}
