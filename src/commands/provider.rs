use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::models::{
    ProviderConfig as StoredProviderConfig, ProviderSettings as StoredProviderSettings,
};
use crate::core::config::provider_resolution::{
    resolve_effective_provider_settings, ProviderResolutionInputs, ResolvedValue, SourceRef,
    SRC_CLI_MODEL, SRC_ENV_STAR_API_KEY, SRC_ENV_STAR_BASE_URL, SRC_ENV_STAR_MODEL,
    SRC_ENV_STAR_OPENAI_COMPATIBLE, SRC_PROVIDER_DEFAULT_BASE_URL, SRC_PROVIDER_ENV_API_KEY,
    SRC_PROVIDER_RULE_OPENAI_COMPATIBLE, SRC_PROVIDER_STORE_ACTIVE_MODEL,
    SRC_PROVIDER_STORE_ACTIVE_PROVIDER, SRC_PROVIDER_STORE_API_KEY, SRC_PROVIDER_STORE_BASE_URL,
    SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE, SRC_SESSION_MODEL, SRC_SESSION_PROVIDER,
    SRC_USER_SETTINGS_API_KEY, SRC_USER_SETTINGS_BASE_URL, SRC_USER_SETTINGS_DEFAULT_MODEL,
    SRC_USER_SETTINGS_OPENAI_COMPATIBLE,
};
use crate::core::config::provider_store::ProviderStore;
use crate::core::config::providers::{self, get_provider_by_id, ALL_PROVIDERS};
use crate::core::config::settings_manager::{get_settings_manager, UserSettings};
use crate::core::i18n;
use crate::types::ChatEntry;
use crate::ui::state::ChatState;
use crate::ui::utils::status::current_provider_id;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ProviderCommand {
    /// Select a provider and optionally set its API key
    Select {
        /// Provider ID (e.g., "anthropic", "deepseek")
        #[arg(required = true)]
        provider_id: String,

        /// API Key (optional, if not set previously)
        api_key: Option<String>,
    },
    /// List all available providers
    List,
    /// Diagnose the currently effective provider configuration
    Doctor,
    /// Set API Key for a provider
    SetKey {
        /// Provider ID
        #[arg(required = true)]
        provider_id: String,

        /// API Key
        #[arg(required = true)]
        api_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderDoctorDiagnosis {
    provider_id: Option<String>,
    provider_name: String,
    provider_source: SourceRef,
    model: ResolvedValue,
    base_url: ResolvedValue,
    api_key: ResolvedValue,
    openai_compatible: bool,
    openai_compatible_source: SourceRef,
    api_key_env_hint: Option<String>,
    input_lines: Vec<String>,
    session_provider_id: Option<String>,
    stored_active_provider_id: Option<String>,
}

fn resolve_provider_command_id(input: &str, config: &StoredProviderConfig) -> String {
    let trimmed = input.trim();
    if let Some(normalized) = providers::normalize_provider_id(trimmed) {
        return normalized;
    }

    config
        .providers
        .keys()
        .find(|provider_id| provider_id.eq_ignore_ascii_case(trimmed))
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

fn user_settings_path_hint() -> String {
    crate::core::config::storage::Storage::global_star_dir()
        .join("user-settings.json")
        .display()
        .to_string()
}

pub async fn execute_provider_command(
    ctx: CommandContext<'_>,
    cmd: ProviderCommand,
) -> CommandResult {
    match cmd {
        ProviderCommand::Select {
            provider_id,
            api_key,
        } => select_provider(ctx, provider_id, api_key).await,
        ProviderCommand::List => list_providers(ctx).await,
        ProviderCommand::Doctor => doctor_provider(ctx).await,
        ProviderCommand::SetKey {
            provider_id,
            api_key,
        } => set_provider_key(ctx, provider_id, api_key).await,
    }
}

async fn select_provider(
    ctx: CommandContext<'_>,
    provider_id: String,
    api_key: Option<String>,
) -> CommandResult {
    let store = ProviderStore::new();
    let config = store.load().await.unwrap_or_default();
    let provider_id = resolve_provider_command_id(&provider_id, &config);

    let built_in = get_provider_by_id(&provider_id);
    let is_custom = config.providers.contains_key(&provider_id);

    if built_in.is_none() && !is_custom {
        return Err(format!("Unknown provider: {}", provider_id));
    }

    if let Some(key) = api_key {
        if !key.trim().is_empty() {
            store
                .set_api_key(&provider_id, &key)
                .await
                .map_err(|e| format!("Failed to save API key: {}", e))?;
        }
    }

    if let Some(metadata) = built_in {
        if metadata.requires_api_key {
            let env_key = providers::resolve_api_key_from_env(&provider_id);
            if env_key.is_none() {
                let stored_key = store
                    .get_api_key(&provider_id)
                    .await
                    .map_err(|e| format!("Failed to read config: {}", e))?;

                if stored_key.is_none() {
                    let env_hint = providers::api_key_env_hint(&provider_id)
                        .unwrap_or_else(|| "UNKNOWN".to_string());
                    ctx.state.chat_history.push(
                        ChatEntry::assistant(format!(
                            "**{}** requires an API Key.\n\nRecommended: save it to local config: `/provider select {} <YOUR_API_KEY>`\n\nOr set environment variable: `{}`.\n\nConfig file: `{}`",
                            metadata.name,
                            provider_id,
                            env_hint,
                            user_settings_path_hint()
                        ))
                        .with_streaming(false),
                    );
                    return Ok(());
                }
            }
        }
    }

    store
        .set_active_provider(&provider_id)
        .await
        .map_err(|e| format!("Failed to set active provider: {}", e))?;

    let new_base_url = store
        .get_base_url(&provider_id)
        .await
        .unwrap_or(None)
        .or_else(|| built_in.and_then(|m| m.default_base_url.map(|s| s.to_string())));

    let new_api_key = providers::resolve_runtime_api_key(
        Some(&provider_id),
        store.get_api_key(&provider_id).await.unwrap_or(None),
    );
    let selected_model = store.get_selected_model(&provider_id).await.unwrap_or(None);

    let _ = ctx
        .agent_tx
        .send(
            crate::runtime::messages::AgentRequest::UpdateProviderConfig {
                provider_id: Some(provider_id.clone()),
                api_key: new_api_key,
                base_url: new_base_url,
                is_openai_compatible: providers::provider_openai_compatible_mode(&provider_id),
                model: selected_model.clone(),
            },
        )
        .await;

    let display_name = built_in.map(|m| m.name).unwrap_or(&provider_id);
    let display_desc = built_in.map(|m| m.description).unwrap_or("Custom Provider");

    ctx.state.chat_history.push(
        ChatEntry::assistant(format!(
            "Switched to **{}** ({})\n\n- provider_id: `{}`\n- config saved to: `{}`{}",
            display_name,
            display_desc,
            provider_id,
            user_settings_path_hint(),
            selected_model
                .as_ref()
                .map(|model| format!("\n- restored_model: `{}`", model))
                .unwrap_or_else(|| "\n- next: choose a model before sending messages".to_string())
        ))
        .with_streaming(false),
    );

    Ok(())
}

async fn list_providers(ctx: CommandContext<'_>) -> CommandResult {
    let store = ProviderStore::new();
    let config = store.load().await.unwrap_or_default();

    use crate::core::config::providers::ProviderCategory;

    // Render a single provider entry as a compact one-liner.
    // Returns (line, is_active) so callers can place it under the right group.
    fn render_provider_line(
        id: &str,
        name: &str,
        description: &str,
        requires_api_key: bool,
        is_active: bool,
        config: &StoredProviderConfig,
    ) -> String {
        let status_icon = if is_active { "🟢" } else { "⚪" };
        let key_status = if !requires_api_key {
            "Not Required"
        } else if providers::resolve_api_key_from_env(id).is_some() {
            "✅ Env"
        } else if let Some(settings) = config.providers.get(id) {
            if providers::normalize_api_key_value(settings.api_key.clone()).is_some() {
                "✅ Config"
            } else {
                "❌ Missing"
            }
        } else {
            "❌ Missing"
        };
        format!(
            "{} **{}** (`{}`) — {} | key: {}",
            status_icon, name, id, description, key_status
        )
    }

    let mut output = String::from("# Available Providers\n\n");
    output.push_str("*Grouped by category. Active provider marked with 🟢.*\n\n");

    let mut listed_ids = std::collections::HashSet::new();

    // Group built-in providers by category, preserving declaration order.
    // Use Vec to keep group order stable (Popular → Chinese → Local).
    let group_order: Vec<(&str, ProviderCategory)> = vec![
        ("Popular", ProviderCategory::Popular),
        ("Chinese", ProviderCategory::Chinese),
        ("Local", ProviderCategory::Local),
    ];

    for (group_title, group_category) in group_order {
        let in_group: Vec<_> = ALL_PROVIDERS
            .iter()
            .filter(|p| p.category == group_category)
            .collect();

        if in_group.is_empty() {
            continue;
        }

        output.push_str(&format!("## {}\n", group_title));
        for provider in in_group {
            listed_ids.insert(provider.id.to_string());
            let is_active = config.active_provider_id.as_deref() == Some(provider.id);
            output.push_str(&format!(
                "- {}\n",
                render_provider_line(
                    provider.id,
                    provider.name,
                    provider.description,
                    provider.requires_api_key,
                    is_active,
                    &config,
                )
            ));
        }
        output.push('\n');
    }

    // Custom providers (user-defined, not built-in) get their own group.
    let custom_providers: Vec<_> = config
        .providers
        .iter()
        .filter(|(id, _)| !listed_ids.contains(id.as_str()))
        .collect();

    if !custom_providers.is_empty() {
        output.push_str("## Custom\n");
        for (id, settings) in custom_providers {
            let is_active = config.active_provider_id.as_deref() == Some(id.as_str());
            let name = settings.name.as_deref().unwrap_or(id);
            let description = settings.description.as_deref().unwrap_or("Custom Provider");
            let requires_key =
                providers::normalize_api_key_value(settings.api_key.clone()).is_some();
            let key_status = if requires_key {
                "✅ Config"
            } else {
                "⚪ Optional"
            };
            let status_icon = if is_active { "🟢" } else { "⚪" };
            output.push_str(&format!(
                "- {} **{}** (`{}`) — {} | type: {} | key: {}\n",
                status_icon,
                name,
                id,
                description,
                settings.r#type.as_deref().unwrap_or("Unknown"),
                key_status
            ));
        }
        output.push('\n');
    }

    output.push_str("---\n");
    output.push_str("Usage: `/provider select <id> [api_key]` — switch active provider\n");
    output.push_str("       `/provider doctor` — diagnose effective config\n");

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(output).with_streaming(false));

    Ok(())
}

async fn doctor_provider(ctx: CommandContext<'_>) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let store = ProviderStore::new();
    let provider_config = store.load().await.unwrap_or_default();

    let diagnosis = build_provider_doctor_diagnosis(ctx.state, &provider_config, &settings);
    let report = render_provider_doctor_report(&diagnosis);

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(report).with_streaming(false));
    Ok(())
}

async fn set_provider_key(
    ctx: CommandContext<'_>,
    provider_id: String,
    api_key: String,
) -> CommandResult {
    let store = ProviderStore::new();
    let config = store.load().await.unwrap_or_default();
    let provider_id = resolve_provider_command_id(&provider_id, &config);

    if get_provider_by_id(&provider_id).is_none() {
        return Err(format!("Unknown provider: {}", provider_id));
    }

    store
        .set_api_key(&provider_id, &api_key)
        .await
        .map_err(|e| format!("Failed to save API key: {}", e))?;

    let config = store.load().await.unwrap_or_default();
    if config.active_provider_id.as_deref() == Some(&provider_id) {
        let base_url = store
            .get_base_url(&provider_id)
            .await
            .unwrap_or(None)
            .or_else(|| {
                get_provider_by_id(&provider_id)
                    .and_then(|m| m.default_base_url.map(|s| s.to_string()))
            });

        let _ = ctx
            .agent_tx
            .send(
                crate::runtime::messages::AgentRequest::UpdateProviderConfig {
                    provider_id: Some(provider_id.clone()),
                    api_key: Some(api_key.clone()),
                    base_url,
                    is_openai_compatible: providers::provider_openai_compatible_mode(&provider_id),
                    model: store.get_selected_model(&provider_id).await.unwrap_or(None),
                },
            )
            .await;
    }

    ctx.state.chat_history.push(
        ChatEntry::assistant(format!(
            "**{}** API Key saved.\n\n- provider_id: `{}`\n- config saved to: `{}`\n- verify with: `/provider doctor`",
            provider_id,
            provider_id,
            user_settings_path_hint()
        ))
        .with_streaming(false),
    );

    Ok(())
}

fn build_provider_doctor_diagnosis(
    state: &ChatState,
    provider_config: &StoredProviderConfig,
    settings: &UserSettings,
) -> ProviderDoctorDiagnosis {
    let resolution = resolve_effective_provider_settings(
        ProviderResolutionInputs {
            session_provider_id: current_provider_id(state),
            session_model: trimmed_non_empty(Some(state.current_model.clone())),
            ..Default::default()
        },
        provider_config,
        settings,
    );
    let stored_settings = resolution
        .provider_id
        .as_deref()
        .and_then(|provider_id| provider_config.providers.get(provider_id));

    ProviderDoctorDiagnosis {
        provider_name: provider_display_name(resolution.provider_id.as_deref(), stored_settings),
        provider_id: resolution.provider_id.clone(),
        provider_source: resolution.provider_source,
        model: resolution.model,
        base_url: resolution.base_url,
        api_key: resolution.api_key,
        openai_compatible: resolution.openai_compatible,
        openai_compatible_source: resolution.openai_compatible_source,
        api_key_env_hint: resolution
            .provider_id
            .as_deref()
            .and_then(providers::api_key_env_hint),
        input_lines: collect_input_lines(
            resolution.provider_id.as_deref(),
            stored_settings,
            settings,
        ),
        session_provider_id: resolution.session_provider_id,
        stored_active_provider_id: resolution.stored_active_provider_id,
    }
}

fn collect_input_lines(
    provider_id: Option<&str>,
    stored_settings: Option<&StoredProviderSettings>,
    settings: &UserSettings,
) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(secret_input_line(
        "`STAR_API_KEY`",
        std::env::var("STAR_API_KEY").ok(),
    ));

    if let Some(provider_id) = provider_id {
        for env_var in providers::api_key_env_candidates(provider_id) {
            lines.push(secret_input_line(
                &format!("`{}`", env_var),
                std::env::var(env_var).ok(),
            ));
        }
    }

    lines.push(secret_input_line(
        &i18n::t(
            "cmd.provider.doctor.input.provider_key",
            "Provider config key",
            "Provider config key",
        ),
        stored_settings.and_then(|settings| settings.api_key.clone()),
    ));
    lines.push(secret_input_line(
        &i18n::t(
            "cmd.provider.doctor.input.legacy_key",
            "Legacy `user-settings.apiKey`",
            "Legacy `user-settings.apiKey`",
        ),
        settings.api_key.clone(),
    ));

    lines.push(value_input_line(
        "`STAR_BASE_URL`",
        std::env::var("STAR_BASE_URL").ok(),
    ));
    lines.push(value_input_line(
        &i18n::t(
            "cmd.provider.doctor.input.provider_base_url",
            "Provider config base URL",
            "Provider config base URL",
        ),
        stored_settings.and_then(|settings| settings.base_url.clone()),
    ));
    lines.push(value_input_line(
        &i18n::t(
            "cmd.provider.doctor.input.legacy_base_url",
            "Legacy `user-settings.baseUrl`",
            "Legacy `user-settings.baseUrl`",
        ),
        settings.base_url.clone(),
    ));

    lines.push(value_input_line(
        "`STAR_OPENAI_COMPATIBLE`",
        std::env::var("STAR_OPENAI_COMPATIBLE").ok(),
    ));

    lines
}

