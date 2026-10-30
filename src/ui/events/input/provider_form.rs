//! Provider 配置表单（新增 / 编辑 provider、填 key、删 provider）。
//!
//! 表单是输入区里最重的一块状态机：字段切换、校验、保存、失败回滚都在这儿，
//! 以前和键盘分发挤在同一个 5600 行的文件里。
//!
use super::*;

/// Model 从设置或内置默认值来。API Key 故意不填（见 `open_provider_form`）。
pub(super) async fn prefill_form_from_provider(
    form: &mut crate::ui::state::palette::ProviderFormState,
    provider_id: &str,
) {
    prefill_form_from_provider_with(
        form,
        provider_id,
        &crate::core::config::provider_store::ProviderStore::new(),
    )
    .await
}

/// 预填的真正实现 —— store 由调用方给，便于测试注入隔离路径。
pub(super) async fn prefill_form_from_provider_with(
    form: &mut crate::ui::state::palette::ProviderFormState,
    provider_id: &str,
    store: &crate::core::config::provider_store::ProviderStore,
) {
    use crate::ui::state::palette::{
        ANTHROPIC_COMPATIBLE_LABEL, OPENAI_COMPATIBLE_LABEL, PROVIDER_FORM_TYPES,
    };

    let settings = store
        .load()
        .await
        .unwrap_or_default()
        .providers
        .get(provider_id)
        .cloned();

    // 类型：已保存的设置优先，否则按内置 provider 推断，再不行就默认 OpenAI
    let type_label = settings
        .as_ref()
        .and_then(|s| s.r#type.as_deref())
        .map(|saved_type| {
            if saved_type == "anthropic-compatible" {
                ANTHROPIC_COMPATIBLE_LABEL
            } else {
                OPENAI_COMPATIBLE_LABEL
            }
        });
    let type_label = type_label.unwrap_or_else(|| {
        match crate::core::config::providers::provider_openai_compatible_mode(provider_id) {
            Some(false) => ANTHROPIC_COMPATIBLE_LABEL,
            _ => OPENAI_COMPATIBLE_LABEL,
        }
    });
    debug_assert!(
        PROVIDER_FORM_TYPES.contains(&type_label),
        "类型标签必须在候选列表里，否则 ←/→ 循环会失配"
    );

    let name = settings
        .as_ref()
        .and_then(|s| s.name.as_deref())
        .or_else(|| crate::core::config::providers::get_provider_by_id(provider_id).map(|p| p.name))
        .unwrap_or("");
    let base_url = crate::core::config::providers::resolve_provider_base_url(
        provider_id,
        settings.as_ref().and_then(|s| s.base_url.clone()),
    )
    .unwrap_or_default();
    // 预填整个候选列表（默认项在最前，见 `ProviderSettings::candidate_models`），
    // 逗号分隔进一个字段 —— 用户能一眼看出"这家配了哪几个模型"，也能直接改。
    // 只读 `selected_model` 的话，配了多个模型的用户打开编辑框只会看到一个，
    // 剩下的悄悄"消失"了。
    let model = settings
        .as_ref()
        .map(|s| s.candidate_models())
        .unwrap_or_default()
        .join(", ");

    form.values = vec![
        type_label.to_string(),
        name.to_string(),
        base_url,
        // API Key 留空：编辑时空着等于"保持原 key"，见 submit 的处理
        String::new(),
        model,
    ];
}

/// 把 `values[active]` 灌进 textarea，并按当前字段设占位符。
pub(super) fn sync_textarea_to_active_field(state: &mut ChatState) {
    use crate::ui::state::palette::PROVIDER_FORM_FIELDS;

    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_placeholder_text(" ");
    textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );

    if let Some(value) = state
        .provider_form
        .values
        .get(state.provider_form.active_field)
    {
        if !value.is_empty() {
            textarea.insert_str(value);
        }
    }
    if let Some((label, hint, _)) = PROVIDER_FORM_FIELDS.get(state.provider_form.active_field) {
        // 编辑模式下 API Key 的占位提示含义不同（空着 = 保持原 key）
        let hint = if *label == "API Key" {
            crate::ui::state::palette::api_key_field_hint(state.provider_form.editing_id.is_some())
        } else {
            *hint
        };
        textarea.set_placeholder_text(hint);
    }
    state.modal_textarea = textarea;
}

