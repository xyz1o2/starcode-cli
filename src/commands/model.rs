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

async fn use_model(ctx: CommandContext<'_>, model_id: String) -> CommandResult {
    // Parse provider/model
    let parts: Vec<&str> = model_id.splitn(2, '/').collect();
    let (provider_id, model_name) = if parts.len() == 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        let store = ProviderStore::new();
        let config = store.load().await.unwrap_or_default();
        if let Some(active) = &config.active_provider_id {
            (active.clone(), parts[0].to_string())
        } else {
            return Err(
                "No active provider is selected. Use `/provider select <provider>` or specify `provider/model`."
                    .to_string(),
            );
        }
    };

    let store = ProviderStore::new();

    // 1. Update Active Provider if changed
    let current_config = store.load().await.unwrap_or_default();
    if current_config.active_provider_id.as_deref() != Some(&provider_id) {
        // Verify provider exists
        let built_in = get_provider_by_id(&provider_id);
        let is_custom = current_config.providers.contains_key(&provider_id);

        if built_in.is_none() && !is_custom {
            return Err(format!("Unknown provider: {}", provider_id));
        }

        store
            .set_active_provider(&provider_id)
            .await
            .map_err(|e| format!("Failed to set active provider: {}", e))?;
    }

    store
        .set_selected_model(&provider_id, &model_name)
        .await
        .map_err(|e| format!("Failed to save selected model: {}", e))?;

    // Update UI state immediately so the model picker / status bar reflects
    // the new model without waiting for the agent to process UpdateModel.
    // The agent's actual client switch is deferred until the current
    // streaming session ends (see session.rs deferred_model), but the UI
    // should not appear frozen during that window.
    ctx.state.current_model = model_name.clone();

    // Notify agent to reload config with new model (processed after current
    // streaming session ends if one is active).
    let _ = ctx
        .agent_tx
        .send(crate::runtime::messages::AgentRequest::UpdateModel {
            model: model_name.to_string(),
            provider_id: Some(provider_id.clone()),
        })
        .await;

    ctx.state.chat_history.push(
        ChatEntry::assistant(format!(
            "✅ Switched to model **{}** (Provider: {})",
            model_name, provider_id
        ))
        .with_streaming(false),
    );

    Ok(())
}
