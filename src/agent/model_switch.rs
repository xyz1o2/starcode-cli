//! 运行中切模型的交接。
//!
//! 用户在 agent 还在跑的时候切模型，这个改动要落在**下一次请求**上 ——
//! 对标 Claude Code（"/model … Claude Code applies the change without waiting
//! for the current response to finish"）与 OpenCode（"the model is sampled at
//! the start of a provider turn; changes admitted after that boundary apply to
//! the next provider turn"）。既不是等整轮结束（Codex 那套），也不是打断当前
//! 请求。
//!
//! 交接靠一个共享槽：运行时把**用户选中的**那对 (model, provider) 放进去，
//! 运行中的回合在每次发请求前取走它、解析成具体的 base_url / key、套到自己的
//! client 上。
//!
//! 为什么不在运行时侧就解析完：解析要读 provider store，也要知道"当前的
//! base_url / key"（provider 没配时得沿用当前值）。回合侧两样都有 —— 它手上
//! 就是当前的 client —— 所以解析放在回合侧，运行时只负责"把用户的意思传过去"。

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::llm::client::StarClient;

/// 用户在运行中选中的模型切换。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingModelSwitch {
    pub model: String,
    /// 归属 provider。`None` = 沿用当前 provider（与 UI 侧
    /// `RuntimeModelSelection.provider_id` 同义）。
    pub provider_id: Option<String>,
}

/// 共享槽：运行时 → 运行中的回合。
pub type ModelSwitchCell = Arc<Mutex<Option<PendingModelSwitch>>>;

pub fn new_cell() -> ModelSwitchCell {
    Arc::new(Mutex::new(None))
}

/// 把用户选中的切换放进槽里。运行中的回合下一次请求会取走它。
pub async fn publish(cell: &ModelSwitchCell, switch: PendingModelSwitch) {
    let mut guard = cell.lock().await;
    *guard = Some(switch);
}

/// 取走槽里的切换（取走即清空 —— 同一次切换只该被一个回合消费）。
pub async fn take(cell: &ModelSwitchCell) -> Option<PendingModelSwitch> {
    let mut guard = cell.lock().await;
    guard.take()
}

/// 一次已经解析好的模型切换：具体到 base_url / key。
#[derive(Clone, Debug)]
pub struct ResolvedModelSwitch {
    pub model: String,
    pub provider_id: Option<String>,
    pub base_url: String,
    pub api_key: String,
    pub is_openai_compatible: Option<bool>,
    /// provider 明确不存在时只改模型名、不换 provider（见 [`resolve`] 的注释）。
    pub switch_provider: bool,
}

