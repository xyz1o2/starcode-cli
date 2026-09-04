use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::provider_store::ProviderStore;
use crate::core::config::providers::get_provider_by_id;
use crate::core::config::settings_manager::get_settings_manager;
use crate::core::config::storage::Storage;
use crate::core::i18n;
use crate::types::ChatEntry;
use std::path::PathBuf;

pub async fn model(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let content = format!("Current model: {}", ctx.state.current_model);
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

pub async fn settings(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let settings_manager = get_settings_manager().await.map_err(|e| e.to_string())?;

    let sub = args.get(0).map(|s| s.to_lowercase());
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
                    .set_placeholder_text(&crate::ui::utils::text::input_placeholder_text());

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
