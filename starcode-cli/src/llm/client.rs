use crate::llm::{create_client, LlmClient, LlmConfig, LlmEvent, LlmProvider};
use crate::llm::{
    API_KEY_NOT_SET, API_KEY_SOURCE_CONFIGURED, API_KEY_SOURCE_ENV_STAR,
    API_KEY_SOURCE_PROVIDER_ENV_PREFIX, API_KEY_SOURCE_UNKNOWN,
    ANTHROPIC_API_URL_PATTERN, ANTHROPIC_API_VERSION,
    AUTH_ERROR_INCORRECT_KEY, AUTH_ERROR_UNAUTHORIZED,
    BEARER_PREFIX, CONTENT_TYPE_JSON, DEFAULT_MAX_TOKENS, DEFAULT_TEMPERATURE,
    ENV_STAR_API_KEY, ENV_STAR_MAX_TOKENS, ENV_STAR_TEMPERATURE,
    HEADER_ANTHROPIC_VERSION, HEADER_AUTHORIZATION,
    HEADER_CONTENT_TYPE, HEADER_X_API_KEY, HTTP_STATUS_401, HTTP_STATUS_402,
    KIMI_CODE_URL_PATTERN, OPENAI_DEFAULT_BASE_URL,
    PAYMENT_ERROR_BALANCE, PAYMENT_ERROR_REQUIRED,
    PROVIDER_ENV_ID_ANTHROPIC, PROVIDER_ENV_ID_DEEPSEEK, PROVIDER_ENV_ID_XIAOMI,
    PROVIDER_NAME_OPENAI_COMPATIBLE,
};
pub use crate::types::StarResponse;
use crate::types::{StarMessage, StarTool};
use crate::utils::logging;
use async_stream::stream;
use futures::{Stream, StreamExt};
use reqwest;
use serde_json::json;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::RwLock;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Zeroize, ZeroizeOnDrop)]
pub struct StarClient {
    pub api_key: String,
    #[zeroize(skip)]
    pub base_url: String,
    #[zeroize(skip)]
    pub model: String,
    #[zeroize(skip)]
    pub is_openai_compatible: bool,
    #[zeroize(skip)]
    pub provider_id: Option<String>,
    #[zeroize(skip)]
    http_client: reqwest::Client,
    #[zeroize(skip)]
    pub default_max_tokens: u32,
    #[zeroize(skip)]
    pub temperature: f32,
    #[zeroize(skip)]
    inner: Arc<Box<dyn LlmClient>>,
    #[zeroize(skip)]
    provider: LlmProvider,
    #[zeroize(skip)]
    provider_env_id: Option<&'static str>,
    #[zeroize(skip)]
    api_key_source: Option<String>,
    #[zeroize(skip)]
    api_key_preview: Option<String>,
    #[zeroize(skip)]
    /// Runtime detection of reasoning support: None = not detected yet, Some(true/false) = detected
    detected_thinking_support: Arc<RwLock<Option<bool>>>,
}


fn infer_provider(
    model_name: &str,
    base_url: &str,
    is_openai_compatible: bool,
    provider_id: Option<&str>,
) -> LlmProvider {
	// 用户显式选择了 Anthropic 兼容第三方 → 直接按 Anthropic 协议走，保留自定义 base_url。
	// 与 openai-compatible 对称，避免把这网关误路由到 URL 推断（其 host 通常不含 "anthropic"）。
	if provider_id == Some("anthropic-compatible") {
		return LlmProvider::Custom("anthropic-compatible".to_string());
	}
	// 用户明确配置了 openai-compatible provider → 跳过所有 URL/模型名推断，
	// 直接使用 OpenAiCompatibleClient，保留用户配置的 base_url。
	// 避免第三方代理（如 api.183399.xyz 提供 deepseek 模型）被错误路由到
	// 官方 SDK（如 LlmProvider::DeepSeek），导致 base_url 丢失。
	if is_openai_compatible {
		return LlmProvider::Custom(PROVIDER_NAME_OPENAI_COMPATIBLE.to_string());
	}
	let url = base_url.to_ascii_lowercase();
	let model = model_name.to_ascii_lowercase();
	if url.contains("anthropic") { LlmProvider::Anthropic }
	else if url.contains("deepseek") || model.contains("deepseek") { LlmProvider::DeepSeek }
	else if url.contains("minimax") || model.contains("minimax") { LlmProvider::MiniMax }
	else if url.contains("xiaomimimo") || model.contains("mimo") || model.contains("milm") { LlmProvider::Xiaomi }
	else if url.contains("openai.com") { LlmProvider::OpenAi }
	else { LlmProvider::Custom(PROVIDER_NAME_OPENAI_COMPATIBLE.to_string()) }
}

