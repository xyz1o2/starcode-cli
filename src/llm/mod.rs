pub mod client;
pub mod error_kind;
pub mod message_pipeline;
mod mock;
mod openai_compatible;
pub mod prompt_cache_ext;
pub mod providers;
pub mod rig_adapter;
pub mod thinking;

pub use crate::core::config::models::ModelInfo;
pub use error_kind::{diagnose, mentions_status_code, retry_after_secs, LlmErrorKind};
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
pub const PROVIDER_ENV_ID_OPENCODE: &str = "opencode";

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

/// 用户显式配置了可用的 base_url 时，请求必须发往那里。
///
/// # 为什么 vendor 分支不能丢 base_url
///
/// `LlmProvider` 的变体按厂商命名，但它实际承载的是**两个独立决定**：用哪套
/// wire 协议，以及发到哪里。`infer_provider`（`client.rs`）会用**模型名**关键词
/// 命中 `DeepSeek` / `OpenAi` 这类官方分支 —— 而官方分支一旦不传 base_url，
/// rig 就回落到厂商硬编码的官方地址。
///
/// 后果是：一个把 DeepSeek 模型挂在自建网关上的用户，只要模型名里带
/// "deepseek"，请求就会静默飞到 `api.deepseek.com`，网关配置和 key 一起失效，
/// 表现为 401。`is_aggregator_provider` 的白名单挡不住这种情况——它枚举的是
/// 网关 id，新增一个网关就重新失效，而它要挡的根本不是某个网关，是**这条规则
/// 本身**。
///
/// 正确的不变量：**模型名只决定协议，base_url 决定目的地。** 任何分支都不得
/// 用前者覆盖后者。
///
/// `BASE_URL_NOT_SET` 是占位符，不是地址 —— 把它拼进 URL 会得到
/// `BASE_URL_NOT_SET/v1`，比回落到官方地址更糟，必须在这里就排除掉。
fn explicit_base_url(base_url: Option<&String>) -> Option<String> {
    base_url
        .map(|u| u.trim())
        .filter(|u| !u.is_empty() && *u != crate::core::config::providers::PLACEHOLDER_BASE_URL)
        .map(str::to_string)
}

