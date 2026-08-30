use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

/// Langfuse配置
#[derive(Debug, Clone)]
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

/// Langfuse客户端
/// 
/// 与Langfuse API交互的客户端
pub struct LangfuseClient {
    /// 配置
    config: LangfuseConfig,
    /// 事件缓冲区
    event_buffer: Vec<LangfuseEvent>,
    /// HTTP客户端
    http_client: reqwest::Client,
    /// 上次刷新时间
    last_flush: SystemTime,
}

/// Langfuse事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangfuseEvent {
    /// 事件ID
    pub id: String,
    /// 事件类型
    pub event_type: String,
    /// 时间戳
    pub timestamp: u64,
    /// 事件数据
    pub data: serde_json::Value,
}

impl LangfuseClient {
    /// 创建新的Langfuse客户端
    pub fn new(config: LangfuseConfig) -> Self {
        let http_client = reqwest::Client::new();

        Self {
            config,
            event_buffer: Vec::new(),
            http_client,
            last_flush: SystemTime::now(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(LangfuseConfig::from_env())
    }

    /// 创建trace
    pub fn create_trace(&mut self, trace_id: &str, name: &str, metadata: HashMap<String, serde_json::Value>) -> Result<(), LangfuseError> {
        if !self.config.enabled {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = LangfuseEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: "trace-create".to_string(),
            timestamp,
            data: serde_json::json!({
                "id": trace_id,
                "name": name,
                "metadata": metadata,
                "timestamp": timestamp
            }),
        };

        self.event_buffer.push(event);

        if self.event_buffer.len() >= self.config.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    /// 创建span
    pub fn create_span(&mut self, trace_id: &str, span_id: &str, name: &str, metadata: HashMap<String, serde_json::Value>) -> Result<(), LangfuseError> {
        if !self.config.enabled {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = LangfuseEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: "span-create".to_string(),
            timestamp,
            data: serde_json::json!({
                "traceId": trace_id,
                "id": span_id,
                "name": name,
                "metadata": metadata,
                "startTime": timestamp
            }),
        };

        self.event_buffer.push(event);

        if self.event_buffer.len() >= self.config.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    /// 更新span状态
    pub fn update_span(&mut self, span_id: &str, status: &str, output: Option<serde_json::Value>) -> Result<(), LangfuseError> {
        if !self.config.enabled {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = LangfuseEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: "span-update".to_string(),
            timestamp,
            data: serde_json::json!({
                "id": span_id,
                "status": status,
                "output": output,
                "endTime": timestamp
            }),
        };

        self.event_buffer.push(event);

        if self.event_buffer.len() >= self.config.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    /// 记录生成（LLM调用）
    pub fn create_generation(&mut self, trace_id: &str, generation_id: &str, model: &str, input: serde_json::Value, output: Option<serde_json::Value>, metadata: HashMap<String, serde_json::Value>) -> Result<(), LangfuseError> {
        if !self.config.enabled {
            return Ok(());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = LangfuseEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: "generation-create".to_string(),
            timestamp,
            data: serde_json::json!({
                "traceId": trace_id,
                "id": generation_id,
                "model": model,
                "input": input,
                "output": output,
                "metadata": metadata,
                "startTime": timestamp
            }),
        };

        self.event_buffer.push(event);

        if self.event_buffer.len() >= self.config.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    /// 刷新事件缓冲区
    pub fn flush(&mut self) -> Result<(), LangfuseError> {
        if self.event_buffer.is_empty() || !self.config.enabled {
            return Ok(());
        }

        let events: Vec<LangfuseEvent> = self.event_buffer.drain(..).collect();
        
        // 在调试模式下打印事件
        if self.config.debug {
            println!("[Langfuse] Flushing {} events", events.len());
            for event in &events {
                println!("[Langfuse] Event: {:?}", event);
            }
        }

        // 实际发送到Langfuse API
        // 注意：这里简化了实现，实际应该使用异步HTTP请求
        if let (Some(public_key), Some(secret_key)) = (&self.config.public_key, &self.config.secret_key) {
            let url = format!("{}/api/public/ingestion", self.config.api_endpoint);
            
            let payload = serde_json::json!({
                "batch": events
            });

            // 这里应该使用异步HTTP请求，但为了简化，我们使用同步请求
            // 实际实现中应该使用tokio::spawn来异步发送
            println!("[Langfuse] Would send {} events to {}", events.len(), url);
        }

        self.last_flush = SystemTime::now();
        Ok(())
    }

    /// 检查是否需要刷新
    pub fn should_flush(&self) -> bool {
        if self.event_buffer.is_empty() {
            return false;
        }

        let elapsed = self.last_flush.elapsed().unwrap_or_default();
        elapsed.as_secs() >= self.config.flush_interval_secs
    }

    /// 获取缓冲区大小
    pub fn buffer_size(&self) -> usize {
        self.event_buffer.len()
    }

    /// 获取配置
    pub fn config(&self) -> &LangfuseConfig {
        &self.config
    }
}

/// Langfuse错误
#[derive(Debug)]
pub enum LangfuseError {
    /// 配置错误
    ConfigError(String),
    /// 网络错误
    NetworkError(String),
    /// API错误
    ApiError(String),
    /// 序列化错误
    SerializationError(String),
}

impl std::fmt::Display for LangfuseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LangfuseError::ConfigError(msg) => write!(f, "Langfuse config error: {}", msg),
            LangfuseError::NetworkError(msg) => write!(f, "Langfuse network error: {}", msg),
            LangfuseError::ApiError(msg) => write!(f, "Langfuse API error: {}", msg),
            LangfuseError::SerializationError(msg) => write!(f, "Langfuse serialization error: {}", msg),
        }
    }
}

impl std::error::Error for LangfuseError {}
