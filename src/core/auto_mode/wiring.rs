//! Auto 档位的**运行时装配** —— 把真实的 LLM 分类器装进策略引擎。
//!
//! # 为什么单独一个模块
//!
//! 装分类器需要三样东西，而它们散落在三个地方：
//!
//! 1. **模型客户端** —— `StarClient`，在 `Agent` 手上；
//! 2. **副模型名** —— `small_fast_model`（"小而快"的旁路模型），要走 provider
//!    解析链（`ProviderStore` / user settings / 环境变量）才能拿到；
//! 3. **对话上下文** —— 存在 `GlobalState` 里（`auto_mode_transcript` 槽位）。
//!
//! 把这段装配集中在这里，`Agent::lazy_init` 只调一个函数，策略层只认 trait。

use std::sync::Arc;

use crate::core::auto_mode::service::{AutoModeClassifier, LlmAutoModeClassifier};

/// 环境变量：关掉 LLM 分类器（auto 档位退回纯本地护栏，0 LLM 开销）。
pub const ENV_CLASSIFIER_ENABLED: &str = "STAR_AUTO_MODE_CLASSIFIER";

/// 环境变量：指定分类器用的模型名（优先于 `small_fast_model`）。
pub const ENV_CLASSIFIER_MODEL: &str = "STAR_AUTO_MODE_CLASSIFIER_MODEL";

/// 读一个布尔环境变量。
fn env_flag(name: &str, default_value: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => {
            let normalized = raw.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => default_value,
    }
}

/// 分类器是否启用。
///
/// 默认**启用** —— auto 档位的意义就是"AI 自主判定"。没有分类器时它退回
/// 本地护栏，只读工具免确认、危险命令弹框，比 `acceptEdits` 宽松但没有
/// 自主判断能力。
///
/// `STAR_AUTO_MODE_CLASSIFIER=0` 可以关掉，回到纯本地护栏。
pub fn classifier_enabled() -> bool {
    env_flag(ENV_CLASSIFIER_ENABLED, true)
}

/// 解析分类器要用的模型名。
///
/// 优先级：
/// 1. `STAR_AUTO_MODE_CLASSIFIER_MODEL`（显式指定）；
/// 2. provider 解析出的 `small_fast_model`（"小而快"的副模型）；
/// 3. `None` —— 用 client 自己的模型。
///
/// 分类任务短、频率高，用主模型既慢又贵，所以副模型是首选。
pub async fn resolve_classifier_model() -> Option<String> {
    if let Ok(explicit) = std::env::var(ENV_CLASSIFIER_MODEL) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 这里必须 await —— 在 async 上下文里 `block_on` 会与当前运行时的
    // 调度器打架（park 住当前线程又等一个同样要跑的 future）。
    let settings = match crate::core::config::settings_manager::get_settings_manager().await {
        Ok(manager) => manager.load_user_settings().await.ok()?,
        Err(_) => return None,
    };
    let provider_config = crate::core::config::provider_store::ProviderStore::new()
        .load()
        .await
        .unwrap_or_default();

    let resolved = crate::core::config::provider_resolution::resolve_effective_provider_settings(
        crate::core::config::provider_resolution::ProviderResolutionInputs::default(),
        &provider_config,
        &settings,
    );

    let selected = select_classifier_model(
        resolved.small_fast_model.value.as_deref(),
        resolved.small_fast_model.source.kind,
    );
    if selected.is_none() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[AutoMode] small_fast_model from {} is not usable as the classifier model — \
             falling back to the client's own model",
            resolved.small_fast_model.source.kind
        ));
    }
    selected
}

/// 外部工具的副模型未绑定当前 client 的端点和凭据，保守回退到 client 自身模型。
/// 显式分类器模型在调用前已处理，不受此过滤影响。
fn select_classifier_model(candidate: Option<&str>, source_kind: &str) -> Option<String> {
    let candidate = candidate.map(str::trim).filter(|c| !c.is_empty())?;
    if is_external_model_source(source_kind) {
        return None;
    }
    Some(candidate.to_string())
}

/// 这个来源是不是「外部工具配置采纳来的」。
fn is_external_model_source(source_kind: &str) -> bool {
    source_kind == crate::core::config::provider_resolution::SRC_CLAUDE_CODE_SMALL_FAST_MODEL
}

