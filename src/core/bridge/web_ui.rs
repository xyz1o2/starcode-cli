/// Bridge Web UI服务器

use serde::{Deserialize, Serialize};

/// Web UI配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebUiConfig {
    /// 是否启用
    pub enabled: bool,
    /// 端口
    pub port: u16,
    /// 静态文件目录
    pub static_dir: Option<String>,
    /// 标题
    pub title: String,
}

impl Default for WebUiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 3001,
            static_dir: None,
            title: "StarCode Bridge".to_string(),
        }
    }
}

/// Web UI服务器
pub struct WebUiServer {
    config: WebUiConfig,
    running: bool,
}

impl WebUiServer {
    /// 创建新的Web UI服务器
    pub fn new(config: WebUiConfig) -> Self {
        Self {
            config,
            running: false,
        }
    }

    /// 启动Web UI服务器
    pub async fn start(&mut self) -> Result<(), WebUiError> {
        if !self.config.enabled {
            return Err(WebUiError::NotEnabled);
        }

        self.running = true;

        // TODO: 实现Web UI服务器
        println!("Web UI server would start on port {}", self.config.port);

        Ok(())
    }

    /// 停止Web UI服务器
    pub async fn stop(&mut self) {
        self.running = false;
    }

    /// 检查是否运行中
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// 获取配置
    pub fn config(&self) -> &WebUiConfig {
        &self.config
    }
}

/// Web UI错误
#[derive(Debug)]
pub enum WebUiError {
    /// 未启用
    NotEnabled,
    /// 绑定错误
    BindError(String),
    /// 运行错误
    RuntimeError(String),
}

impl std::fmt::Display for WebUiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebUiError::NotEnabled => write!(f, "Web UI is not enabled"),
            WebUiError::BindError(e) => write!(f, "Bind error: {}", e),
            WebUiError::RuntimeError(e) => write!(f, "Runtime error: {}", e),
        }
    }
}

impl std::error::Error for WebUiError {}
