use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::provider_store::ProviderStore;
use crate::core::config::providers::get_provider_by_id;
use crate::core::config::settings_manager::{get_settings_manager, UserSettings};
use crate::core::config::storage::Storage;
use crate::core::i18n;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// Settings that `/config set` understands. The order is also the help order.
const SETTABLE_KEYS: &str =
    "model, lang, thinking, output-style, context-window, sandbox, sandbox-mode";

const CONFIG_USAGE: &str = "\
Usage:
  /config                         Show the current configuration overview
  /config show                    Same as /config
  /config get <key>               Show one value
  /config set <key> <value>       Change one setting
  /config key=value [key=value…]  Change one or more settings (Claude Code style)
  /config reset                   Reset preferences (keeps credentials/providers)
  /config export [path]           Write a redacted config snapshot
  /config import <path>           Import preferences from a snapshot";

/// `/config` — Claude Code-style configuration command.
///
/// The subcommands are declared in `ALL_COMMANDS`, so they must do what their
/// descriptions say. Before this dispatcher existed `/config set` silently fell
/// through to the overview text and `config-set` printed a fake success message.
pub async fn config(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let Some(first) = args.first() else {
        return settings(ctx, Vec::new()).await;
    };

    match first.to_ascii_lowercase().as_str() {
        "show" | "list" => settings(ctx, Vec::new()).await,
        "get" => {
            let Some(key) = args.get(1) else {
                return reply(ctx, format!("Usage: /config get <key>\n\n{}", CONFIG_USAGE));
            };
            show_value(ctx, key).await
        }
        "set" => set_from_args(ctx, args[1..].to_vec()).await,
        "reset" => reset_preferences(ctx).await,
        "export" => export_snapshot(ctx, args[1..].to_vec()).await,
        "import" => import_snapshot(ctx, args[1..].to_vec()).await,
        "help" | "--help" | "-h" => reply(ctx, CONFIG_USAGE.to_string()),
        // `/config lang en` keeps working exactly like `/settings lang en`.
        "lang" | "language" | "ui-lang" | "ui_language" | "uilanguage" => settings(ctx, args).await,
        other if other.contains('=') => set_from_args(ctx, args).await,
        other => reply(
            ctx,
            format!(
                "Unknown config subcommand: `{}`.\n\n{}",
                other, CONFIG_USAGE
            ),
        ),
    }
}

fn reply(mut ctx: CommandContext<'_>, message: String) -> CommandResult {
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(message).with_streaming(false));
    Ok(())
}

/// Map every accepted spelling to one canonical key.
///
/// Keys are compared case-insensitively and `_` is treated as `-`, so
/// `context_window`, `contextWindow`, and `context-window` all land on the same
/// setting instead of silently doing nothing.
pub(crate) fn canonical_config_key(key: &str) -> Option<&'static str> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "model" | "default-model" | "defaultmodel" | "active-model" | "activemodel" => {
            Some("model")
        }
        "lang" | "language" | "ui-language" | "uilanguage" => Some("lang"),
        "thinking" | "thinking-effort" | "reasoning-effort" | "effort" => Some("thinking"),
        "output-style" | "outputstyle" => Some("output-style"),
        "context-window" | "contextwindow" | "context" => Some("context-window"),
        "sandbox" | "sandbox-enabled" => Some("sandbox"),
        "sandbox-mode" | "sandboxmode" => Some("sandbox-mode"),
        _ => None,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enable" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disable" | "disabled" => Some(false),
        _ => None,
    }
}

fn normalize_thinking(value: &str) -> Option<&'static str> {
    // Use the same spelling table as `/effort` and `STAR_THINKING_EFFORT`, so
    // `/config thinking=xhigh`, `thinking=max`, and `thinking=false` all agree
    // with the runtime instead of each command accepting a different subset.
    let value = value.trim().to_ascii_lowercase();
    let value = match value.as_str() {
        "med" | "normal" => "medium",
        _ => value.as_str(),
    };
    crate::llm::thinking::parse_effort(value).map(|effort| effort.as_str())
}

fn normalize_output_style(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "default" | "normal" | "balanced" => Some("default"),
        "concise" | "short" | "brief" => Some("concise"),
        "verbose" | "detailed" | "explanatory" => Some("verbose"),
        _ => None,
    }
}

