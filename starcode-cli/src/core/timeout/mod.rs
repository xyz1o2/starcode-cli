/// 全局超时管理器
/// 
/// 防止单个操作卡住过久

use std::time::Duration;

/// 超时配置
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// LLM调用超时（秒）
    pub llm_timeout_secs: u64,
    /// 工具执行超时（秒）
    pub tool_timeout_secs: u64,
    /// 压缩超时（秒）
    pub compact_timeout_secs: u64,
    /// 流式空闲超时（秒）
    pub stream_idle_timeout_secs: u64,
    /// 确认对话超时（秒）
    pub confirm_timeout_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            llm_timeout_secs: 120,      // 2分钟
            tool_timeout_secs: 60,       // 1分钟
            compact_timeout_secs: 30,    // 30秒
            stream_idle_timeout_secs: 30, // 30秒
            confirm_timeout_secs: 300,   // 5分钟
        }
    }
}

impl TimeoutConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let llm_timeout_secs = std::env::var("STAR_LLM_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);

        let tool_timeout_secs = std::env::var("STAR_TOOL_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let compact_timeout_secs = std::env::var("STAR_COMPACT_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let stream_idle_timeout_secs = std::env::var("STAR_STREAM_IDLE_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let confirm_timeout_secs = std::env::var("STAR_CONFIRM_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        Self {
            llm_timeout_secs,
            tool_timeout_secs,
            compact_timeout_secs,
            stream_idle_timeout_secs,
            confirm_timeout_secs,
        }
    }

    /// 获取LLM超时Duration
    pub fn llm_timeout(&self) -> Duration {
        Duration::from_secs(self.llm_timeout_secs)
    }

    /// 获取工具超时Duration
    pub fn tool_timeout(&self) -> Duration {
        Duration::from_secs(self.tool_timeout_secs)
    }

    /// 获取压缩超时Duration
    pub fn compact_timeout(&self) -> Duration {
        Duration::from_secs(self.compact_timeout_secs)
    }

    /// 获取流式空闲超时Duration
    pub fn stream_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.stream_idle_timeout_secs)
    }

    /// 获取确认超时Duration
    pub fn confirm_timeout(&self) -> Duration {
        Duration::from_secs(self.confirm_timeout_secs)
    }
}

/// 超时管理器
pub struct TimeoutManager {
    config: TimeoutConfig,
}

impl TimeoutManager {
    /// 创建新的超时管理器
    pub fn new(config: TimeoutConfig) -> Self {
        Self { config }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(TimeoutConfig::from_env())
    }

    /// 获取配置
    pub fn config(&self) -> &TimeoutConfig {
        &self.config
    }

    /// 执行带超时的操作
    pub async fn execute_with_timeout<F, T>(
        &self,
        operation: F,
        timeout: Duration,
        operation_name: &str,
    ) -> Result<T, TimeoutError>
    where
        F: std::future::Future<Output = T>,
    {
        match tokio::time::timeout(timeout, operation).await {
            Ok(result) => Ok(result),
            Err(_) => {
                log::warn!("Operation '{}' timed out after {:?}", operation_name, timeout);
                Err(TimeoutError {
                    operation: operation_name.to_string(),
                    timeout_secs: timeout.as_secs(),
                })
            }
        }
    }

    /// 执行LLM调用（带超时）
    pub async fn execute_llm_call<F, T>(&self, operation: F) -> Result<T, TimeoutError>
    where
        F: std::future::Future<Output = T>,
    {
        self.execute_with_timeout(operation, self.config.llm_timeout(), "LLM call").await
    }

    /// 执行工具调用（带超时）
    pub async fn execute_tool_call<F, T>(&self, operation: F) -> Result<T, TimeoutError>
    where
        F: std::future::Future<Output = T>,
    {
        self.execute_with_timeout(operation, self.config.tool_timeout(), "Tool execution").await
    }

    /// 执行压缩（带超时）
    pub async fn execute_compact<F, T>(&self, operation: F) -> Result<T, TimeoutError>
    where
        F: std::future::Future<Output = T>,
    {
        self.execute_with_timeout(operation, self.config.compact_timeout(), "Compaction").await
    }
}

/// 超时错误
#[derive(Debug)]
pub struct TimeoutError {
    /// 操作名称
    pub operation: String,
    /// 超时秒数
    pub timeout_secs: u64,
}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Operation '{}' timed out after {} seconds", self.operation, self.timeout_secs)
    }
}

impl std::error::Error for TimeoutError {}
