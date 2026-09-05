pub mod client;
pub mod message_pipeline;
mod mock;
mod openai_compatible;
pub mod providers;
pub mod rig_adapter;
pub mod thinking;

pub use crate::core::config::models::ModelInfo;
pub use mock::MockClient;
pub use openai_compatible::OpenAiCompatibleClient;
pub use providers::{BedrockProvider, GeminiProvider, GrokProvider, VertexProvider};

use crate::types::{StarMessage, StarResponse, StarTool, StarToolCall, StarUsage};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::pin::Pin;
use std::time::Duration;
use zeroize::{Zeroize, ZeroizeOnDrop};

// ─────────────────────────────────────────────────────────────────────
// Shared constants — eliminate magic values across the LLM module
// ─────────────────────────────────────────────────────────────────────

/// Sentinel value indicating an API key has not been configured.
pub const API_KEY_NOT_SET: &str = "API_KEY_NOT_SET";

// ── HTTP defaults ──
const HTTP_POOL_IDLE_TIMEOUT_SECS: u64 = 90;
const HTTP_POOL_MAX_IDLE_PER_HOST: usize = 8;
const TCP_KEEPALIVE_SECS: u64 = 60;
const DEFAULT_LLM_TIMEOUT_SECS: u64 = 120;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;

// ── LLM defaults ──
pub const DEFAULT_MAX_TOKENS: u32 = 8192;
pub const DEFAULT_TEMPERATURE: f32 = 0.7;
/// Lower default for OpenAI-compatible providers (more deterministic output)
pub const DEFAULT_COMPAT_TEMPERATURE: f32 = 0.1;

// ── API paths ──
pub const API_PATH_CHAT_COMPLETIONS: &str = "/chat/completions";

// ── SSE parsing ──
pub const SSE_DATA_PREFIX: &str = "data: ";
pub const SSE_DONE_MARKER: &str = "[DONE]";
pub const FINISH_REASON_STOP: &str = "stop";
pub const FINISH_REASON_NULL: &str = "null";
pub const CALL_TYPE_FUNCTION: &str = "function";
pub const EMPTY_JSON_OBJECT: &str = "{}";
pub const AUTO_TOOL_CALL_ID_PREFIX: &str = "call_auto_";

// ── Tool ID normalization ──
/// Prefix used by proxy/adapter layers to wrap tool call IDs.
pub const PROXY_TOOL_ID_PREFIX: &str = "call_function_";

// ── HTTP headers ──
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_AUTHORIZATION: &str = "Authorization";
pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const BEARER_PREFIX: &str = "Bearer ";
pub const HEADER_X_API_KEY: &str = "x-api-key";
pub const HEADER_ANTHROPIC_VERSION: &str = "anthropic-version";
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";
pub const ANTHROPIC_API_URL_PATTERN: &str = "api.anthropic.com";

// ── Provider default URLs ──
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub const MINIMAX_DEFAULT_BASE_URL: &str = "https://api.minimax.chat/v1";
pub const XIAOMI_DEFAULT_BASE_URL: &str = "https://api.xiaomimimo.com/v1";

// ── Provider env IDs ──
pub const PROVIDER_ENV_ID_DEEPSEEK: &str = "deepseek";
pub const PROVIDER_ENV_ID_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_ENV_ID_XIAOMI: &str = "xiaomi";
pub const PROVIDER_NAME_OPENAI_COMPATIBLE: &str = "openai-compatible";

// ── Environment variable names ──
pub const ENV_STAR_API_KEY: &str = "STAR_API_KEY";
pub const ENV_STAR_LLM_TIMEOUT: &str = "STAR_LLM_TIMEOUT";
pub const ENV_STAR_CONNECT_TIMEOUT: &str = "STAR_CONNECT_TIMEOUT";
pub const ENV_STAR_MAX_TOKENS: &str = "STAR_MAX_TOKENS";
pub const ENV_STAR_TEMPERATURE: &str = "STAR_TEMPERATURE";
pub const ENV_STAR_LLM_VERBOSE_LOG: &str = "STAR_LLM_VERBOSE_LOG";
pub const ENV_STAR_STREAM_TIMEOUT: &str = "STAR_STREAM_TIMEOUT";
pub const ENV_STAR_LLM_INITIAL_REQUEST_RETRIES: &str = "STAR_LLM_INITIAL_REQUEST_RETRIES";
pub const ENV_STAR_LLM_RETRY_BASE_DELAY_MS: &str = "STAR_LLM_RETRY_BASE_DELAY_MS";
/// Thinking/reasoning effort level for supported models (none, low, medium, high)
pub const ENV_STAR_THINKING_EFFORT: &str = "STAR_THINKING_EFFORT";

// ── Error matching patterns ──
pub const HTTP_STATUS_401: &str = "401";
pub const HTTP_STATUS_402: &str = "402";
pub const AUTH_ERROR_UNAUTHORIZED: &str = "Unauthorized";
pub const AUTH_ERROR_INCORRECT_KEY: &str = "Incorrect API key";
pub const PAYMENT_ERROR_REQUIRED: &str = "Payment Required";
pub const PAYMENT_ERROR_BALANCE: &str = "Insufficient Balance";

// ── Debug/truncation limits ──
pub const DEBUG_PAYLOAD_MAX_CHARS: usize = 200;
pub const DEBUG_ERROR_RESPONSE_MAX_CHARS: usize = 500;
pub const DEBUG_ERROR_SHORT_MAX_CHARS: usize = 200;
pub const SUMMARY_TRUNCATE_MAX_CHARS: usize = 1000;
pub const SEGMENT_SUMMARY_ASSISTANT_MAX_CHARS: usize = 500;
pub const SEGMENT_SUMMARY_TOOL_MAX_CHARS: usize = 800;