/// Apply one `key=value` pair to an in-memory settings object.
///
/// Pure on purpose: the command loads settings, applies every assignment here,
/// and only writes the file once all of them validated. A typo in the second
/// assignment therefore cannot leave the first one half-persisted.
pub(crate) fn apply_config_value(
    settings: &mut UserSettings,
    key: &str,
    raw_value: &str,
) -> Result<(&'static str, String), String> {
    let canonical = canonical_config_key(key).ok_or_else(|| {
        format!(
            "Unknown setting `{}`. Supported settings: {}.",
            key.trim(),
            SETTABLE_KEYS
        )
    })?;
    let value = raw_value.trim();
    if value.is_empty() {
        return Err(format!("Setting `{}` needs a value.", canonical));
    }

    match canonical {
        "model" => {
            if value.split_whitespace().count() != 1 {
                return Err("Model names cannot contain spaces.".to_string());
            }
            settings.default_model = Some(value.to_string());
            settings.active_model = Some(value.to_string());
            Ok((canonical, value.to_string()))
        }
        "lang" => {
            let normalized = i18n::normalize_language_setting(value)
                .ok_or_else(|| "Invalid language. Supported: auto, en, zh-CN.".to_string())?;
            settings.ui_language = Some(normalized.to_string());
            Ok((canonical, normalized.to_string()))
        }
        "thinking" => {
            let normalized = normalize_thinking(value).ok_or_else(|| {
                "Invalid thinking effort. Supported: off, low, medium, high, xhigh.".to_string()
            })?;
            settings.thinking_effort = Some(normalized.to_string());
            Ok((canonical, normalized.to_string()))
        }
        "output-style" => {
            let normalized = normalize_output_style(value).ok_or_else(|| {
                "Invalid output style. Supported: default, concise, verbose.".to_string()
            })?;
            settings.output_style = Some(normalized.to_string());
            Ok((canonical, normalized.to_string()))
        }
        "context-window" => {
            let tokens: u32 = value
                .parse()
                .map_err(|_| "Context window must be a positive integer (tokens).".to_string())?;
            if !(1_000..=10_000_000).contains(&tokens) {
                return Err("Context window must be between 1000 and 10000000 tokens.".to_string());
            }
            settings.context_window = Some(tokens);
            Ok((canonical, tokens.to_string()))
        }
        "sandbox" => {
            let enabled = parse_bool(value)
                .ok_or_else(|| "Sandbox expects on/off (or true/false).".to_string())?;
            settings
                .sandbox
                .get_or_insert_with(crate::core::config::settings_manager::SandboxSettings::default)
                .enabled = enabled;
            Ok((canonical, if enabled { "on" } else { "off" }.to_string()))
        }
        "sandbox-mode" => {
            let mode = value.to_ascii_lowercase();
            if !matches!(
                mode.as_str(),
                "bubblewrap" | "seatbelt" | "docker" | "opensandbox"
            ) {
                return Err(
                    "Invalid sandbox mode. Supported: bubblewrap, seatbelt, docker, opensandbox."
                        .to_string(),
                );
            }
            settings
                .sandbox
                .get_or_insert_with(crate::core::config::settings_manager::SandboxSettings::default)
                .mode = Some(mode.clone());
            Ok((canonical, mode))
        }
        _ => unreachable!("canonical_config_key and apply_config_value disagree"),
    }
}

/// Turn `/config set` arguments into `(key, value)` pairs.
///
/// Both Claude Code's `key=value` form and the more discoverable
/// `set key value` form are accepted. Values may contain spaces in the latter
/// form (`/config set model my model` stays one value); the `=` form must quote
/// by the shell rather than being split here.
pub(crate) fn parse_config_assignments(args: &[String]) -> Result<Vec<(String, String)>, String> {
    if args.is_empty() {
        return Err(format!(
            "Missing key and value.\n\nUsage: /config set <key> <value> (or /config key=value)\nSupported settings: {}",
            SETTABLE_KEYS
        ));
    }

    if args.iter().any(|arg| arg.contains('=')) {
        let mut assignments = Vec::new();
        for arg in args {
            let Some((key, value)) = arg.split_once('=') else {
                return Err(format!(
                    "Mixed syntax: `{}` is not `key=value`. Use either all `key=value` pairs or `/config set <key> <value>`.",
                    arg
                ));
            };
            let key = key.trim();
            if key.is_empty() {
                return Err(format!("Missing key in `{}`.", arg));
            }
            assignments.push((key.to_string(), value.trim().to_string()));
        }
        return Ok(assignments);
    }

    if args.len() < 2 {
        return Err(format!(
            "Missing value for `{}`.\n\nUsage: /config set <key> <value>\nSupported settings: {}",
            args[0], SETTABLE_KEYS
        ));
    }

    Ok(vec![(args[0].clone(), args[1..].join(" "))])
}