fn provider_env_id(provider: &LlmProvider, _base_url: &str) -> Option<&'static str> {
	match provider {
		LlmProvider::DeepSeek => Some(PROVIDER_ENV_ID_DEEPSEEK),
		LlmProvider::Anthropic => Some(PROVIDER_ENV_ID_ANTHROPIC),
		LlmProvider::Xiaomi => Some(PROVIDER_ENV_ID_XIAOMI),
		_ => None,
	}
}

struct ResolvedApiKey {
	value: String,
	source: Option<String>,
	preview: Option<String>,
}
fn format_api_key_preview(api_key: &str) -> Option<String> {
    let normalized = api_key.trim();
    if normalized.is_empty() || normalized == API_KEY_NOT_SET {
        return None;
    }

    let prefix: String = normalized.chars().take(4).collect();
    Some(format!(
        "{}... (len={})",
        prefix,
        normalized.chars().count()
    ))
}

fn resolve_api_key_details(api_key: &str, provider_env_id: Option<&'static str>) -> ResolvedApiKey {
    if let Some((value, source)) =
        crate::core::config::providers::resolve_runtime_api_key_with_source(
            provider_env_id,
            Some(api_key.to_string()),
        )
    {
        return ResolvedApiKey {
            preview: format_api_key_preview(&value),
            source: Some(source),
            value,
        };
    }

    ResolvedApiKey {
        value: API_KEY_NOT_SET.to_string(),
        source: None,
        preview: None,
    }
}

fn format_api_key_source(source: Option<&str>) -> String {
    match source {
        Some(value) if value.starts_with(API_KEY_SOURCE_PROVIDER_ENV_PREFIX) => {
            let env_var = value.trim_start_matches(API_KEY_SOURCE_PROVIDER_ENV_PREFIX);
            format!("provider env `{}`", env_var)
        }
        Some(API_KEY_SOURCE_CONFIGURED) => "current runtime config".to_string(),
        Some(API_KEY_SOURCE_ENV_STAR) => "`STAR_API_KEY`".to_string(),
        Some(value) => format!("`{}`", value),
        None => API_KEY_SOURCE_UNKNOWN.to_string(),
    }
}

/// Last-resort API key resolution: if the stored key is a placeholder, try
/// `STAR_API_KEY` and known provider env vars. This covers the case where a
/// generic OpenAI-compatible provider (provider_env_id == None) cannot resolve
/// keys via provider-specific env vars.
fn resolve_effective_api_key(stored_key: &str) -> String {
    if stored_key != API_KEY_NOT_SET && !stored_key.trim().is_empty() {
        return stored_key.to_string();
    }
    if let Some(key) = try_env_api_key(ENV_STAR_API_KEY) {
        return key;
    }
    // Scan known provider env vars as last resort
    for env_var in &["OPENAI_API_KEY", "DEEPSEEK_API_KEY", "ANTHROPIC_API_KEY"] {
        if let Some(key) = try_env_api_key(env_var) {
            return key;
        }
    }
    stored_key.to_string()
}