/// 解析一次模型选择：归属 provider、base_url、api_key。
///
/// 与 `StarAgent::apply_model_selection` 同一套规则 —— 同一个 provider 既有
/// 配置又有内建元数据时以配置为准；provider 明确不存在时**不**静默降级到当前
/// provider，而是报错（否则请求会继续发往另一家，表现为"切了没反应"）。
pub async fn resolve(
    model: &str,
    provider_id: Option<&str>,
    current: &StarClient,
) -> Result<ResolvedModelSwitch, String> {
    if model.trim().is_empty() {
        return Err("Model name cannot be empty.".to_string());
    }

    let Some(pid) = provider_id else {
        // 不换 provider：只改模型名，base_url / key 沿用当前的。
        return Ok(ResolvedModelSwitch {
            model: model.to_string(),
            provider_id: current.provider_id.clone(),
            base_url: current.base_url.clone(),
            api_key: current.api_key.clone(),
            is_openai_compatible: None,
            switch_provider: false,
        });
    };

    let store = crate::core::config::provider_store::ProviderStore::new();
    let config = store
        .load()
        .await
        .map_err(|error| format!("Could not load provider '{}' configuration: {}", pid, error))?;
    let configured = config
        .providers
        .keys()
        .any(|id| id.eq_ignore_ascii_case(pid));
    let built_in = crate::core::config::providers::get_provider_by_id(pid).is_some();
    if !configured && !built_in {
        return Err(format!("Provider '{}' is not configured.", pid));
    }

    let settings = config
        .providers
        .iter()
        .find_map(|(id, settings)| id.eq_ignore_ascii_case(pid).then_some(settings));

    let (base_url, api_key) = if let Some(settings) = settings {
        let url = settings
            .base_url
            .clone()
            .or_else(|| {
                crate::core::config::providers::get_provider_by_id(pid)
                    .and_then(|meta| meta.default_base_url)
                    .map(|url| url.to_string())
            })
            .unwrap_or_else(|| current.base_url.clone());
        let key = crate::core::config::providers::resolve_runtime_api_key(
            Some(pid),
            settings.api_key.clone(),
        )
        .unwrap_or_else(|| current.api_key.clone());
        (url, key)
    } else if let Some(meta) = crate::core::config::providers::get_provider_by_id(pid) {
        let url = meta
            .default_base_url
            .map(|url| url.to_string())
            .unwrap_or_else(|| current.base_url.clone());
        let key = crate::core::config::providers::resolve_runtime_api_key(Some(pid), None)
            .unwrap_or_else(|| current.api_key.clone());
        (url, key)
    } else {
        // 既没有该 provider 的配置、也没有内建元数据：只改模型名，不切 provider。
        //
        // 走到这里说明上面的校验和这次查找对"provider 存不存在"给出了不同答案。
        // 静默只改模型名的话，请求会继续发往上一次那个 provider —— 表现为
        // "切了没反应"。所以留痕。
        crate::utils::logging::append_debug_log_line(&format!(
            "[ModelSwitch] provider '{}' has neither stored config nor built-in metadata; \
             setting model '{}' WITHOUT switching provider",
            pid, model
        ));
        return Ok(ResolvedModelSwitch {
            model: model.to_string(),
            provider_id: current.provider_id.clone(),
            base_url: current.base_url.clone(),
            api_key: current.api_key.clone(),
            is_openai_compatible: None,
            switch_provider: false,
        });
    };

    let is_openai_compatible = crate::core::config::providers::resolve_openai_compatible_mode(
        pid,
        settings.and_then(|settings| settings.r#type.as_deref()),
    );

    Ok(ResolvedModelSwitch {
        model: model.to_string(),
        provider_id: Some(pid.to_string()),
        base_url,
        api_key,
        is_openai_compatible,
        switch_provider: true,
    })
}

/// 把解析好的切换套到 client 上。
pub fn apply_to_client(client: &mut StarClient, switch: &ResolvedModelSwitch) {
    if switch.switch_provider {
        client.switch_provider(
            &switch.model,
            &switch.base_url,
            &switch.api_key,
            switch.is_openai_compatible,
            switch.provider_id.clone(),
        );
    } else {
        client.set_model(&switch.model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> StarClient {
        StarClient::new(
            "test-key",
            Some("model-a".to_string()),
            Some("https://example.invalid/v1".to_string()),
            Some(false),
            None,
        )
    }

    /// 放进槽里的切换只能被取走一次 —— 否则同一份意图会被两个回合各消费一次。
    #[tokio::test]
    async fn a_published_switch_is_taken_exactly_once() {
        let cell = new_cell();
        assert!(take(&cell).await.is_none(), "一开始是空的");

        publish(
            &cell,
            PendingModelSwitch {
                model: "model-b".to_string(),
                provider_id: Some("local".to_string()),
            },
        )
        .await;

        let taken = take(&cell).await.expect("切换应当被取走");
        assert_eq!(taken.model, "model-b");
        assert_eq!(taken.provider_id.as_deref(), Some("local"));
        assert!(take(&cell).await.is_none(), "同一次切换只该被消费一次");
    }

    /// 不切 provider 时沿用当前的 endpoint —— 否则请求会发去别处。
    #[tokio::test]
    async fn resolving_without_a_provider_keeps_the_current_endpoint() {
        let client = client();
        let resolved = resolve("model-b", None, &client).await.expect("resolve");

        assert_eq!(resolved.model, "model-b");
        assert!(!resolved.switch_provider, "不该换 provider");
        assert_eq!(resolved.base_url, client.base_url);
        assert_eq!(resolved.api_key, client.api_key);
    }

    /// provider 不存在时必须报错，不能静默降级到当前 provider —— 那会表现为
    /// "切了没反应"（请求继续发往另一家）。
    #[tokio::test]
    async fn an_unconfigured_provider_is_rejected() {
        let client = client();
        let err = resolve("model-b", Some("no-such-provider-xyz"), &client)
            .await
            .expect_err("不存在的 provider 必须被拒");
        assert!(err.contains("is not configured"), "理由要说清楚: {}", err);
    }

    /// 套用切换：不换 provider 时只改模型名。
    #[test]
    fn applying_a_model_only_switch_changes_the_model() {
        let mut client = client();
        let resolved = ResolvedModelSwitch {
            model: "model-b".to_string(),
            provider_id: None,
            base_url: client.base_url.clone(),
            api_key: client.api_key.clone(),
            is_openai_compatible: None,
            switch_provider: false,
        };

        apply_to_client(&mut client, &resolved);

        assert_eq!(client.get_current_model(), "model-b");
        assert_eq!(client.base_url, "https://example.invalid/v1");
    }

    /// 空模型名直接拒掉。
    #[tokio::test]
    async fn an_empty_model_is_rejected() {
        let client = client();
        assert!(resolve("  ", None, &client).await.is_err());
    }
}
