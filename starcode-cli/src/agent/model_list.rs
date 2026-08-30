//! Model list - fetch available models from providers
//!
//! Responsibilities:
//! - List models from active provider
//! - Fallback models based on provider type
//! - Dynamic fetch from all configured providers

use crate::types::ModelInfo;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::sync::RwLock;

/// 模型列表缓存
struct ModelCache {
    models: Vec<ModelInfo>,
    timestamp: Instant,
    provider_id: String,
}

static MODEL_CACHE: OnceLock<RwLock<Option<ModelCache>>> = OnceLock::new();

/// 缓存有效期（60秒）
const CACHE_TTL_SECS: u64 = 60;

/// 获取缓存的模型列表
fn get_cached_models(provider_id: &str) -> Option<Vec<ModelInfo>> {
    let cache_lock = MODEL_CACHE.get_or_init(|| RwLock::new(None));
    let cache = cache_lock.read().ok()?;
    let cached = cache.as_ref()?;

    // 检查缓存是否有效（同一 provider 且未过期）
    if cached.provider_id == provider_id && cached.timestamp.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
        Some(cached.models.clone())
    } else {
        None
    }
}

/// 设置模型列表缓存
fn set_cached_models(provider_id: String, models: Vec<ModelInfo>) {
    let cache_lock = MODEL_CACHE.get_or_init(|| RwLock::new(None));
    if let Ok(mut cache) = cache_lock.write() {
        *cache = Some(ModelCache {
            models,
            timestamp: Instant::now(),
            provider_id,
        });
    }
}

/// List available models from all providers
pub async fn list_models(
    star_client: &crate::llm::client::StarClient,
) -> Result<Vec<ModelInfo>, String> {
    let mut models: Vec<ModelInfo> = Vec::new();
    let mut active_pid_for_list: Option<String> = None;

    // Log current client state for debugging
    crate::utils::logging::append_debug_log_line(&format!(
        "[ListModels] Client state: base_url={}, model={}",
        star_client.base_url, star_client.model
    ));

    // 1. Try to get from cache first
    let store = crate::core::config::provider_store::ProviderStore::new();
    let current_provider_id = if let Ok(config) = store.load().await {
        config.active_provider_id.clone().unwrap_or_default()
    } else {
        String::new()
    };

    if let Some(cached_models) = get_cached_models(&current_provider_id) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] Using cached models for provider '{}', count={}",
            current_provider_id, cached_models.len()
        ));
        return Ok(cached_models);
    }

    // 2. Try to fetch from API
    let list_result = {
        let store = crate::core::config::provider_store::ProviderStore::new();
        if let Ok(config) = store.load().await {
            if let Some(active_pid) = &config.active_provider_id {
                active_pid_for_list = Some(active_pid.clone());
                if let Some(p) = config.providers.get(active_pid) {
                    let base_url = p
                        .base_url
                        .clone()
                        .unwrap_or_else(|| star_client.base_url.clone());

                    // IMPORTANT: do not let empty api_key override env-based resolution.
                    let api_key = p.api_key.clone().filter(|k| !k.trim().is_empty());

                    let tmp = crate::llm::client::StarClient::new(
                        api_key.as_deref().unwrap_or("API_KEY_NOT_SET"),
                        None,
                        Some(base_url),
                        None,
                        Some(active_pid.clone()),
                    );
                    tmp.list_models().await
                } else {
                    star_client.list_models().await
                }
            } else {
                star_client.list_models().await
            }
        } else {
            star_client.list_models().await
        }
    };

    match list_result {
        Ok(mut remote_models) => {
            if let Some(pid) = &active_pid_for_list {
                for m in &mut remote_models {
                    m.provider = pid.clone();
                }
            }
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] API returned {} models",
                remote_models.len()
            ));
            models.extend(remote_models);
        }
        Err(e) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] Failed to list models from API: {}. Using fallback list...",
                e
            ));
        }
    }

    // 2. No hardcoded fallback models — API must provide the model list

    // 3. Load locally configured models AND dynamically fetch from all configured providers
    let store = crate::core::config::provider_store::ProviderStore::new();
    if let Ok(config) = store.load().await {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] Loading all configured models from providers.json. Found {} providers.",
            config.providers.len()
        ));

        let mut fetch_futures = Vec::new();

        for (pid, provider) in &config.providers {
            // A. Explicitly configured models
            if let Some(configured_models) = &provider.models {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Provider '{}' has {} models.",
                    pid,
                    configured_models.len()
                ));
                for (model_id, _) in configured_models {
                    if !models.iter().any(|m| m.id == *model_id) {
                        models.push(ModelInfo::new(model_id.clone(), pid.clone()));
                    }
                }
            } else {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Provider '{}' has no models configured.",
                    pid
                ));
            }

            // B. Dynamic Parallel Fetching
            let is_current = provider
                .base_url
                .as_ref()
                .map(|url| url.trim_end_matches('/') == star_client.base_url.trim_end_matches('/'))
                .unwrap_or(false);

            if !is_current {
                if let Some(base_url) = &provider.base_url {
                    let api_key = provider.api_key.clone().unwrap_or_default();
                    let base_url = base_url.clone();
                    let pid = pid.clone();

                    crate::utils::logging::append_debug_log_line(&format!(
                        "[ListModels] Spawning fetch task for provider '{}' ({})",
                        pid, base_url
                    ));

                    fetch_futures.push(tokio::spawn(async move {
                        let client = crate::llm::client::StarClient::new(
                            &api_key,
                            None,
                            Some(base_url),
                            None,
                            Some(pid.clone()),
                        );

                        // 3 second timeout for dynamic fetching
                        let result =
                            tokio::time::timeout(Duration::from_secs(3), client.list_models())
                                .await;

                        match result {
                            Ok(Ok(mut fetched_models)) => {
                                for m in &mut fetched_models {
                                    m.provider = pid.clone();
                                }
                                Some(fetched_models)
                            }
                            Ok(Err(e)) => {
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[ListModels] Failed to fetch from '{}': {}",
                                    pid, e
                                ));
                                None
                            }
                            Err(_) => {
                                crate::utils::logging::append_debug_log_line(&format!(
                                    "[ListModels] Timeout fetching from '{}'",
                                    pid
                                ));
                                None
                            }
                        }
                    }));
                }
            }
        }

        // Wait for all fetches
        if !fetch_futures.is_empty() {
            let results = futures::future::join_all(fetch_futures).await;
            for res in results {
                if let Ok(Some(fetched_models)) = res {
                    for m in fetched_models {
                        if !models.iter().any(|existing| existing.id == m.id) {
                            models.push(m);
                        }
                    }
                }
            }
        }
    }

    // 4. Ensure current model is in the list
    let current = star_client.model.clone();
    if !models.iter().any(|m| m.id == current) && !current.is_empty() {
        let provider = detect_provider_name(&star_client.base_url);
        models.insert(0, ModelInfo::new(current, provider));
    }

    // 5. Cache the result
    let provider_id = active_pid_for_list.unwrap_or_else(|| current_provider_id.clone());
    if !provider_id.is_empty() && !models.is_empty() {
        set_cached_models(provider_id, models.clone());
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] Cached {} models for provider '{}'",
            models.len(), current_provider_id
        ));
    }

    Ok(models)
}

