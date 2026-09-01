/// LSP客户端

use serde::{Deserialize, Serialize};

/// LSP请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRequest {
    /// 请求ID
    pub id: u64,
    /// 方法
    pub method: String,
    /// 参数
    pub params: Option<serde_json::Value>,
}

/// LSP响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResponse {
    /// 请求ID
    pub id: u64,
    /// 结果
    pub result: Option<serde_json::Value>,
    /// 错误
    pub error: Option<LspError>,
}

/// LSP错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspError {
    /// 错误代码
    pub code: i32,
    /// 错误消息
    pub message: String,
    /// 错误数据
    pub data: Option<serde_json::Value>,
}

/// LSP客户端
pub struct LspClient {
    /// 请求ID计数器
    request_id: u64,
    /// 待发送的请求
    pending_requests: Vec<LspRequest>,
}

impl LspClient {
    /// 创建新的LSP客户端
    pub fn new() -> Self {
        Self {
            request_id: 0,
            pending_requests: Vec::new(),
        }
    }

    /// 创建请求
    pub fn create_request(&mut self, method: &str, params: Option<serde_json::Value>) -> u64 {
        self.request_id += 1;
        let id = self.request_id;

        let request = LspRequest {
            id,
            method: method.to_string(),
            params,
        };

        self.pending_requests.push(request);
        id
    }

    /// 获取待发送的请求
    pub fn pending_requests(&self) -> &[LspRequest] {
        &self.pending_requests
    }

    /// 清空待发送的请求
    pub fn clear_pending_requests(&mut self) {
        self.pending_requests.clear();
    }

    /// 处理响应
    pub fn handle_response(&self, response: &LspResponse) -> Result<Option<serde_json::Value>, LspClientError> {
        if let Some(error) = &response.error {
            return Err(LspClientError::ServerError(error.message.clone()));
        }

        Ok(response.result.clone())
    }
}

/// LSP客户端错误
#[derive(Debug)]
pub enum LspClientError {
    /// 服务器错误
    ServerError(String),
    /// 网络错误
    NetworkError(String),
    /// 解析错误
    ParseError(String),
}

impl std::fmt::Display for LspClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LspClientError::ServerError(e) => write!(f, "LSP server error: {}", e),
            LspClientError::NetworkError(e) => write!(f, "LSP network error: {}", e),
            LspClientError::ParseError(e) => write!(f, "LSP parse error: {}", e),
        }
    }
}

impl std::error::Error for LspClientError {}
