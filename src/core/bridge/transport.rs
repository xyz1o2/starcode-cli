/// Bridge传输层
/// 
/// 对标claude-code-main的src/bridge/replBridgeTransport.ts
/// 提供WebSocket、Stdio和HTTP传输

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::collections::VecDeque;
use tokio::sync::mpsc;

/// 传输类型
#[derive(Debug, Clone, PartialEq)]
pub enum TransportType {
    WebSocket,
    Stdio,
    Http,
}

/// 传输消息
#[derive(Debug, Clone)]
pub enum TransportMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close,
}

/// 传输配置
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// 传输类型
    pub transport_type: TransportType,
    /// 最大消息大小
    pub max_message_size: usize,
    /// 心跳间隔（秒）
    pub heartbeat_interval_secs: u64,
    /// 重连次数
    pub max_reconnect_attempts: u32,
    /// 超时（秒）
    pub timeout_secs: u64,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            transport_type: TransportType::WebSocket,
            max_message_size: 1024 * 1024, // 1MB
            heartbeat_interval_secs: 30,
            max_reconnect_attempts: 3,
            timeout_secs: 60,
        }
    }
}

/// 传输trait
#[async_trait]
pub trait Transport: Send + Sync {
    /// 发送消息
    async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError>;
    
    /// 接收消息
    async fn receive(&mut self) -> Result<TransportMessage, TransportError>;
    
    /// 关闭传输
    async fn close(&mut self) -> Result<(), TransportError>;
    
    /// 检查是否连接
    fn is_connected(&self) -> bool;
    
    /// 获取传输类型
    fn transport_type(&self) -> TransportType;
    
    /// 获取配置
    fn config(&self) -> &TransportConfig;
}

/// WebSocket传输
/// 
/// 对标claude-code-main的WebSocket传输实现
pub struct WebSocketTransport {
    /// 连接ID
    id: String,
    /// 是否连接
    connected: bool,
    /// 配置
    config: TransportConfig,
    /// 消息队列
    message_queue: VecDeque<TransportMessage>,
    /// 发送通道
    tx: Option<mpsc::UnboundedSender<TransportMessage>>,
    /// 接收通道
    rx: Option<mpsc::UnboundedReceiver<TransportMessage>>,
}

impl WebSocketTransport {
    /// 创建新的WebSocket传输
    pub fn new(id: String) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        
        Self {
            id,
            connected: true,
            config: TransportConfig::default(),
            message_queue: VecDeque::new(),
            tx: Some(tx),
            rx: Some(rx),
        }
    }

    /// 使用配置创建
    pub fn with_config(id: String, config: TransportConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        
        Self {
            id,
            connected: true,
            config,
            message_queue: VecDeque::new(),
            tx: Some(tx),
            rx: Some(rx),
        }
    }

    /// 获取连接ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 重连
    pub async fn reconnect(&mut self) -> Result<(), TransportError> {
        // TODO: 实现重连逻辑
        self.connected = true;
        Ok(())
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        // 检查消息大小
        match &message {
            TransportMessage::Text(text) => {
                if text.len() > self.config.max_message_size {
                    return Err(TransportError::SendError("Message too large".to_string()));
                }
            }
            TransportMessage::Binary(data) => {
                if data.len() > self.config.max_message_size {
                    return Err(TransportError::SendError("Message too large".to_string()));
                }
            }
            _ => {}
        }

        // 发送消息
        if let Some(tx) = &self.tx {
            tx.send(message).map_err(|e| TransportError::SendError(e.to_string()))?;
        }

        Ok(())
    }

    async fn receive(&mut self) -> Result<TransportMessage, TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        // 从队列中获取消息
        if let Some(message) = self.message_queue.pop_front() {
            return Ok(message);
        }

        // 从通道接收消息
        if let Some(rx) = &mut self.rx {
            match tokio::time::timeout(
                std::time::Duration::from_secs(self.config.timeout_secs),
                rx.recv()
            ).await {
                Ok(Some(message)) => Ok(message),
                Ok(None) => Err(TransportError::ConnectionClosed),
                Err(_) => Err(TransportError::Timeout),
            }
        } else {
            Err(TransportError::NotConnected)
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.connected = false;
        self.tx = None;
        self.rx = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn transport_type(&self) -> TransportType {
        TransportType::WebSocket
    }

    fn config(&self) -> &TransportConfig {
        &self.config
    }
}

/// Stdio传输
pub struct StdioTransport {
    /// 是否连接
    connected: bool,
    /// 配置
    config: TransportConfig,
}

impl StdioTransport {
    /// 创建新的Stdio传输
    pub fn new() -> Self {
        Self {
            connected: true,
            config: TransportConfig::default(),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&mut self, message: TransportMessage) -> Result<(), TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        match message {
            TransportMessage::Text(text) => {
                println!("{}", text);
            }
            _ => {}
        }

        Ok(())
    }

    async fn receive(&mut self) -> Result<TransportMessage, TransportError> {
        if !self.connected {
            return Err(TransportError::NotConnected);
        }

        // 从stdin读取
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)
            .map_err(|e| TransportError::ReceiveError(e.to_string()))?;
        
        Ok(TransportMessage::Text(input.trim().to_string()))
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.connected = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Stdio
    }

    fn config(&self) -> &TransportConfig {
        &self.config
    }
}

/// 传输错误
#[derive(Debug)]
pub enum TransportError {
    /// 未连接
    NotConnected,
    /// 超时
    Timeout,
    /// 发送错误
    SendError(String),
    /// 接收错误
    ReceiveError(String),
    /// 连接关闭
    ConnectionClosed,
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::NotConnected => write!(f, "Not connected"),
            TransportError::Timeout => write!(f, "Timeout"),
            TransportError::SendError(e) => write!(f, "Send error: {}", e),
            TransportError::ReceiveError(e) => write!(f, "Receive error: {}", e),
            TransportError::ConnectionClosed => write!(f, "Connection closed"),
            TransportError::ConfigError(e) => write!(f, "Config error: {}", e),
        }
    }
}

impl std::error::Error for TransportError {}
