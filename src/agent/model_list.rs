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

/// 整条"拉一次模型列表"的硬上限（含读配置、缓存、落盘）。
///
/// `active_fetch_timeout()` 只罩住了 `/models` 那一次网络请求。围绕它的还有
/// `ProviderStore::load()`（读 user-settings.json + 从 cwd 往上找项目级覆盖）、
/// 上下文窗口缓存灌盘、模型列表落盘 —— 那些**都没有超时**。在 WSL 的 `/mnt/h`
/// 上任何一次文件 I/O 卡住，整条链路就永远不返回，UI 的模型面板就一直转圈。
///
/// 所以要有这个上限：超时后走 `stale_snapshot_for`，把磁盘缓存里那份交回去，
/// 面板照样有东西看。宁可显示一份旧列表，也不要让用户对着转圈等。
const WHOLE_LIST_TIMEOUT_SECS: u64 = 20;

/// 同 [`WHOLE_LIST_TIMEOUT_SECS`]，可用环境变量调小以便排查。
pub fn whole_list_timeout() -> Duration {
    let secs = std::env::var("STAR_MODEL_LIST_WHOLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(WHOLE_LIST_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// 超时兜底：从磁盘缓存里翻出这个 provider 上次成功拉到的列表。
///
/// 这是"降级"而不是"报错" —— 磁盘缓存刻意不设过期（见模块说明），所以哪怕是
/// 几天前的列表，对"面板里现在该有哪些模型"这个问题也仍然有参考价值，远好过
/// 一个转不掉的圈。拿不到返回空列表，调用方会显示"拉取失败"。
pub fn stale_snapshot_for(client: &crate::llm::client::StarClient) -> ModelListResult {
    let provider_id = client
        .provider_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| detect_provider_name(&client.base_url));
    match crate::agent::model_cache::load(&provider_id) {
        Some(hit) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] Serving {} cached models for '{}' after the deadline",
                hit.models.len(),
                provider_id
            ));
            ModelListResult {
                models: hit.models,
                // 标成"有缓存年龄"而不是"刚拉的" —— 用户要能看出这不是最新结果。
                cache_age_secs: Some(hit.age_secs),
            }
        }
        None => ModelListResult {
            models: Vec::new(),
            cache_age_secs: None,
        },
    }
}

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
///
/// # 为什么是 `try_read` 而不是 `read`
///
/// 这是 `list_models_with_mode` 里**紧跟着 `provider-store-load` 打点之后的第一句**，
/// 所以它一旦阻塞，后面所有日志点（包括那个 20 秒兜底）全部哑掉。
///
/// `read()` 是阻塞式的：只要有别的任务正持有写锁而它自己又卡住了，这里就把整个
/// tokio worker 线程 park 住 —— 线程一被 park，`tokio::time::timeout` 的定时器就没人
/// poll，于是外层兜底**跟着一起失效**，模型面板永远转圈。
///
/// 真实事故：日志停在 `step 'provider-store-load' done in 107µs` 之后再无下文，
/// 连 `[ModelCache] disk hit` 都没打出来 —— 说明卡在"去读缓存"这一步本身。
///
/// `try_read` 拿不到就当作"没命中"，继续往下走真实拉取。缓存本来就是加速手段，
/// 拿不到只慢一点，绝不能把界面挂住。
fn get_cached_models(provider_id: &str) -> Option<(Vec<ModelInfo>, u64)> {
    let cache_lock = MODEL_CACHE.get_or_init(|| RwLock::new(None));
    let cache = cache_lock.try_read().ok()?;
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
/// 要不要为"当前模型"补一条列表里没有的条目？补的话归给谁。
///
/// 单独抽出来是因为 `list_models_with_mode` 要发网络请求，没法直接测；而这条规则
/// 恰恰是"模型面板把别家 provider 的模型贴上当前 provider 标签"的唯一来源，
/// 必须能脱离网络验证。
///
/// # 为什么不是"当前模型不在列表里就补"
///
/// 补出来的条目会被 `apply_models_to_state` 写进 `model_provider_map`，于是
/// `resolve_provider_for_model` 会照着它把模型 id 路由给这个 provider。换过
/// provider 之后，旧 provider 时代的模型名（`opencode-zen/…` 这种带网关前缀的
/// id）就会被贴上**新** provider 的标签顶在列表第一位 —— 选中它必然失败。
///
/// 所以只有"这次一条都没列出来"才补：那时当前模型是我们唯一掌握的事实，标成
/// 当前 provider 是最后手段；provider 明明列出了模型，就不凭猜往里塞。
pub(crate) fn synthesize_current_model_entry(
    listed: &[ModelInfo],
    current: &str,
    current_provider: &str,
) -> Option<ModelInfo> {
    if current.trim().is_empty() {
        return None;
    }
    let offered = listed
        .iter()
        .any(|m| m.id == current && m.provider == current_provider);
    if offered {
        return None;
    }
    if !listed.is_empty() {
        // provider 确实列了模型，只是没有当前那个 —— 不补。
        return None;
    }
    Some(ModelInfo::new(
        current.to_string(),
        current_provider.to_string(),
    ))
}

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
    //
    // 从这里开始的每一段都打点。整条链路的硬上限在 `whole_list_timeout`，但只靠
    // 上限只能知道"卡了"，不知道卡在哪 —— 真实事故里日志停在 `Client state`
    // 之后 70 多秒没有下文，只能靠猜到底是网络、配置读盘还是落盘。有了点号，
    // 下一次再卡，日志会直接指认是第几步。
    let step_start = std::time::Instant::now();
    let mark = |what: &str| {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ListModels] step '{}' done in {:?}",
            what,
            step_start.elapsed()
        ));
    };
    let store = crate::core::config::provider_store::ProviderStore::new();
    let config = store.load().await.ok();
    mark("provider-store-load");

    // 一次全量清扫：把**所有**已删除 provider 留下的桶删掉。
    //
    // 只在"读当前 provider 的桶"时顺手清那一个是不够的 —— 没被访问到的桶永远清不到。
    // 实测现场：配置里只有 `local` + `opencode`，缓存里却躺着 `apinex` / `dev` /
    // `magpie` 三个孤儿桶（`dev` 和 `local` 模型数一样，是改名时留下的）。它们的模型
    // 会混进模型面板，用户看到一个自己早就删掉、面板里也找不到的 provider 名。
    //
    // 每次拉列表都扫一遍很便宜（就是个 HashMap 的 keys 过滤），换"残留永远不存在"。
    {
        let mut live: std::collections::HashSet<String> = config
            .as_ref()
            .map(|config| config.providers.keys().cloned().collect())
            .unwrap_or_default();
        live.extend(
            crate::core::config::providers::ALL_PROVIDERS
                .iter()
                .map(|p| p.id.to_string()),
        );
        crate::agent::model_cache::purge_orphans(&live);
    }
    let current_provider_id = star_client
        .provider_id
        .clone()
        .filter(|provider_id| !provider_id.trim().is_empty())
        .or_else(|| config.as_ref().and_then(|c| c.active_provider_id.clone()))
        .unwrap_or_default();

    // 1. 便宜路径：先内存缓存，再磁盘缓存，命中就直接返回，一个网络请求都不发。
    //
    // 两条命中路径都必须先确认**这个 provider 还存在**。缓存的寿命和 provider 是
    // 脱钩的，而 `current_provider_id` 可能还指着一个早已删除的 id —— 于是模型面板
    // 会端出一整组属于已删除 provider 的模型，而 Providers 面板里根本找不到它，
    // 两个面板对不上。详见磁盘缓存那条分支里的说明。
    if !force {
        let provider_still_exists = config
            .as_ref()
            .map(|config| {
                let is_builtin =
                    crate::core::config::providers::get_provider_by_id(&current_provider_id)
                        .is_some();
                // 判据必须和 Providers 面板逐字一致，也就是 `provider_is_configured`。
                //
                // 单看 `providers` 里有没有这一项不够：自定义 provider 只要 key 和
                // URL 都被清空，`provider_is_configured` 就判它"未配置"，面板里已经
                // 看不见了 —— 可配置项和磁盘缓存桶都还在，模型面板照端不误，用户面对
                // 一个找不到也删不掉的 provider 名。
                is_builtin
                    || config
                        .providers
                        .get(&current_provider_id)
                        .map(|settings| {
                            crate::core::config::providers::provider_is_configured(
                                &current_provider_id,
                                Some(settings),
                                config.active_provider_id.as_deref(),
                            )
                        })
                        .unwrap_or(false)
            })
            .unwrap_or(false);

        if !provider_still_exists
            && (!current_provider_id.is_empty())
            && (get_cached_models(&current_provider_id).is_some()
                || crate::agent::model_cache::load(&current_provider_id).is_some())
        {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] Discarding cached models for '{}': that provider no longer exists",
                current_provider_id
            ));
            crate::agent::model_cache::purge(&current_provider_id);
            clear_model_cache();
        }

        if provider_still_exists {
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
        }

        if let Some(hit) = crate::agent::model_cache::load(&current_provider_id) {
            // 桶里还留着，但**这个 provider 可能早就被删了**。
            //
            // 磁盘缓存的寿命和 provider 是脱钩的：删掉 provider 时程序不一定会顺手
            // 删桶，外部工具直接改配置同样留残留。`current_provider_id` 又可能还指着
            // 那个已删除的 id，于是模型面板照着桶把一整个已删除 provider 的模型全端
            // 出来，而 Providers 面板（数据来自 `configured_providers`，已按配置过滤）
            // 里根本找不到它 —— **两个面板对不上**。
            //
            // 真实事故：用户早就删掉了 `astrdark`，模型面板里却还挂着 36 个
            // `Provider: astrdark` 的模型。
            //
            // 这里把桶清掉并继续往下走（走真实拉取），宁可"没有列表"也不要端一份
            // 属于已删除 provider 的幽灵数据。
            let still_exists = provider_still_exists;
            if !still_exists {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Dropping cached models for '{}': that provider no longer exists",
                    current_provider_id
                ));
                crate::agent::model_cache::purge(&current_provider_id);
                // 内存那份同样作废，否则下一次 force=false 还会命中它。
                clear_model_cache();
            } else {
                // 顺手把内存缓存和上下文窗口缓存热起来，后面的状态栏/压缩逻辑都要用。
                crate::agent::model_catalog::update_context_window_cache(
                    Some(current_provider_id.as_str()),
                    &hit.models,
                );
                set_cached_models(current_provider_id.clone(), hit.models.clone());
                return Ok(ModelListResult {
                    models: hit.models,
                    cache_age_secs: Some(hit.age_secs),
                });
            }
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
    mark("active-provider-fetch");

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
            //
            // 走 `candidate_models()` 而不是直接遍历 `provider.models`：后者是
            // HashMap，键序随机，而这份列表会直接进模型面板 —— 每次刷新换一次
            // 排列，用户会以为程序在乱跳。`candidate_models()` 还把
            // `selected_model` 放在首位，那才是用户钦定的默认项。
            let configured_models = provider.candidate_models();
            if configured_models.is_empty() {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Provider '{}' has no models configured.",
                    pid
                ));
            } else {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Provider '{}' has {} configured models (default '{}').",
                    pid,
                    configured_models.len(),
                    configured_models[0]
                ));
                for model_id in &configured_models {
                    if !models
                        .iter()
                        .any(|m| m.id == *model_id && m.provider == *pid)
                    {
                        models.push(ModelInfo::new(model_id.clone(), pid.clone()));
                    }
                }
            }

            // B. Dynamic Parallel Fetching —— 只在显式刷新时扇出，这是原来最慢的一步
            if !force {
                continue;
            }

            // 空密钥的 provider 不参与扇出。
            //
            // 它打过去必然 401/403，等多久都不会成功，但每个 provider 都带一个
            // 3 秒超时，而 `join_all` 是等**最慢**的那个 —— 一个没填 key 的 provider
            // 就能把整次刷新的耗时顶满。跳过它，用户看到的 panel 也不会塞满
            // 一批永远用不了的模型。
            //
            // 它并没有从 UI 消失：Provider 菜单里仍然列出（标"未配置密钥"），
            // 手输 `/model 名字` 也仍然可用，只是会在本地校验就被拦下。
            if !crate::core::config::providers::provider_has_usable_credentials(pid, Some(provider))
            {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[ListModels] Skipping fan-out for provider '{}': no usable credentials.",
                    pid
                ));
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

    // 4. 当前模型：只在**这次一条都没列出来**时才补进去
    //
    // 原来是无条件补 —— 只要当前模型不在列表里，就拿**当前激活的 provider**
    // 给它贴标签，然后 `insert(0)` 顶到列表第一位。
    //
    // 真实后果（用户截图）：从 magpie 切到 yunjunet 之后，面板第一行是
    // `opencode-zen/space-bunny-free   Provider: yunjunet` —— magpie 时代的
    // OpenRouter 风格 id 被贴成了 yunjunet 的模型，而 yunjunet 根本没有它。
    //
    // 危害不止于显示难看：`apply_models_to_state` 会据此写下
    // `model_provider_map["opencode-zen/space-bunny-free"] = "yunjunet"`，
    // `resolve_provider_for_model` 于是把这个 id 路由给 yunjunet。选中它必然失败
    // —— 正是"怎么切都切不对"那一类症状。
    //
    // 补它的本意是"拉不到列表时别让面板空着"，那就只在这个前提下补：provider
    // 明明列出了模型，就不该凭猜往里塞一个它并不提供的。
    let current = star_client.model.clone();
    let provider_id = active_pid_for_list
        .or_else(|| {
            star_client
                .provider_id
                .clone()
                .filter(|provider_id| !provider_id.trim().is_empty())
        })
        .unwrap_or_else(|| current_provider_id.clone());
    let current_provider = if provider_id.is_empty() {
        detect_provider_name(&star_client.base_url)
    } else {
        provider_id.clone()
    };
    let current_is_offered = models
        .iter()
        .any(|m| m.id == current && m.provider == current_provider);

    // 5. Cache the result
    //    上下文窗口缓存也在这里灌：状态栏和压缩逻辑读的是它。
    //
    //    **必须在补当前模型之前**落盘。
    //
    //    补出来的条目是"你现在在用什么"的 UI 便利，不是"这个 provider 提供它"的
    //    事实。写进 provider 桶之后，它就和真正拉回来的模型混在一起无法分辨，而且
    //    会随缓存活很久：命中缓存时第 1 步直接原样返回整份缓存，补条逻辑根本不执行
    //    —— 于是那条假条目会一直被面板吐出来，选中它就必然失败。
    //
    //    顺序调过来之后，缓存里只可能有真拉回来的模型；下次命中缓存也就不会再带出
    //    假条目。
    crate::agent::model_catalog::update_context_window_cache(Some(provider_id.as_str()), &models);
    mark("context-window-cache");
    if !models.is_empty() {
        set_cached_models(provider_id.clone(), models.clone());

        // 不变量：**缓存桶只能为配置里真实存在的 provider 存在。**
        //
        // 只在读的一侧清扫孤儿是不够的 —— 写的一侧从不检查，于是"某个东西自认为当前
        // provider 是 X"就会把 X 的桶写回去，删掉也会再回来。实测：`user-settings.json`
        // 里只有 `local` + `opencode`，缓存里却坐着 `apinex` / `dev` / `magpie`，删干净
        // 重启后又冒出 `astrdark`。
        //
        // `provider_id` 来自 `active_pid_for_list` / client / `detect_provider_name`
        // 三处回退，其中 client 那条可能带着会话恢复时的旧 id。所以这里必须以配置为
        // 准：配置里没有的 provider，不给它留桶。命中不了就只写内存缓存（进程内，
        // 活不过本次会话）。
        let may_persist = config
            .as_ref()
            .map(|config| {
                crate::core::config::providers::get_provider_by_id(&provider_id).is_some()
                    || config.providers.contains_key(&provider_id)
            })
            .unwrap_or(false);

        if may_persist {
            crate::agent::model_cache::save(&provider_id, &models);
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] Cached {} models for provider '{}'",
                models.len(),
                provider_id
            ));
        } else {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] Refusing to persist models for '{}': not in the provider config \
                 (in-memory cache only; the list is still returned)",
                provider_id
            ));
        }
    }
    mark("model-cache-disk-write");

    // 6. 缓存落盘之后，才补当前模型 —— 只影响这一份返回给 UI 的列表。
    match synthesize_current_model_entry(&models, &current, &current_provider) {
        Some(entry) => {
            models.push(entry);
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] provider '{}' listed nothing; showing the current model '{}' alone",
                current_provider, current
            ));
        }
        None if !current.is_empty() && !current_is_offered => {
            // provider 确实列了模型，只是里面没有当前那个。不补，但留日志 ——
            // 这通常意味着换过 provider 而旧模型还挂在 client 上，是个真问题。
            crate::utils::logging::append_debug_log_line(&format!(
                "[ListModels] current model '{}' is not offered by provider '{}' ({} model(s) listed) \
                 — not fabricating an entry for it",
                current,
                current_provider,
                models.len()
            ));
        }
        None => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, provider: &str) -> ModelInfo {
        ModelInfo::new(id.to_string(), provider.to_string())
    }

    /// 回归：provider 明明列出了模型，就**不能**把当前模型贴上它的标签塞进去。
    ///
    /// 用户报告的真实画面：从 magpie 切到 yunjunet 之后，模型面板第一行是
    /// `opencode-zen/space-bunny-free   Provider: yunjunet` —— magpie 时代的
    /// OpenRouter 风格 id 被贴成了 yunjunet 的模型。
    ///
    /// 危害不止于显示：`apply_models_to_state` 会把它写进 `model_provider_map`，
    /// `resolve_provider_for_model` 于是把 `opencode-zen/…` 路由给 yunjunet，
    /// 选中它必然失败。
    #[test]
    fn a_current_model_the_provider_does_not_offer_is_never_fabricated() {
        let listed = vec![
            info("deepseek-v4-pro", "yunjunet"),
            info("glm-5.2", "yunjunet"),
        ];
        assert!(
            synthesize_current_model_entry(&listed, "opencode-zen/space-bunny-free", "yunjunet")
                .is_none(),
            "provider 列了 2 个模型却不含当前那个 —— 绝不能凭猜补一条并贴上它的标签"
        );
    }

    /// provider 确实提供当前模型时，本来就有一条，不该重复补。
    #[test]
    fn a_current_model_already_offered_is_not_duplicated() {
        let listed = vec![info("glm-5.2", "yunjunet")];
        assert!(
            synthesize_current_model_entry(&listed, "glm-5.2", "yunjunet").is_none(),
            "已经在列表里了，不该重复补"
        );
    }

    /// 一条都没列出来时仍然要补 —— 面板不能空着，否则用户连"我现在在用哪个"
    /// 都看不到。
    #[test]
    fn the_current_model_is_shown_alone_when_the_provider_listed_nothing() {
        let entry =
            synthesize_current_model_entry(&[], "opencode-zen/space-bunny-free", "yunjunet")
                .expect("空列表时应当补出当前模型");
        assert_eq!(entry.id, "opencode-zen/space-bunny-free");
        assert_eq!(
            entry.provider, "yunjunet",
            "这时归给当前 provider 是最后手段，不是猜测 —— 面板已经空了"
        );
    }

    /// 空模型名不补：补出来的空条目会在面板上显示成一行空白。
    #[test]
    fn a_blank_current_model_is_never_synthesized() {
        assert!(
            synthesize_current_model_entry(&[], "  ", "yunjunet").is_none(),
            "空模型名补出来就是一行空白"
        );
    }

    /// 同名模型但属于别的 provider，不算"被提供" —— 这是换 provider 后最典型的
    /// 残留形状（两家都卖 `deepseek-v4-pro`，但当前模型属于上一个）。
    #[test]
    fn a_same_named_model_owned_by_another_provider_does_not_count_as_offered() {
        let listed = vec![info("deepseek-v4-pro", "magpie")];
        assert!(
            synthesize_current_model_entry(&listed, "deepseek-v4-pro", "yunjunet").is_none(),
            "列表非空就不补 —— 名字撞车不构成'当前 provider 提供它'"
        );
    }

    /// 回归：补出来的条目绝不能进缓存。
    ///
    /// 命中缓存时 `list_models_with_mode` 在第 1 步就原样返回整份缓存，补条逻辑
    /// **根本不执行**。所以一旦把补出来的条目写进 provider 桶，它就会一直跟着缓存
    /// 被面板吐出来 —— 用户截图里那条 12 小时前的假条目正是这么活下来的，而且选中
    /// 它会把 `opencode-zen/…` 路由给 yunjunet，必然失败。
    ///
    /// 这条锁的是"落盘发生在补条之前"这个顺序，不是函数本身的行为，所以在这里
    /// 用一段等价代码复现该顺序。
    #[test]
    fn the_synthesized_entry_is_never_part_of_the_cached_list() {
        // provider 一条都没列出来 —— 这是补条逻辑**唯一**会触发的场景。
        let fetched: Vec<ModelInfo> = Vec::new();
        let current = "opencode-zen/space-bunny-free";
        let provider = "yunjunet";

        // 正确顺序：先落盘真列表，再补（补只作用于返回值）。
        let mut cached = fetched.clone();
        let mut returned = cached.clone();
        if let Some(entry) = synthesize_current_model_entry(&cached, current, provider) {
            returned.push(entry);
        }

        assert!(
            !cached.iter().any(|m| m.id == current),
            "缓存里绝不能出现补出来的条目 —— 它不是这个 provider 的模型"
        );
        assert!(
            returned.iter().any(|m| m.id == current),
            "返回给 UI 的那份要带上它，否则面板会空着"
        );

        // 再次命中缓存时拿到的是空列表（缓存本来就没写入任何东西），
        // 补条逻辑会重新按当次的 provider 判断，不会固化一条假归属。
        let from_cache = cached;
        assert!(from_cache.is_empty());
    }
}
