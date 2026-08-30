use crate::core::config::models::{
    ProviderConfig as StoredProviderConfig, ProviderSettings as StoredProviderSettings,
};
use crate::core::config::providers;
use crate::core::config::settings_manager::UserSettings;

pub const SRC_MISSING: &str = "missing";
pub const SRC_SESSION_PROVIDER: &str = "session.current_provider";
pub const SRC_PROVIDER_STORE_ACTIVE_PROVIDER: &str = "provider_store.active_provider_id";
pub const SRC_SESSION_MODEL: &str = "session.current_model";
pub const SRC_CLI_MODEL: &str = "cli.model";
pub const SRC_ENV_STAR_MODEL: &str = "env.STAR_MODEL";
pub const SRC_PROVIDER_STORE_ACTIVE_MODEL: &str = "provider_store.active_model";
pub const SRC_USER_SETTINGS_DEFAULT_MODEL: &str = "user_settings.default_model";
pub const SRC_CLI_STAR_BASE_URL: &str = "cli.STAR_BASE_URL";
pub const SRC_ENV_STAR_BASE_URL: &str = "env.STAR_BASE_URL";
pub const SRC_PROVIDER_STORE_BASE_URL: &str = "provider_store.base_url";
pub const SRC_PROVIDER_DEFAULT_BASE_URL: &str = "provider_default.base_url";
pub const SRC_USER_SETTINGS_BASE_URL: &str = "user_settings.base_url";
pub const SRC_CLI_STAR_API_KEY: &str = "cli.STAR_API_KEY";
pub const SRC_ENV_STAR_API_KEY: &str = "env.STAR_API_KEY";
pub const SRC_PROVIDER_STORE_API_KEY: &str = "provider_store.api_key";
pub const SRC_PROVIDER_ENV_API_KEY: &str = "provider_env.api_key";
pub const SRC_USER_SETTINGS_API_KEY: &str = "user_settings.api_key";
pub const SRC_PROVIDER_RULE_OPENAI_COMPATIBLE: &str = "provider_rule.openai_compatible";
pub const SRC_ENV_STAR_OPENAI_COMPATIBLE: &str = "env.STAR_OPENAI_COMPATIBLE";
pub const SRC_USER_SETTINGS_OPENAI_COMPATIBLE: &str = "user_settings.is_openai_compatible";
pub const SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE: &str = "runtime_default.openai_compatible";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub kind: &'static str,
    pub detail: Option<String>,
}

impl SourceRef {
    pub fn new(kind: &'static str) -> Self {
        Self { kind, detail: None }
    }