/// 表单字段切换：当前 textarea 内容存回 `values[active]`，再切到目标字段。
/// `forward = false` 时往上走。字段区两端不循环，但末字段往下进操作行、
/// 操作行往下回 Type（循环一圈，免得 Tab 到了底就卡住）。
pub(super) fn switch_provider_form_field(state: &mut ChatState, forward: bool) {
    use crate::ui::state::palette::PROVIDER_FORM_FIELDS;

    let active = state.provider_form.active_field;
    let on_actions = state.provider_form.on_actions;
    let last = PROVIDER_FORM_FIELDS.len().saturating_sub(1);

    let (next_field, next_on_actions) = match (forward, on_actions, active) {
        (true, true, _) => (0, false),                 // 操作行 → Type
        (true, false, a) if a >= last => (last, true), // 末字段 → 操作行
        (true, false, a) => (a + 1, false),
        (false, true, _) => (last, false), // 操作行 → 末字段
        (false, false, 0) => return,       // Type 顶头：不循环，停住
        (false, false, a) => (a - 1, false),
    };

    // 只要正离开一个文本字段，就得把 textarea 里的内容存走
    if !on_actions && (next_field != active || next_on_actions) {
        save_active_field_to_form(state);
    }
    state.provider_form.active_field = next_field;
    state.provider_form.on_actions = next_on_actions;
    state.provider_form.error = None;
    sync_textarea_to_active_field(state);
}

/// 把当前 textarea 的内容存回 `values[active]`。Type 字段（下标 0）不是文本，
/// textarea 里的东西不该覆盖它，直接跳过。
pub(super) fn save_active_field_to_form(state: &mut ChatState) {
    if state.provider_form.active_field == 0 {
        return;
    }
    let value = collect_modal_input(&state.modal_textarea);
    if let Some(slot) = state
        .provider_form
        .values
        .get_mut(state.provider_form.active_field)
    {
        *slot = value;
    }
}

/// 取消表单、回上一层（Esc 和操作行上的 `‹ Back` 共用）。
///
/// 底层若还压着别的模态（如 Plugins 弹窗），只关输入框留在那里；否则回到
/// 打开表单前的 palette / quick menu。
pub(super) fn cancel_provider_form(state: &mut ChatState) {
    state.input_context = None;
    state.exit_input_modal();
    if !state.is_modal_open() {
        show_palette_mode(state, state.palette_mode.clone());
    }
}

