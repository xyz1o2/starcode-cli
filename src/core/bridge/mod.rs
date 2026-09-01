/// Bridge远程控制系统
///
/// 对标claude-code-main的src/bridge/
/// 提供远程控制、WebSocket消息传输、JWT认证和Web UI功能
pub mod api;
pub mod auth;
pub mod config;
pub mod connection;
pub mod message;
pub mod session;
pub mod transport;
pub mod web_ui;

pub use api::BridgeApi;
pub use auth::{JwtAuth, JwtToken};
pub use config::BridgeConfig;
pub use connection::{BridgeConnection, ConnectionStatus};
pub use message::{BridgeMessage, MessageType};
pub use session::{SessionManager, SessionState};
pub use transport::{Transport, WebSocketTransport};
pub use web_ui::WebUiServer;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Bridge管理器
///
/// 管理远程控制连接、会话和消息传输
pub struct BridgeManager {
    /// 配置
    config: BridgeConfig,
    /// 连接管理
    connections: Arc<RwLock<HashMap<String, BridgeConnection>>>,
    /// 会话管理
    session_manager: Arc<RwLock<SessionManager>>,
    /// JWT认证
    auth: Arc<JwtAuth>,
    /// 消息队列
    message_queue: Arc<RwLock<Vec<BridgeMessage>>>,
    /// 运行状态
    running: Arc<RwLock<bool>>,
}

impl BridgeManager {
    /// 创建新的Bridge管理器
    pub fn new(config: BridgeConfig) -> Self {
        let auth = Arc::new(JwtAuth::new(config.jwt_secret.clone().unwrap_or_default()));
        let session_manager = Arc::new(RwLock::new(SessionManager::new()));

        Self {
            config,
            connections: Arc::new(RwLock::new(HashMap::new())),
            session_manager,
            auth,
            message_queue: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(BridgeConfig::from_env())
    }

    /// 启动Bridge服务
    pub async fn start(&self) -> Result<(), BridgeError> {
        if !self.config.enabled {
            return Err(BridgeError::NotEnabled);
        }

        let mut running = self.running.write().await;
        if *running {
            return Err(BridgeError::AlreadyRunning);
        }

        *running = true;

        // 启动WebSocket服务器
        let config = self.config.clone();
        let connections = self.connections.clone();
        let auth = self.auth.clone();
        let session_manager = self.session_manager.clone();

        tokio::spawn(async move {
            if let Err(e) =
                Self::run_websocket_server(config, connections, auth, session_manager).await
            {
                eprintln!("WebSocket server error: {}", e);
            }
        });

        Ok(())
    }

    /// 停止Bridge服务
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// 运行WebSocket服务器
    async fn run_websocket_server(
        config: BridgeConfig,
        connections: Arc<RwLock<HashMap<String, BridgeConnection>>>,
        auth: Arc<JwtAuth>,
        session_manager: Arc<RwLock<SessionManager>>,
    ) -> Result<(), BridgeError> {
        use tokio::net::TcpListener;

        let addr = format!("0.0.0.0:{}", config.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| BridgeError::BindError(e.to_string()))?;

        println!("Bridge TCP server listening on {}", addr);

        while let Ok((stream, peer)) = listener.accept().await {
            let connections = connections.clone();
            let auth = auth.clone();
            let session_manager = session_manager.clone();

            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_connection(stream, peer, connections, auth, session_manager).await
                {
                    eprintln!("Connection error: {}", e);
                }
            });
        }

