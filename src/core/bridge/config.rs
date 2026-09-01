/// Bridge配置
use serde::{Deserialize, Serialize};

/// Bridge配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// 是否启用
    pub enabled: bool,
    /// 监听端口
    pub port: u16,
    /// JWT密钥
    pub jwt_secret: Option<String>,
    /// 认证令牌
    pub auth_token: Option<String>,
    /// 允许的源
    pub allowed_origins: Vec<String>,
    /// 最大连接数
    pub max_connections: usize,
    /// 心跳间隔（秒）
    pub heartbeat_interval_secs: u64,
    /// 会话超时（秒）
    pub session_timeout_secs: u64,
    /// 是否启用Web UI
    pub web_ui_enabled: bool,
    /// Web UI端口
    pub web_ui_port: u16,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 3000,
            jwt_secret: None,
            auth_token: None,
            allowed_origins: vec!["*".to_string()],
            max_connections: 100,
            heartbeat_interval_secs: 30,
            session_timeout_secs: 3600,
            web_ui_enabled: true,
            web_ui_port: 3001,
        }
    }
}

impl BridgeConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_BRIDGE_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let port = std::env::var("STAR_BRIDGE_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);

        let jwt_secret = std::env::var("STAR_BRIDGE_JWT_SECRET").ok();
        let auth_token = std::env::var("STAR_BRIDGE_AUTH_TOKEN").ok();

        let max_connections = std::env::var("STAR_BRIDGE_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let heartbeat_interval_secs = std::env::var("STAR_BRIDGE_HEARTBEAT_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let session_timeout_secs = std::env::var("STAR_BRIDGE_SESSION_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        let web_ui_enabled = std::env::var("STAR_BRIDGE_WEB_UI_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let web_ui_port = std::env::var("STAR_BRIDGE_WEB_UI_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3001);

        Self {
            enabled,
            port,
            jwt_secret,
            auth_token,
            allowed_origins: vec!["*".to_string()],
            max_connections,
            heartbeat_interval_secs,
            session_timeout_secs,
            web_ui_enabled,
            web_ui_port,
        }
    }
}
