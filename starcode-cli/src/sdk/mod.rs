/// SDK编程接口
/// 
/// 对标claude-code-main的src/entrypoints/sdk/
/// 提供第三方集成和编程接口

pub mod client;
pub mod config;
pub mod types;

pub use client::StarCodeClient;
pub use config::SDKConfig;
pub use types::*;

use serde::{Deserialize, Serialize};

/// SDK请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SDKRequest {
    /// 请求ID
    pub id: String,
    /// 方法名
    pub method: String,
    /// 参数
    pub params: serde_json::Value,
    /// 会话ID
    pub session_id: Option<String>,
}

/// SDK响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SDKResponse {
    /// 请求ID
    pub request_id: String,
    /// 是否成功
    pub success: bool,
    /// 结果
    pub result: Option<serde_json::Value>,
    /// 错误
    pub error: Option<SDKError>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
}

/// SDK错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SDKError {
    /// 错误代码
    pub code: String,
    /// 错误消息
    pub message: String,
    /// 错误详情
    pub details: Option<serde_json::Value>,
}

/// SDK管理器
pub struct SDKManager {
    /// 配置
    config: SDKConfig,
    /// 客户端列表
    clients: std::collections::HashMap<String, StarCodeClient>,
}

impl SDKManager {
    /// 创建新的SDK管理器
    pub fn new(config: SDKConfig) -> Self {
        Self {
            config,
            clients: std::collections::HashMap::new(),
        }
    }

    /// 注册客户端
    pub fn register_client(&mut self, client: StarCodeClient) -> String {
        let id = client.id().to_string();
        self.clients.insert(id.clone(), client);
        id
    }

    /// 获取客户端
    pub fn get_client(&self, client_id: &str) -> Option<&StarCodeClient> {
        self.clients.get(client_id)
    }

    /// 处理请求
    pub async fn handle_request(&self, request: SDKRequest) -> SDKResponse {
        let start = std::time::Instant::now();
        
        // 根据方法分发请求
        let result = match request.method.as_str() {
            "chat" => self.handle_chat_request(&request).await,
            "tool" => self.handle_tool_request(&request).await,
            "session" => self.handle_session_request(&request).await,
            _ => Err(SDKError {
                code: "METHOD_NOT_FOUND".to_string(),
                message: format!("Unknown method: {}", request.method),
                details: None,
            }),
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => SDKResponse {
                request_id: request.id,
                success: true,
                result: Some(value),
                error: None,
                duration_ms,
            },
            Err(error) => SDKResponse {
                request_id: request.id,
                success: false,
                result: None,
                error: Some(error),
                duration_ms,
            },
        }
    }

    /// 处理聊天请求
    async fn handle_chat_request(&self, request: &SDKRequest) -> Result<serde_json::Value, SDKError> {
        // TODO: 实现聊天请求处理
        Ok(serde_json::json!({ "message": "Chat request received" }))
    }

    /// 处理工具请求
    async fn handle_tool_request(&self, request: &SDKRequest) -> Result<serde_json::Value, SDKError> {
        // TODO: 实现工具请求处理
        Ok(serde_json::json!({ "message": "Tool request received" }))
    }

    /// 处理会话请求
    async fn handle_session_request(&self, request: &SDKRequest) -> Result<serde_json::Value, SDKError> {
        // TODO: 实现会话请求处理
        Ok(serde_json::json!({ "message": "Session request received" }))
    }
}