fn build_provider_doctor_notes(diagnosis: &ProviderDoctorDiagnosis) -> Vec<String> {
    let mut notes = Vec::new();
    let provider_id = diagnosis.provider_id.as_deref();

    if provider_id.is_none() {
        notes.push(i18n::t(
            "cmd.provider.doctor.note.no_provider",
            "No effective provider is selected. Use Ctrl+P -> Providers or `/provider select <provider>` first.",
            "No effective provider is selected. Use Ctrl+P -> Providers or `/provider select <provider>` first.",
        ));
        return notes;
    }

    if let (Some(session_provider), Some(stored_provider)) = (
        diagnosis.session_provider_id.as_deref(),
        diagnosis.stored_active_provider_id.as_deref(),
    ) {
        if session_provider != stored_provider {
            notes.push(
                i18n::t(
                    "cmd.provider.doctor.note.provider_mismatch",
                    "Current session provider differs from saved `activeProviderId`. Re-select provider before testing.",
                    "Current session provider differs from saved `activeProviderId`. Re-select provider before testing.",
                )
                .replace("{session}", session_provider)
                .replace("{stored}", stored_provider),
            );
        }
    }

    if let Some(provider_id) = provider_id {
        if get_provider_by_id(provider_id)
            .map(|provider| provider.requires_api_key)
            .unwrap_or(false)
            && diagnosis.api_key.value.is_none()
        {
            let env_hint = diagnosis
                .api_key_env_hint
                .as_deref()
                .unwrap_or("provider-specific env");
            notes.push(
                i18n::t(
                    "cmd.provider.doctor.note.missing_key",
                    "This provider requires an API key, but no effective key found. Check saved key, `STAR_API_KEY`, or `{hint}`.",
                    "This provider requires an API key, but no effective key found. Check saved key, `STAR_API_KEY`, or `{hint}`.",
                )
                .replace("{hint}", env_hint),
            );
        }

        if providers::provider_requires_manual_base_url(provider_id)
            && diagnosis.base_url.value.is_none()
        {
            notes.push(i18n::t(
                "cmd.provider.doctor.note.missing_base_url",
                "This provider requires a manual base URL, but no effective value was found.",
                "This provider requires a manual base URL, but no effective value was found.",
            ));
        }
    }

    if provider_id == Some("kimi-code") {
        if diagnosis.openai_compatible {
            notes.push(i18n::t(
                "cmd.provider.doctor.note.kimi_openai_compatible",
                "`Kimi For Coding` should not run in OpenAI compatible mode; state looks wrong.",
                "`Kimi For Coding` should not run in OpenAI compatible mode; state looks wrong.",
            ));
        }

        if let Some(base_url) = diagnosis.base_url.value.as_deref() {
            if !base_url
                .to_ascii_lowercase()
                .contains("api.kimi.com/coding")
            {
                notes.push(i18n::t(
                    "cmd.provider.doctor.note.kimi_base_url",
                    "`Kimi For Coding` should use `https://api.kimi.com/coding/v1` as its base URL.",
                    "`Kimi For Coding` should use `https://api.kimi.com/coding/v1` as its base URL.",
                ));
            }
        }

        if diagnosis.api_key.source.kind == SRC_PROVIDER_ENV_API_KEY
            && diagnosis.api_key.source.detail.as_deref() == Some("MOONSHOT_API_KEY")
        {
            notes.push(i18n::t(
                "cmd.provider.doctor.note.kimi_moonshot_env",
                "`Kimi For Coding` is currently using `MOONSHOT_API_KEY`. Prefer a dedicated `KIMI_API_KEY` to avoid mixing it with a regular Moonshot key.",
                "`Kimi For Coding` is currently using `MOONSHOT_API_KEY`. Prefer a dedicated `KIMI_API_KEY` to avoid mixing it with a regular Moonshot key.",
            ));
        }
    }

    if provider_id == Some("moonshot") {
        if diagnosis
            .base_url
            .value
            .as_deref()
            .map(|value| value.to_ascii_lowercase().contains("api.kimi.com/coding"))
            .unwrap_or(false)
        {
            notes.push(i18n::t(
                "cmd.provider.doctor.note.moonshot_kimi_base_url",
                "`Moonshot AI` is currently pointing at the `Kimi For Coding` endpoint. This usually means provider and base URL are mixed up.",
                "`Moonshot AI` is currently pointing at the `Kimi For Coding` endpoint. This usually means provider and base URL are mixed up.",
            ));
        }
    }

    if provider_id == Some("anthropic") {
        if diagnosis.base_url.value.as_deref() == Some("https://api.anthropic.com") {
            notes.push(i18n::t(
                "cmd.provider.doctor.note.anthropic_base_url_v1",
                "Anthropic base URL is missing `/v1`. Change it to `https://api.anthropic.com/v1` or requests may hit wrong endpoint and return 403 or `Request not allowed`.",
                "Anthropic base URL is missing `/v1`. Change it to `https://api.anthropic.com/v1` or requests may hit wrong endpoint and return 403 or `Request not allowed`.",
            ));
        }
    }

    if notes.is_empty() {
        notes.push(i18n::t(
            "cmd.provider.doctor.note.ok",
            "No obvious local configuration conflict was found. If 401 persists, the provider is most likely rejecting the current key.",
            "No obvious local configuration conflict was found. If 401 persists, the provider is most likely rejecting the current key.",
        ));
    }

    notes
}

