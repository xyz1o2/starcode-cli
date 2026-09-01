/// Langfuse可观测性系统
/// 
/// 对标claude-code-main的src/services/langfuse/
/// 提供LLM调用追踪、span管理和性能监控功能

pub mod client;
pub mod convert;
pub mod sanitize;
pub mod tracing;
pub mod integration;

pub use client::LangfuseClient;
pub use convert::EventConverter;
pub use sanitize::DataSanitizer;
pub use tracing::{TraceManager, Trace, Span, SpanStatus};
pub use integration::{LangfuseIntegration, IntegrationConfig};

use serde::{Deserialize, Serialize};

/// Langfuse配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangfuseConfig {
    /// 是否启用
    pub enabled: bool,
    /// 公钥
    pub public_key: Option<String>,
    /// 秘钥
    pub secret_key: Option<String>,
    /// API端点
    pub api_endpoint: String,
    /// 批量发送大小
    pub batch_size: usize,
    /// 发送间隔（秒）
    pub flush_interval_secs: u64,
    /// 是否启用调试模式
    pub debug: bool,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_key: None,
            secret_key: None,
            api_endpoint: "https://cloud.langfuse.com".to_string(),
            batch_size: 10,
            flush_interval_secs: 30,
            debug: false,
        }
    }
}

impl LangfuseConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("LANGFUSE_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let public_key = std::env::var("LANGFUSE_PUBLIC_KEY").ok();
        let secret_key = std::env::var("LANGFUSE_SECRET_KEY").ok();

        let api_endpoint = std::env::var("LANGFUSE_API_ENDPOINT")
            .ok()
            .unwrap_or_else(|| "https://cloud.langfuse.com".to_string());

        let batch_size = std::env::var("LANGFUSE_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let flush_interval_secs = std::env::var("LANGFUSE_FLUSH_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let debug = std::env::var("LANGFUSE_DEBUG")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            enabled,
            public_key,
            secret_key,
            api_endpoint,
            batch_size,
            flush_interval_secs,
            debug,
        }
    }

    /// 检查配置是否有效
    pub fn is_valid(&self) -> bool {
        self.enabled && self.public_key.is_some() && self.secret_key.is_some()
    }
}
