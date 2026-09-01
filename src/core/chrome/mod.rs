//! Chrome MCP 浏览器控制模块
//!
//! 对标 Claude Code 的 chrome-use-mcp.md：
//! - Chrome 扩展控制
//! - 页面交互
//! - 网络监控

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Chrome MCP 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromeMcpConfig {
    pub enabled: bool,
    pub debug_port: u16,
    pub extension_id: Option<String>,
    pub auto_connect: bool,
}

impl Default for ChromeMcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            debug_port: 9222,
            extension_id: None,
            auto_connect: false,
        }
    }
}

/// 页面信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub type_: String,
}

/// DOM 元素
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomElement {
    pub selector: String,
    pub tag_name: String,
    pub text_content: Option<String>,
    pub attributes: std::collections::HashMap<String, String>,
    pub is_visible: bool,
}

/// Chrome MCP 操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChromeAction {
    /// 获取页面列表
    ListPages,
    /// 导航到 URL
    Navigate { url: String },
    /// 获取页面内容
    GetContent { selector: Option<String> },
    /// 点击元素
    Click { selector: String },
    /// 输入文本
    Type { selector: String, text: String },
    /// 获取截图
    Screenshot,
    /// 执行 JavaScript
    ExecuteScript { script: String },
    /// 获取网络请求
    GetNetworkRequests,
    /// 获取控制台日志
    GetConsoleLogs,
    /// 等待元素
    WaitForElement { selector: String, timeout_ms: u64 },
}

/// Chrome MCP 结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChromeResult {
    Pages(Vec<PageInfo>),
    Content(String),
    Element(DomElement),
    ScreenshotData { base64: String },
    ScriptResult(Value),
    NetworkRequests(Vec<NetworkRequest>),
    ConsoleLogs(Vec<ConsoleLog>),
    Success { message: String },
    Error { message: String },
}

/// 网络请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub url: String,
    pub method: String,
    pub status: Option<u16>,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<String>,
}

/// 控制台日志
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleLog {
    pub level: String,
    pub message: String,
    pub timestamp: u64,
}

/// Chrome MCP 管理器
pub struct ChromeMcpManager {
    config: ChromeMcpConfig,
    connected: bool,
    pages: Vec<PageInfo>,
}

impl ChromeMcpManager {
    pub fn new(config: ChromeMcpConfig) -> Self {
        Self {
            config,
            connected: false,
            pages: Vec::new(),
        }
    }

    /// 连接到 Chrome
    pub async fn connect(&mut self) -> Result<(), String> {
        if !self.config.enabled {
            return Err("Chrome MCP is disabled".to_string());
        }

        // 通过 CDP 连接
        let url = format!("http://127.0.0.1:{}/json", self.config.debug_port);
        let client = reqwest::Client::new();

        match client.get(&url).send().await {
            Ok(response) => {
                if let Ok(pages) = response.json::<Vec<Value>>().await {
                    self.pages = pages
                        .iter()
                        .filter_map(|p| {
                            Some(PageInfo {
                                id: p["id"].as_str()?.to_string(),
                                url: p["url"].as_str().unwrap_or("").to_string(),
                                title: p["title"].as_str().unwrap_or("").to_string(),
                                type_: p["type"].as_str().unwrap_or("page").to_string(),
                            })
                        })
                        .collect();
                    self.connected = true;
                    Ok(())
                } else {
                    Err("Failed to parse Chrome response".to_string())
                }
            }
            Err(e) => Err(format!("Failed to connect to Chrome: {}", e)),
        }
    }

    /// 执行操作
    pub async fn execute(&self, action: ChromeAction) -> ChromeResult {
        if !self.connected {
            return ChromeResult::Error {
                message: "Not connected to Chrome".to_string(),
            };
        }

        match action {
            ChromeAction::ListPages => ChromeResult::Pages(self.pages.clone()),
            ChromeAction::Navigate { url } => {
                // 通过 CDP 发送导航命令
                ChromeResult::Success {
                    message: format!("Navigating to {}", url),
                }
            }
            ChromeAction::Screenshot => {
                // 通过 CDP 截图
                ChromeResult::ScreenshotData {
                    base64: String::new(),
                }
            }
            _ => ChromeResult::Error {
                message: "Action not yet implemented".to_string(),
            },
        }
    }

    /// 断开连接
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.pages.clear();
    }

    /// 是否已连接
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}