/// Last explicit thinking assignment wins, matching how repeated `key=value`
/// pairs behave in the settings object.
fn runtime_thinking_from_applied(
    applied: &[(&'static str, String)],
) -> Option<crate::types::ThinkingEffort> {
    applied
        .iter()
        .rev()
        .find(|(key, _)| *key == "thinking")
        .and_then(|(_, value)| crate::llm::thinking::parse_effort(value))
}

fn runtime_context_from_applied(
    applied: &[(&'static str, String)],
) -> Option<crate::core::context_policy::ContextWindowSelection> {
    applied
        .iter()
        .rev()
        .find(|(key, _)| *key == "context-window")
        .and_then(|(_, value)| value.parse::<u32>().ok())
        .map(crate::core::context_policy::ContextWindowSelection::Fixed)
}

pub(crate) async fn set_from_args(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // `extended::config_set` calls this with the already-stripped argument list;
    // accepting a leading `set` too keeps both entry points honest.
    let args = if args.first().map(|s| s.as_str()) == Some("set") {
        args[1..].to_vec()
    } else {
        args
    };
    let assignments = match parse_config_assignments(&args) {
        Ok(assignments) => assignments,
        Err(message) => return reply(ctx, message),
    };

    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let mut settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;

    let mut applied: Vec<(&'static str, String)> = Vec::new();
    for (key, value) in assignments {
        match apply_config_value(&mut settings, &key, &value) {
            Ok(applied_value) => applied.push(applied_value),
            Err(message) => return reply(ctx, format!("❌ {}\n\n{}", message, CONFIG_USAGE)),
        }
    }

    settings_manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())?;

    // Language takes effect immediately; the old `/settings lang` path does the
    // same reload, so `/config lang=zh-CN` must not require a restart.
    if let Some((_, lang)) = applied.iter().rev().find(|(key, _)| *key == "lang") {
        let resolved = i18n::resolve_ui_language(Some(lang));
        if let Ok(cwd) = std::env::current_dir() {
            let _ = i18n::reload_for_language(resolved, &cwd);
        }
        ctx.state
            .textarea
            .set_placeholder_text(crate::ui::utils::text::input_placeholder_text());
    }

    // A model assignment is a request to the worker, not just a file edit: the
    // next request must use the new model without restarting the session.
    let mut runtime_notes = Vec::new();
    if let Some((_, model)) = applied.iter().rev().find(|(key, _)| *key == "model") {
        let (qualified, model_name) = crate::commands::model::split_provider_qualified_model(
            model,
            &ctx.state.configured_providers,
        );
        let provider_id = qualified.or_else(|| {
            ctx.state
                .current_provider_id
                .clone()
                .or_else(|| settings.active_provider_id.clone())
        });
        if !crate::ui::events::input::request_runtime_model_change(
            ctx.state,
            model_name,
            provider_id,
            ctx.agent_tx,
        )
        .await
        {
            runtime_notes.push(
                "Saved, but the runtime did not accept the model switch; use `/model` after the worker is ready."
                    .to_string(),
            );
        }
    }

    // Thinking effort and context-window overrides live in the runtime, not just
    // in settings. Save-only made `/config thinking=high` look successful while
    // the next request kept the old effort, so send them through the same
    // revisioned path `/effort` and the context palette use.
    let runtime_thinking = runtime_thinking_from_applied(&applied);
    let runtime_context = runtime_context_from_applied(&applied);
    if runtime_thinking.is_some() || runtime_context.is_some() {
        let accepted = crate::ui::events::input::request_runtime_settings(
            ctx.state,
            None,
            runtime_context,
            runtime_thinking,
            ctx.agent_tx,
        )
        .await;
        if !accepted {
            runtime_notes.push(
                "Saved, but the runtime did not accept the thinking/context change; it applies after the worker is ready."
                    .to_string(),
            );
        }
    }

    let mut lines = vec!["✅ Configuration updated:".to_string()];
    lines.extend(
        applied
            .iter()
            .map(|(key, value)| format!("- {}: {}", key, value)),
    );
    for note in runtime_notes {
        lines.push(format!("⚠️ {}", note));
    }
    reply(ctx, lines.join("\n"))
}

async fn show_value(mut ctx: CommandContext<'_>, key: &str) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let Some(canonical) = canonical_config_key(key) else {
        return reply(
            ctx,
            format!(
                "Unknown setting `{}`. Supported settings: {}.",
                key, SETTABLE_KEYS
            ),
        );
    };
    reply(
        ctx,
        format!(
            "{} = {}",
            canonical,
            render_config_value(&settings, canonical)
        ),
    )
}

fn render_config_value(settings: &UserSettings, key: &str) -> String {
    match key {
        "model" => settings
            .active_model
            .as_deref()
            .or(settings.default_model.as_deref())
            .map(str::to_string)
            .unwrap_or_else(|| "(not set)".to_string()),
        "lang" => settings
            .ui_language
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        "thinking" => settings
            .thinking_effort
            .clone()
            .unwrap_or_else(|| "off".to_string()),
        "output-style" => settings
            .output_style
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        "context-window" => settings
            .context_window
            .map(|value| value.to_string())
            .unwrap_or_else(|| "auto".to_string()),
        "sandbox" => match settings.sandbox.as_ref() {
            Some(sandbox) if sandbox.enabled => "on".to_string(),
            _ => "off".to_string(),
        },
        "sandbox-mode" => settings
            .sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.mode.clone())
            .unwrap_or_else(|| "auto".to_string()),
        _ => "(unknown)".to_string(),
    }
}

pub(crate) async fn reset_preferences(mut ctx: CommandContext<'_>) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let mut settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    reset_preferences_in_place(&mut settings);
    settings_manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())?;

    // Reset also means "back to the system language now", not "after restart".
    let resolved = i18n::resolve_ui_language(None);
    if let Ok(cwd) = std::env::current_dir() {
        let _ = i18n::reload_for_language(resolved, &cwd);
    }
    ctx.state
        .textarea
        .set_placeholder_text(crate::ui::utils::text::input_placeholder_text());

    reply(
        ctx,
        "✅ Configuration preferences reset to defaults.\n\
         Credentials, providers, and project settings were left untouched."
            .to_string(),
    )
}

/// Reset only user preferences. Wiping `UserSettings` would also delete API
/// keys and provider definitions, which is never what "reset config" should do.
pub(crate) fn reset_preferences_in_place(settings: &mut UserSettings) {
    settings.default_model = None;
    settings.active_model = None;
    settings.ui_language = None;
    settings.thinking_effort = None;
    settings.output_style = None;
    settings.context_window = None;
    settings.sandbox = None;
}

/// Serialize settings with every credential-looking field replaced. `/config
/// export` used to dump `user-settings.json` verbatim into chat, including the
/// API key and provider keys.
pub(crate) fn redacted_settings_json(settings: &UserSettings) -> Result<String, String> {
    let mut value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    redact_secrets(&mut value);
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

fn redact_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "api_key"
                        | "apikey"
                        | "api-key"
                        | "password"
                        | "secret"
                        | "access_token"
                        | "refresh_token"
                ) {
                    *child = serde_json::Value::String("[redacted]".to_string());
                } else {
                    redact_secrets(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_secrets(item);
            }
        }
        _ => {}
    }
}