// ── Kimi Code ──
pub const KIMI_CODE_URL_PATTERN: &str = "api.kimi.com/coding";

// ── API key source labels ──
pub const API_KEY_SOURCE_CONFIGURED: &str = "configured";
pub const API_KEY_SOURCE_ENV_STAR: &str = "env:STAR_API_KEY";
pub const API_KEY_SOURCE_UNKNOWN: &str = "unknown";
pub const API_KEY_SOURCE_PROVIDER_ENV_PREFIX: &str = "provider_env:";

/// Shared HTTP client builder for all LLM providers.
/// Forces HTTP/1.1 (SSE over HTTP/2 is unreliable on many LLM APIs)
/// and enables gzip/brotli decompression.
pub fn build_http_client(timeout_secs: u64, connect_timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .http1_only()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .connect_timeout(std::time::Duration::from_secs(connect_timeout_secs))
        .pool_idle_timeout(std::time::Duration::from_secs(HTTP_POOL_IDLE_TIMEOUT_SECS))
        .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE_PER_HOST)
        .tcp_keepalive(std::time::Duration::from_secs(TCP_KEEPALIVE_SECS))
        .tcp_nodelay(true)
        .build()
        .unwrap_or_default()
}

fn default_timeout() -> u64 {
    std::env::var(ENV_STAR_LLM_TIMEOUT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LLM_TIMEOUT_SECS)
}

fn default_connect_timeout() -> u64 {
    std::env::var(ENV_STAR_CONNECT_TIMEOUT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS)
}

pub fn verbose_logging_enabled() -> bool {
    std::env::var(ENV_STAR_LLM_VERBOSE_LOG)
        .ok()
        .map(|v| {
            let normalized = v.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "on" | "yes")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmEvent {
    Trace { event: String, payload: Value },
    TextChunk(String),
    ReasoningChunk(String),
    ToolCall(StarToolCall),
    UsageUpdate(StarUsage),
    Finish(String),
    Error(String),
}

#[derive(Debug)]
pub enum LlmError {
    ProviderError(String),
    NetworkError(String),
    ParsingError(String),
    NotImplemented(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            LlmError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            LlmError::ParsingError(msg) => write!(f, "Parsing error: {}", msg),
            LlmError::NotImplemented(msg) => write!(f, "Not implemented: {}", msg),
        }
    }
}

impl std::error::Error for LlmError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
pub enum LlmProvider {
    OpenAi,
    Anthropic,
    DeepSeek,
    MiniMax,
    Xiaomi,
    Mock,
    Custom(String),
}

impl LlmProvider {}

impl std::str::FromStr for LlmProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(LlmProvider::OpenAi),
            "anthropic" | "claude" => Ok(LlmProvider::Anthropic),
            "deepseek" => Ok(LlmProvider::DeepSeek),
            "minimax" => Ok(LlmProvider::MiniMax),
            "xiaomi" | "mimo" => Ok(LlmProvider::Xiaomi),
            "mock" => Ok(LlmProvider::Mock),
            other => Ok(LlmProvider::Custom(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub api_key: String,
    #[zeroize(skip)]
    pub base_url: Option<String>,
    #[zeroize(skip)]
    pub model: String,
}

pub fn create_client(config: LlmConfig) -> Box<dyn LlmClient> {
    let base_url = config.base_url.clone();

    match &config.provider {
        LlmProvider::OpenAi => Box::new(rig_adapter::RigAdapter::openai(
            config.api_key.clone(),
            config.model.clone(),
        )),
        LlmProvider::Anthropic => Box::new(rig_adapter::RigAdapter::anthropic(
            config.api_key.clone(),
            config.model.clone(),
        )),
        LlmProvider::DeepSeek => Box::new(rig_adapter::RigAdapter::deepseek(
            config.api_key.clone(),
            config.model.clone(),
        )),
        LlmProvider::MiniMax => Box::new(rig_adapter::RigAdapter::openai_compatible(
            config.api_key.clone(),
            config.model.clone(),
            base_url.unwrap_or_else(|| MINIMAX_DEFAULT_BASE_URL.to_string()),
            "minimax".to_string(),
        )),
        LlmProvider::Xiaomi => Box::new(rig_adapter::RigAdapter::openai_compatible(
            config.api_key.clone(),
            config.model.clone(),
            base_url.unwrap_or_else(|| XIAOMI_DEFAULT_BASE_URL.to_string()),
            "xiaomi".to_string(),
        )),
        LlmProvider::Mock => Box::new(MockClient::new(None)),
        LlmProvider::Custom(name) => {
            if name == "anthropic-compatible" {
                match &base_url {
                    Some(u) if !u.trim().is_empty() => {
                        Box::new(rig_adapter::RigAdapter::anthropic_with_base_url(
                            config.api_key.clone(),
                            config.model.clone(),
                            u.clone(),
                        ))
                    }
                    _ => Box::new(rig_adapter::RigAdapter::anthropic(
                        config.api_key.clone(),
                        config.model.clone(),
                    )),
                }
            } else {
                Box::new(rig_adapter::RigAdapter::openai_compatible(
                    config.api_key.clone(),
                    config.model.clone(),
                    base_url.unwrap_or_default(),
                    name.clone(),
                ))
            }
        }
    }
}

#[async_trait]
pub trait LlmClient: Send + Sync + std::fmt::Debug {
    async fn chat_completion(
        &self,
        messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>>;

    async fn chat_stream_events(
        &self,
        _messages: Vec<StarMessage>,
        _tools: Option<Vec<StarTool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
    {
        Err(LlmError::NotImplemented("chat_stream_events".to_string()))
    }

    fn get_model_info(&self) -> Option<ModelInfo>;
}
