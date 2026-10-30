//! 命令面板动作的执行（execute_palette_action）。
//!
//! 面板条目选中后真正干活的地方：切模型、切 provider、换主题、开关审批模式……
//!
use super::*;

pub(crate) async fn execute_palette_action(
    state: &mut ChatState,
    action: PaletteAction,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        PaletteAction::Navigate(mode) => {
            state.palette_history.push(state.palette_mode.clone());

            if matches!(mode, PaletteMode::Model) && state.available_models.is_empty() {
                // 先用磁盘缓存把列表填上：面板立刻能显示真实模型，
                // 不会先闪一下 "Loading models..." 再等一个多半会命中
                // 缓存的请求回来。随后那条请求照发，拿到新数据再覆盖。
                let pid = state
                    .current_provider_id
                    .clone()
                    .unwrap_or_else(|| crate::agent::model_cache::DEFAULT_BUCKET.to_string());
                crate::ui::services::stream::seed_models_from_disk_cache(state, &pid);

                let _ = agent_tx
                    .send(AgentRequest::ListModels { force: false })
                    .await;
            }

            // Entering from the settings modal (ShowStatus closes the palette)
            // must reopen the palette, otherwise navigation appears to do nothing.
            state.pop_modal();
            let items = crate::ui::components::palette::get_items(&mode, state);
            state.palette_mode = mode;
            state.palette_items = items;
            state.selected_palette_index = 0;
            state.palette_filter.clear();
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::ShowStatus => {
            state.close_palette();
            state.open_status_modal();
        }
        PaletteAction::ShowModelMenu => {
            if state.available_models.is_empty() {
                // 同上：先吃磁盘缓存，避免假 loading。
                let pid = state
                    .current_provider_id
                    .clone()
                    .unwrap_or_else(|| crate::agent::model_cache::DEFAULT_BUCKET.to_string());
                let seeded = crate::ui::services::stream::seed_models_from_disk_cache(state, &pid);
                // 只有缓存也没能给出列表时才标 loading —— 有缓存可显示时
                // 面板已经是完整列表，不该再显示"正在获取"。
                if !seeded {
                    state.awaiting_models = true;
                }
                let _ = agent_tx
                    .send(AgentRequest::ListModels { force: false })
                    .await;
            }
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::RefreshModels => {
            // 显式刷新：跳过内存/磁盘缓存，扇出到所有已配置 provider。
            // 面板留在原地（不 push 历史，否则 Back 会退回自己），只把 "⟳ Fetching..."
            // 的状态刷出来；列表到货后由 StreamMessage::ModelsList 重建 items。
            state.awaiting_models = true;
            if !state.is_palette_open() {
                state.palette_history.clear();
                state.open_palette(crate::ui::state::palette::PaletteMode::Model);
            } else {
                state.palette_mode = crate::ui::state::palette::PaletteMode::Model;
            }
            state.palette_items = crate::ui::components::palette::get_items(
                &crate::ui::state::palette::PaletteMode::Model,
                state,
            );
            let _ = agent_tx
                .send(AgentRequest::ListModels { force: true })
                .await;
        }
        PaletteAction::ShowProviderMenu => {
            open_provider_selection_menu(state, None, true);
        }
        PaletteAction::OpenMcpModal => {
            state.close_palette();
            state.open_mcp_modal();
            crate::ui::state::modal::load_mcp_server_rows(state).await;
        }
        PaletteAction::OpenMarketModal => {
            state.open_market_modal().await;
        }
        PaletteAction::ShowSessionMenu => {
            let sessions = crate::utils::session_manager::list_session_summaries()
                .await
                .unwrap_or_default();
            open_session_selection_menu(state, true, sessions);
        }
        PaletteAction::SetModel { model, provider_id } => {
            // If conversation has history, preserve the existing confirmation gate. Only the
            // eventual worker acknowledgement may change the active model display.
            let has_history = state.chat_history.iter().any(|entry| {
                !entry.is_welcome
                    && matches!(
                        entry.entry_type,
                        ChatEntryType::User | ChatEntryType::Assistant
                    )
                    && !entry.content.trim().is_empty()
            });

            if has_history
                && !state.pending_model_confirmation
                && !state.waive_next_model_confirmation
            {
                state.close_palette();
                // 归属随这次意图一起存下。确认框是有延迟的，等按 `y` 时列表和
                // 磁盘缓存都可能已经变了，那时再推导就可能推到一个已被删除的
                // provider 身上（见 `PendingModelChange`）。
                state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
                    model: model.clone(),
                    provider_id: provider_id.clone(),
                });
                state.pending_model_confirmation = true;
                // 提示本体由状态栏那一行常驻渲染（见 status_line::build_status_lines）——
                // 只写进 `current_status_line` 的话，空闲状态下它根本不显示。
                state.current_status_line = Some(crate::core::i18n::t(
                    "ui.model.confirm_switch",
                    &format!("切换模型到 {}? (y/n)", model),
                    &format!("Switch model to {}? (y/n)", model),
                ));
                return Ok(());
            }

            state.close_palette();
            state.pending_model_confirmation = false;
            state.waive_next_model_confirmation = false;
            let provider_id = provider_id
                .clone()
                .or_else(|| resolve_provider_for_model(state, &model));
            if request_runtime_model_change(state, model.clone(), provider_id, agent_tx).await {
                state.current_status_line = Some(format!("Switching to model {}…", model));
            }
        }
        PaletteAction::SetAgentMode(mode) => {
            state.close_palette();
            let approval_mode = match mode.as_str() {
                "yolo" => crate::types::ApprovalMode::Yolo,
                "plan" => crate::types::ApprovalMode::Plan,
                _ => crate::types::ApprovalMode::Default,
            };
            state.approval_mode = approval_mode.clone();
            let _ = agent_tx
                .send(AgentRequest::SetApprovalMode(approval_mode))
                .await;
        }
        PaletteAction::SetContextWindow(selection) => {
            state.close_palette();
            if request_runtime_settings(state, None, Some(selection), None, agent_tx).await {
                state.current_status_line = Some(format!(
                    "Requesting context window {}…",
                    selection.display()
                ));
            }
        }
        PaletteAction::InputContextWindow => {
            state.close_palette();
            state.enter_input_modal();
            state.input_modal_title = "Context Window Size".to_string();
            state.input_modal_prompt =
                "Enter a positive size (e.g. 128k, 1M, 1048576), or auto:".to_string();
            state.input_modal_value = String::new();
            let mut textarea = tui_textarea::TextArea::default();
            textarea.set_cursor_line_style(ratatui::style::Style::default());
            textarea.set_placeholder_text("128k");
            textarea.set_cursor_style(
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
            );
            state.modal_textarea = textarea;
            state.input_context = Some(crate::ui::state::palette::InputContext::ContextWindow);
        }
        PaletteAction::SetThinkingEffort(effort) => {
            state.close_palette();
            if request_runtime_settings(state, None, None, Some(effort.clone()), agent_tx).await {
                state.current_status_line =
                    Some(format!("Requesting {}…", effort.notification_text()));
            }
        }
        PaletteAction::SetTheme(theme_name) => {
            state.close_palette();
            if state.theme_manager.set_theme(&theme_name) {
                state.current_status_line = Some(format!("Theme: {}", theme_name));
                // Persist to user settings
                tokio::spawn(async move {
                    if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                        if let Ok(mut settings) = mgr.load_user_settings().await {
                            settings.ui_language = Some(theme_name); // Reuse ui_language field for now
                            let _ = mgr.save_user_settings(&settings).await;
                        }
                    }
                });
            }
        }
        PaletteAction::Back => {
            if let Some(prev_mode) = state.palette_history.pop() {
                state.palette_mode = prev_mode.clone();
                state.palette_items = crate::ui::components::palette::get_items(&prev_mode, state);
                state.selected_palette_index = 0;
                state.palette_filter.clear();
            } else {
                state.close_palette();
            }
        }
        PaletteAction::ExecuteCommand(cmd) => {
            state.close_palette();
            state.input = cmd;
            crate::ui::app::logic::enqueue_user_message(state, state.input.clone(), agent_tx)
                .await?;
            state.input.clear();
        }
        PaletteAction::TypeCommand(cmd) => {
            state.close_palette();
            state.input = cmd;
        }
        PaletteAction::SelectProvider(provider_id) => {
            state.available_models.clear();
            // 列表被清空了，但模型面板**马上**就要打开（见本分支末尾的
            // `push_palette_mode(Model)`）。不给 `awaiting_models` 置位，那个面板就会
            // 以"空列表 + 没有加载态"的样子弹出来 —— 用户看到的是一片空白、既没有
            // 模型可选、也没有任何说明在加载。按 Enter 挑不了任何东西，体感就是
            // "彻底没反应"。
            //
            // 这条路径以前**没有**置位，而 `submit_provider_form` 那条有 —— 同一个
            // "打开模型面板"的场景，两条路径的表现不一致。
            state.pending_model_provider = Some(provider_id.clone());
            // 切 provider 是明确意图：紧接着的第一次模型选择免 (y/n) 确认
            state.waive_next_model_confirmation = true;
            // 清除模型缓存，因为切换了 provider
            crate::agent::model_list::clear_model_cache();
            let store = crate::core::config::provider_store::ProviderStore::new();
            let selected_model = store.get_selected_model(&provider_id).await.unwrap_or(None);
            state.pending_provider_selected_model = selected_model.clone();
            state.current_provider_id = Some(provider_id.clone());
            if let Some(model) = selected_model.clone() {
                state.current_model = model;
            } else {
                state.current_model.clear();
            }
            if crate::core::config::providers::get_provider_by_id(&provider_id)
                .map(|provider| !provider.requires_api_key)
                .unwrap_or(false)
            {
                register_configured_provider(state, &provider_id);
            }

            let pid = provider_id.clone();
            let tx = agent_tx.clone();
            tokio::spawn(async move {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_provider(&pid).await;

                let api_key = crate::core::config::providers::resolve_runtime_api_key(
                    Some(&pid),
                    store.get_api_key(&pid).await.unwrap_or(None),
                );

                let base_url = crate::core::config::providers::resolve_provider_base_url(
                    &pid,
                    store.get_base_url(&pid).await.unwrap_or(None),
                );

                // Check is_openai_compatible: built-in check first, then custom provider type
                let provider_config = store.load().await.unwrap_or_default();
                let is_openai_compat =
                    crate::core::config::providers::resolve_openai_compatible_mode(
                        &pid,
                        provider_config
                            .providers
                            .get(&pid)
                            .and_then(|s| s.r#type.as_deref()),
                    );

                let _ = tx
                    .send(AgentRequest::UpdateProviderConfig {
                        provider_id: Some(pid.clone()),
                        api_key: api_key.clone(),
                        base_url,
                        is_openai_compatible: is_openai_compat,
                        model: selected_model,
                    })
                    .await;

                let _ = tx.send(AgentRequest::ListModels { force: false }).await;
            });

            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!(
                    "{} {}，{}",
                    crate::core::i18n::t("provider.selected", "已选择", "Selected"),
                    provider_id,
                    crate::core::i18n::t(
                        "provider.next_select_model",
                        "接下来选择模型",
                        "now select a model"
                    )
                ),
            );
            // 先用磁盘缓存把面板填上：这个 provider 以前拉过的话，一按就是完整列表，
            // 既不用空白闪一下，也不必等网络。缓存按 provider 分桶，切回用过的
            // provider 是秒开。缓存为空才真的进入"加载中"。
            let seeded =
                crate::ui::services::stream::seed_models_from_disk_cache(state, &provider_id);
            state.awaiting_models = !seeded;
            state.awaiting_models_since = if seeded {
                None
            } else {
                Some(std::time::Instant::now())
            };
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::EditProvider(provider_id) => {
            open_provider_form(state, Some(provider_id)).await;
        }
        PaletteAction::OpenProviderForm => {
            open_provider_form(state, None).await;
        }
        PaletteAction::DeleteProvider(provider_id) => {
            delete_custom_provider(state, &provider_id, agent_tx).await;
        }
        PaletteAction::ClearProviderKey(provider_id) => {
            clear_provider_key(state, &provider_id, agent_tx).await;
        }
        PaletteAction::ToggleFeature(feature) => match feature.as_str() {
            "context_viz" => {
                if matches!(
                    state.top_modal(),
                    Some(crate::ui::state::modal::Modal::ContextViz)
                ) {
                    state.pop_modal();
                } else {
                    state.open_context_viz();
                    state.context_breakdown = build_context_breakdown(state);
                }
            }
            _ => {}
        },
        PaletteAction::SetOutputStyle(style) => {
            state.close_palette();
            let style_clone = style.clone();
            tokio::spawn(async move {
                if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                    if let Ok(mut settings) = mgr.load_user_settings().await {
                        settings.output_style = Some(style_clone);
                        let _ = mgr.save_user_settings(&settings).await;
                    }
                }
            });
            crate::ui::app::logic::emit_status_text(state, 0, &format!("Output style: {}", style));
        }
        PaletteAction::ShowLogSelector => {
            state.close_palette();
            state.open_log_selector();
            state.log_selector_state.begin_loading();

            match crate::utils::session_manager::list_session_summaries().await {
                Ok(sessions) => {
                    state
                        .log_selector_state
                        .set_sessions(sessions.into_iter().map(Into::into).collect());
                }
                Err(error) => {
                    let error = error.to_string();
                    state.log_selector_state.set_load_error(error.clone());
                    crate::ui::app::logic::emit_status_text(
                        state,
                        0,
                        &format!("Could not load saved sessions: {}", error),
                    );
                }
            }
        }
        PaletteAction::ShowContextViz => {
            state.close_palette();
            state.open_context_viz();
            state.context_breakdown = build_context_breakdown(state);
        }
        PaletteAction::ToggleVimMode => {
            state.close_palette();
            state.vim_enabled = !state.vim_enabled;
            if state.vim_enabled {
                state.vim_state = crate::ui::vim::VimState::new();
            }
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("Vim mode: {}", if state.vim_enabled { "ON" } else { "OFF" }),
            );
        }
        PaletteAction::ToggleMouseCapture => {
            state.close_palette();
            // 真的去改终端模式：转义序列直接写 stdout。失败要说真话，
            // 并且把 state 里的标志回滚 —— 否则下次启动会带着错误的初值。
            let want = !state.mouse_capture_enabled;
            match crate::ui::utils::term_recovery::set_mouse_capture(want) {
                Ok(()) => {
                    state.mouse_capture_enabled = want;
                    state.current_status_line = Some(if want {
                        "Mouse capture: ON (terminal text selection unavailable)".to_string()
                    } else {
                        "Mouse capture: OFF (terminal text selection restored; wheel = arrow keys)"
                            .to_string()
                    });
                }
                Err(e) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[MOUSE] toggle failed: {}",
                        e
                    ));
                    state.current_status_line = Some(format!("Mouse capture toggle failed: {e}"));
                }
            }
        }
        PaletteAction::ToggleUiVerbose => {
            state.close_palette();
            state.ui_verbose = !state.ui_verbose;
            state.rendered_cache.clear();
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!(
                    "Verbose UI: {}",
                    if state.ui_verbose { "ON" } else { "OFF" }
                ),
            );
        }
        PaletteAction::CreatePr => {
            state.close_palette();
            // Send a message to the agent to create a PR
            let msg = crate::core::i18n::t(
                "ui.pr.create_request",
                "请帮我创建一个 Pull Request。使用 `gh pr create` 命令，自动填充标题和描述。",
                "Please help me create a Pull Request. Use `gh pr create` command with auto-filled title and description.",
            ).to_string();
            let message_id = state.next_message_id;
            state.next_message_id += 1;
            state.chat_history.push(crate::types::ChatEntry::new(
                crate::types::ChatEntryType::User,
                msg.clone(),
            ));
            let _ = agent_tx
                .send(crate::runtime::messages::AgentRequest::SendMessage {
                    message_id,
                    message: msg,
                })
                .await;
        }
        PaletteAction::ToggleColorblindMode => {
            state.close_palette();
            state.colorblind_mode = !state.colorblind_mode;
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &crate::core::i18n::t(
                    "ui.status.colorblind",
                    &format!(
                        "色盲模式: {}",
                        if state.colorblind_mode {
                            "开启"
                        } else {
                            "关闭"
                        }
                    ),
                    &format!(
                        "Colorblind mode: {}",
                        if state.colorblind_mode { "ON" } else { "OFF" }
                    ),
                ),
            );
        }
    }

    Ok(())
}
