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

/// 提供商与模型共同构成能力缓存身份：相同 model id 可以由不同网关以不同
/// 上下文容量提供，绝不能互相污染。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelCapabilityKey {
    provider_id: String,
    model_id: String,
}

impl ModelCapabilityKey {
    fn new(provider_id: Option<&str>, model_id: &str) -> Self {
        Self {
            provider_id: provider_id.unwrap_or_default().to_string(),
            model_id: model_id.to_string(),
        }
    }
}

/// 由 `/models` 返回的名义容量，按 provider + model 保存。
static MODEL_CONTEXT_WINDOW_CACHE: RwLock<Option<HashMap<ModelCapabilityKey, u32>>> =
    RwLock::new(None);
/// provider 明确返回上下文溢出后记录的会话安全上限，不能覆盖用户的 Fixed 选择。
static PROVIDER_SAFE_CONTEXT_CAP_CACHE: RwLock<Option<HashMap<ModelCapabilityKey, u32>>> =
    RwLock::new(None);

/// 从 provider + model 作用域的缓存中查询模型上下文窗口。
pub fn get_cached_context_window(provider_id: Option<&str>, model_name: &str) -> Option<u32> {
    let key = ModelCapabilityKey::new(provider_id, model_name);
    MODEL_CONTEXT_WINDOW_CACHE
        .read()
        .ok()
        .and_then(|guard| guard.as_ref()?.get(&key).copied())
}

/// 查询 provider 明确报告的会话安全容量上限。
pub fn get_provider_safe_context_cap(provider_id: Option<&str>, model_name: &str) -> Option<u32> {
    let key = ModelCapabilityKey::new(provider_id, model_name);
    PROVIDER_SAFE_CONTEXT_CAP_CACHE
        .read()
        .ok()
        .and_then(|guard| guard.as_ref()?.get(&key).copied())
}

/// 更新指定 provider 的模型上下文窗口缓存。
pub fn update_context_window_cache(provider_id: Option<&str>, models: &[ModelInfo]) {
    if let Ok(mut guard) = MODEL_CONTEXT_WINDOW_CACHE.write() {
        let cache = guard.get_or_insert_with(HashMap::new);
        for model in models {
            if let Some(context_window) = model.context_window {
                let resolved_provider = if model.provider.is_empty() {
                    provider_id
                } else {
                    Some(model.provider.as_str())
                };
                cache.insert(
                    ModelCapabilityKey::new(resolved_provider, &model.id),
                    context_window,
                );
            }
        }
    }
}