/// 提交 provider 表单（新增或编辑）。
///
/// 保存 provider 配置（一次 load+save，见 `ProviderStore::add_custom_provider`）：
/// 编辑模式沿用原 id，新增模式由名称或 URL host 派生 id。model 留空时走自动
/// 获取：发 `ListModels`（便宜路径，先吃两级缓存），面板进 Model 列表等结果；
/// 网络结果回来时 `list_models_with_mode` 会把列表写回内存缓存和磁盘缓存
/// （`model_cache`），`StreamMessage::ModelsList` 再把 UI 列表换掉 —— 下次开
/// 面板就直接命中缓存，不用再等网络。
pub(super) async fn submit_provider_form(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    save_active_field_to_form(state);

    let form = state.provider_form.clone();
    let is_editing = form.editing_id.is_some();
    let provider_type = form.provider_type().to_string();
    let base_url = normalize_modal_base_url(form.base_url());
    let name = form.name().trim().to_string();
    let model = form.model().trim().to_string();

    if base_url.is_empty() {
        state.provider_form.error = Some("Base URL is required.".to_string());
        return Ok(true);
    }

    let store = crate::core::config::provider_store::ProviderStore::new();

    // 编辑模式沿用原 id；新增模式由名称 / URL host 派生并去重
    let provider_id = match form.editing_id.clone() {
        Some(id) => id,
        None => {
            let base_id = crate::core::config::provider_store::derive_provider_id(&name, &base_url);
            store.resolve_unique_provider_id(&base_id).await
        }
    };

    // 编辑模式下 API Key 留空 = 保持原 key。add_custom_provider 是整字段覆盖写，
    // 传 None 会把已存的 key 抹掉，所以这里把存着的 key 读出来原样写回。
    let mut api_key = normalize_modal_api_key(form.api_key());
    if api_key.is_empty() && is_editing {
        api_key = store
            .get_api_key(&provider_id)
            .await
            .unwrap_or(None)
            .unwrap_or_default();
    }

    // 模型字段是多值的：先取出解析后的名字列表，`add_custom_provider` 只拿首项当
    // **候选列表的头**（`default_model`），其余交给下面的 `set_candidate_models`
    // 写进 `models`。
    //
    // 注意这里写的是 `default_model` 而不是 `selected_model` —— 后者是"当前正在
    // 用的模型"，用户可能在 `/models` 面板里选了列表里的第二项。编辑 provider
    // 是在改**这张表**，不该顺手把用户当前用的模型改回列表头。
    let model_names = crate::core::config::models::split_model_names(&model);
    let default_model = model_names.first().map(String::as_str);

    store
        .add_custom_provider(
            &provider_id,
            if name.is_empty() { None } else { Some(&name) },
            &provider_type,
            Some(&base_url),
            if api_key.is_empty() {
                None
            } else {
                Some(&api_key)
            },
            default_model,
        )
        .await
        .map_err(|e| format!("Failed to save provider: {}", e))?;

    // 必须在 add_custom_provider 之后：那个方法会整体覆盖 `default_model`，
    // 而 `models` 里的手写 `context_window` / `alias` 要原样留着。
    if !store
        .set_candidate_models(&provider_id, &model)
        .await
        .map_err(|e| format!("Failed to save provider models: {}", e))?
    {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Error: provider '{}' vanished while saving models.",
                provider_id
            ),
        );
        return Ok(true);
    }

    // 激活 provider
    let _ = store.set_active_provider(&provider_id).await;
    register_configured_provider(state, &provider_id);
    state.current_provider_id = Some(provider_id.clone());

    let resolved_key = crate::core::config::providers::resolve_runtime_api_key(
        Some(&provider_id),
        if api_key.is_empty() {
            None
        } else {
            Some(api_key)
        },
    );
    let is_openai_compat = provider_type == "openai-compatible";
    let _ = agent_tx
        .send(AgentRequest::UpdateProviderConfig {
            provider_id: Some(provider_id.clone()),
            api_key: resolved_key,
            base_url: Some(base_url.clone()),
            is_openai_compatible: Some(is_openai_compat),
            // worker 只认一个"当前模型"，多出来的那些由 `models` 负责（面板）。
            model: default_model.map(str::to_string),
        })
        .await;

    state.exit_input_modal();
    close_quick_menus(state);
    state.provider_form = crate::ui::state::palette::ProviderFormState::new();

    let verb = if is_editing { "Updated" } else { "Created" };
    let display_name = if name.is_empty() {
        provider_id.as_str()
    } else {
        name.as_str()
    };

    if model_names.is_empty() {
        // 一个模型都没配 = 完全交给 API 自动发现。面板立刻能开，不用等网络。
        state.available_models.clear();
        state.available_models_info.clear();
        state.model_provider_map.clear();
        state.current_model.clear();
        // 先试磁盘缓存：这个 provider 以前拉过的话，面板打开就是完整列表，
        // 既不用闪 "Loading models..."，也不必空等。缓存按 provider 分桶，
        // 所以切回以前用过的 provider 是秒开。
        let seeded = crate::ui::services::stream::seed_models_from_disk_cache(state, &provider_id);
        state.awaiting_models = !seeded;
        state.awaiting_models_since = if seeded {
            None
        } else {
            Some(std::time::Instant::now())
        };
        let _ = agent_tx
            .send(AgentRequest::ListModels { force: false })
            .await;
        state.palette_history.clear();
        state.open_palette(PaletteMode::Model);
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "{} provider '{}' — {}",
                verb,
                display_name,
                if seeded {
                    "model list restored from cache"
                } else {
                    "fetching model list..."
                }
            ),
        );
    } else {
        // 配了名字：面板列出「声明的候选集 + API 实际提供的」，不必等网络 ——
        // 声明的排前面（用户的优先级），API 发现的补在后面。
        //
        // 之前这里是把 `available_models` 整个**替换**成声明的那几个，于是 API
        // 明明还返回了另外十几个模型，面板上却只看得到用户写的那一个 —— 得再点
        // 一次刷新才补得回来。用户描述的"下次再选时候它会只有这一个模型"就是这个。
        crate::ui::services::stream::merge_declared_and_discovered_models(
            state,
            &provider_id,
            &model_names,
        );
        state.available_models_info.clear();

        // 「当前正在用的模型」**不**因为编辑了表单就被改回列表头 ——
        // 用户可能刚在 `/models` 面板里选了列表里的第二项。只有当前模型压根
        // 不在候选集里（用户把它删了）才回落。
        let current_still_valid =
            !state.current_model.is_empty() && model_names.contains(&state.current_model);
        let default_model = if current_still_valid {
            state.current_model.clone()
        } else {
            model_names[0].clone()
        };
        state.current_model = default_model.clone();
        let _ = agent_tx
            .send(AgentRequest::SetModel {
                model: default_model.clone(),
                provider_id: Some(provider_id.clone()),
            })
            .await;
        state.palette_history.clear();
        state.open_palette(PaletteMode::Model);
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "{} provider '{}' — default '{}', {} model{} configured.",
                verb,
                display_name,
                default_model,
                model_names.len(),
                if model_names.len() == 1 { "" } else { "s" }
            ),
        );
    }

    Ok(true)
}