fn render_provider_doctor_report(diagnosis: &ProviderDoctorDiagnosis) -> String {
    let mut report = String::new();
    report.push_str(&i18n::t(
        "cmd.provider.doctor.title",
        "# Provider Doctor\n\n",
        "# Provider Doctor\n\n",
    ));

    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.provider",
            "Effective provider",
            "Effective provider",
        ),
        format_provider_label(diagnosis.provider_id.as_deref(), &diagnosis.provider_name),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.provider_source",
            "Provider source",
            "Provider source",
        ),
        describe_source(&diagnosis.provider_source),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.model",
            "Current model",
            "Current model"
        ),
        format_optional_code(diagnosis.model.value.as_deref()),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.model_source",
            "Model source",
            "Model source"
        ),
        describe_source(&diagnosis.model.source),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t("cmd.provider.doctor.base_url", "Base URL", "Base URL"),
        format_optional_code(diagnosis.base_url.value.as_deref()),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.base_url_source",
            "Base URL source",
            "Base URL source",
        ),
        describe_source(&diagnosis.base_url.source),
    ));
    report.push_str(&format!(
        "- {}: `{}`\n",
        i18n::t(
            "cmd.provider.doctor.openai_compatible",
            "OpenAI Compatible",
            "OpenAI compatible",
        ),
        diagnosis.openai_compatible,
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.openai_compatible_source",
            "Compatibility source",
            "Compatibility source",
        ),
        describe_source(&diagnosis.openai_compatible_source),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.api_key",
            "Effective API key",
            "Effective API key",
        ),
        diagnosis
            .api_key
            .value
            .as_deref()
            .map(mask_secret)
            .map(|value| format!("`{}`", value))
            .unwrap_or_else(|| "-".to_string()),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.api_key_source",
            "API key source",
            "API key source",
        ),
        describe_source(&diagnosis.api_key.source),
    ));
    report.push_str(&format!(
        "- {}: {}\n",
        i18n::t(
            "cmd.provider.doctor.api_key_hint",
            "Provider env hint",
            "Provider env hint",
        ),
        format_optional_code(diagnosis.api_key_env_hint.as_deref()),
    ));

    report.push('\n');
    report.push_str(&i18n::t(
        "cmd.provider.doctor.inputs",
        "## Inputs\n",
        "## Inputs\n",
    ));
    for line in &diagnosis.input_lines {
        report.push_str(line);
        report.push('\n');
    }

    report.push('\n');
    report.push_str(&i18n::t(
        "cmd.provider.doctor.checks",
        "## Checks\n",
        "## Checks\n",
    ));
    for note in build_provider_doctor_notes(diagnosis) {
        report.push_str("- ");
        report.push_str(&note);
        report.push('\n');
    }

    report
}

