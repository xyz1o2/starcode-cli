/// 重试管理器
/// 
/// 对标claude-code-main的src/services/api/withRetry.ts
/// 提供智能重试机制，防止对话卡住

use std::time::Duration;

/// 重试配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// 最大重试次数
    pub max_retries: u32,
    /// 基础延迟（毫秒）
    pub base_delay_ms: u64,
    /// 最大延迟（毫秒）
    pub max_delay_ms: u64,
    /// 529错误最大重试次数
    pub max_529_retries: u32,
    /// 是否启用持久化重试
    pub persistent_retry: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            base_delay_ms: 500,
            max_delay_ms: 32000,
            max_529_retries: 3,
            persistent_retry: false,
        }
    }
}

impl RetryConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let max_retries = std::env::var("STAR_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let base_delay_ms = std::env::var("STAR_RETRY_BASE_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);

        let max_delay_ms = std::env::var("STAR_RETRY_MAX_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(32000);

        let max_529_retries = std::env::var("STAR_MAX_529_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let persistent_retry = std::env::var("STAR_PERSISTENT_RETRY")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            max_retries,
            base_delay_ms,
            max_delay_ms,
            max_529_retries,
            persistent_retry,
        }
    }
}

/// 重试管理器
pub struct RetryManager {
    config: RetryConfig,
    /// 当前重试次数
    current_retries: u32,
    /// 529错误次数
    error_529_count: u32,
    /// 最后错误时间
    last_error_time: Option<std::time::Instant>,
}

impl RetryManager {
    /// 创建新的重试管理器
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            current_retries: 0,
            error_529_count: 0,
            last_error_time: None,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(RetryConfig::from_env())
    }

    /// 检查是否应该重试
    pub fn should_retry(&mut self, error: &str) -> bool {
        self.current_retries += 1;
        self.last_error_time = Some(std::time::Instant::now());

        // 检查是否是529错误
        if error.contains("529") || error.contains("overloaded") {
            self.error_529_count += 1;
            if self.error_529_count >= self.config.max_529_retries {
                return false;
            }
        }

        // 检查是否超过最大重试次数
        if self.current_retries >= self.config.max_retries {
            // 如果启用持久化重试，继续重试
            if self.config.persistent_retry {
                return true;
            }
            return false;
        }

        true
    }

    /// 获取重试延迟
    pub fn get_retry_delay(&self) -> Duration {
        // 指数退避延迟
        let delay_ms = self.config.base_delay_ms * 2u64.pow(self.current_retries - 1);
        let delay_ms = delay_ms.min(self.config.max_delay_ms);
        
        // 添加抖动
        let jitter = (rand::random::<f64>() * 0.25 * delay_ms as f64) as u64;
        Duration::from_millis(delay_ms + jitter)
    }

    /// 重置重试计数器
    pub fn reset(&mut self) {
        self.current_retries = 0;
        self.error_529_count = 0;
        self.last_error_time = None;
    }

    /// 获取当前重试次数
    pub fn current_retries(&self) -> u32 {
        self.current_retries
    }

    /// 获取529错误次数
    pub fn error_529_count(&self) -> u32 {
        self.error_529_count
    }
}
