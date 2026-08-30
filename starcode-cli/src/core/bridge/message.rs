/// Bridge消息类型

use serde::{Deserialize, Serialize};

/// 消息类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    /// 命令
    Command,
    /// 查询
    Query,
    /// 响应
    Response,
    /// 事件
    Event,
    /// 心跳
    Ping,
    /// 心跳响应
    Pong,
    /// 错误
    Error,
    /// 状态更新
    StatusUpdate,
    /// 会话消息
    SessionMessage,
}

/// Bridge消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMessage {
    /// 消息ID
    pub id: String,
    /// 消息类型
    pub message_type: MessageType,
    /// 方法名
    pub method: Option<String>,
    /// 参数
    pub params: Option<serde_json::Value>,
    /// 结果
    pub result: Option<serde_json::Value>,
    /// 错误
    pub error: Option<ErrorMessage>,
    /// 时间戳
    pub timestamp: i64,
    /// JWT令牌
    pub token: Option<String>,
    /// 会话ID
    pub session_id: Option<String>,
}

/// 错误消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessage {
    /// 错误代码
    pub code: i32,
    /// 错误消息
    pub message: String,
    /// 错误数据
    pub data: Option<serde_json::Value>,
}

impl BridgeMessage {
    /// 创建新消息
    pub fn new(message_type: MessageType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message_type,
            method: None,
            params: None,
            result: None,
            error: None,
            timestamp: chrono::Utc::now().timestamp(),
            token: None,
            session_id: None,
        }
    }

    /// 创建命令消息
    pub fn command(method: &str, params: serde_json::Value) -> Self {
        let mut msg = Self::new(MessageType::Command);
        msg.method = Some(method.to_string());
        msg.params = Some(params);
        msg
    }

    /// 创建查询消息
    pub fn query(method: &str) -> Self {
        let mut msg = Self::new(MessageType::Query);
        msg.method = Some(method.to_string());
        msg
    }

    /// 创建响应消息
    pub fn response(result: serde_json::Value) -> Self {
        let mut msg = Self::new(MessageType::Response);
        msg.result = Some(result);
        msg
    }

    /// 创建错误消息
    pub fn error(code: i32, message: &str) -> Self {
        let mut msg = Self::new(MessageType::Error);
        msg.error = Some(ErrorMessage {
            code,
            message: message.to_string(),
            data: None,
        });
        msg
    }

    /// 创建心跳消息
    pub fn ping() -> Self {
        Self::new(MessageType::Ping)
    }

    /// 创建心跳响应
    pub fn pong() -> Self {
        Self::new(MessageType::Pong)
    }
}