fn try_env_api_key(env_var: &str) -> Option<String> {
    let key = std::env::var(env_var).ok()?;
    let trimmed = key.trim();
    if trimmed.is_empty() || crate::core::config::providers::is_placeholder_api_key(trimmed) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// As a final fallback when env vars don't have a usable key, scan the provider
/// store for a configured provider whose base_url matches ours and return its
/// API key. Falls back to any valid key if no URL match is found.
async fn load_api_key_from_store(base_url: &str) -> Option<String> {
    let store = crate::core::config::provider_store::ProviderStore::new();
    let config = store.load().await.ok()?;
    // 1) Exact base_url match first — most likely to be the right key
    for (_pid, settings) in &config.providers {
        if let Some(ref url) = settings.base_url {
            if url.trim() == base_url.trim() {
                let key = crate::core::config::providers::resolve_runtime_api_key(
                    Some(_pid),
                    settings.api_key.clone(),
                )?;
                if key != API_KEY_NOT_SET && !key.trim().is_empty() {
                    return Some(key);
                }
            }
        }
    }
    // 2) Last resort: any valid key from any provider
    for (_pid, settings) in &config.providers {
        let key = crate::core::config::providers::resolve_runtime_api_key(
            Some(_pid),
            settings.api_key.clone(),
        )?;
        if key != API_KEY_NOT_SET && !key.trim().is_empty() {
            return Some(key);
        }
    }
    None
}

fn missing_api_key_error(provider_env_id: Option<&'static str>) -> String {
    if let Some(pid) = provider_env_id {
        if crate::core::config::providers::resolve_api_key_from_env(pid).is_some() {
            let env_hint = crate::core::config::providers::api_key_env_hint(pid)
                .unwrap_or_else(|| "ENV".to_string());
            return format!(
                "✦ API key not set in config. Found key in environment variable(s) {}, but it's not being used. Please run `/provider select {}` to apply it.",
                env_hint, pid
            );
        }
    }

    // Generic / OpenAI-compatible provider: check STAR_API_KEY as last resort
    if let Ok(key) = std::env::var(ENV_STAR_API_KEY) {
        if !key.trim().is_empty()
            && !crate::core::config::providers::is_placeholder_api_key(&key)
        {
            return format!(
                "✦ API key not set in config, but `STAR_API_KEY` is present. If this key is for the current provider, set it via Ctrl+P → Providers, or unset `STAR_API_KEY` to use provider-specific settings."
            );
        }
    }

    "✦ No API key configured. Open the command palette with Ctrl+P, then choose Providers before proceeding.".to_string()
}

fn authentication_error_message(
    base_url: &str,
    provider_env_id: Option<&'static str>,
    model: &str,
    api_key_source: Option<&str>,
    api_key_preview: Option<&str>,
    original_error: &str,
) -> String {
    let lower_base_url = base_url.to_ascii_lowercase();
    let provider_label = provider_env_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{} ({})", PROVIDER_NAME_OPENAI_COMPATIBLE, base_url));
    let debug_context = format!(
        "Provider: {}\nModel: {}\nKey Source: {}\nKey Preview: {}",
        provider_label,
        if model.trim().is_empty() { "-" } else { model },
        format_api_key_source(api_key_source),
        api_key_preview.unwrap_or("-"),
    );

    if lower_base_url.contains("deepseek") || matches!(provider_env_id, Some(PROVIDER_ENV_ID_DEEPSEEK)) {
        return format!(
            "✦ DeepSeek 认证失败 (401)：当前生效的 API Key 无效、已过期，或命中了错误的 Key 来源。请先检查 `DEEPSEEK_API_KEY`、Provider 已保存的 Key，以及 `STAR_API_KEY` 是否混用；也可以运行 `/provider doctor` 查看当前运行时命中的来源。\n{}\nOriginal Error: {}",
            debug_context, original_error
        );
    }

    format!(
        "✦ API Key Error (401): The configured API key is invalid or expired. Open the command palette with Ctrl+P, then choose Providers.\n{}\nOriginal Error: {}",
        debug_context, original_error
    )
}

impl Clone for StarClient {
    fn clone(&self) -> Self {
        Self {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            is_openai_compatible: self.is_openai_compatible,
            http_client: self.http_client.clone(),
            default_max_tokens: self.default_max_tokens,
            temperature: self.temperature,
            inner: self.inner.clone(),
            provider: self.provider.clone(),
            provider_env_id: self.provider_env_id,
            api_key_source: self.api_key_source.clone(),
            api_key_preview: self.api_key_preview.clone(),
            detected_thinking_support: self.detected_thinking_support.clone(),
            provider_id: self.provider_id.clone(),
        }
    }
}

impl StarClient {
    pub fn new(
        api_key: &str,
        model: Option<String>,
        base_url: Option<String>,
        is_openai_compatible: Option<bool>,
        provider_id: Option<String>,
    ) -> Self {
        let default_max_tokens = std::env::var(ENV_STAR_MAX_TOKENS)
            .ok()
            .and_then(|val| val.parse().ok())
            .unwrap_or(DEFAULT_MAX_TOKENS);
        let temperature = std::env::var(ENV_STAR_TEMPERATURE)
            .ok()
            .and_then(|val| val.parse().ok())
            .unwrap_or(DEFAULT_TEMPERATURE);

        // Determine provider - model must be provided by caller
        let model_name = model.clone().unwrap_or_default();
        let url_str = base_url
            .clone()
            .unwrap_or_else(|| OPENAI_DEFAULT_BASE_URL.to_string());

        let provider = infer_provider(
            &model_name,
            &url_str,
            is_openai_compatible.unwrap_or(false),
            provider_id.as_deref(),
        );
        let provider_env_id = provider_env_id(&provider, &url_str);
        let resolved_api_key = resolve_api_key_details(api_key, provider_env_id);

        let config = LlmConfig {
            provider: provider.clone(),
            api_key: resolved_api_key.value.clone(),
            base_url: Some(url_str.clone()),
            model: model_name.clone(),
        };

        let client = create_client(config);

        let http_client =
            super::build_http_client(super::default_timeout(), super::default_connect_timeout());

        Self {
            api_key: resolved_api_key.value,
            base_url: url_str,
            model: model_name,
            is_openai_compatible: is_openai_compatible.unwrap_or(true),
            provider_id,
            http_client,
            default_max_tokens,
            temperature,
            inner: Arc::new(client),
            provider,
            provider_env_id,
            api_key_source: resolved_api_key.source,
            api_key_preview: resolved_api_key.preview,
            detected_thinking_support: Arc::new(RwLock::new(None)),
        }
    }

    /// Check if the model supports thinking/reasoning.
    /// Returns runtime detection result if available, otherwise falls back to model name detection.
    pub fn supports_thinking(&self) -> bool {
        // First check runtime detection
        if let Ok(detected) = self.detected_thinking_support.read() {
            if let Some(supports) = *detected {
                return supports;
            }
        }
        // Fallback to model name detection
        crate::core::config::models::is_thinking_model(&self.model)
    }

    /// Update the runtime detection result for thinking support.
    fn update_thinking_detection(&self, has_reasoning: bool) {
        if let Ok(mut detected) = self.detected_thinking_support.write() {
            *detected = Some(has_reasoning);
        }
    }

    /// Reset the thinking detection (e.g., when model changes)
    fn reset_thinking_detection(&self) {
        if let Ok(mut detected) = self.detected_thinking_support.write() {
            *detected = None;
        }
    }

    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
        self.reset_thinking_detection();
        self.recreate_inner();
    }

    pub fn set_api_key(&mut self, api_key: &str) {
        self.api_key = api_key.to_string();
        self.recreate_inner();
    }

    pub fn set_base_url(&mut self, base_url: &str) {
        self.base_url = base_url.to_string();
        self.recreate_inner();
    }

    pub fn switch_provider(
        &mut self,
        model: &str,
        base_url: &str,
        api_key: &str,
        is_openai_compatible: Option<bool>,
        provider_id: Option<String>,
    ) {
        self.model = model.to_string();
        self.base_url = base_url.to_string();
        self.api_key = api_key.to_string();
        if let Some(flag) = is_openai_compatible {
            self.is_openai_compatible = flag;
        }
        if provider_id.is_some() {
            self.provider_id = provider_id;
        }
        self.reset_thinking_detection();
        self.recreate_inner();
    }

    pub fn recreate_inner(&mut self) {
        let provider = infer_provider(
            &self.model,
            &self.base_url,
            self.is_openai_compatible,
            self.provider_id.as_deref(),
        );
        let provider_env_id = provider_env_id(&provider, &self.base_url);
        let provider_requires_key = provider_env_id
            .and_then(|pid| crate::core::config::providers::get_provider_by_id(pid))
            .map(|p| p.requires_api_key)
            .unwrap_or(true);
        let resolved_api_key = if self.api_key.trim().is_empty() {
            // Explicitly cleared key: skip env-var fallback so nothing is injected
            ResolvedApiKey {
                value: API_KEY_NOT_SET.to_string(),
                source: None,
                preview: None,
            }
        } else if !provider_requires_key
            && crate::core::config::providers::is_placeholder_api_key(&self.api_key)
        {
            // Provider does not require a key and no real key was explicitly set –
            // do NOT inject STAR_API_KEY or other env keys meant for other providers.
            ResolvedApiKey {
                value: API_KEY_NOT_SET.to_string(),
                source: None,
                preview: None,
            }
        } else {
            let r = resolve_api_key_details(&self.api_key, provider_env_id);
            // When provider_env_id is None (generic OpenAI-compatible), the
            // provider-specific env-var lookup is skipped. If resolution produced
            // a placeholder, scan known env vars as a last resort.
            if r.value == API_KEY_NOT_SET && provider_env_id.is_none() {
                let fallback = resolve_effective_api_key(&self.api_key);
                if fallback != API_KEY_NOT_SET {
                    self.api_key = fallback.clone();
                    ResolvedApiKey {
                        value: fallback,
                        source: Some("fallback".to_string()),
                        preview: format_api_key_preview(&self.api_key),
                    }
                } else {
                    r
                }
            } else {
                if r.value != self.api_key {
                    self.api_key = r.value.clone();
                }
                r
            }
        };
        self.provider = provider.clone();
        self.provider_env_id = provider_env_id;
        self.api_key_source = resolved_api_key.source.clone();
        self.api_key_preview = resolved_api_key.preview.clone();

        let config = LlmConfig {
            provider,
            api_key: resolved_api_key.value,
            base_url: Some(self.base_url.clone()),
            model: self.model.clone(),
        };

        let client = create_client(config);
        self.inner = Arc::new(client);
    }

    pub fn get_current_model(&self) -> &str {
        &self.model
    }

    pub fn is_kimi_code_provider(&self) -> bool {
        self.base_url.contains("api.kimi.com/coding")
    }

    pub async fn chat(
        &self,
        mut messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
        _model: Option<String>,
        _search_options: Option<()>,
    ) -> Result<StarResponse, Box<dyn std::error::Error + Send + Sync>> {
        let pipeline = super::message_pipeline::pipeline_for(
            self.provider_env_id,
            &self.model,
            self.supports_thinking(),
        );
        pipeline.run(&mut messages);

        let provider_requires_key = self
            .provider_env_id
            .and_then(|pid| crate::core::config::providers::get_provider_by_id(pid))
            .map(|p| p.requires_api_key)
            .unwrap_or(true);
        // Last-resort fallback: when the stored key is a placeholder and the
        // provider is generic (no provider_env_id), try STAR_API_KEY / known
        // provider env vars, then the provider store as a final fallback.
        let effective_api_key = resolve_effective_api_key(&self.api_key);
        let effective_api_key = if effective_api_key == API_KEY_NOT_SET {
            load_api_key_from_store(&self.base_url).await
                .unwrap_or(effective_api_key)
        } else {
            effective_api_key
        };
        if provider_requires_key
            && (effective_api_key == API_KEY_NOT_SET || effective_api_key.trim().is_empty())
        {
            return Err(missing_api_key_error(self.provider_env_id).into());
        }

        match self.inner.chat_completion(messages, tools).await {
            Ok(resp) => {
                // Runtime detection: check if response contains reasoning_content
                let has_reasoning = resp
                    .choices
                    .iter()
                    .any(|c| c.message.reasoning_content.is_some());
                self.update_thinking_detection(has_reasoning);
                Ok(resp)
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains(HTTP_STATUS_401)
                    || err_str.contains(AUTH_ERROR_UNAUTHORIZED)
                    || err_str.contains(AUTH_ERROR_INCORRECT_KEY)
                {
                    return Err(authentication_error_message(
                        &self.base_url,
                        self.provider_env_id,
                        &self.model,
                        self.api_key_source.as_deref(),
                        self.api_key_preview.as_deref(),
                        &err_str,
                    )
                    .into());
                }
                if err_str.contains(HTTP_STATUS_402)
                    || err_str.contains(PAYMENT_ERROR_REQUIRED)
                    || err_str.contains(PAYMENT_ERROR_BALANCE)
                {
                    return Err(format!("✦ Payment Error (402): Insufficient balance or credit limit exceeded. Please check your provider account (e.g., OpenAI/DeepSeek billing).\nOriginal Error: {}", err_str).into());
                }
                Err(e)
            }
        }
    }

    pub async fn chat_stream(
        &self,
        mut messages: Vec<StarMessage>,
        tools: Option<Vec<StarTool>>,
        _model: Option<String>,
        _search_options: Option<()>,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>,
                    > + Send
                    + Sync,
            >,
        >,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let pipeline = super::message_pipeline::pipeline_for(
            self.provider_env_id,
            &self.model,
            self.supports_thinking(),
        );
        pipeline.run(&mut messages);

        let provider_requires_key = self
            .provider_env_id
            .and_then(|pid| crate::core::config::providers::get_provider_by_id(pid))
            .map(|p| p.requires_api_key)
            .unwrap_or(true);
        let effective_api_key = resolve_effective_api_key(&self.api_key);
        let effective_api_key = if effective_api_key == API_KEY_NOT_SET {
            load_api_key_from_store(&self.base_url).await
                .unwrap_or(effective_api_key)
        } else {
            effective_api_key
        };
        if provider_requires_key
            && (effective_api_key == API_KEY_NOT_SET || effective_api_key.trim().is_empty())
        {
            return Err(Box::new(std::io::Error::other(missing_api_key_error(
                self.provider_env_id,
            ))));
        }

        let inner_client = self.inner.clone();
        let thinking_detection = self.detected_thinking_support.clone();
        let verbose_debug_logging = crate::utils::logging::is_verbose_debug_logging_enabled();

        if verbose_debug_logging {
            crate::utils::logging::append_debug_log_line(
                "[DEBUG] StarClient: Calling chat_stream_events",
            );
        }
        // Clone before consuming in chat_stream_events — needed for
        // the non-streaming fallback path.
        let fallback_messages = messages.clone();
        let fallback_tools = tools.clone();

        let event_stream = match inner_client.chat_stream_events(messages, tools).await {
            Ok(s) => {
                if verbose_debug_logging {
                    crate::utils::logging::append_debug_log_line(
                        "[DEBUG] StarClient: chat_stream_events returned OK",
                    );
                }
                s
            }
            Err(e) => {
                if verbose_debug_logging {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[DEBUG] StarClient: chat_stream_events error: {}",
                        e
                    ));
                }
                let err_str = e.to_string();
                if err_str.contains(HTTP_STATUS_401)
                    || err_str.contains(AUTH_ERROR_UNAUTHORIZED)
                    || err_str.contains(AUTH_ERROR_INCORRECT_KEY)
                {
                    return Err(Box::new(std::io::Error::other(
                        authentication_error_message(
                            &self.base_url,
                            self.provider_env_id,
                            &self.model,
                            self.api_key_source.as_deref(),
                            self.api_key_preview.as_deref(),
                            &err_str,
                        ),
                    )));
                }
                if err_str.contains(HTTP_STATUS_402)
                    || err_str.contains(PAYMENT_ERROR_REQUIRED)
                    || err_str.contains(PAYMENT_ERROR_BALANCE)
                {
                    return Err(Box::new(std::io::Error::other(
                        format!("✦ Payment Error (402): Insufficient balance or credit limit exceeded. Please check your provider account (e.g., OpenAI/DeepSeek billing).\nOriginal Error: {}", err_str)
                    )));
                }
                // Non-streaming fallback: mirroring claude-code's pattern,
                // when the streaming endpoint fails with a transient error
                // (timeout, connection drop, gateway 504) fall back to a
                // synchronous completion call so the user gets a result
                // instead of an error.
                if !err_str.contains(HTTP_STATUS_401) {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[FALLBACK] chat_stream_events failed ({}), attempting non-streaming fallback",
                        &err_str.chars().take(200).collect::<String>(),
                    ));
                    match inner_client.chat_completion(fallback_messages, fallback_tools).await {
                        Ok(response) => {
                            crate::utils::logging::append_debug_log_line(
                                "[FALLBACK] non-streaming fallback succeeded",
                            );
                            let events: Vec<Result<_, Box<dyn std::error::Error + Send + Sync>>> =
                                crate::llm::client::StarClient::build_events_from_response(response);
                            let stream = Box::pin(stream! {
                                for event in events {
                                    yield event;
                                }
                            });
                            return Ok(stream);
                        }
                        Err(fallback_err) => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[FALLBACK] non-streaming fallback also failed: {}",
                                fallback_err,
                            ));
                        }
                    }
                }
                return Err(Box::new(e));
            }
        };

        // Adapt LlmEvent stream to legacy serde_json::Value stream
        let stream = Box::pin(stream! {
            let mut event_stream = event_stream;
            let mut tool_call_index: u64 = 0;

            while let Some(event_result) = event_stream.next().await {
                match event_result {
                    Ok(event) => {
                        match event {
                            LlmEvent::Trace { event, payload } => {
                                let json = json!({
                                    "star_trace": {
                                        "event": event,
                                        "payload": payload,
                                    }
                                });
                                yield Ok(json);
                            },
                            LlmEvent::TextChunk(text) => {
                                let json = json!({
                                    "choices": [{
                                        "delta": { "content": text }
                                    }]
                                });
                                yield Ok(json);
                            },
                            LlmEvent::ReasoningChunk(text) => {
                                // Runtime detection: mark as supporting reasoning
                                if let Ok(mut detected) = thinking_detection.write() {
                                    *detected = Some(true);
                                }
                                let json = json!({
                                    "choices": [{
                                        "delta": { "reasoning_content": text }
                                    }]
                                });
                                yield Ok(json);
                            },
                            LlmEvent::ToolCall(tool_call) => {
                                let idx = tool_call_index;
                                tool_call_index += 1;
                                let json = json!({
                                    "choices": [{
                                        "delta": {
                                            "tool_calls": [{
                                                "index": idx,
                                                "id": tool_call.id,
                                                "type": "function",
                                                "function": {
                                                    "name": tool_call.function.name,
                                                    "arguments": tool_call.function.arguments
                                                }
                                            }]
                                        }
                                    }]
                                });
                                yield Ok(json);
                            },
                            LlmEvent::Finish(reason) => {
                                let json = json!({
                                    "choices": [{
                                        "finish_reason": reason
                                    }]
                                });
                                yield Ok(json);
                            },
                            LlmEvent::UsageUpdate(usage) => {
                                let json = json!({
                                    "usage": {
                                        "prompt_tokens": usage.prompt_tokens,
                                        "completion_tokens": usage.completion_tokens,
                                        "total_tokens": usage.total_tokens,
                                    }
                                });
                                yield Ok(json);
                            },
                            LlmEvent::Error(msg) => {
                                yield Err(Box::new(std::io::Error::other(msg)) as Box<dyn std::error::Error + Send + Sync>);
                            }
                        }
                    },
                    Err(e) => {
                        yield Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                    }
                }
            }
        });

        Ok(stream)
    }

    /// Convert a non-streaming StarResponse into a sequence of streamable
    /// JSON events, mirroring the format produced by the SSE adapter.
    /// Used by the non-streaming fallback in chat_stream.
    pub fn build_events_from_response(
        response: StarResponse,
    ) -> Vec<Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>> {
        let mut events: Vec<Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>> = Vec::new();
        for choice in &response.choices {
            if let Some(reasoning) = &choice.message.reasoning_content {
                if !reasoning.is_empty() {
                    events.push(Ok(json!({
                        "choices": [{
                            "delta": { "reasoning_content": reasoning }
                        }]
                    })));
                }
            }
            if let Some(content) = &choice.message.content {
                if !content.is_empty() {
                    events.push(Ok(json!({
                        "choices": [{
                            "delta": { "content": content }
                        }]
                    })));
                }
            }
            if let Some(tool_calls) = &choice.message.tool_calls {
                for (i, tc) in tool_calls.iter().enumerate() {
                    events.push(Ok(json!({
                        "choices": [{
                            "delta": {
                                "tool_calls": [{
                                    "index": i,
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.function.name,
                                        "arguments": tc.function.arguments,
                                    }
                                }]
                            }
                        }]
                    })));
                }
            }
            events.push(Ok(json!({
                "choices": [{
                    "finish_reason": choice.finish_reason
                }]
            })));
        }
        if let Some(usage) = &response.usage {
            events.push(Ok(json!({
                "usage": {
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                }
            })));
        }
        events
    }

    pub async fn chat_completion_simple(
        &self,
        prompt: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let message = StarMessage::user(prompt.to_string());

        let response = self.chat(vec![message], None, None, None).await?;

        if let Some(choice) = response.choices.first() {
            if let Some(content) = &choice.message.content {
                return Ok(content.clone());
            }
        }

        Err("No content in response".into())
    }

    pub async fn list_models(&self) -> Result<Vec<crate::types::ModelInfo>, String> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));

        // Detect provider from base URL
        let provider = self.detect_provider();

        logging::append_agent_log_line(&format!(
            "➡️  GET {} [Provider: {}, KeyPrefix: {}...]",
            url,
            provider,
            self.api_key.chars().take(8).collect::<String>()
        ));

        let mut req = self
            .http_client
            .get(&url)
            .header(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON);

        if !self.api_key.is_empty() && self.api_key != API_KEY_NOT_SET {
            if self.base_url.contains(ANTHROPIC_API_URL_PATTERN) {
                req = req
                    .header(HEADER_X_API_KEY, &self.api_key)
                    .header(HEADER_ANTHROPIC_VERSION, ANTHROPIC_API_VERSION);
            } else {
                req = req.header(HEADER_AUTHORIZATION, format!("{}{}", BEARER_PREFIX, self.api_key));
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("request error: {}", e))?;
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(format!(
                "status {} body {} [BaseURL: {}, KeyPrefix: {}...]",
                status,
                body_text,
                self.base_url,
                self.api_key.chars().take(8).collect::<String>()
            ));
        }

        let json: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| format!("parse error {}: {}", body_text, e))?;
        let models = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| "no data array".to_string())?;

        let model_infos: Vec<crate::types::ModelInfo> = models
            .iter()
            .filter_map(|m| {
                let id = m.get("id")?.as_str()?;
                let display_name = m.get("name").and_then(|n| n.as_str());

                // 尝试从 API 响应中提取上下文窗口大小
                // Anthropic: max_input_tokens
                // OpenAI-compatible 有 context_window 的提供商
                let context_window = m
                    .get("max_input_tokens")
                    .or_else(|| m.get("context_window"))
                    .or_else(|| m.get("contextWindow"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);

                // 自动判断模型是否支持 thinking/reasoning
                let supports_thinking = crate::core::config::models::is_thinking_model(id);

                let mut model_info = crate::types::ModelInfo::new(id, &provider);
                if let Some(name) = display_name {
                    model_info = model_info.with_display_name(name);
                }
                if let Some(ctx) = context_window {
                    model_info = model_info.with_context_window(ctx);
                }
                model_info = model_info.with_supports_thinking(supports_thinking);
                Some(model_info)
            })
            .collect();

        if model_infos.is_empty() {
            Err("data array empty".to_string())
        } else {
            Ok(model_infos)
        }
    }

    /// Detect provider name from base URL
    fn detect_provider(&self) -> String {
        match &self.provider {
            LlmProvider::OpenAi => "OpenAI".to_string(),
            LlmProvider::Anthropic => "Anthropic".to_string(),
            LlmProvider::DeepSeek => "DeepSeek".to_string(),
            LlmProvider::MiniMax => "MiniMax".to_string(),
            LlmProvider::Xiaomi => "Xiaomi".to_string(),
            LlmProvider::Mock => "Mock".to_string(),
            LlmProvider::Custom(name) => name.clone(),
        }
    }
}

 