async fn export_snapshot(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let path = args
        .first()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".star").join("config-export.json"));

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = redacted_settings_json(&settings)?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| e.to_string())?;
    reply(
        ctx,
        format!(
            "✅ Wrote a redacted configuration snapshot to `{}`.\n\
             API keys are replaced with [redacted].",
            path.display()
        ),
    )
}

pub(crate) async fn import_snapshot(
    mut ctx: CommandContext<'_>,
    args: Vec<String>,
) -> CommandResult {
    let Some(path) = args.first() else {
        return reply(ctx, "Usage: /config import <path>".to_string());
    };
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) => return reply(ctx, format!("❌ Cannot read `{}`: {}", path, error)),
    };
    let imported: UserSettings = match serde_json::from_str(&content) {
        Ok(settings) => settings,
        Err(error) => {
            return reply(
                ctx,
                format!("❌ `{}` is not a valid config snapshot: {}", path, error),
            )
        }
    };

    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
    let mut settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let changed = merge_preferences(&mut settings, &imported);
    if changed.is_empty() {
        return reply(
            ctx,
            format!("No preferences found in `{}`; nothing changed.", path),
        );
    }
    settings_manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())?;
    reply(
        ctx,
        format!(
            "✅ Imported {} preference(s) from `{}`: {}\n\
             Credentials and provider definitions were not imported.",
            changed.len(),
            path,
            changed.join(", ")
        ),
    )
}

/// Copy only non-secret preference fields, returning the names that changed.
pub(crate) fn merge_preferences(
    destination: &mut UserSettings,
    source: &UserSettings,
) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if source.default_model.is_some() {
        destination.default_model = source.default_model.clone();
        changed.push("model");
    }
    if source.active_model.is_some() {
        destination.active_model = source.active_model.clone();
    }
    if source.ui_language.is_some() {
        destination.ui_language = source.ui_language.clone();
        changed.push("lang");
    }
    if source.thinking_effort.is_some() {
        destination.thinking_effort = source.thinking_effort.clone();
        changed.push("thinking");
    }
    if source.output_style.is_some() {
        destination.output_style = source.output_style.clone();
        changed.push("output-style");
    }
    if source.context_window.is_some() {
        destination.context_window = source.context_window;
        changed.push("context-window");
    }
    if source.sandbox.is_some() {
        destination.sandbox = source.sandbox.clone();
        changed.push("sandbox");
    }
    changed
}