/// 构造分类器并装进策略引擎。
///
/// 返回装没装上 —— 关掉分类器、provider 没配好、或者拿不到共享状态时都返回
/// `false`，此时 auto 档位退化为纯本地护栏（仍然工作，只是不再自主判定）。
pub async fn install_auto_classifier(
    client: &crate::llm::client::StarClient,
    message_bus: &Arc<crate::core::confirmation_bus::MessageBus>,
    global_state: &Arc<crate::core::state::GlobalState>,
) -> bool {
    if !classifier_enabled() {
        crate::utils::logging::append_debug_log_line(
            "[AutoMode] LLM classifier disabled by env — using local guardrails only",
        );
        return false;
    }

    let model = resolve_classifier_model().await;
    let timeout_secs = LlmAutoModeClassifier::timeout_from_env();

    // 熔断包装：provider 挂掉时不让每个工具调用都白等一次超时。
    let (breaker_threshold, breaker_cooldown) =
        crate::core::auto_mode::service::CircuitBreakingClassifier::params_from_env();
    let classifier: Arc<dyn AutoModeClassifier> = Arc::new(
        crate::core::auto_mode::service::CircuitBreakingClassifier::new(
            Arc::new(LlmAutoModeClassifier::new(
                client.clone(),
                model.clone(),
                timeout_secs,
            )),
            breaker_threshold,
            breaker_cooldown,
        ),
    );

    // 对话上下文提供者：每次判定都重新读共享状态里的最新摘要。
    //
    // 用 `tokio::sync::RwLock` 读一个槽位 —— 不能用 `blocking_read`，
    // 那会在 async 运行时里阻塞 executor 线程。
    let transcript_state = global_state.clone();
    let transcript_provider: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(move || {
        // 这里拿不到 async 上下文，所以用 `try_read` 拿不到就返回空串 ——
        // 分类器拿不到上下文时判不准，但它**只收紧不放宽**，拿不到上下文
        // 最坏的结果是多问一次人，不会误放行。
        transcript_state
            .try_auto_mode_transcript()
            .unwrap_or_default()
    });

    message_bus
        .set_auto_classifier(classifier, Some(transcript_provider))
        .await;

    crate::utils::logging::append_debug_log_line(&format!(
        "[AutoMode] LLM classifier installed (model={:?}, timeout={}s)",
        model, timeout_secs
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::provider_resolution as pr;

    /// 本项目自己 provider 配置给出的副模型 —— 直接采用。
    #[test]
    fn project_owned_small_fast_model_is_used() {
        let picked = select_classifier_model(
            Some("small-fast-model"),
            pr::SRC_PROVIDER_STORE_ACTIVE_MODEL,
        );
        assert_eq!(picked.as_deref(), Some("small-fast-model"));
    }

    /// 环境变量给出的副模型（`STAR_SMALL_FAST_MODEL`）也是自己这一侧的。
    #[test]
    fn env_owned_small_fast_model_is_used() {
        let picked =
            select_classifier_model(Some("small-fast-model"), pr::SRC_ENV_STAR_SMALL_FAST_MODEL);
        assert_eq!(picked.as_deref(), Some("small-fast-model"));
    }

    /// 外部工具配置采纳来的副模型：**无法确认**与当前 provider 匹配 →
    /// 放弃，退回 client 自己的模型（`None`），而不是拿它去撞网关。
    #[test]
    fn externally_adopted_small_fast_model_is_not_adopted() {
        let picked = select_classifier_model(
            Some("some-other-provider/model"),
            pr::SRC_CLAUDE_CODE_SMALL_FAST_MODEL,
        );
        assert_eq!(
            picked, None,
            "外部来源的副模型与当前 provider 是否匹配无法确认，不该采用"
        );
    }

    /// 没有候选（provider 没配副模型）→ `None` = 用 client 自己的模型。
    ///
    /// 这是**正常**路径，不是「没有分类器」：分类器照样装上，只是模型名
    /// 留空，由 client 用当前模型跑分类。
    #[test]
    fn no_candidate_means_use_the_client_model() {
        assert_eq!(select_classifier_model(None, pr::SRC_MISSING), None);
    }

    /// 空串 / 纯空白不算候选。
    #[test]
    fn blank_candidates_are_ignored() {
        assert_eq!(select_classifier_model(Some("   "), pr::SRC_MISSING), None);
        assert_eq!(select_classifier_model(Some(""), pr::SRC_MISSING), None);
    }

    /// 候选值两端的空白要被裁掉再采用。
    #[test]
    fn candidates_are_trimmed() {
        let picked = select_classifier_model(
            Some("  small-fast-model  "),
            pr::SRC_PROVIDER_STORE_ACTIVE_MODEL,
        );
        assert_eq!(picked.as_deref(), Some("small-fast-model"));
    }
}