fn provider_display_name(
    provider_id: Option<&str>,
    stored_settings: Option<&StoredProviderSettings>,
) -> String {
    if let Some(provider_id) = provider_id {
        if let Some(metadata) = get_provider_by_id(provider_id) {
            return metadata.name.to_string();
        }
        if let Some(name) = stored_settings
            .and_then(|settings| settings.name.as_deref())
            .and_then(|value| trimmed_non_empty(Some(value.to_string())))
        {
            return name;
        }
        return provider_id.to_string();
    }

    i18n::t("cmd.provider.doctor.provider_missing", "Not set", "Not set")
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

fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "-".to_string();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 6 {
        return "***".to_string();
    }

    let head_len = if chars.len() <= 10 { 2 } else { 4 };
    let tail_len = if chars.len() <= 10 { 2 } else { 4 };
    let head: String = chars.iter().take(head_len).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(tail_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    format!("{}...{}", head, tail)
}

fn secret_input_line(label: &str, raw_value: Option<String>) -> String {
    match raw_value {
        Some(raw_value) => {
            if let Some(normalized) = providers::normalize_api_key_value(Some(raw_value.clone())) {
                return format!(
                    "- {}: {} {}",
                    label,
                    status_label("set"),
                    format!("`{}`", mask_secret(&normalized))
                );
            }

            if raw_value.trim().is_empty() {
                return format!("- {}: {}", label, status_label("missing"));
            }

            format!("- {}: {}", label, status_label("placeholder"))
        }
        None => format!("- {}: {}", label, status_label("missing")),
    }
}

fn value_input_line(label: &str, raw_value: Option<String>) -> String {
    match trimmed_non_empty(raw_value) {
        Some(value) => format!("- {}: {} `{}`", label, status_label("set"), value),
        None => format!("- {}: {}", label, status_label("missing")),
    }
}

fn status_label(kind: &str) -> String {
    match kind {
        "set" => i18n::t("cmd.provider.doctor.status.set", "set", "set"),
        "placeholder" => i18n::t(
            "cmd.provider.doctor.status.placeholder",
            "placeholder/empty",
            "placeholder/empty",
        ),
        _ => i18n::t("cmd.provider.doctor.status.missing", "missing", "missing"),
    }
}

fn describe_source(source: &SourceRef) -> String {
    match source.kind {
        SRC_SESSION_PROVIDER => i18n::t(
            "cmd.provider.doctor.source.session_provider",
            "current session provider",
            "current session provider",
        ),
        SRC_PROVIDER_STORE_ACTIVE_PROVIDER => i18n::t(
            "cmd.provider.doctor.source.active_provider",
            "saved `activeProviderId`",
            "saved `activeProviderId`",
        ),
        SRC_SESSION_MODEL => i18n::t(
            "cmd.provider.doctor.source.session_model",
            "current session model",
            "current session model",
        ),
        SRC_CLI_MODEL => i18n::t(
            "cmd.provider.doctor.source.cli_model",
            "CLI `--model`",
            "CLI `--model`",
        ),
        SRC_ENV_STAR_MODEL => "`STAR_MODEL`".to_string(),
        SRC_PROVIDER_STORE_ACTIVE_MODEL => i18n::t(
            "cmd.provider.doctor.source.active_model",
            "saved `activeModel`",
            "saved `activeModel`",
        ),
        SRC_USER_SETTINGS_DEFAULT_MODEL => i18n::t(
            "cmd.provider.doctor.source.default_model",
            "`user-settings.defaultModel`",
            "`user-settings.defaultModel`",
        ),
        SRC_ENV_STAR_BASE_URL => "`STAR_BASE_URL`".to_string(),
        SRC_PROVIDER_STORE_BASE_URL => i18n::t(
            "cmd.provider.doctor.source.provider_base_url",
            "provider config base URL",
            "provider config base URL",
        ),
        SRC_PROVIDER_DEFAULT_BASE_URL => i18n::t(
            "cmd.provider.doctor.source.provider_default_base_url",
            "provider default base URL",
            "provider default base URL",
        ),
        SRC_USER_SETTINGS_BASE_URL => i18n::t(
            "cmd.provider.doctor.source.user_base_url",
            "`user-settings.baseUrl`",
            "`user-settings.baseUrl`",
        ),
        SRC_ENV_STAR_API_KEY => "`STAR_API_KEY`".to_string(),
        SRC_PROVIDER_STORE_API_KEY => i18n::t(
            "cmd.provider.doctor.source.provider_key",
            "provider config key",
            "provider config key",
        ),
        SRC_PROVIDER_ENV_API_KEY => {
            let env_var = source.detail.as_deref().unwrap_or("provider env");
            format!(
                "{} `{}`",
                i18n::t(
                    "cmd.provider.doctor.source.provider_env_key",
                    "provider-specific env",
                    "provider-specific env",
                ),
                env_var
            )
        }
        SRC_USER_SETTINGS_API_KEY => i18n::t(
            "cmd.provider.doctor.source.user_key",
            "`user-settings.apiKey`",
            "`user-settings.apiKey`",
        ),
        SRC_PROVIDER_RULE_OPENAI_COMPATIBLE => i18n::t(
            "cmd.provider.doctor.source.provider_rule",
            "provider rule",
            "provider rule",
        ),
        SRC_ENV_STAR_OPENAI_COMPATIBLE => "`STAR_OPENAI_COMPATIBLE`".to_string(),
        SRC_USER_SETTINGS_OPENAI_COMPATIBLE => i18n::t(
            "cmd.provider.doctor.source.user_openai_compatible",
            "`user-settings.isOpenAICompatible`",
            "`user-settings.isOpenAICompatible`",
        ),
        SRC_RUNTIME_DEFAULT_OPENAI_COMPATIBLE => i18n::t(
            "cmd.provider.doctor.source.runtime_default",
            "runtime default",
            "runtime default",
        ),
        _ => i18n::t("cmd.provider.doctor.source.missing", "missing", "missing"),
    }
}

fn format_provider_label(provider_id: Option<&str>, provider_name: &str) -> String {
    match provider_id {
        Some(provider_id) => format!("{} (`{}`)", provider_name, provider_id),
        None => provider_name.to_string(),
    }
}

fn format_optional_code(value: Option<&str>) -> String {
    value
        .map(|value| format!("`{}`", value))
        .unwrap_or_else(|| "-".to_string())
}