pub async fn model(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = format!("Current model: {}", ctx.state.current_model);
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

pub async fn settings(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;

    let sub = args.first().map(|s| s.to_lowercase());
    if let Some(subcmd) = sub.as_deref() {
        if matches!(
            subcmd,
            "lang" | "language" | "ui-lang" | "ui_language" | "uilanguage"
        ) {
            if let Some(raw_lang) = args.get(1) {
                let Some(normalized) = i18n::normalize_language_setting(raw_lang) else {
                    let msg = i18n::t(
                        "cmd.settings.lang.invalid",
                        "❌ 语言无效。支持: auto | en | zh-CN",
                        "❌ Invalid language. Supported: auto | en | zh-CN",
                    );
                    ctx.state
                        .chat_history
                        .push(ChatEntry::assistant(msg).with_streaming(false));
                    return Ok(());
                };

                settings_manager
                    .update_user_setting("uiLanguage", normalized)
                    .await
                    .map_err(|e| e.to_string())?;

                let resolved = i18n::resolve_ui_language(Some(normalized));
                let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
                i18n::reload_for_language(resolved, &cwd).map_err(|e| e.to_string())?;
                ctx.state
                    .textarea
                    .set_placeholder_text(crate::ui::utils::text::input_placeholder_text());

                let template = i18n::t(
                    "cmd.settings.lang.set",
                    "✅ 界面语言已设置为 {lang}（生效: {resolved}）",
                    "✅ UI language set to {lang} (effective: {resolved})",
                );
                let content = template
                    .replace("{lang}", normalized)
                    .replace("{resolved}", resolved.as_code());
                ctx.state
                    .chat_history
                    .push(ChatEntry::assistant(content).with_streaming(false));
                return Ok(());
            }

            return show_settings_help(ctx, settings_manager).await;
        }
    }

    show_settings_help(ctx, settings_manager).await
}

async fn show_settings_help(
    ctx: CommandContext<'_>,
    settings_manager: crate::core::config::settings_manager::SettingsManager,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let settings = settings_manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    let provider_store = ProviderStore::new();
    let provider_config = provider_store.load().await.unwrap_or_default();

    let current = settings.ui_language.as_deref().unwrap_or("auto");
    let resolved = i18n::resolve_ui_language(settings.ui_language.as_deref());
    let storage = Storage::new(cwd.clone());
    let user_settings_path = dirs::home_dir()
        .map(|home| home.join(".star").join("user-settings.json"))
        .unwrap_or_else(|| Storage::global_star_dir().join("user-settings.json"));
    let project_settings_path = storage.workspace_settings_path();
    let system_settings_path = std::env::var("STAR_CLI_SYSTEM_SETTINGS_PATH")
        .map(PathBuf::from)
        .unwrap_or_default();
    let current_model = first_non_empty(&[
        Some(ctx.state.current_model.as_str()),
        provider_config
            .active_provider_id
            .as_ref()
            .and_then(|provider_id| {
                provider_config
                    .providers
                    .get(provider_id)
                    .and_then(|provider| provider.selected_model.as_deref())
            }),
        provider_config.active_model.as_deref(),
        settings.default_model.as_deref(),
    ]);
    let active_provider = describe_active_provider(&provider_config);

    let template = i18n::t(
        "cmd.settings.help",
        "设置概览：\n- 当前模型: {model}\n- 当前 Provider: {provider}\n- 审批模式: {approval}\n- 已配置 Provider 数: {configured}\n- 界面语言: {current}\n- 生效语言: {resolved}\n\n配置文件：\n- 用户: {user_path}\n- 项目: {project_path}\n- 系统: {system_path}\n\n常用入口：\n- Ctrl+P: 打开命令面板\n- Providers: Ctrl+P -> Providers\n- Model: Ctrl+P -> Model\n- 语言: /settings lang <auto|en|zh-CN>\n- 诊断: /doctor",
        "Settings Overview:\n- Current model: {model}\n- Active provider: {provider}\n- Approval mode: {approval}\n- Configured providers: {configured}\n- UI language: {current}\n- Effective language: {resolved}\n\nConfig files:\n- User: {user_path}\n- Project: {project_path}\n- System: {system_path}\n\nQuick actions:\n- Ctrl+P: open command palette\n- Providers: Ctrl+P -> Providers\n- Model: Ctrl+P -> Model\n- Language: /settings lang <auto|en|zh-CN>\n- Diagnostics: /doctor",
    );
    let content = template
        .replace("{model}", current_model)
        .replace("{provider}", &active_provider)
        .replace("{approval}", approval_mode_label(&ctx.state.approval_mode))
        .replace(
            "{configured}",
            &ctx.state.configured_providers.len().to_string(),
        )
        .replace("{current}", current)
        .replace("{resolved}", resolved.as_code())
        .replace("{user_path}", &format_path_status(&user_settings_path))
        .replace(
            "{project_path}",
            &format_path_status(&project_settings_path),
        )
        .replace("{system_path}", &format_path_status(&system_settings_path));

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));

    Ok(())
}