/// Detect provider name from base URL
pub(crate) fn detect_provider_name(base_url: &str) -> String {
    let url = base_url.to_lowercase();
    if url.contains("openai.com") {
        "OpenAI".to_string()
    } else if url.contains("anthropic") || url.contains("claude") {
        "Anthropic".to_string()
    } else if url.contains("api.kimi.com/coding") {
        "Kimi Code".to_string()
    } else if url.contains("google") || url.contains("vertex") || url.contains("generativelanguage")
    {
        "Google".to_string()
    } else if url.contains("azure") || url.contains("microsoft") {
        "Azure".to_string()
    } else if url.contains("deepseek") {
        "DeepSeek".to_string()
    } else if url.contains("moonshot") || url.contains("kimi") {
        "Moonshot".to_string()
    } else if url.contains("dashscope") || url.contains("aliyun") {
        "Alibaba".to_string()
    } else if url.contains("siliconflow") {
        "SiliconFlow".to_string()
    } else if url.contains("zhipu") || url.contains("glm") {
        "Zhipu".to_string()
    } else if url.contains("doubao") || url.contains("volces") {
        "Doubao".to_string()
    } else if url.contains("localhost") || url.contains("127.0.0.1") || url.contains("lmstudio") {
        "LM Studio".to_string()
    } else if url.contains("ollama") {
        "Ollama".to_string()
    } else {
        "Local/Other".to_string()
    }
}

/// 清除模型列表缓存（切换 provider 时调用）
pub fn clear_model_cache() {
    if let Some(cache_lock) = MODEL_CACHE.get() {
        if let Ok(mut cache) = cache_lock.write() {
            *cache = None;
            crate::utils::logging::append_debug_log_line("[ListModels] Model cache cleared");
        }
    }
}