        Ok(())
    }

    /// 处理单个连接
    async fn handle_connection(
        stream: tokio::net::TcpStream,
        peer: std::net::SocketAddr,
        connections: Arc<RwLock<HashMap<String, BridgeConnection>>>,
        auth: Arc<JwtAuth>,
        session_manager: Arc<RwLock<SessionManager>>,
    ) -> Result<(), BridgeError> {
        let connection_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let connection = BridgeConnection {
            id: connection_id.clone(),
            client_type: "tcp".to_string(),
            status: ConnectionStatus::Connected,
            connected_at: now,
            last_activity: now,
            peer_address: Some(peer.to_string()),
            session_id: None,
        };

        // 注册连接
        {
            let mut conns = connections.write().await;
            conns.insert(connection_id.clone(), connection);
        }

        // 创建会话
        {
            let mut session_mgr = session_manager.write().await;
            session_mgr.create_session(&connection_id);
        }

        // 处理TCP消息
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut reader, mut writer) = stream.into_split();
        let mut buffer = vec![0u8; 4096];

        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    // 更新活动时间
                    {
                        let mut conns = connections.write().await;
                        if let Some(conn) = conns.get_mut(&connection_id) {
                            conn.last_activity = chrono::Utc::now().timestamp();
                        }
                    }

                    // 处理消息
                    let text = String::from_utf8_lossy(&buffer[..n]).to_string();
                    if let Err(e) =
                        Self::process_message(&text, &connection_id, &auth, &session_manager).await
                    {
                        eprintln!("Message processing error: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("TCP read error: {}", e);
                    break;
                }
            }
        }

        // 清理连接
        {
            let mut conns = connections.write().await;
            conns.remove(&connection_id);
        }

        {
            let mut session_mgr = session_manager.write().await;
            session_mgr.remove_session(&connection_id);
        }

        Ok(())
    }

    /// 处理消息
    async fn process_message(
        text: &str,
        connection_id: &str,
        auth: &JwtAuth,
        session_manager: &Arc<RwLock<SessionManager>>,
    ) -> Result<(), BridgeError> {
        let message: BridgeMessage =
            serde_json::from_str(text).map_err(|e| BridgeError::ParseError(e.to_string()))?;

        // 验证JWT令牌
        if let Some(token) = &message.token {
            if !auth.verify_token(token) {
                return Err(BridgeError::Unauthorized);
            }
        }

        // 处理消息
        match message.message_type {
            MessageType::Command => {
                // 执行命令
                let mut session_mgr = session_manager.write().await;
                session_mgr.handle_command(connection_id, &message);
            }
            MessageType::Query => {
                // 查询状态
                let session_mgr = session_manager.read().await;
                session_mgr.handle_query(connection_id, &message);
            }
            MessageType::Ping => {
                // 心跳响应
                // TODO: 发送Pong响应
            }
            _ => {}
        }

        Ok(())
    }

    /// 注册连接
    pub async fn register_connection(&self, client_type: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let connection = BridgeConnection {
            id: id.clone(),
            client_type: client_type.to_string(),
            status: ConnectionStatus::Connected,
            connected_at: now,
            last_activity: now,
            peer_address: None,
            session_id: None,
        };

        let mut connections = self.connections.write().await;
        connections.insert(id.clone(), connection);
        id
    }

    /// 断开连接
    pub async fn disconnect(&self, connection_id: &str) {
        let mut connections = self.connections.write().await;
        if let Some(conn) = connections.get_mut(connection_id) {
            conn.status = ConnectionStatus::Disconnected;
        }
    }

    /// 获取连接信息
    pub async fn get_connection(&self, connection_id: &str) -> Option<BridgeConnection> {
        let connections = self.connections.read().await;
        connections.get(connection_id).cloned()
    }

    /// 获取所有连接
    pub async fn get_all_connections(&self) -> Vec<BridgeConnection> {
        let connections = self.connections.read().await;
        connections.values().cloned().collect()
    }

    /// 获取活跃连接数
    pub async fn active_connections(&self) -> usize {
        let connections = self.connections.read().await;
        connections
            .values()
            .filter(|c| matches!(c.status, ConnectionStatus::Connected))
            .count()
    }

    /// 发送消息
    pub async fn send_message(
        &self,
        connection_id: &str,
        message: BridgeMessage,
    ) -> Result<(), BridgeError> {
        // TODO: 实现WebSocket消息发送
        Ok(())
    }

    /// 广播消息
    pub async fn broadcast_message(&self, message: BridgeMessage) -> Result<(), BridgeError> {
        // TODO: 实现消息广播
        Ok(())
    }

    /// 获取配置
    pub fn config(&self) -> &BridgeConfig {
        &self.config
    }

    /// 检查是否运行中
    pub async fn is_running(&self) -> bool {
        let running = self.running.read().await;
        *running
    }
}

/// Bridge错误
#[derive(Debug)]
pub enum BridgeError {
    /// 未启用
    NotEnabled,
    /// 已运行
    AlreadyRunning,
    /// 绑定错误
    BindError(String),
    /// WebSocket错误
    WebSocketError(String),
    /// 解析错误
    ParseError(String),
    /// 未授权
    Unauthorized,
    /// 连接关闭
    ConnectionClosed,
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::NotEnabled => write!(f, "Bridge is not enabled"),
            BridgeError::AlreadyRunning => write!(f, "Bridge is already running"),
            BridgeError::BindError(e) => write!(f, "Failed to bind: {}", e),
            BridgeError::WebSocketError(e) => write!(f, "WebSocket error: {}", e),
            BridgeError::ParseError(e) => write!(f, "Parse error: {}", e),
            BridgeError::Unauthorized => write!(f, "Unauthorized"),
            BridgeError::ConnectionClosed => write!(f, "Connection closed"),
            BridgeError::ConfigError(e) => write!(f, "Config error: {}", e),
        }
    }
}

impl std::error::Error for BridgeError {}
