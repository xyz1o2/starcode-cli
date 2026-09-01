/// 消息类型定义
/// 
/// 对标claude-code-main的src/types/message.ts

use serde::{Deserialize, Serialize};

/// 消息角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    /// 系统消息
    System,
    /// 用户消息
    User,
    /// 助手消息
    Assistant,
    /// 工具消息
    Tool,
}

/// 消息状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageStatus {
    /// 创建中
    Creating,
    /// 发送中
    Sending,
    /// 已发送
    Sent,
    /// 接收中
    Receiving,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

/// 消息元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMetadata {
    /// 消息ID
    pub id: String,
    /// 会话ID
    pub session_id: String,
    /// 时间戳
    pub timestamp: i64,
    /// 模型
    pub model: Option<String>,
    /// Provider
    pub provider: Option<String>,
    /// Token使用量
    pub token_usage: Option<TokenUsage>,
    /// 持续时间（毫秒）
    pub duration_ms: Option<u64>,
    /// 父消息ID
    pub parent_id: Option<String>,
    /// 标签
    pub tags: Vec<String>,
}

/// Token使用量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 提示token数
    pub prompt_tokens: u32,
    /// 完成token数
    pub completion_tokens: u32,
    /// 总token数
    pub total_tokens: u32,
    /// 缓存token数
    pub cached_tokens: Option<u32>,
}

/// 消息内容类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    /// 文本内容
    Text(String),
    /// 代码内容
    Code { language: String, code: String },
    /// 图片内容
    Image { url: String, alt: Option<String> },
    /// 工具调用
    ToolCall(ToolCallMessage),
    /// 工具结果
    ToolResult(ToolResultMessage),
    /// 多部分内容
    MultiPart(Vec<MessageContent>),
}

/// 工具调用消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    /// 工具调用ID
    pub id: String,
    /// 工具名称
    pub name: String,
    /// 参数
    pub arguments: serde_json::Value,
}

/// 工具结果消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    /// 工具调用ID
    pub tool_call_id: String,
    /// 结果内容
    pub content: String,
    /// 是否错误
    pub is_error: bool,
}
