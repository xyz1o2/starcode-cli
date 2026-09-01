use crate::core::config::provider_store::ProviderStore;
use crate::llm::client::StarClient;
use crate::types::ModelInfo;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// 模型列表内存缓存有效期 — 避免每次切换模型都全量网络拉取
const MODEL_LIST_CACHE_TTL: Duration = Duration::from_secs(60);
/// 当前提供商 /models 拉取的短超时 — 不能复用 LLM 客户端的 120s 超时，
/// 否则第三方中转站接口慢时会阻塞模型切换几十秒
const CURRENT_PROVIDER_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

static MODEL_LIST_CACHE: RwLock<Option<(Instant, Vec<ModelInfo>)>> = RwLock::new(None);

/// 清空模型列表缓存（下次 ListModels 强制刷新）
pub fn invalidate_model_list_cache() {
    if let Ok(mut guard) = MODEL_LIST_CACHE.write() {
        *guard = None;
    }
}

fn cached_model_list() -> Option<Vec<ModelInfo>> {
    let guard = MODEL_LIST_CACHE.read().ok()?;
    let (at, models) = guard.as_ref()?;
    if at.elapsed() < MODEL_LIST_CACHE_TTL {
        Some(models.clone())
    } else {
        None
    }
}

fn store_model_list(models: &[ModelInfo]) {
    if let Ok(mut guard) = MODEL_LIST_CACHE.write() {
        *guard = Some((Instant::now(), models.to_vec()));
    }
}

/// 全局模型上下文窗口缓存：模型名 -> context_window（tokens）
/// 从 API /models 端点提取（如 Anthropic 的 max_input_tokens）后填充
static MODEL_CONTEXT_WINDOW_CACHE: RwLock<Option<HashMap<String, u32>>> = RwLock::new(None);

/// 从全局缓存中查询模型的上下文窗口大小
pub fn get_cached_context_window(model_name: &str) -> Option<u32> {
    MODEL_CONTEXT_WINDOW_CACHE
        .read()
        .ok()
        .and_then(|guard| guard.as_ref()?.get(model_name).copied())
}

/// 更新上下文窗口缓存
pub fn update_context_window_cache(models: &[ModelInfo]) {
    if let Ok(mut guard) = MODEL_CONTEXT_WINDOW_CACHE.write() {
        let cache = guard.get_or_insert_with(HashMap::new);
        for m in models {
            if let Some(ctx) = m.context_window {
                cache.insert(m.id.clone(), ctx);
            }
        }
    }
}

/// 当 API 返回 "context window exceeds limit" 错误时，将缓存的上下文窗口减半
/// 这样后续压缩会更激进，避免重复同样的错误
pub fn halve_cached_context_window(model_name: &str) {
    if let Ok(mut guard) = MODEL_CONTEXT_WINDOW_CACHE.write() {
        let cache = guard.get_or_insert_with(HashMap::new);
        let old = cache.get(model_name).copied().unwrap_or(200_000);
        let new = (old / 2).max(32_000); // 不低于 32K
        cache.insert(model_name.to_string(), new);
        crate::utils::logging::append_debug_log_line(&format!(
            "[CTX_WINDOW] Reduced context window for '{}': {} -> {}",
            model_name, old, new
        ));
    }
}

pub(crate) async fn list_models_for_client(
    star_client: &StarClient,
) -> Result<Vec<ModelInfo>, String> {
    // 0. TTL 缓存命中时直接返回，避免每次切换模型都重新拉取全部提供商
    if let Some(cached) = cached_model_list() {
        return Ok(cached);
    }

    let mut models: Vec<ModelInfo> = Vec::new();

    // 1. Try to fetch from API — 短超时，避免慢速 /models 端点阻塞模型切换
    match tokio::time::timeout(CURRENT_PROVIDER_FETCH_TIMEOUT, star_client.list_models()).await {
        Ok(Ok(remote_models)) => {
            models.extend(remote_models);
        }
        Ok(Err(e)) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] Failed to list models from API: {}. Using fallback list...",
                e
            ));
        }
        Err(_) => {
            crate::utils::logging::append_debug_log_line(
                "[WARN] Timed out listing models from current provider API (5s). Using fallback list...",
            );
        }
    }

    // 2. No hardcoded fallback models — API must provide the model list

    // 3. Load locally configured models AND dynamically fetch from all configured providers
    let store = ProviderStore::new();
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
            // Skip if it's the current provider (already fetched in step 1)
            // We use loose comparison on base_url
            let is_current = if let Some(url) = &provider.base_url {
                url.trim_end_matches('/') == star_client.base_url.trim_end_matches('/')
            } else {
                false
            };

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
                        // Create a temporary client with short timeout logic (handled by timeout wrapper)
                        let client = StarClient::new(
                            &api_key,
                            None,
                            Some(base_url),
                            None,
                            Some(pid.clone()),
                        );

                        // 3 second timeout for dynamic fetching
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            client.list_models(),
                        )
                        .await;

                        match result {
                            Ok(Ok(fetched_models)) => Some(fetched_models),
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
        let provider = crate::agent::model_list::detect_provider_name(&star_client.base_url);
        models.insert(0, ModelInfo::new(current, provider));
    }

    // 5. 更新全局上下文窗口缓存（用于后续压缩和状态栏显示）
    update_context_window_cache(&models);

    // 6. 写入模型列表 TTL 缓存
    store_model_list(&models);

    Ok(models)
}
