use crate::core::config::external_configs::{self, AdoptedExternalProfile, ExternalConfigs};
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
pub const SRC_ENV_ANTHROPIC_MODEL: &str = "env.ANTHROPIC_MODEL";
pub const SRC_ENV_STAR_SMALL_FAST_MODEL: &str = "env.STAR_SMALL_FAST_MODEL";
pub const SRC_ENV_ANTHROPIC_SMALL_FAST_MODEL: &str = "env.ANTHROPIC_SMALL_FAST_MODEL";
pub const SRC_PROVIDER_STORE_ACTIVE_MODEL: &str = "provider_store.active_model";
pub const SRC_USER_SETTINGS_DEFAULT_MODEL: &str = "user_settings.default_model";
pub const SRC_CLI_STAR_BASE_URL: &str = "cli.STAR_BASE_URL";
pub const SRC_ENV_STAR_BASE_URL: &str = "env.STAR_BASE_URL";
pub const SRC_ENV_ANTHROPIC_BASE_URL: &str = "env.ANTHROPIC_BASE_URL";
pub const SRC_PROVIDER_STORE_BASE_URL: &str = "provider_store.base_url";
pub const SRC_PROVIDER_DEFAULT_BASE_URL: &str = "provider_default.base_url";
pub const SRC_USER_SETTINGS_BASE_URL: &str = "user_settings.base_url";
pub const SRC_CLI_STAR_API_KEY: &str = "cli.STAR_API_KEY";
pub const SRC_ENV_STAR_API_KEY: &str = "env.STAR_API_KEY";
pub const SRC_ENV_ANTHROPIC_API_KEY: &str = "env.ANTHROPIC_API_KEY";
pub const SRC_PROVIDER_STORE_API_KEY: &str = "provider_store.api_key";
pub const SRC_PROVIDER_ENV_API_KEY: &str = "provider_env.api_key";
pub const SRC_USER_SETTINGS_API_KEY: &str = "user_settings.api_key";
pub const SRC_PROVIDER_RULE_OPENAI_COMPATIBLE: &str = "provider_rule.openai_compatible";
pub const SRC_ENV_STAR_OPENAI_COMPATIBLE: &str = "env.STAR_OPENAI_COMPATIBLE";
pub const SRC_USER_SETTINGS_OPENAI_COMPATIBLE: &str = "user_settings.is_openai_compatible";
pub const SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE: &str = "runtime_default.openai_compatible";

/// 兼容层贡献的 SourceRef。命名沿用「来源_字段」，与上面那批保持一致。
pub const SRC_CLAUDE_CODE_MODEL: &str = "claude_code.model";
pub const SRC_CLAUDE_CODE_BASE_URL: &str = "claude_code.base_url";
pub const SRC_CLAUDE_CODE_API_KEY: &str = "claude_code.api_key";
pub const SRC_CLAUDE_CODE_SMALL_FAST_MODEL: &str = "claude_code.small_fast_model";
pub const SRC_CODEX_MODEL: &str = "codex.model";
pub const SRC_CODEX_BASE_URL: &str = "codex.base_url";
pub const SRC_CODEX_API_KEY: &str = "codex.api_key";

/// 兼容层里被采纳的那个工具提供了哪些字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalField {
    Model,
    BaseUrl,
    ApiKey,
    SmallFastModel,
}

