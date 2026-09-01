/// Bridge API

use super::BridgeManager;
use super::message::{BridgeMessage, MessageType};

/// Bridge API
pub struct BridgeApi {
    /// Bridge管理器
    manager: std::sync::Arc<BridgeManager>,
}

impl BridgeApi {
    /// 创建新的Bridge API
    pub fn new(manager: std::sync::Arc<BridgeManager>) -> Self {
        Self { manager }
    }

    /// 执行命令
    pub async fn execute_command(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, ApiError> {
        let message = BridgeMessage::command(method, params);
        
        // TODO: 发送命令并等待响应
        Ok(serde_json::Value::Null)
    }

    /// 查询状态
    pub async fn query_status(&self, method: &str) -> Result<serde_json::Value, ApiError> {
        let message = BridgeMessage::query(method);
        
        // TODO: 发送查询并等待响应
        Ok(serde_json::Value::Null)
    }

    /// 获取连接列表
    pub async fn list_connections(&self) -> Result<Vec<serde_json::Value>, ApiError> {
        let connections = self.manager.get_all_connections().await;
        let result: Vec<serde_json::Value> = connections.iter()
            .map(|c| serde_json::to_value(c).unwrap_or_default())
            .collect();
        Ok(result)
    }

    /// 获取连接信息
    pub async fn get_connection(&self, connection_id: &str) -> Result<serde_json::Value, ApiError> {
        let connection = self.manager.get_connection(connection_id).await;
        match connection {
            Some(conn) => Ok(serde_json::to_value(conn).unwrap_or_default()),
            None => Err(ApiError::NotFound),
        }
    }

    /// 断开连接
    pub async fn disconnect(&self, connection_id: &str) -> Result<(), ApiError> {
        self.manager.disconnect(connection_id).await;
        Ok(())
    }

    /// 获取活跃连接数
    pub async fn active_connections(&self) -> Result<usize, ApiError> {
        Ok(self.manager.active_connections().await)
    }

    /// 发送消息
    pub async fn send_message(&self, connection_id: &str, message: BridgeMessage) -> Result<(), ApiError> {
        self.manager.send_message(connection_id, message).await
            .map_err(|e| ApiError::SendError(e.to_string()))
    }

    /// 广播消息
    pub async fn broadcast_message(&self, message: BridgeMessage) -> Result<(), ApiError> {
        self.manager.broadcast_message(message).await
            .map_err(|e| ApiError::SendError(e.to_string()))
    }

    /// 检查是否运行中
    pub async fn is_running(&self) -> bool {
        self.manager.is_running().await
    }
}

/// API错误
#[derive(Debug)]
pub enum ApiError {
    /// 未找到
    NotFound,
    /// 发送错误
    SendError(String),
    /// 超时
    Timeout,
    /// 未授权
    Unauthorized,
    /// 内部错误
    InternalError(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotFound => write!(f, "Not found"),
            ApiError::SendError(e) => write!(f, "Send error: {}", e),
            ApiError::Timeout => write!(f, "Timeout"),
            ApiError::Unauthorized => write!(f, "Unauthorized"),
            ApiError::InternalError(e) => write!(f, "Internal error: {}", e),
        }
    }
}

impl std::error::Error for ApiError {}