fn first_non_empty<'a>(values: &[Option<&'a str>]) -> &'a str {
    values
        .iter()
        .copied()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("-")
}

fn describe_active_provider(config: &crate::core::config::models::ProviderConfig) -> String {
    let Some(provider_id) = config
        .active_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return "-".to_string();
    };

    if let Some(provider) = get_provider_by_id(provider_id) {
        return format!("{} ({})", provider.name, provider_id);
    }

    if let Some(settings) = config.providers.get(provider_id) {
        if let Some(name) = settings
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return format!("{} ({})", name, provider_id);
        }
    }

    provider_id.to_string()
}

fn approval_mode_label(mode: &crate::types::ApprovalMode) -> &'static str {
    match mode {
        crate::types::ApprovalMode::Default => "Auto",
        crate::types::ApprovalMode::AcceptEdits => "Accept edits",
        crate::types::ApprovalMode::Plan => "Plan",
        crate::types::ApprovalMode::Yolo => "Yolo",
    }
}

fn format_path_status(path: &std::path::Path) -> String {
    let status = if path.exists() { "exists" } else { "missing" };
    format!("{} ({})", path.display(), status)
}

/// `/lang [code]` — 查看或切换界面语言
/// 等同于 `/settings lang [code]`，是更简短的入口
pub async fn lang(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        // No argument: show current language info
        let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;
        let settings = settings_manager
            .load_user_settings()
            .await
            .map_err(|e| e.to_string())?;
        let current = settings.ui_language.as_deref().unwrap_or("auto");
        let resolved = i18n::resolve_ui_language(settings.ui_language.as_deref());
        let msg = i18n::t(
            "cmd.settings.lang.current",
            "当前界面语言: {lang}（生效: {resolved}）\n用法: /lang <auto|en|zh-CN>",
            "Current UI language: {lang} (effective: {resolved})\nUsage: /lang <auto|en|zh-CN>",
        )
        .replace("{lang}", current)
        .replace("{resolved}", resolved.as_code());
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    }

    // Forward to /settings lang <code>
    let mut fwd = vec!["lang".to_string()];
    fwd.extend(args);
    settings(ctx, fwd).await
}