    pub fn with_detail(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedValue {
    pub value: Option<String>,
    pub source: SourceRef,
}

impl ResolvedValue {
    pub fn present(value: String, source: SourceRef) -> Self {
        Self {
            value: Some(value),
            source,
        }
    }

    pub fn missing(source: SourceRef) -> Self {
        Self {
            value: None,
            source,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderResolutionInputs {
    pub session_provider_id: Option<String>,
    pub session_model: Option<String>,
    pub cli_model: Option<String>,
    pub cli_base_url: Option<String>,
    pub cli_api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveProviderResolution {
    pub provider_id: Option<String>,
    pub provider_source: SourceRef,
    pub session_provider_id: Option<String>,
    pub stored_active_provider_id: Option<String>,
    pub model: ResolvedValue,
    pub base_url: ResolvedValue,
    pub api_key: ResolvedValue,
    pub openai_compatible: bool,
    pub openai_compatible_source: SourceRef,
}

#[derive(Debug, Clone, Default)]
struct EnvSnapshot {
    star_model: Option<String>,
    star_base_url: Option<String>,
    star_api_key: Option<String>,
    star_openai_compatible: Option<String>,
}

impl EnvSnapshot {
    fn capture() -> Self {
        Self {
            star_model: std::env::var("STAR_MODEL").ok(),
            star_base_url: std::env::var("STAR_BASE_URL").ok(),
            star_api_key: std::env::var("STAR_API_KEY").ok(),
            star_openai_compatible: std::env::var("STAR_OPENAI_COMPATIBLE").ok(),
        }
    }
}

pub fn resolve_effective_provider_settings(
    inputs: ProviderResolutionInputs,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
) -> EffectiveProviderResolution {
    let env = EnvSnapshot::capture();
    resolve_effective_provider_settings_with_env(
        inputs,
        provider_config,
        settings,
        &env,
    )
}

fn resolve_effective_provider_settings_with_env(
    inputs: ProviderResolutionInputs,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
    env: &EnvSnapshot,
) -> EffectiveProviderResolution {
    let session_provider_id = inputs
        .session_provider_id
        .as_deref()
        .and_then(normalize_provider_id_or_keep);
    let stored_active_provider_id = provider_config
        .active_provider_id
        .as_deref()
        .and_then(normalize_provider_id_or_keep);
    let (provider_id, provider_source) = resolve_effective_provider_id(
        session_provider_id.clone(),
        stored_active_provider_id.clone(),
    );
    // When no explicit active provider is set but providers are configured,
    // auto-select the first one so its API key / base URL can be resolved.
    let provider_id = provider_id.or_else(|| {
        provider_config
            .providers
            .keys()
            .next()
            .cloned()
    });
    let stored_settings = provider_id.as_deref().and_then(|provider_id| {
        provider_config
            .providers
            .get(provider_id)
            .or_else(|| {
                provider_config.providers.iter().find_map(|(k, v)| {
                    if k.eq_ignore_ascii_case(provider_id) {
                        Some(v)
                    } else {
                        None
                    }
                })
            })
    });

    EffectiveProviderResolution {
        provider_id: provider_id.clone(),
        provider_source,
        session_provider_id,
        stored_active_provider_id,
        model: resolve_model(&inputs, provider_config, settings, env),
        base_url: resolve_base_url(
            &inputs,
            provider_id.as_deref(),
            stored_settings,
            settings,
            env,
        ),
        api_key: resolve_api_key(
            &inputs,
            provider_id.as_deref(),
            stored_settings,
            settings,
            env,
        ),
        openai_compatible: resolve_openai_compatible(
            provider_id.as_deref(),
            settings,
            env,
        )
        .0,
        openai_compatible_source: resolve_openai_compatible(
            provider_id.as_deref(),
            settings,
            env,
        )
        .1,
    }
}

fn resolve_effective_provider_id(
    session_provider_id: Option<String>,
    stored_active_provider_id: Option<String>,
) -> (Option<String>, SourceRef) {
    if let Some(provider_id) = session_provider_id {
        return (Some(provider_id), SourceRef::new(SRC_SESSION_PROVIDER));
    }

    if let Some(provider_id) = stored_active_provider_id {
        return (
            Some(provider_id),
            SourceRef::new(SRC_PROVIDER_STORE_ACTIVE_PROVIDER),
        );
    }

    (None, SourceRef::new(SRC_MISSING))
}

fn resolve_model(
    inputs: &ProviderResolutionInputs,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
    env: &EnvSnapshot,
) -> ResolvedValue {
    if let Some(model) = trimmed_non_empty(inputs.session_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_SESSION_MODEL));
    }

    if let Some(model) = trimmed_non_empty(inputs.cli_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_CLI_MODEL));
    }

    if let Some(model) = trimmed_non_empty(env.star_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_ENV_STAR_MODEL));
    }

    if let Some(active_provider_id) = provider_config.active_provider_id.as_deref() {
        if let Some(model) = provider_config
            .providers
            .get(active_provider_id)
            .and_then(|provider| trimmed_non_empty(provider.selected_model.clone()))
        {
            return ResolvedValue::present(model, SourceRef::new(SRC_PROVIDER_STORE_ACTIVE_MODEL));
        }
    }

    if let Some(model) = trimmed_non_empty(provider_config.active_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_PROVIDER_STORE_ACTIVE_MODEL));
    }

    if let Some(model) = trimmed_non_empty(settings.default_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_USER_SETTINGS_DEFAULT_MODEL));
    }

    ResolvedValue::missing(SourceRef::new(SRC_MISSING))
}

