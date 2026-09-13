use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::config::provider_store::ProviderStore;
use crate::core::config::providers::{get_provider_by_id, ALL_PROVIDERS};
use crate::types::ChatEntry;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ModelCommand {
    /// List all available models from all providers
    List,
    /// Switch to a specific model (format: provider_id/model_id)
    Use {
        #[arg(required = true)]
        model_id: String,
    },
}

pub async fn execute_model_command(ctx: CommandContext<'_>, cmd: ModelCommand) -> CommandResult {
    match cmd {
        ModelCommand::List => list_models(ctx).await,
        ModelCommand::Use { model_id } => use_model(ctx, model_id).await,
    }
}

async fn list_models(mut ctx: CommandContext<'_>) -> CommandResult {
    // 优先显示实际从 API 获取到的模型列表（含当前模型标记），而不是静态占位文本
    if !ctx.state.available_models.is_empty() {
        let current = &ctx.state.current_model;
        let mut output = String::from("# Available Models\n\n");
        output.push_str(&format!(
            "Current: `{}`\n\n",
            if current.is_empty() { "-" } else { current }
        ));
        let mut sorted = ctx.state.available_models.clone();
        sorted.sort();
        for m in &sorted {
            let marker = if m == current { " ← current" } else { "" };
            let provider = ctx
                .state
                .model_provider_map
                .get(m)
                .map(|p| format!(" ({})", p))
                .unwrap_or_default();
            output.push_str(&format!("- `{}`{}{}\n", m, provider, marker));
        }
        output.push_str("\nTip: switch with `/model <name>` or the model picker.\n");
        // 列表可能来自磁盘缓存，标一下年龄，免得用户对着旧列表找新模型。
        // 措辞复用面板那份格式化函数，两处保持一致。
        if let Some(age) =
            crate::ui::components::palette::format_cache_age(ctx.state.models_list_age_secs())
        {
            output.push_str(&format!(
                "List was fetched {} — refresh via `/model` → `⟳ Fetch model list from API`.\n",
                age
            ));
        }
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(output).with_streaming(false));
        return Ok(());
    }

    let store = ProviderStore::new();
    let config = store.load().await.unwrap_or_default();

    let mut output = String::from("# Available Models\n\n");
    output.push_str(
        "*(Model list not fetched yet — run `/model` to fetch and pick interactively)*\n\n",
    );
    let mut listed_providers = std::collections::HashSet::new();

    // 1. Built-in Providers
    for provider in ALL_PROVIDERS {
        listed_providers.insert(provider.id.to_string());
        output.push_str(&format!("### {} ({})\n", provider.name, provider.id));

        // Models are fetched dynamically from API
        if provider.category == crate::core::config::providers::ProviderCategory::Local {
            output.push_str("- *(Dynamic - check local instance)*\n");
        } else {
            output.push_str("- *(Fetch models via API or see provider docs)*\n");
        }
        output.push_str("\n");
    }

    // 2. Custom Providers
    if !config.providers.is_empty() {
        output.push_str("### Custom / Configured Providers\n");
        for (id, settings) in &config.providers {
            if listed_providers.contains(id) {
                continue;
            }

            let name = settings.name.as_deref().unwrap_or(id);
            output.push_str(&format!("#### {} (`{}`)\n", name, id));

            if let Some(models) = &settings.models {
                for (model_id, model_config) in models {
                    let model_name = model_config.name.as_deref().unwrap_or(model_id);
                    output.push_str(&format!("- `{}` ({})\n", model_id, model_name));
                }
            } else {
                output.push_str("- *(No models explicitly configured)*\n");
            }
            output.push_str("\n");
        }
    }

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(output).with_streaming(false));

    Ok(())
}

async fn use_model(mut ctx: CommandContext<'_>, model_id: String) -> CommandResult {
    // Bare model names resolve against the worker-confirmed active provider. Disk settings are
    // persistence output, not runtime authority, so they must not select this transition.
    let parts: Vec<&str> = model_id.splitn(2, '/').collect();
    let (provider_id, model_name) = if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else if let Some(provider_id) = ctx.state.current_provider_id.clone() {
        (provider_id, parts[0].to_string())
    } else {
        return Err(
            "No active provider is confirmed yet. Specify `provider/model` or wait for startup to finish."
                .to_string(),
        );
    };

    let accepted = crate::ui::events::input::request_runtime_model_change(
        ctx.state,
        model_name.clone(),
        Some(provider_id.clone()),
        ctx.agent_tx,
    )
    .await;
    if !accepted {
        return Err("Runtime settings could not reach the agent worker.".to_string());
    }

    ctx.state.chat_history.push(
        ChatEntry::assistant(format!(
            "Requested switch to model **{}** (Provider: {}). It takes effect after worker confirmation.",
            model_name, provider_id
        ))
        .with_streaming(false),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::messages::AgentRequest;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn bare_model_uses_confirmed_provider_without_changing_active_state() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_model = "active-model".to_string();
        state.current_provider_id = Some("active-provider".to_string());
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        use_model(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            "target-model".to_string(),
        )
        .await
        .unwrap();

        let Some(AgentRequest::UpdateRuntimeSettings(mutation)) = agent_rx.recv().await else {
            panic!("expected revisioned model request");
        };
        assert_eq!(mutation.ui_revision, 1);
        assert_eq!(
            mutation.persistence,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent
        );
        assert_eq!(
            mutation.model,
            Some(crate::runtime::messages::RuntimeModelSelection {
                model: "target-model".to_string(),
                provider_id: Some("active-provider".to_string()),
            })
        );
        assert_eq!(state.current_model, "active-model");
        assert_eq!(
            state.current_provider_id.as_deref(),
            Some("active-provider")
        );
        assert!(matches!(
            state.runtime_settings_persistence,
            Some(crate::ui::state::store::RuntimeSettingsPersistence::Pending { ui_revision: 1 })
        ));
    }
}