impl<'a> AdoptedExternalProfile<'a> {
    fn get(self, field: ExternalField) -> Option<&'a str> {
        match field {
            ExternalField::Model => self.model,
            ExternalField::BaseUrl => self.base_url,
            ExternalField::ApiKey => self.api_key,
            ExternalField::SmallFastModel => self.small_fast_model,
        }
    }

    fn source_kind(self, field: ExternalField) -> &'static str {
        match (self.tool, field) {
            (external_configs::ExternalTool::ClaudeCode, ExternalField::Model) => {
                SRC_CLAUDE_CODE_MODEL
            }
            (external_configs::ExternalTool::ClaudeCode, ExternalField::BaseUrl) => {
                SRC_CLAUDE_CODE_BASE_URL
            }
            (external_configs::ExternalTool::ClaudeCode, ExternalField::ApiKey) => {
                SRC_CLAUDE_CODE_API_KEY
            }
            (external_configs::ExternalTool::ClaudeCode, ExternalField::SmallFastModel) => {
                SRC_CLAUDE_CODE_SMALL_FAST_MODEL
            }
            (external_configs::ExternalTool::Codex, ExternalField::Model) => SRC_CODEX_MODEL,
            (external_configs::ExternalTool::Codex, ExternalField::BaseUrl) => SRC_CODEX_BASE_URL,
            (external_configs::ExternalTool::Codex, ExternalField::ApiKey) => SRC_CODEX_API_KEY,
            // Codex 的 config.toml 没有这个概念，见 `external_configs` 里的说明。
            (external_configs::ExternalTool::Codex, ExternalField::SmallFastModel) => SRC_MISSING,
        }
    }
}

/// 取兼容层里某个字段的值，连同它的 SourceRef。
///
/// 字段缺失时返回 `None`，调用方继续沿自己的优先级链往下找 —— **不**去另一个
/// 工具那里借：Claude Code 的模型配 Codex 的网关，那种组合两边都不认。
fn adopted(
    external: &ExternalConfigs,
    field: ExternalField,
) -> Option<(String, SourceRef)> {
    let profile = external.adopted()?;
    let value = profile.get(field)?.trim();
    if value.is_empty() {
        return None;
    }
    Some((
        value.to_string(),
        SourceRef::new(profile.source_kind(field)),
    ))
}

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
    /// 「小而快」的副模型（`/fast`、标题生成这类旁路调用用）。
    /// 与 `model` 独立解析：主模型没配副模型是很常见的。
    pub small_fast_model: ResolvedValue,
}

#[derive(Debug, Clone, Default)]
struct EnvSnapshot {
    star_model: Option<String>,
    star_base_url: Option<String>,
    star_api_key: Option<String>,
    star_openai_compatible: Option<String>,
    star_small_fast_model: Option<String>,
    // Claude Code compatible env vars
    anthropic_model: Option<String>,
    anthropic_base_url: Option<String>,
    anthropic_api_key: Option<String>,
    anthropic_small_fast_model: Option<String>,
}

impl EnvSnapshot {
    fn capture() -> Self {
        Self {
            star_model: std::env::var("STAR_MODEL").ok(),
            star_base_url: std::env::var("STAR_BASE_URL").ok(),
            star_api_key: std::env::var("STAR_API_KEY").ok(),
            star_openai_compatible: std::env::var("STAR_OPENAI_COMPATIBLE").ok(),
            star_small_fast_model: std::env::var("STAR_SMALL_FAST_MODEL").ok(),
            // Claude Code compatible env vars (fallback when STAR_* not set)
            anthropic_model: std::env::var("ANTHROPIC_MODEL").ok(),
            anthropic_base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            anthropic_small_fast_model: std::env::var("ANTHROPIC_SMALL_FAST_MODEL").ok(),
        }
    }
}

pub fn resolve_effective_provider_settings(
    inputs: ProviderResolutionInputs,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
) -> EffectiveProviderResolution {
    let env = EnvSnapshot::capture();
    resolve_effective_provider_settings_with_env_and_external(
        inputs,
        provider_config,
        settings,
        &env,
        external_configs::load(),
    )
}