fn resolve_base_url(
    inputs: &ProviderResolutionInputs,
    provider_id: Option<&str>,
    stored_settings: Option<&StoredProviderSettings>,
    settings: &UserSettings,
    env: &EnvSnapshot,
) -> ResolvedValue {
    if let Some(base_url) = trimmed_non_empty(inputs.cli_base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_CLI_STAR_BASE_URL));
    }

    if let Some(base_url) = trimmed_non_empty(env.star_base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_ENV_STAR_BASE_URL));
    }

    if let Some(base_url) =
        stored_settings.and_then(|settings| trimmed_non_empty(settings.base_url.clone()))
    {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_PROVIDER_STORE_BASE_URL));
    }

    if let Some(provider_id) = provider_id {
        if let Some(base_url) = providers::get_provider_by_id(provider_id)
            .and_then(|provider| provider.default_base_url)
            .and_then(|value| trimmed_non_empty(Some(value.to_string())))
        {
            return ResolvedValue::present(base_url, SourceRef::new(SRC_PROVIDER_DEFAULT_BASE_URL));
        }
    }

    if let Some(base_url) = trimmed_non_empty(settings.base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_USER_SETTINGS_BASE_URL));
    }

    ResolvedValue::missing(SourceRef::new(SRC_MISSING))
}

fn resolve_api_key(
    inputs: &ProviderResolutionInputs,
    provider_id: Option<&str>,
    stored_settings: Option<&StoredProviderSettings>,
    settings: &UserSettings,
    env: &EnvSnapshot,
) -> ResolvedValue {
    if let Some(api_key) = providers::normalize_api_key_value(inputs.cli_api_key.clone()) {
        return ResolvedValue::present(api_key, SourceRef::new(SRC_CLI_STAR_API_KEY));
    }

    if let Some(provider_id) = provider_id {
        if let Some((api_key, env_var)) =
            providers::resolve_api_key_from_env_with_source(provider_id)
        {
            return ResolvedValue::present(
                api_key,
                SourceRef::with_detail(SRC_PROVIDER_ENV_API_KEY, env_var),
            );
        }
    }

    if let Some(api_key) = stored_settings
        .and_then(|settings| providers::normalize_api_key_value(settings.api_key.clone()))
    {
        return ResolvedValue::present(api_key, SourceRef::new(SRC_PROVIDER_STORE_API_KEY));
    }

    if let Some(api_key) = providers::normalize_api_key_value(env.star_api_key.clone()) {
        return ResolvedValue::present(api_key, SourceRef::new(SRC_ENV_STAR_API_KEY));
    }

    if let Some(api_key) = providers::normalize_api_key_value(settings.api_key.clone()) {
        return ResolvedValue::present(api_key, SourceRef::new(SRC_USER_SETTINGS_API_KEY));
    }

    ResolvedValue::missing(SourceRef::new(SRC_MISSING))
}

fn resolve_openai_compatible(
    provider_id: Option<&str>,
    settings: &UserSettings,
    env: &EnvSnapshot,
) -> (bool, SourceRef) {
    if let Some(provider_id) = provider_id {
        if let Some(value) = providers::provider_openai_compatible_mode(provider_id) {
            return (value, SourceRef::new(SRC_PROVIDER_RULE_OPENAI_COMPATIBLE));
        }
    }

    if let Some(value) = parse_truthy_value(env.star_openai_compatible.as_deref()) {
        return (value, SourceRef::new(SRC_ENV_STAR_OPENAI_COMPATIBLE));
    }

    if let Some(value) = settings.is_openai_compatible {
        return (value, SourceRef::new(SRC_USER_SETTINGS_OPENAI_COMPATIBLE));
    }

    (true, SourceRef::new(SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE))
}

fn normalize_provider_id_or_keep(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(providers::normalize_provider_id(trimmed).unwrap_or_else(|| trimmed.to_string()))
    }
}

fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_truthy_value(value: Option<&str>) -> Option<bool> {
    let value = value?;
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(!matches!(normalized.as_str(), "0" | "false" | "off" | "no"))
}

 