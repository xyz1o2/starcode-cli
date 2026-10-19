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
///
/// # 为什么用 `try_write` 而不是 `write`
///
/// 这是 `list_models_with_mode` **磁盘缓存命中路径上的第一句话**，也就是说：只要
/// 磁盘缓存命中（它不设过期，实际等于永远命中），每次开模型面板都会走到这里。
///
/// 而 `write()` 是**阻塞式**的：只要有别的线程正持有同一把锁的读锁且自己卡住，
/// 这里就把整个线程 park 住。调用方是 tokio worker 线程 —— 线程一旦被 park，
/// `tokio::time::timeout` 的定时器就再也没人 poll，于是外层那个 20 秒兜底
/// **也跟着失效**。真实事故：日志停在 `[ModelCache] disk hit for 'opencode'` 之后
/// 53 秒没有任何后续，整个模型面板卡死，只能按 Esc 取消。
///
/// `try_write` 拿不到锁就直接跳过：这份缓存只是**预热**，丢一次毫无后果 ——
/// 下次调用会再写一遍，而状态栏、压缩逻辑这些读者拿不到就退回到别的来源。
/// 宁可少预热一次，也不能为了预热把用户界面挂住。
pub fn update_context_window_cache(provider_id: Option<&str>, models: &[ModelInfo]) {
    // 没有任何模型带 context_window 时不必去抢那把锁。
    if !models.iter().any(|m| m.context_window.is_some()) {
        return;
    }
    let Ok(mut guard) = MODEL_CONTEXT_WINDOW_CACHE.try_write() else {
        // 有人在读（或已经卡在读上）。跳过这次预热。
        return;
    };
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

#[cfg(test)]
mod non_blocking_tests {
    use super::*;

    /// 回归：缓存预热**绝不能**阻塞调用方。
    ///
    /// 真实事故：模型面板卡死 53 秒，日志停在 `[ModelCache] disk hit for
    /// 'opencode': 84 models` 之后毫无后续，只能按 Esc 取消。而外层那个 20 秒
    /// 兜底也没生效 —— 因为 `write()` 把 tokio worker 线程整个 park 住了，定时器
    /// 再没人 poll。
    ///
    /// 这里用一把**被故意占住的写锁**来复现那个形状：旧的 `write()` 在这里会直接
    /// 死锁（测试超时），`try_write()` 则必须立刻返回。
    #[test]
    fn cache_warmup_never_blocks_even_when_the_lock_is_held() {
        // 先占住写锁，制造"有别的线程卡在这把锁上"的局面。
        let _held = MODEL_CONTEXT_WINDOW_CACHE.write();

        let mut model = ModelInfo::new("some-model".to_string(), "some-provider".to_string());
        model.context_window = Some(128_000);
        let models = vec![model];

        // 这一句在旧实现下会永久阻塞；现在必须立刻返回。
        update_context_window_cache(Some("some-provider"), &models);
    }

    /// 没有模型带 context_window 时不该去抢锁 —— 一次没必要的抢锁就是一次
    /// 潜在的等待。
    #[test]
    fn a_list_without_context_windows_does_not_touch_the_lock() {
        let models = vec![ModelInfo::new("no-window".to_string(), "p".to_string())];
        // 正常路径不能坏：能拿锁时照写不误。
        update_context_window_cache(Some("p"), &models);
        assert_eq!(get_cached_context_window(Some("p"), "no-window"), None);
    }
}
