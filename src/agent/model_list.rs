//! Model list - fetch available models from providers
//!
//! Responsibilities:
//! - List models from active provider
//! - Fallback models based on provider type
//! - Dynamic fetch from all configured providers
//!
//! # 两条路径，不要混
//!
//! 打开 `/model` 面板走的是**便宜路径**（`force = false`）：内存缓存 → 磁盘缓存
//! （`agent::model_cache`）→ 都没有才去拉一次活动 provider，并且带超时。它**不会**
//! 扇出到所有已配置 provider。
//!
//! 面板里的 `⟳` 显式刷新走**完整路径**（`force = true`）：跳过两级缓存，活动
//! provider + 其余每个已配置 provider 并发拉取，结果写回缓存。
//!
//! 这样区分是因为原来两条路径是同一条：每次开面板都全量扇出，活动 provider 那次
//! 请求还没有超时，慢的中转站能把面板卡住几十秒。

use crate::types::ModelInfo;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// 模型列表缓存
struct ModelCache {
    models: Vec<ModelInfo>,
    timestamp: Instant,
    provider_id: String,
}

static MODEL_CACHE: OnceLock<RwLock<Option<ModelCache>>> = OnceLock::new();

/// 缓存有效期（60秒）
const CACHE_TTL_SECS: u64 = 60;

/// 活动 provider `/models` 的默认超时。中转站的 `/models` 有时要好几秒，
/// 但绝不该让面板无限等 —— 拉不到就先给缓存/本地配置的列表。
const ACTIVE_FETCH_TIMEOUT_SECS: u64 = 8;

/// 其余 provider 并发拉取的超时（仅显式刷新时用到）。
const OTHER_FETCH_TIMEOUT_SECS: u64 = 3;

fn active_fetch_timeout() -> Duration {
    let secs = std::env::var("STAR_MODEL_LIST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(ACTIVE_FETCH_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// 一次列表查询的结果，附带"这份数据有多旧"。
pub struct ModelListResult {
    pub models: Vec<ModelInfo>,
    /// `None` = 本次刚从 API 拉的；`Some(n)` = 来自缓存，n 秒前拉的。
    pub cache_age_secs: Option<u64>,
}

/// 获取缓存的模型列表
fn get_cached_models(provider_id: &str) -> Option<(Vec<ModelInfo>, u64)> {
    let cache_lock = MODEL_CACHE.get_or_init(|| RwLock::new(None));
    let cache = cache_lock.read().ok()?;
    let cached = cache.as_ref()?;

    // 检查缓存是否有效（同一 provider 且未过期）
    let age = cached.timestamp.elapsed();
    if cached.provider_id == provider_id && age < Duration::from_secs(CACHE_TTL_SECS) {
        Some((cached.models.clone(), age.as_secs()))
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

/// List available models from all providers（兼容旧调用点：只要列表，走便宜路径）
pub async fn list_models(
    star_client: &crate::llm::client::StarClient,
) -> Result<Vec<ModelInfo>, String> {
    list_models_with_mode(star_client, false)
        .await
        .map(|r| r.models)
}

/// 列出模型。`force = true` 时跳过缓存并扇出到所有已配置 provider。
pub async fn list_models_with_mode(
    star_client: &crate::llm::client::StarClient,
    force: bool,
) -> Result<ModelListResult, String> {
    let mut models: Vec<ModelInfo> = Vec::new();

    // Log current client state for debugging
    crate::utils::logging::append_debug_log_line(&format!(
        "[ListModels] Client state: base_url={}, model={}, force={}",
        star_client.base_url, star_client.model, force
    ));

    // providers.json 只读一次：原来这里前后 load 了三四遍。
    let store = crate::core::config::provider_store::ProviderStore::new();
    let config = store.load().await.ok();
    let current_provider_id = config
        .as_ref()
        .and_then(|c| c.active_provider_id.clone())
        .unwrap_or_default();

    // 1. 便宜路径：先内存缓存，再磁盘缓存，命中就直接返回，一个网络请求都不发。
    if !force {
        if let Some((cached_models, age)) = get_cached_models(&current_provider_id) {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] Using in-memory cache for provider '{}', count={}",
                current_provider_id,
                cached_models.len()
            ));
            return Ok(ModelListResult {
                models: cached_models,
                cache_age_secs: Some(age),
            });
        }

        if let Some(hit) = crate::agent::model_cache::load(&current_provider_id) {
            // 顺手把内存缓存和上下文窗口缓存热起来，后面的状态栏/压缩逻辑都要用。
            crate::agent::model_catalog::update_context_window_cache(&hit.models);
            set_cached_models(current_provider_id.clone(), hit.models.clone());
            return Ok(ModelListResult {
                models: hit.models,
                cache_age_secs: Some(hit.age_secs),
            });
        }
    }

    // 2. 拉活动 provider（带超时）
    let mut active_pid_for_list: Option<String> = None;
    let list_result = {
        let active = config
            .as_ref()
            .and_then(|c| c.active_provider_id.as_ref().map(|pid| (c, pid)));

        if let Some((config, active_pid)) = active {
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
                fetch_with_timeout(&tmp).await
            } else {
                fetch_with_timeout(star_client).await
            }
        } else {
            fetch_with_timeout(star_client).await
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

    // 3. Load locally configured models AND (only on an explicit refresh)
    //    dynamically fetch from all configured providers
    if let Some(config) = config.as_ref() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] Loading all configured models from providers.json. Found {} providers.",
            config.providers.len()
        ));

        let mut fetch_futures = Vec::new();

        for (pid, provider) in &config.providers {
            // A. Explicitly configured models（本地读，永远做）
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

            // B. Dynamic Parallel Fetching —— 只在显式刷新时扇出，这是原来最慢的一步
            if !force {
                continue;
            }

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

                        let result = tokio::time::timeout(
                            Duration::from_secs(OTHER_FETCH_TIMEOUT_SECS),
                            client.list_models(),
                        )
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
    //    上下文窗口缓存也在这里灌：状态栏和压缩逻辑读的是它
    //    （`model_catalog::get_cached_context_window`），而唯一往里写的那条路径
    //    `model_catalog::list_models_for_client` 没有任何调用方。
    crate::agent::model_catalog::update_context_window_cache(&models);
    let provider_id = active_pid_for_list.unwrap_or_else(|| current_provider_id.clone());
    if !models.is_empty() {
        set_cached_models(provider_id.clone(), models.clone());
        crate::agent::model_cache::save(&provider_id, &models);
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] Cached {} models for provider '{}'",
            models.len(),
            provider_id
        ));
    }

    Ok(ModelListResult {
        models,
        cache_age_secs: None,
    })
}

/// 拉一次 `/models`，超时按超时算失败。
async fn fetch_with_timeout(
    client: &crate::llm::client::StarClient,
) -> Result<Vec<ModelInfo>, String> {
    match tokio::time::timeout(active_fetch_timeout(), client.list_models()).await {
        Ok(res) => res,
        Err(_) => Err(format!(
            "timed out after {}s listing models",
            active_fetch_timeout().as_secs()
        )),
    }
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
