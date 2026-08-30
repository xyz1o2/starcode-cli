/// ACP协议 (Agent Client Protocol)
/// 
/// 对标claude-code-main的src/services/acp/
/// Agent客户端协议实现

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ACP消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AcpMessageType {
    /// 初始化
    Initialize,
    /// 初始化响应
    InitializeResult,
    /// 执行命令
    ExecuteCommand,
    /// 命令结果
    CommandResult,
    /// 进度更新
    ProgressUpdate,
    /// 错误
    Error,
    /// 关闭
    Close,
}

/// ACP消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpMessage {
    /// 消息ID
    pub id: String,
    /// 消息类型
    pub message_type: AcpMessageType,
    /// 方法名
    pub method: Option<String>,
    /// 参数
    pub params: Option<serde_json::Value>,
    /// 结果
    pub result: Option<serde_json::Value>,
    /// 错误
    pub error: Option<AcpError>,
}

/// ACP错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpError {
    /// 错误代码
    pub code: i32,
    /// 错误消息
    pub message: String,
    /// 错误数据
    pub data: Option<serde_json::Value>,
}

/// ACP配置
#[derive(Debug, Clone)]
pub struct AcpConfig {
    /// 是否启用
    pub enabled: bool,
    /// 传输类型
    pub transport: String,
    /// 超时（秒）
    pub timeout_secs: u64,
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: "stdio".to_string(),
            timeout_secs: 30,
        }
    }
}

/// ACP客户端
pub struct AcpClient {
    config: AcpConfig,
    /// 消息队列
    message_queue: Vec<AcpMessage>,
    /// 请求ID计数器
    request_id: u64,
}

impl AcpClient {
    pub fn new(config: AcpConfig) -> Self {
        Self {
            config,
            message_queue: Vec::new(),
            request_id: 0,
        }
    }

    /// 发送初始化请求
    pub fn send_initialize(&mut self) -> String {
        self.request_id += 1;
        let id = format!("req_{}", self.request_id);

        let message = AcpMessage {
            id: id.clone(),
            message_type: AcpMessageType::Initialize,
            method: Some("initialize".to_string()),
            params: Some(serde_json::json!({
                "capabilities": {
                    "tools": true,
                    "resources": true,
                }
            })),
            result: None,
            error: None,
        };

        self.message_queue.push(message);
        id
    }

    /// 发送命令执行请求
    pub fn send_execute_command(&mut self, command: &str, args: serde_json::Value) -> String {
        self.request_id += 1;
        let id = format!("req_{}", self.request_id);

        let message = AcpMessage {
            id: id.clone(),
            message_type: AcpMessageType::ExecuteCommand,
            method: Some(command.to_string()),
            params: Some(args),
            result: None,
            error: None,
        };

        self.message_queue.push(message);
        id
    }

    /// 处理响应
    pub fn handle_response(&mut self, response: AcpMessage) -> Option<serde_json::Value> {
        match response.message_type {
            AcpMessageType::InitializeResult => {
                Some(response.result.unwrap_or(serde_json::Value::Null))
            }
            AcpMessageType::CommandResult => {
                Some(response.result.unwrap_or(serde_json::Value::Null))
            }
            AcpMessageType::Error => {
                eprintln!("ACP Error: {:?}", response.error);
                None
            }
            _ => None,
        }
    }

    /// 获取待发送的消息
    pub fn pending_messages(&self) -> &[AcpMessage] {
        &self.message_queue
    }

    /// 清空消息队列
    pub fn clear_messages(&mut self) {
        self.message_queue.clear();
    }
}
