/// SDK类型定义

use serde::{Deserialize, Serialize};

/// 聊天请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// 消息
    pub message: String,
    /// 会话ID
    pub session_id: Option<String>,
    /// 模型
    pub model: Option<String>,
    /// 温度
    pub temperature: Option<f32>,
    /// 最大token数
    pub max_tokens: Option<u32>,
}

/// 聊天响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// 响应内容
    pub content: String,
    /// 会话ID
    pub session_id: String,
    /// Token使用量
    pub token_usage: TokenUsage,
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
}

/// 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    /// 工具名称
    pub tool: String,
    /// 输入参数
    pub input: serde_json::Value,
    /// 会话ID
    pub session_id: Option<String>,
}

/// 工具调用响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    /// 是否成功
    pub success: bool,
    /// 输出
    pub output: Option<String>,
    /// 错误
    pub error: Option<String>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
}

/// 会话信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// 会话ID
    pub id: String,
    /// 创建时间
    pub created_at: i64,
    /// 最后活动时间
    pub last_activity: i64,
    /// 消息数
    pub message_count: u32,
    /// 状态
    pub status: SessionStatus,
}

/// 会话状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    /// 活跃
    Active,
    /// 暂停
    Paused,
    /// 已结束
    Ended,
}
