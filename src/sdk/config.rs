/// SDK配置

/// SDK配置
#[derive(Debug, Clone)]
pub struct SDKConfig {
    /// 是否启用
    pub enabled: bool,
    /// 监听端口
    pub port: u16,
    /// API密钥
    pub api_key: Option<String>,
    /// 最大客户端数
    pub max_clients: usize,
    /// 超时（秒）
    pub timeout_secs: u64,
}

impl Default for SDKConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 3002,
            api_key: None,
            max_clients: 10,
            timeout_secs: 60,
        }
    }
}

impl SDKConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_SDK_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let port = std::env::var("STAR_SDK_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3002);

        let api_key = std::env::var("STAR_SDK_API_KEY").ok();

        Self {
            enabled,
            port,
            api_key,
            max_clients: 10,
            timeout_secs: 60,
        }
    }
}
