/// SDK客户端

use super::{SDKRequest, SDKResponse, SDKError};

/// SDK客户端
pub struct StarCodeClient {
    /// 客户端ID
    id: String,
    /// 客户端名称
    name: String,
    /// 连接状态
    connected: bool,
}

impl StarCodeClient {
    /// 创建新的SDK客户端
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            connected: false,
        }
    }

    /// 获取客户端ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 获取客户端名称
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 连接
    pub fn connect(&mut self) -> Result<(), SDKError> {
        self.connected = true;
        Ok(())
    }

    /// 断开连接
    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    /// 检查是否连接
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}