/// 同样的解析，但环境与外部配置都由调用方给。测试靠它避开真实进程环境。
fn resolve_effective_provider_settings_with_env_and_external(
    inputs: ProviderResolutionInputs,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
    env: &EnvSnapshot,
    external: &ExternalConfigs,
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
    let provider_id = provider_id.or_else(|| provider_config.providers.keys().next().cloned());
    let stored_settings = provider_id.as_deref().and_then(|provider_id| {
        provider_config.providers.get(provider_id).or_else(|| {
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
        model: resolve_model(&inputs, provider_config, settings, env, external),
        base_url: resolve_base_url(
            &inputs,
            provider_id.as_deref(),
            stored_settings,
            settings,
            env,
            external,
        ),
        api_key: resolve_api_key(
            &inputs,
            provider_id.as_deref(),
            stored_settings,
            settings,
            env,
            external,
        ),
        openai_compatible: resolve_openai_compatible(provider_id.as_deref(), settings, env).0,
        openai_compatible_source: resolve_openai_compatible(provider_id.as_deref(), settings, env)
            .1,
        small_fast_model: resolve_small_fast_model(env, external),
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
    external: &ExternalConfigs,
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

    // Claude Code compatible env var (fallback)
    if let Some(model) = trimmed_non_empty(env.anthropic_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_ENV_ANTHROPIC_MODEL));
    }

    // Claude Code / Codex 的配置文件（fallback）。
    if let Some((model, source)) = adopted(external, ExternalField::Model) {
        return ResolvedValue::present(model, source);
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

/// 「小而快」副模型的解析链。
///
/// 刻意**不**从 `model` 退化：主模型可能是唯一配好的那个，把它抄一份当副模型
/// 等于让 `/fast` 什么也没换。配了就是配了，没配就是没有。
fn resolve_small_fast_model(env: &EnvSnapshot, external: &ExternalConfigs) -> ResolvedValue {
    if let Some(model) = trimmed_non_empty(env.star_small_fast_model.clone()) {
        return ResolvedValue::present(model, SourceRef::new(SRC_ENV_STAR_SMALL_FAST_MODEL));
    }

    if let Some(model) = trimmed_non_empty(env.anthropic_small_fast_model.clone()) {
        return ResolvedValue::present(
            model,
            SourceRef::new(SRC_ENV_ANTHROPIC_SMALL_FAST_MODEL),
        );
    }

    if let Some((model, source)) = adopted(external, ExternalField::SmallFastModel) {
        return ResolvedValue::present(model, source);
    }

    ResolvedValue::missing(SourceRef::new(SRC_MISSING))
}

fn resolve_base_url(
    inputs: &ProviderResolutionInputs,
    provider_id: Option<&str>,
    stored_settings: Option<&StoredProviderSettings>,
    settings: &UserSettings,
    env: &EnvSnapshot,
    external: &ExternalConfigs,
) -> ResolvedValue {
    if let Some(base_url) = trimmed_non_empty(inputs.cli_base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_CLI_STAR_BASE_URL));
    }

    if let Some(base_url) = trimmed_non_empty(env.star_base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_ENV_STAR_BASE_URL));
    }

    // Claude Code compatible env var (fallback)
    if let Some(base_url) = trimmed_non_empty(env.anthropic_base_url.clone()) {
        return ResolvedValue::present(base_url, SourceRef::new(SRC_ENV_ANTHROPIC_BASE_URL));
    }

    // Claude Code / Codex 的配置文件（fallback），见 `external_configs`。
    if let Some((base_url, source)) = adopted(external, ExternalField::BaseUrl) {
        return ResolvedValue::present(base_url, source);
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
    external: &ExternalConfigs,
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

    // Claude Code compatible env var (fallback)
    if let Some(api_key) = providers::normalize_api_key_value(env.anthropic_api_key.clone()) {
        return ResolvedValue::present(api_key, SourceRef::new(SRC_ENV_ANTHROPIC_API_KEY));
    }

    // Claude Code 的 `ANTHROPIC_AUTH_TOKEN` / Codex 的 `auth.json`（fallback），
    // 见 `external_configs`。端点和 key 一律来自被采纳的那一个工具。
    if let Some((api_key, source)) = adopted(external, ExternalField::ApiKey) {
        if let Some(normalized) = providers::normalize_api_key_value(Some(api_key)) {
            return ResolvedValue::present(normalized, source);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::external_configs::{ClaudeCodeConfig, CodexConfig};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// 空快照 = 「一个变量都没导出」。每个用例只填自己关心的那几个，
    /// 于是断言只失败在真正想验的那条链上。
    fn empty_env() -> EnvSnapshot {
        EnvSnapshot::default()
    }

    fn empty_external() -> ExternalConfigs {
        ExternalConfigs::default()
    }

    fn claude_with_model(model: &str) -> ExternalConfigs {
        ExternalConfigs {
            claude: Some(ClaudeCodeConfig {
                source_path: PathBuf::from("/home/u/.claude/settings.json"),
                model: Some(model.to_string()),
                ..Default::default()
            }),
            codex: None,
        }
    }

    fn full_claude() -> ExternalConfigs {
        ExternalConfigs {
            claude: Some(ClaudeCodeConfig {
                source_path: PathBuf::from("/home/u/.claude/settings.json"),
                model: Some("claude-sonnet-4-5".to_string()),
                base_url: Some("https://cc.example.com".to_string()),
                api_key: Some("cc-token".to_string()),
                small_fast_model: Some("claude-haiku-4-5".to_string()),
                tier_models: Default::default(),
            }),
            codex: None,
        }
    }

    fn full_codex() -> ExternalConfigs {
        ExternalConfigs {
            claude: None,
            codex: Some(CodexConfig {
                source_path: PathBuf::from("/home/u/.codex/config.toml"),
                auth_path: Some(PathBuf::from("/home/u/.codex/auth.json")),
                model: Some("gpt-5-codex".to_string()),
                provider_id: Some("my-gateway".to_string()),
                base_url: Some("https://codex.example.com/v1".to_string()),
                env_key: Some("MY_GATEWAY_KEY".to_string()),
                wire_api: Some("responses".to_string()),
                api_key: Some("codex-key".to_string()),
            }),
        }
    }

    fn resolve(
        inputs: ProviderResolutionInputs,
        provider_config: StoredProviderConfig,
        settings: UserSettings,
        env: EnvSnapshot,
        external: ExternalConfigs,
    ) -> EffectiveProviderResolution {
        resolve_effective_provider_settings_with_env_and_external(
            inputs, &provider_config, &settings, &env, &external,
        )
    }

    /// 全部照抄在 Claude Code 里配置过的人，starcode 必须能直接起 ——
    /// 这就是这次兼容要解决的问题。
    #[test]
    fn claude_code_settings_supply_a_usable_configuration() {
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            full_claude(),
        );

        assert_eq!(resolution.model.value.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(resolution.model.source.kind, SRC_CLAUDE_CODE_MODEL);
        assert_eq!(resolution.base_url.value.as_deref(), Some("https://cc.example.com"));
        assert_eq!(resolution.base_url.source.kind, SRC_CLAUDE_CODE_BASE_URL);
        assert_eq!(resolution.api_key.value.as_deref(), Some("cc-token"));
        assert_eq!(resolution.api_key.source.kind, SRC_CLAUDE_CODE_API_KEY);
        assert_eq!(
            resolution.small_fast_model.value.as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(
            resolution.small_fast_model.source.kind,
            SRC_CLAUDE_CODE_SMALL_FAST_MODEL
        );
    }

    #[test]
    fn codex_settings_supply_a_usable_configuration() {
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            full_codex(),
        );

        assert_eq!(resolution.model.value.as_deref(), Some("gpt-5-codex"));
        assert_eq!(resolution.model.source.kind, SRC_CODEX_MODEL);
        assert_eq!(
            resolution.base_url.value.as_deref(),
            Some("https://codex.example.com/v1")
        );
        assert_eq!(resolution.base_url.source.kind, SRC_CODEX_BASE_URL);
        assert_eq!(resolution.api_key.value.as_deref(), Some("codex-key"));
        assert_eq!(resolution.api_key.source.kind, SRC_CODEX_API_KEY);
        // Codex 没有「副模型」这个概念，不该硬凑一个出来。
        assert_eq!(resolution.small_fast_model.value, None);
    }

    /// 显式配置永远压过兼容层：`STAR_MODEL` / `--model` 是用户为 starcode
    /// 亲手设的，被 `~/.claude/settings.json` 盖掉就本末倒置了。
    #[test]
    fn star_model_and_cli_model_beat_the_compat_layer() {
        let env = EnvSnapshot {
            star_model: Some("star-model".to_string()),
            ..empty_env()
        };
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            env,
            full_claude(),
        );
        assert_eq!(resolution.model.value.as_deref(), Some("star-model"));
        assert_eq!(resolution.model.source.kind, SRC_ENV_STAR_MODEL);

        let resolution = resolve(
            ProviderResolutionInputs {
                cli_model: Some("cli-model".to_string()),
                ..Default::default()
            },
            StoredProviderConfig::default(),
            UserSettings::default(),
            EnvSnapshot::default(),
            full_claude(),
        );
        assert_eq!(resolution.model.value.as_deref(), Some("cli-model"));
        assert_eq!(resolution.model.source.kind, SRC_CLI_MODEL);
    }

    /// `ANTHROPIC_*` 是用户为**当前这个 shell** 导出的，比磁盘上的配置文件
    /// 更靠近本次意图，所以仍然排在兼容层前面。
    #[test]
    fn anthropic_env_beat_the_compat_layer() {
        let env = EnvSnapshot {
            anthropic_model: Some("env-model".to_string()),
            ..empty_env()
        };
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            env,
            full_claude(),
        );
        assert_eq!(resolution.model.value.as_deref(), Some("env-model"));
        assert_eq!(resolution.model.source.kind, SRC_ENV_ANTHROPIC_MODEL);
    }

    /// 兼容层填的是空档，不是抢位置：starcode 自己存的 provider 配置
    /// （`selected_model` / `baseUrl` / `apiKey`）必须照旧生效。
    #[test]
    fn the_provider_store_still_wins_over_the_compat_layer() {
        let provider_config = StoredProviderConfig {
            active_provider_id: Some("openai".to_string()),
            active_model: Some("store-model".to_string()),
            providers: HashMap::from([(
                "openai".to_string(),
                StoredProviderSettings {
                    base_url: Some("https://store.example.com".to_string()),
                    api_key: Some("store-key".to_string()),
                    selected_model: Some("store-model".to_string()),
                    ..Default::default()
                },
            )]),
        };

        let resolution = resolve(
            ProviderResolutionInputs::default(),
            provider_config,
            UserSettings::default(),
            empty_env(),
            full_claude(),
        );

        assert_eq!(resolution.model.value.as_deref(), Some("store-model"));
        assert_eq!(resolution.model.source.kind, SRC_PROVIDER_STORE_ACTIVE_MODEL);
        assert_eq!(
            resolution.base_url.value.as_deref(),
            Some("https://store.example.com")
        );
        assert_eq!(resolution.base_url.source.kind, SRC_PROVIDER_STORE_BASE_URL);
        assert_eq!(resolution.api_key.value.as_deref(), Some("store-key"));
        assert_eq!(resolution.api_key.source.kind, SRC_PROVIDER_STORE_API_KEY);
    }

    /// 两个工具都配了模型时只采纳 Claude Code 那一套 —— 端点与 key
    /// 必须跟模型同源，否则拼出来的东西两边都不认。
    #[test]
    fn an_adopted_profile_never_mixes_the_two_tools() {
        let external = ExternalConfigs {
            claude: Some(ClaudeCodeConfig {
                source_path: PathBuf::from("/home/u/.claude/settings.json"),
                // 只配了模型：端点与 key 一律不许从 Codex 借。
                model: Some("claude-sonnet-4-5".to_string()),
                ..Default::default()
            }),
            codex: full_codex().codex,
        };

        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            external,
        );

        assert_eq!(resolution.model.source.kind, SRC_CLAUDE_CODE_MODEL);
        assert_eq!(resolution.base_url.value, None);
        assert_eq!(resolution.base_url.source.kind, SRC_MISSING);
        assert_eq!(resolution.api_key.value, None);
    }

    /// Claude Code 只配了端点、没配模型时不采纳，于是 Codex 那套
    /// （模型 + 端点 + key 齐全）整体接管 —— 而不是 Claude 的端点
    /// 配 Codex 的模型。
    #[test]
    fn a_model_less_claude_profile_defers_to_a_complete_codex_profile() {
        let external = ExternalConfigs {
            claude: Some(ClaudeCodeConfig {
                source_path: PathBuf::from("/home/u/.claude/settings.json"),
                base_url: Some("https://cc.example.com".to_string()),
                ..Default::default()
            }),
            codex: full_codex().codex,
        };

        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            external,
        );

        assert_eq!(resolution.model.source.kind, SRC_CODEX_MODEL);
        assert_eq!(
            resolution.base_url.value.as_deref(),
            Some("https://codex.example.com/v1")
        );
        assert_eq!(resolution.base_url.source.kind, SRC_CODEX_BASE_URL);
    }

    #[test]
    fn star_small_fast_model_beats_the_claude_one() {
        let env = EnvSnapshot {
            star_small_fast_model: Some("star-fast".to_string()),
            anthropic_small_fast_model: Some("env-fast".to_string()),
            ..empty_env()
        };
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            env,
            full_claude(),
        );
        assert_eq!(resolution.small_fast_model.value.as_deref(), Some("star-fast"));
        assert_eq!(
            resolution.small_fast_model.source.kind,
            SRC_ENV_STAR_SMALL_FAST_MODEL
        );
    }

    /// 没有配副模型就是没有。主模型不能拿来顶 —— 那会让 `/fast`
    /// 「开启」之后模型根本没变，用户看不出自己按了没用。
    #[test]
    fn the_main_model_is_never_substituted_for_the_small_fast_one() {
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            ExternalConfigs {
                claude: Some(ClaudeCodeConfig {
                    model: Some("claude-sonnet-4-5".to_string()),
                    ..Default::default()
                }),
                codex: None,
            },
        );

        assert_eq!(resolution.model.value.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(resolution.small_fast_model.value, None);
        assert_eq!(resolution.small_fast_model.source.kind, SRC_MISSING);
    }

    /// 兼容层缺的那个字段要能落到 starcode 自己的默认值上，而不是整条链断掉。
    #[test]
    fn a_missing_compat_field_falls_through_to_user_settings() {
        let settings = UserSettings {
            base_url: Some("https://user-settings.example.com".to_string()),
            api_key: Some("user-settings-key".to_string()),
            default_model: Some("user-settings-model".to_string()),
            ..Default::default()
        };

        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            settings,
            empty_env(),
            full_claude(),
        );

        // 模型来自兼容层，端点与 key 没配到那儿 —— 落到用户设置。
        assert_eq!(resolution.model.source.kind, SRC_CLAUDE_CODE_MODEL);
        assert_eq!(
            resolution.base_url.value.as_deref(),
            Some("https://user-settings.example.com")
        );
        assert_eq!(resolution.base_url.source.kind, SRC_USER_SETTINGS_BASE_URL);
        assert_eq!(
            resolution.api_key.value.as_deref(),
            Some("user-settings-key")
        );
        assert_eq!(resolution.api_key.source.kind, SRC_USER_SETTINGS_API_KEY);
    }

    /// 兼容层里那串凭据要走一遍 `normalize_api_key_value`：否则
    /// `"  "`、`API_KEY_NOT_SET` 这类占位值会被当成真 key 发出去。
    #[test]
    fn a_placeholder_key_in_the_compat_layer_is_rejected() {
        let external = ExternalConfigs {
            claude: Some(ClaudeCodeConfig {
                model: Some("claude-sonnet-4-5".to_string()),
                api_key: Some("API_KEY_NOT_SET".to_string()),
                ..Default::default()
            }),
            codex: None,
        };
        let settings = UserSettings {
            api_key: Some("user-settings-key".to_string()),
            ..Default::default()
        };

        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            settings,
            empty_env(),
            external,
        );

        assert_eq!(resolution.api_key.value.as_deref(), Some("user-settings-key"));
        assert_eq!(resolution.api_key.source.kind, SRC_USER_SETTINGS_API_KEY);
    }

    /// 什么外部配置都没有时，行为与加兼容层之前逐字一致。
    #[test]
    fn nothing_configured_anywhere_still_resolves_to_missing() {
        let resolution = resolve(
            ProviderResolutionInputs::default(),
            StoredProviderConfig::default(),
            UserSettings::default(),
            empty_env(),
            empty_external(),
        );

        assert_eq!(resolution.model.source.kind, SRC_MISSING);
        assert_eq!(resolution.base_url.source.kind, SRC_MISSING);
        assert_eq!(resolution.api_key.source.kind, SRC_MISSING);
        assert_eq!(resolution.small_fast_model.source.kind, SRC_MISSING);
    }
}