pub fn create_client(config: LlmConfig) -> Box<dyn LlmClient> {
    let base_url = config.base_url.clone();
    let explicit_url = explicit_base_url(base_url.as_ref());

    // rig never omits the `Authorization` header: passing a placeholder makes it
    // emit `Bearer API_KEY_NOT_SET`, which upstream rejects as an invalid key.
    // OpenCode Zen's free models (`space-bunny-free`) work with no key at all and
    // accept an empty Bearer header, so demote the placeholder to an empty string
    // before the client is actually constructed.
    let api_key = if config.api_key == API_KEY_NOT_SET
        && base_url
            .as_deref()
            .is_some_and(|u| crate::core::config::providers::is_opencode_zen(None, u))
    {
        String::new()
    } else {
        config.api_key.clone()
    };

    match &config.provider {
        LlmProvider::OpenAi => match explicit_url {
            Some(u) => Box::new(rig_adapter::RigAdapter::openai_with_base_url(
                api_key.clone(),
                config.model.clone(),
                u,
            )),
            None => Box::new(rig_adapter::RigAdapter::openai(
                api_key.clone(),
                config.model.clone(),
            )),
        },
        LlmProvider::Anthropic => {
            // Respect an explicit base_url: OpenCode Zen serves `claude-*` from its
            // own `/messages` endpoint, so ignoring base_url would send the request
            // to api.anthropic.com and fail auth. Falls back to rig's default.
            match explicit_url {
                Some(u) => Box::new(rig_adapter::RigAdapter::anthropic_with_base_url(
                    api_key.clone(),
                    config.model.clone(),
                    u,
                )),
                None => Box::new(rig_adapter::RigAdapter::anthropic(
                    api_key.clone(),
                    config.model.clone(),
                )),
            }
        }
        LlmProvider::DeepSeek => match explicit_url {
            Some(u) => Box::new(rig_adapter::RigAdapter::deepseek_with_base_url(
                api_key.clone(),
                config.model.clone(),
                u,
            )),
            None => Box::new(rig_adapter::RigAdapter::deepseek(
                api_key.clone(),
                config.model.clone(),
            )),
        },
        LlmProvider::MiniMax => Box::new(rig_adapter::RigAdapter::openai_compatible(
            api_key.clone(),
            config.model.clone(),
            explicit_url
                .clone()
                .unwrap_or_else(|| MINIMAX_DEFAULT_BASE_URL.to_string()),
            "minimax".to_string(),
        )),
        LlmProvider::Xiaomi => Box::new(rig_adapter::RigAdapter::openai_compatible(
            api_key.clone(),
            config.model.clone(),
            explicit_url
                .clone()
                .unwrap_or_else(|| XIAOMI_DEFAULT_BASE_URL.to_string()),
            "xiaomi".to_string(),
        )),
        LlmProvider::Mock => Box::new(MockClient::new(None)),
        LlmProvider::Custom(name) => {
            if name == "anthropic-compatible" {
                match explicit_url.clone() {
                    Some(u) => Box::new(rig_adapter::RigAdapter::anthropic_with_base_url(
                        api_key.clone(),
                        config.model.clone(),
                        u,
                    )),
                    None => Box::new(rig_adapter::RigAdapter::anthropic(
                        api_key.clone(),
                        config.model.clone(),
                    )),
                }
            } else {
                Box::new(rig_adapter::RigAdapter::openai_compatible(
                    api_key.clone(),
                    config.model.clone(),
                    // 注意这里**不能**换成 explicit_url：OpenAiCompatible 的
                    // base_url 是 String 而非 Option，没有地址时 rig 会拿到空串，
                    // 构建出一个指向 "" 的客户端。保持原有行为，缺失地址由
                    // client.rs 的 PLACEHOLDER 告警负责报出来。
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

#[cfg(test)]
mod create_client_tests {
    use super::*;

    fn cfg(provider: LlmProvider, base_url: Option<&str>) -> LlmConfig {
        LlmConfig {
            provider,
            api_key: "test-key".to_string(),
            base_url: base_url.map(str::to_string),
            model: "deepseek-v4.1-flash:free".to_string(),
        }
    }

    /// Debug 输出里带 `RigAdapter` 的全部字段（含 base_url），用它观察接线结果，
    /// 免得为了测试给 `LlmClient` 再加一个只为断言存在的 downcast 方法。
    fn debug_of(client: &Box<dyn LlmClient>) -> String {
        format!("{:?}", client)
    }

    #[test]
    fn explicit_base_url_rejects_missing_empty_and_placeholder() {
        assert_eq!(explicit_base_url(None), None);
        assert_eq!(explicit_base_url(Some(&String::new())), None);
        assert_eq!(explicit_base_url(Some(&"   ".to_string())), None);
        // 占位符不是地址：放行它会拼出 `BASE_URL_NOT_SET/v1` 这种请求 URL。
        assert_eq!(
            explicit_base_url(Some(
                &crate::core::config::providers::PLACEHOLDER_BASE_URL.to_string()
            )),
            None
        );
    }

    #[test]
    fn explicit_base_url_accepts_and_trims_real_urls() {
        assert_eq!(
            explicit_base_url(Some(&"  https://tokenharbor.ai/v1  ".to_string())),
            Some("https://tokenharbor.ai/v1".to_string())
        );
    }

    /// 回归：自建网关上挂着别家厂商命名的模型时，请求必须留在网关上。
    ///
    /// 这是 401 的根因——`deepseek-v4.1-flash:free` 挂在 tokenharbor.ai，
    /// 但 DeepSeek 分支曾丢弃 base_url，请求飞到 api.deepseek.com，
    /// 网关的 key 自然不被承认。
    #[test]
    fn deepseek_branch_keeps_a_configured_gateway_base_url() {
        let client = create_client(cfg(
            LlmProvider::DeepSeek,
            Some("https://tokenharbor.ai/v1"),
        ));
        assert!(
            debug_of(&client).contains("https://tokenharbor.ai/v1"),
            "DeepSeek 分支吞掉了显式配置的 base_url: {}",
            debug_of(&client)
        );
    }

    /// 同样的静默丢失也存在于 OpenAi 分支，一并钉住。
    #[test]
    fn openai_branch_keeps_a_configured_gateway_base_url() {
        let client = create_client(cfg(LlmProvider::OpenAi, Some("https://gateway.example/v1")));
        assert!(
            debug_of(&client).contains("https://gateway.example/v1"),
            "OpenAi 分支吞掉了显式配置的 base_url: {}",
            debug_of(&client)
        );
    }

    /// 占位符必须退回 rig 的官方默认地址，而不是把占位符当地址发出去。
    #[test]
    fn placeholder_base_url_falls_back_to_the_vendor_default() {
        let client = create_client(cfg(
            LlmProvider::DeepSeek,
            Some(crate::core::config::providers::PLACEHOLDER_BASE_URL),
        ));
        let debug = debug_of(&client);
        assert!(
            !debug.contains("BASE_URL_NOT_SET"),
            "占位符被当成了真实地址: {}",
            debug
        );
    }

    /// 没有配置地址时行为不变：交给 rig 用厂商官方地址。
    #[test]
    fn absent_base_url_still_uses_the_vendor_default() {
        let client = create_client(cfg(LlmProvider::DeepSeek, None));
        assert!(
            debug_of(&client).contains("base_url: None"),
            "无地址时不该构造出自定义端点: {}",
            debug_of(&client)
        );
    }
}
