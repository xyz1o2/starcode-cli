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

/// 把用户输入拆成 `(provider_id, model_name)`。
///
/// # 为什么不能看见 `/` 就拆
///
/// 带命名空间的模型 id 是真实存在的（`opencode-zen/space-bunny-free`、
/// `openrouter/xiaomi/mimo-v2.6-flash`），无条件按第一个 `/` 拆会把
/// `opencode-zen/space-bunny-free` 读成"provider 是 opencode-zen" —— 而那根本不是
/// 一个已知的 provider id，于是要么报错、要么静默落到当前 provider，用户完全
/// 不知道自己写的东西被曲解了。
///
/// 规则：**只有左边那一段确实是已知 provider 时才按 `provider/model` 拆，
/// 否则整串当模型名。** 这样两种写法都成立，且不必用户记住该用哪种。
pub(crate) fn split_provider_qualified_model(
    input: &str,
    configured_providers: &[String],
) -> (Option<String>, String) {
    let raw = input.trim();
    let Some((head, tail)) = raw.split_once('/') else {
        return (None, raw.to_string());
    };
    let head = head.trim();
    let tail = tail.trim();
    if head.is_empty() || tail.is_empty() {
        return (None, raw.to_string());
    }
    // 左边必须是一个**真实的** provider：内建的，或用户配过的自定义 id。
    //
    // 不能用 `canonical_provider_id` —— 它对任意非空串都返回 `Some(小写)`，
    // 那是个"规范化"函数，不是"认不认识"函数，用它当判据等于永远为真，规则就废了。
    // `normalize_provider_id` 才是真的在查表（id / 显示名 / 别名），认不出来的返回
    // `None`。自定义 id 由调用方另判（见 `use_model`）。
    let known = crate::core::config::providers::normalize_provider_id(head).is_some()
        || configured_providers.iter().any(|id| id == head);
    if known {
        (Some(head.to_string()), tail.to_string())
    } else {
        (None, raw.to_string())
    }
}

async fn use_model(mut ctx: CommandContext<'_>, model_id: String) -> CommandResult {
    // Bare model names resolve against the worker-confirmed active provider. Disk settings are
    // persistence output, not runtime authority, so they must not select this transition.
    let (qualified_provider, model_name) =
        split_provider_qualified_model(&model_id, &ctx.state.configured_providers);
    let (provider_id, model_name) =
        match qualified_provider {
            Some(provider_id) => (provider_id, model_name),
            None => match ctx.state.current_provider_id.clone() {
                Some(provider_id) => (provider_id, model_name),
                None => return Err(
                    "No active provider is confirmed yet. Specify `provider/model` or wait for \
                     startup to finish."
                        .to_string(),
                ),
            },
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

    /// 回归：带命名空间、但那段**不是**已知 provider 的模型 id，必须整串当模型名。
    ///
    /// 真实事故：`/model acme-internal/claude-x` 这类写法，被读成"provider 是
    /// acme-internal"，于是要么报"provider 未配置"、要么静默落到当前 provider，
    /// 用户完全不知道自己写的东西被曲解了。
    #[test]
    fn namespaced_model_ids_are_not_split_as_provider_model() {
        for input in [
            "acme-internal/claude-x",
            "my-gateway/qwen-max",
            "not-a-provider/gpt-5",
        ] {
            let (provider, model) = split_provider_qualified_model(input, &[]);
            assert_eq!(provider, None, "{input:?} 的左半段不是 provider");
            assert_eq!(model, input, "{input:?} 必须整串当模型名");
        }
    }

    /// `opencode-zen` 看着像命名空间，其实是 `opencode` 的别名（`normalize_provider_id`
    /// 把 `-`/`_`/空格都压掉再匹配别名表）。所以 `opencode-zen/space-bunny-free`
    /// **应当**被拆成 provider=opencode + model=space-bunny-free —— 这一条钉住的
    /// 是"别名也算已知 provider"，别让上面的规则把它一起误伤。
    #[test]
    fn a_provider_alias_prefix_still_splits() {
        let (provider, model) =
            split_provider_qualified_model("opencode-zen/space-bunny-free", &[]);
        assert_eq!(
            provider.as_deref(),
            Some("opencode-zen"),
            "别名是合法 provider 前缀，必须拆"
        );
        assert_eq!(model, "space-bunny-free");
    }

    /// 反向：真正的 provider 前缀仍然要拆 —— 免鉴权内建也在内。
    #[test]
    fn an_actual_provider_prefix_is_still_split() {
        for head in ["opencode", "openai", "anthropic", "deepseek"] {
            let (provider, model) =
                split_provider_qualified_model(&format!("{head}/some-model"), &[]);
            assert_eq!(
                provider.as_deref(),
                Some(head),
                "{head} 是内建 provider，必须能拆"
            );
            assert_eq!(model, "some-model");
        }
    }

    /// 没有 `/` 的名字照旧不带 provider（由调用方落到当前 provider）。
    #[test]
    fn a_bare_name_carries_no_provider() {
        let (provider, model) = split_provider_qualified_model("gpt-5", &[]);
        assert_eq!(provider, None);
        assert_eq!(model, "gpt-5");
    }

    /// 两段以上（`openrouter/xiaomi/mimo`）只在第一段是 provider 时才拆，
    /// 剩下的整段是模型名 —— OpenRouter 风格的 id 本来就带斜杠。
    #[test]
    fn a_multi_segment_id_keeps_its_tail_intact() {
        let (provider, model) =
            split_provider_qualified_model("openrouter/xiaomi/mimo-v2.6-flash", &[]);
        assert_eq!(provider.as_deref(), Some("openrouter"));
        assert_eq!(model, "xiaomi/mimo-v2.6-flash");
    }

    /// 前后缺段时不该吞掉用户输入。
    #[test]
    fn a_malformed_qualified_name_is_taken_literally() {
        for input in ["/model", "provider/", "  /x  "] {
            let (provider, model) = split_provider_qualified_model(input, &[]);
            assert_eq!(provider, None, "不该从 {input:?} 里拆出 provider");
            assert_eq!(model, input.trim());
        }
    }

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