/// 当 provider 明确报告了确切的 context-window 上限时，记录当前
/// provider + model 的安全容量。单纯的溢出只证明请求过大，不能推断精确上限，
/// 因此 `reported_cap` 为 `None` 时不会写入猜测值。
///
/// 该证据仅影响 Auto 的最终容量；Fixed 选择会保留请求值并由 policy 报告降级。
pub fn record_provider_safe_context_cap(
    provider_id: Option<&str>,
    model_name: &str,
    reported_cap: Option<u32>,
) -> Option<u32> {
    let reported_cap = reported_cap.filter(|cap| *cap > 0)?;
    let key = ModelCapabilityKey::new(provider_id, model_name);
    let nominal_cap = get_cached_context_window(provider_id, model_name);
    // Provider 报出的值只能受已知名义容量约束；产品默认值不是 provider 证据，
    // 不能把一个已确认的大容量错误地截断到默认 200K。
    let safe_cap = nominal_cap.map_or(reported_cap, |nominal| reported_cap.min(nominal));

    if let Ok(mut guard) = PROVIDER_SAFE_CONTEXT_CAP_CACHE.write() {
        let cache = guard.get_or_insert_with(HashMap::new);
        let prior_cap = cache.get(&key).copied();
        let effective_cap = prior_cap.map_or(safe_cap, |prior_cap| prior_cap.min(safe_cap));
        cache.insert(key, effective_cap);
        crate::utils::logging::append_debug_log_line(&format!(
            "[CTX_WINDOW] Recorded provider-reported context capacity for '{}': {} -> {}",
            model_name,
            prior_cap.unwrap_or(safe_cap),
            effective_cap
        ));
        Some(effective_cap)
    } else {
        Some(safe_cap)
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
        Ok(Ok(mut remote_models)) => {
            let provider_id = star_client.provider_id.clone().unwrap_or_else(|| {
                crate::agent::model_list::detect_provider_name(&star_client.base_url)
            });
            for model in &mut remote_models {
                if model.provider.is_empty() {
                    model.provider = provider_id.clone();
                }
            }
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
                    if !models
                        .iter()
                        .any(|m| m.id == *model_id && m.provider == *pid)
                    {
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
                            Ok(Ok(mut fetched_models)) => {
                                for model in &mut fetched_models {
                                    if model.provider.is_empty() {
                                        model.provider = pid.clone();
                                    }
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
                        if !models
                            .iter()
                            .any(|existing| existing.id == m.id && existing.provider == m.provider)
                        {
                            models.push(m);
                        }
                    }
                }
            }
        }
    }

    // 4. Ensure current model is in the list
    let current = star_client.model.clone();
    let current_provider = star_client
        .provider_id
        .clone()
        .unwrap_or_else(|| crate::agent::model_list::detect_provider_name(&star_client.base_url));
    if !models
        .iter()
        .any(|m| m.id == current && m.provider == current_provider)
        && !current.is_empty()
    {
        models.insert(0, ModelInfo::new(current, current_provider.clone()));
    }

    // 5. 更新全局上下文窗口缓存（用于后续压缩和状态栏显示）
    update_context_window_cache(Some(current_provider.as_str()), &models);

    // 6. 写入模型列表 TTL 缓存
    store_model_list(&models);

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_capabilities_are_scoped_to_provider_and_model() {
        let provider_a = "model-catalog-test-provider-a";
        let provider_b = "model-catalog-test-provider-b";
        let model = "model-catalog-test-model";

        update_context_window_cache(
            Some(provider_a),
            &[ModelInfo::new(model, provider_a).with_context_window(1_000_000)],
        );
        update_context_window_cache(
            Some(provider_b),
            &[ModelInfo::new(model, provider_b).with_context_window(200_000)],
        );

        assert_eq!(
            get_cached_context_window(Some(provider_a), model),
            Some(1_000_000)
        );
        assert_eq!(
            get_cached_context_window(Some(provider_b), model),
            Some(200_000)
        );
        assert_eq!(get_cached_context_window(None, model), None);
    }

    #[test]
    fn reported_safe_cap_without_nominal_evidence_is_not_clamped_to_the_product_default() {
        let provider = "model-catalog-test-unbounded-safe-cap-provider";
        let model = "model-catalog-test-unbounded-safe-cap-model";

        assert_eq!(
            record_provider_safe_context_cap(Some(provider), model, Some(1_000_000)),
            Some(1_000_000)
        );
        assert_eq!(
            get_provider_safe_context_cap(Some(provider), model),
            Some(1_000_000)
        );
    }

    #[test]
    fn provider_safe_cap_uses_only_reported_limits_and_never_exceeds_nominal() {
        let provider = "model-catalog-test-safe-cap-provider";
        let model = "model-catalog-test-safe-cap-model";
        update_context_window_cache(
            Some(provider),
            &[ModelInfo::new(model, provider).with_context_window(16_000)],
        );

        assert_eq!(
            record_provider_safe_context_cap(Some(provider), model, None),
            None
        );
        assert_eq!(get_provider_safe_context_cap(Some(provider), model), None);
        assert_eq!(
            record_provider_safe_context_cap(Some(provider), model, Some(32_000)),
            Some(16_000)
        );
        assert_eq!(
            get_provider_safe_context_cap(Some(provider), model),
            Some(16_000)
        );
    }
}