/// 把一个 provider id 记进 UI 的已配置列表，已存在就提到最前（新的在前）。
pub(super) fn register_configured_provider(state: &mut ChatState, provider_id: &str) {
    state.configured_providers.retain(|id| id != provider_id);
    state
        .configured_providers
        .insert(0, provider_id.to_string());
}

/// 清空某个 provider 已保存的 API Key。
///
/// # 为什么必须立刻通知 worker
///
/// 磁盘上的 key 和运行中的 client 是两份状态。只改磁盘的话，正在跑的 client 还会
/// 继续用旧 key 发请求 —— 用户明明点了"清空"，行为却一点没变。`UpdateProviderConfig`
/// 的 `api_key: None` 表示"不变"，所以这里传 `Some(String::new())`：经过
/// `normalize_api_key_value` 之后正好落成 `None`，语义就是"没有 key"。
///
/// # 环境变量不受影响
///
/// `provider_has_usable_credentials` 会回落到 `resolve_api_key_from_env`。清掉存储值
/// 之后，如果对应环境变量还在，请求依然会带上它 —— 这是**正确**的：用户能自己决定
/// 要不要 `unset`。状态栏必须把这件事说清楚，否则"我明明清了，怎么还认证通过"
/// 会变成下一个查不到原因的问题。
pub(super) async fn clear_provider_key(
    state: &mut ChatState,
    provider_id: &str,
    agent_tx: &mpsc::Sender<AgentRequest>,
) {
    let store = crate::core::config::provider_store::ProviderStore::new();
    let mut config = match store.load().await {
        Ok(config) => config,
        Err(e) => {
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("Error: could not read provider config: {}", e),
            );
            return;
        }
    };
    let Some(settings) = config.providers.get_mut(provider_id) else {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Error: provider '{}' has no stored configuration.",
                provider_id
            ),
        );
        return;
    };

    let had_key = settings
        .api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    settings.api_key = None;

    if let Err(e) = store.save(&config).await {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("Error: Failed to clear the API key: {}", e),
        );
        return;
    }

    // 运行中的 client 也要换掉，否则旧 key 会一直发出去。
    let _ = agent_tx
        .send(AgentRequest::UpdateProviderConfig {
            provider_id: Some(provider_id.to_string()),
            // `None` 在这里表示"保持不变"，所以要给一个空串让它归一化成"没有 key"。
            api_key: Some(String::new()),
            base_url: None,
            is_openai_compatible: None,
            model: None,
        })
        .await;

    // Provider 菜单上那枚"⚠ no API key"标记是现算的，重新拉一次名单让它更新。
    let _ = agent_tx.send(AgentRequest::LoadConfiguredProviders).await;

    crate::utils::logging::append_debug_log_line(&format!(
        "[ProviderKey] Cleared the stored API key for '{}' (had one: {})",
        provider_id, had_key
    ));
    crate::ui::app::logic::emit_status_text(
        state,
        0,
        &format!(
            "{}",
            crate::core::i18n::t(
                "provider.key_cleared",
                &format!(
                    "已清空 '{}' 保存的 API Key（环境变量仍可能生效）",
                    provider_id
                ),
                &format!(
                    "Cleared the stored API key for '{}' (environment variables still apply)",
                    provider_id
                ),
            )
        ),
    );
}