pub async fn theme(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        // 打开交互式主题选择器（↑↓ 实时预览 / Enter 应用 / Esc 取消恢复）
        let themes = crate::ui::components::highlight::theme_picker::available_themes();
        let current_name = ctx.state.theme_manager.current().name.clone();
        ctx.state.open_theme_picker();
        ctx.state.selected_theme_index = themes
            .iter()
            .position(|t| t.name == current_name)
            .unwrap_or(0);
        return Ok(());
    }

    let sub = args[0].clone();
    match sub.as_str() {
        "next" => {
            ctx.state.theme_manager.next_theme();
            let name = ctx.state.theme_manager.current().name.clone();
            ctx.state.chat_history.push(
                ChatEntry::assistant(format!("Switched to theme: {}", name)).with_streaming(false),
            );
        }
        "list" => {
            let themes = ctx.state.theme_manager.list_themes();
            let current_name = ctx.state.theme_manager.current().name.clone();
            let mut lines = vec!["Available themes:".to_string()];
            for name in themes {
                let marker = if name == current_name {
                    " (current)"
                } else {
                    ""
                };
                lines.push(format!("  - {}{}", name, marker));
            }
            ctx.state
                .chat_history
                .push(ChatEntry::assistant(lines.join("\n")).with_streaming(false));
        }
        name => {
            let name_owned = name.to_string();
            if ctx.state.theme_manager.set_theme(&name_owned) {
                ctx.state.chat_history.push(
                    ChatEntry::assistant(format!("Switched to theme: {}", name_owned))
                        .with_streaming(false),
                );
            } else {
                ctx.state.chat_history.push(
                    ChatEntry::assistant(format!(
                        "Unknown theme: {}. Use /theme list to see available themes.",
                        name_owned
                    ))
                    .with_streaming(false),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn runtime_requests_reflect_the_last_applied_value() {
        let applied = vec![
            ("thinking", "high".to_string()),
            ("context-window", "128000".to_string()),
            ("thinking", "max".to_string()),
        ];
        assert_eq!(
            runtime_thinking_from_applied(&applied),
            Some(crate::types::ThinkingEffort::Xhigh)
        );
        assert_eq!(
            runtime_context_from_applied(&applied),
            Some(crate::core::context_policy::ContextWindowSelection::Fixed(
                128_000
            ))
        );

        assert_eq!(
            runtime_thinking_from_applied(&[("lang", "en".to_string())]),
            None,
            "unrelated assignments must not touch runtime effort"
        );
        assert_eq!(
            runtime_context_from_applied(&[("context-window", "auto".to_string())]),
            None
        );
    }

    #[test]
    fn config_keys_accept_the_spellings_users_actually_type() {
        for key in ["model", "default_model", "defaultModel", "active-model"] {
            assert_eq!(canonical_config_key(key), Some("model"), "{key}");
        }
        for key in ["lang", "language", "uiLanguage", "ui_language"] {
            assert_eq!(canonical_config_key(key), Some("lang"), "{key}");
        }
        assert_eq!(
            canonical_config_key("contextWindow"),
            Some("context-window")
        );
        assert_eq!(canonical_config_key("reasoning_effort"), Some("thinking"));
        assert_eq!(canonical_config_key("outputStyle"), Some("output-style"));
        assert_eq!(canonical_config_key("sandbox_enabled"), Some("sandbox"));
        assert_eq!(canonical_config_key("not-a-setting"), None);
    }

    #[test]
    fn applying_values_normalizes_and_updates_the_expected_fields() {
        let mut settings = UserSettings::default();

        assert_eq!(
            apply_config_value(&mut settings, "model", " openai/gpt-5 ").unwrap(),
            ("model", "openai/gpt-5".to_string())
        );
        assert_eq!(settings.default_model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(settings.active_model.as_deref(), Some("openai/gpt-5"));

        assert_eq!(
            apply_config_value(&mut settings, "lang", "zh-cn").unwrap(),
            ("lang", "zh-CN".to_string())
        );
        assert_eq!(settings.ui_language.as_deref(), Some("zh-CN"));

        assert_eq!(
            apply_config_value(&mut settings, "thinking", "HIGH").unwrap(),
            ("thinking", "high".to_string())
        );
        assert_eq!(settings.thinking_effort.as_deref(), Some("high"));
        assert_eq!(
            apply_config_value(&mut settings, "thinking", "max").unwrap(),
            ("thinking", "xhigh".to_string()),
            "config must accept the same top effort spelling as /effort"
        );
        assert_eq!(
            apply_config_value(&mut settings, "thinking", "false").unwrap(),
            ("thinking", "off".to_string())
        );
        assert_eq!(settings.thinking_effort.as_deref(), Some("off"));

        assert_eq!(
            apply_config_value(&mut settings, "output_style", "brief").unwrap(),
            ("output-style", "concise".to_string())
        );
        assert_eq!(settings.output_style.as_deref(), Some("concise"));

        assert_eq!(
            apply_config_value(&mut settings, "context-window", "128000").unwrap(),
            ("context-window", "128000".to_string())
        );
        assert_eq!(settings.context_window, Some(128_000));

        assert_eq!(
            apply_config_value(&mut settings, "sandbox", "on").unwrap(),
            ("sandbox", "on".to_string())
        );
        assert_eq!(settings.sandbox.as_ref().map(|s| s.enabled), Some(true));

        assert_eq!(
            apply_config_value(&mut settings, "sandbox-mode", "DOCKER").unwrap(),
            ("sandbox-mode", "docker".to_string())
        );
        assert_eq!(
            settings.sandbox.as_ref().and_then(|s| s.mode.as_deref()),
            Some("docker")
        );
    }

    #[test]
    fn invalid_values_are_rejected_without_partial_mutation() {
        let mut settings = UserSettings::default();
        for (key, value) in [
            ("model", "two words"),
            ("lang", "klingon"),
            ("thinking", "turbo"),
            ("output-style", "poetic"),
            ("context-window", "0"),
            ("context-window", "abc"),
            ("sandbox", "maybe"),
            ("sandbox-mode", "jail"),
            ("unknown", "value"),
        ] {
            let error = apply_config_value(&mut settings, key, value).unwrap_err();
            assert!(!error.is_empty(), "{key}");
        }
        assert!(settings.default_model.is_none());
        assert!(settings.ui_language.is_none());
        assert!(settings.thinking_effort.is_none());
        assert!(settings.output_style.is_none());
        assert!(settings.context_window.is_none());
        assert!(settings.sandbox.is_none());
    }

    #[test]
    fn assignments_support_both_set_forms() {
        assert_eq!(
            parse_config_assignments(&args(&["model=gpt-5", "lang=zh-CN"])).unwrap(),
            vec![
                ("model".to_string(), "gpt-5".to_string()),
                ("lang".to_string(), "zh-CN".to_string())
            ]
        );
        assert_eq!(
            parse_config_assignments(&args(&["output-style", "very", "concise"])).unwrap(),
            vec![("output-style".to_string(), "very concise".to_string())]
        );
        assert!(parse_config_assignments(&[]).is_err());
        assert!(parse_config_assignments(&args(&["model"])).is_err());
        assert!(
            parse_config_assignments(&args(&["model=x", "lang"])).is_err(),
            "mixed key=value and bare tokens must not be guessed"
        );
        assert!(parse_config_assignments(&args(&["=x"])).is_err());
    }

    #[test]
    fn defaults_are_readable_in_get_output() {
        let settings = UserSettings::default();
        assert_eq!(render_config_value(&settings, "model"), "(not set)");
        assert_eq!(render_config_value(&settings, "lang"), "auto");
        assert_eq!(render_config_value(&settings, "thinking"), "off");
        assert_eq!(render_config_value(&settings, "output-style"), "default");
        assert_eq!(render_config_value(&settings, "context-window"), "auto");
        assert_eq!(render_config_value(&settings, "sandbox"), "off");
        assert_eq!(render_config_value(&settings, "sandbox-mode"), "auto");
    }

    #[test]
    fn export_redacts_keys_at_every_level() {
        let mut settings = UserSettings::default();
        settings.api_key = Some("sk-live-super-secret".to_string());
        let json = redacted_settings_json(&settings).unwrap();
        assert!(!json.contains("sk-live-super-secret"));
        assert!(json.contains("[redacted]"));

        let mut nested = serde_json::json!({
            "api_key": "top",
            "providers": {
                "p": { "apiKey": "nested", "password": "hunter2", "base_url": "https://example.test" }
            }
        });
        redact_secrets(&mut nested);
        let rendered = serde_json::to_string(&nested).unwrap();
        assert!(!rendered.contains("top"));
        assert!(!rendered.contains("nested"));
        assert!(!rendered.contains("hunter2"));
        assert!(
            rendered.contains("https://example.test"),
            "URLs stay visible"
        );
    }

    #[test]
    fn reset_keeps_credentials_and_providers() {
        let mut settings = UserSettings {
            api_key: Some("key".to_string()),
            base_url: Some("https://example.test".to_string()),
            providers: Some(std::collections::HashMap::new()),
            ..UserSettings::default()
        };
        settings.default_model = Some("model".to_string());
        settings.ui_language = Some("zh-CN".to_string());

        reset_preferences_in_place(&mut settings);

        assert!(settings.api_key.is_some());
        assert!(settings.base_url.is_some());
        assert!(settings.providers.is_some());
        assert!(settings.default_model.is_none());
        assert!(settings.ui_language.is_none());
    }

    #[test]
    fn import_merges_only_preferences() {
        let mut destination = UserSettings {
            api_key: Some("local-key".to_string()),
            base_url: Some("https://local.test".to_string()),
            ..UserSettings::default()
        };
        let source = UserSettings {
            api_key: Some("imported-key".to_string()),
            base_url: Some("https://imported.test".to_string()),
            default_model: Some("imported-model".to_string()),
            ui_language: Some("en".to_string()),
            output_style: Some("concise".to_string()),
            ..UserSettings::default()
        };

        let changed = merge_preferences(&mut destination, &source);

        assert_eq!(
            changed,
            vec!["model", "lang", "output-style"],
            "only present preference fields are reported"
        );
        assert_eq!(destination.default_model.as_deref(), Some("imported-model"));
        assert_eq!(destination.ui_language.as_deref(), Some("en"));
        assert_eq!(destination.output_style.as_deref(), Some("concise"));
        assert_eq!(
            destination.api_key.as_deref(),
            Some("local-key"),
            "import must never overwrite credentials"
        );
        assert_eq!(destination.base_url.as_deref(), Some("https://local.test"));
    }
}
