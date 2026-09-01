/// Bridge连接管理

use serde::{Deserialize, Serialize};

/// 连接状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConnectionStatus {
    /// 断开连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 错误
    Error(String),
}

/// Bridge连接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConnection {
    /// 连接ID
    pub id: String,
    /// 客户端类型
    pub client_type: String,
    /// 连接状态
    pub status: ConnectionStatus,
    /// 连接时间
    pub connected_at: i64,
    /// 最后活动时间
    pub last_activity: i64,
    /// 对端地址
    pub peer_address: Option<String>,
    /// 会话ID
    pub session_id: Option<String>,
}

impl BridgeConnection {
    /// 创建新连接
    pub fn new(id: String, client_type: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id,
            client_type,
            status: ConnectionStatus::Connected,
            connected_at: now,
            last_activity: now,
            peer_address: None,
            session_id: None,
        }
    }

    /// 更新活动时间
    pub fn update_activity(&mut self) {
        self.last_activity = chrono::Utc::now().timestamp();
    }

    /// 检查是否超时
    pub fn is_timeout(&self, timeout_secs: i64) -> bool {
        let now = chrono::Utc::now().timestamp();
        now - self.last_activity > timeout_secs
    }
}