/// 删除一个自定义 provider。
///
/// 删除范围是本地配置里 `providers.<id>` 整项 —— endpoint、API key、选中的模型
/// 都在这一项里。如果它正是当前生效的 provider，`active_provider_id` /
/// `active_model` 也要一起清掉，否则下次启动会解析到一个已经不存在的 provider。
///
/// 内置 provider 不走这里（面板里也不给入口）：它们的"配置"只是 key 和 URL，
/// 删掉整项没有意义。
pub(super) async fn delete_custom_provider(
    state: &mut ChatState,
    provider_id: &str,
    agent_tx: &mpsc::Sender<AgentRequest>,
) {
    if crate::core::config::providers::get_provider_by_id(provider_id).is_some() {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Warning: '{}' is a built-in provider and cannot be removed.",
                provider_id
            ),
        );
        return;
    }

    let store = crate::core::config::provider_store::ProviderStore::new();
    let mut config = store.load().await.unwrap_or_default();
    let existed = config.providers.remove(provider_id).is_some();
    let was_active = config.active_provider_id.as_deref() == Some(provider_id);
    if was_active {
        config.active_provider_id = None;
        config.active_model = None;
    }

    if let Err(e) = store.save(&config).await {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("Error: Failed to remove {}: {}", provider_id, e),
        );
        return;
    }

    state.configured_providers.retain(|id| id != provider_id);
    state.available_models.clear();
    state.available_models_info.clear();
    state.model_provider_map.retain(|_, pid| pid != provider_id);
    // 磁盘缓存里那个桶必须一起删掉。留着的话，全桶扫描会把里面的模型名认领给
    // 这个已经不存在的 provider，切换请求被 worker 以 "Provider '<x>' is not
    // configured." 拒掉 —— 而那个 reason 只落到状态栏、不写日志，用户看到的就是
    // "刚删完 provider，模型就切不动了"。
    crate::agent::model_cache::purge(provider_id);
    if was_active || state.current_provider_id.as_deref() == Some(provider_id) {
        state.current_provider_id = None;
        state.current_model.clear();
        state.pending_model_provider = None;
        state.pending_provider_selected_model = None;
    }

    // 模型列表按 provider 扇出，删完必须重拉，否则列表里还留着已删 provider 的模型。
    let _ = agent_tx
        .send(AgentRequest::ListModels { force: true })
        .await;

    let message = if !existed {
        format!(
            "'{}' was not in the local config; nothing to remove.",
            provider_id
        )
    } else if was_active {
        format!(
            "Removed provider '{}'. It was the active one — pick another provider.",
            provider_id
        )
    } else {
        format!("Removed provider '{}'.", provider_id)
    };
    crate::ui::app::logic::emit_status_text(state, 0, &message);

    // 回到 provider 列表：条目由 `get_items` 现算，重开一次才能反映删除结果。
    state.palette_history.clear();
    state.open_palette(PaletteMode::Provider);
}
