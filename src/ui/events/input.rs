pub(crate) use super::clipboard_paste::push_cursor_off_sentinel_pub;
use super::clipboard_paste::{
    collect_modal_input, detect_file_paths, insert_file_paste_block, insert_image_paste_block,
    insert_paste_block, maybe_auto_fold_input, needs_manual_base_url_confirmation,
    normalize_modal_api_key, normalize_modal_base_url, push_cursor_off_sentinel,
    reset_main_textarea, save_clipboard_image, sync_input_from_textarea,
};
use super::clipboard_paste::{PASTE_ENTER_GUARD_MS, RAPID_PASTE_KEY_INTERVAL_MS};
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::runtime::messages::AgentRequest;
use crate::types::{ApprovalMode, ChatEntryType};
use crate::ui::state::palette::{PaletteAction, PaletteMode};
use crate::ui::state::ChatState;

/// Result from clipboard operations (to avoid blocking async runtime)
enum ClipboardResult {
    Image(String, u32, u32),
    Text(String),
}

/// 尝试从剪贴板读取图片并保存到 .star/images/，返回 (相对路径, 宽, 高)

fn clear_suggestion_overlays(state: &mut ChatState) {
    state.show_command_hints = false;
    state.command_hints.clear();
    state.show_mention_hints = false;
    state.mention_hints.clear();
}

fn close_quick_menus(state: &mut ChatState) {
    state.show_provider_menu = false;
    state.show_session_menu = false;
}

/// 压入 kill ring（容量 10，Claude Code readline 风格）
fn push_kill(state: &mut ChatState, text: String) {
    if text.is_empty() {
        return;
    }
    state.kill_ring.push_back(text);
    while state.kill_ring.len() > 10 {
        state.kill_ring.pop_front();
    }
    state.kill_ring_pos = None;
}

/// 从 kill ring 指定索引 yank：先删除上次 yank 的内容（yank-pop），再插入
fn yank_from_ring(state: &mut ChatState, idx: usize) {
    if state.last_yank_len > 0 {
        for _ in 0..state.last_yank_len {
            state.textarea.move_cursor(tui_textarea::CursorMove::Back);
            state.textarea.delete_char(); // 删除光标前刚 yank 的内容
        }
    }
    if let Some(text) = state.kill_ring.get(idx) {
        let text = text.clone();
        state.textarea.insert_str(&text);
        state.last_yank_len = text.chars().count();
        state.kill_ring_pos = Some(idx);
    } else {
        state.last_yank_len = 0;
    }
    sync_input_from_textarea(state);
    crate::ui::components::command_suggestions::on_input_changed(state);
}

/// Build context breakdown from current state for the context visualization.
fn build_context_breakdown(
    state: &ChatState,
) -> crate::ui::components::highlight::context_viz::TokenBreakdown {
    let total = state.token_count;
    // Heuristic breakdown
    crate::ui::components::highlight::context_viz::TokenBreakdown {
        system_prompt: (total as f64 * 0.20) as u32,
        conversation: (total as f64 * 0.50) as u32,
        tool_outputs: (total as f64 * 0.25) as u32,
        context_files: (total as f64 * 0.05) as u32,
        total,
        max_context: get_max_context_for_model(&state.current_model),
    }
}

fn get_max_context_for_model(model: &str) -> u32 {
    let lower = model.to_lowercase();
    if lower.contains("claude-opus") || lower.contains("claude-sonnet") {
        200_000
    } else if lower.contains("claude-haiku") {
        200_000
    } else if lower.contains("gpt-4o") || lower.contains("gpt-4-turbo") {
        128_000
    } else if lower.contains("deepseek") {
        128_000
    } else if lower.contains("gemini") {
        1_000_000
    } else {
        128_000
    }
}

fn open_provider_selection_menu(
    state: &mut ChatState,
    back: Option<crate::ui::state::QuickMenuKind>,
    origin_palette: bool,
) {
    clear_suggestion_overlays(state);
    state.close_palette();
    state.show_input_modal = false;
    state.show_status_modal = false;
    close_quick_menus(state);
    state.quick_menu_back = back;
    state.quick_menu_origin_palette = origin_palette;
    state.show_provider_menu = true;
    state.selected_provider_index = 0;
}

fn open_session_selection_menu(
    state: &mut ChatState,
    origin_palette: bool,
    sessions: Vec<crate::utils::session_manager::SessionSummary>,
) {
    clear_suggestion_overlays(state);
    state.close_palette();
    state.show_input_modal = false;
    state.show_status_modal = false;
    close_quick_menus(state);
    state.quick_menu_back = None;
    state.quick_menu_origin_palette = origin_palette;
    state.available_sessions = sessions;
    state.show_session_menu = true;
    state.selected_session_index = 0;
}

fn navigate_back_from_quick_menu(state: &mut ChatState) {
    if matches!(
        state.quick_menu_back,
        Some(crate::ui::state::QuickMenuKind::Provider)
    ) {
        if state.quick_menu_origin_palette {
            // Came from palette Provider mode – return there instead of quick overlay
            close_quick_menus(state);
            show_palette_mode(state, crate::ui::state::PaletteMode::Provider);
        } else {
            open_provider_selection_menu(state, None, false);
        }
        return;
    }

    close_quick_menus(state);
    if state.quick_menu_origin_palette {
        show_palette_mode(state, state.palette_mode.clone());
    } else {
        state.quick_menu_back = None;
        state.quick_menu_origin_palette = false;
    }
}

pub(crate) fn show_palette_mode(state: &mut ChatState, mode: PaletteMode) {
    state.show_status_modal = false;
    close_quick_menus(state);
    state.quick_menu_back = None;
    state.quick_menu_origin_palette = false;
    state.show_input_modal = false;
    state.open_palette(mode);
}

pub(crate) fn show_provider_api_key_modal(
    state: &mut ChatState,
    provider_id: &str,
    edit_mode: bool,
    has_saved_key: bool,
) {
    state.show_status_modal = false;
    close_quick_menus(state);
    state.close_palette();
    state.show_input_modal = true;
    state.input_modal_title = if edit_mode {
        format!("Edit API Key for {}", provider_id)
    } else {
        format!("Enter API Key for {}", provider_id)
    };
    state.input_modal_prompt = if has_saved_key {
        "An API key is already saved. Paste a new key and press Enter to replace it:".to_string()
    } else {
        "Paste your API key below and press Enter:".to_string()
    };
    state.input_modal_value = String::new();

    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_placeholder_text("Enter API Key...");
    textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );
    state.modal_textarea = textarea;

    state.input_context = Some(crate::ui::state::palette::InputContext::ProviderKey {
        provider_id: provider_id.to_string(),
    });
}

fn show_provider_base_url_modal(state: &mut ChatState, provider_id: &str, initial_value: String) {
    state.show_status_modal = false;
    close_quick_menus(state);
    state.close_palette();
    state.show_input_modal = true;
    state.input_modal_title = format!("Set Base URL for {}", provider_id);
    state.input_modal_prompt = if initial_value.trim().is_empty() {
        "Enter Base URL:".to_string()
    } else {
        "Review or update the prefilled Base URL, then press Enter:".to_string()
    };
    state.input_modal_value = initial_value;

    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.insert_str(&state.input_modal_value);
    textarea.set_cursor_style(
        ratatui::style::Style::default()
            .bg(ratatui::style::Color::Blue)
            .fg(ratatui::style::Color::White),
    );
    state.modal_textarea = textarea;

    state.input_context = Some(crate::ui::state::palette::InputContext::ProviderBaseUrl {
        provider_id: provider_id.to_string(),
    });
}

pub(crate) async fn execute_palette_action(
    state: &mut ChatState,
    action: PaletteAction,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        PaletteAction::Navigate(mode) => {
            state.palette_history.push(state.palette_mode.clone());

            if matches!(mode, PaletteMode::Model) && state.available_models.is_empty() {
                let _ = agent_tx.send(AgentRequest::ListModels).await;
            }

            // Entering from the settings modal (ShowStatus closes the palette)
            // must reopen the palette, otherwise navigation appears to do nothing.
            state.show_status_modal = false;
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
                state.awaiting_models = true;
                let _ = agent_tx.send(AgentRequest::ListModels).await;
            }
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
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
        PaletteAction::SetModel(model) => {
            // If conversation has history, show confirmation
            let has_history = state.chat_history.iter().any(|e| {
                !e.is_welcome
                    && (e.entry_type == ChatEntryType::User
                        || e.entry_type == ChatEntryType::Assistant)
                    && !e.content.trim().is_empty()
            });

            if has_history && !state.pending_model_confirmation {
                state.close_palette();
                state.pending_model_change = Some(model.clone());
                state.pending_model_confirmation = true;
                state.current_status_line = Some(format!(
                    "{} (y/n)",
                    crate::core::i18n::t(
                        "ui.model.confirm_switch",
                        &format!("切换模型到 {}? (y/n)", model),
                        &format!("Switch model to {}? (y/n)", model),
                    )
                ));
                return Ok(());
            }

            state.close_palette();
            state.pending_model_confirmation = false;
            state.pending_model_change = Some(model.clone());
            state.current_model = model.clone();
            // 从 model_provider_map 查找对应提供商，失败则保持当前提供商
            let provider_id = state
                .model_provider_map
                .get(&model)
                .cloned()
                .or_else(|| state.current_provider_id.clone());
            state.current_provider_id = provider_id.clone();
            let _ = agent_tx
                .send(AgentRequest::SetModel {
                    model: model.clone(),
                    provider_id,
                })
                .await;
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!(
                    "已切换模型 {}，提供商 {}",
                    state.current_model,
                    state.current_provider_id.as_deref().unwrap_or("?")
                ),
            );
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
        PaletteAction::SetContextWindow(size_str) => {
            state.close_palette();
            if size_str == "auto" {
                state.context_window_override = None;
                state.current_status_line = Some("Context Window: auto".to_string());
                tokio::spawn(async move {
                    if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                        if let Ok(mut settings) = mgr.load_user_settings().await {
                            settings.context_window = None;
                            let _ = mgr.save_user_settings(&settings).await;
                        }
                    }
                });
            } else if size_str == "custom" {
                // Show input modal for custom context window
                state.show_input_modal = true;
                state.input_modal_title = "Context Window Size".to_string();
                state.input_modal_prompt =
                    "Enter context window size (e.g. 128k, 200k, 512k, 1M, 2000000):".to_string();
                state.input_modal_value = String::new();
                let mut textarea = tui_textarea::TextArea::default();
                textarea.set_cursor_line_style(ratatui::style::Style::default());
                textarea.set_placeholder_text("128k");
                textarea.set_cursor_style(
                    ratatui::style::Style::default()
                        .add_modifier(ratatui::style::Modifier::REVERSED),
                );
                state.modal_textarea = textarea;
                state.input_context = Some(crate::ui::state::palette::InputContext::ContextWindow);
            } else {
                // Parse preset like "128k", "1M"
                let tokens = parse_context_window_str(&size_str);
                if let Some(tokens) = tokens {
                    state.context_window_override = Some(tokens);
                    state.current_status_line = Some(format!("Context Window: {}k", tokens / 1000));
                    tokio::spawn(async move {
                        if let Ok(mgr) =
                            crate::core::config::settings_manager::SettingsManager::new()
                        {
                            if let Ok(mut settings) = mgr.load_user_settings().await {
                                settings.context_window = Some(tokens);
                                let _ = mgr.save_user_settings(&settings).await;
                            }
                        }
                    });
                }
            }
        }
        PaletteAction::SetThinkingEffort(level) => {
            state.close_palette();
            let effort = match level.as_str() {
                "low" => crate::types::ThinkingEffort::Low,
                "medium" => crate::types::ThinkingEffort::Medium,
                "high" => crate::types::ThinkingEffort::High,
                _ => crate::types::ThinkingEffort::Off,
            };
            state.thinking_effort = effort;
            // Persist to user settings
            let effort_str = state.thinking_effort.as_str().to_string();
            tokio::spawn(async move {
                if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                    if let Ok(mut settings) = mgr.load_user_settings().await {
                        settings.thinking_effort = Some(effort_str);
                        let _ = mgr.save_user_settings(&settings).await;
                    }
                }
            });
            state.current_status_line = Some(format!(
                "Thinking: {}",
                state.thinking_effort.display_name()
            ));
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
            state.pending_model_provider = Some(provider_id.clone());
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
                state.configured_providers.insert(provider_id.clone());
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
                    crate::core::config::providers::provider_openai_compatible_mode(&pid).or_else(
                        || {
                            provider_config
                                .providers
                                .get(&pid)
                                .and_then(|s| s.r#type.as_deref().map(|t| t == "openai-compatible"))
                        },
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

                let _ = tx.send(AgentRequest::ListModels).await;
            });

            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("已选择 {}，接下来选择模型", provider_id),
            );
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::InputApiKey(provider_id) => {
            state.quick_menu_back = Some(crate::ui::state::QuickMenuKind::Provider);
            state.quick_menu_origin_palette = true;
            let store = crate::core::config::provider_store::ProviderStore::new();
            let has_saved_key = store
                .get_api_key(&provider_id)
                .await
                .unwrap_or(None)
                .is_some();
            show_provider_api_key_modal(state, &provider_id, false, has_saved_key);
            crate::ui::app::logic::emit_status_text(state, 0, "Please enter API Key...");
        }
        PaletteAction::InputBaseUrl(provider_id) => {
            state.quick_menu_back = Some(crate::ui::state::QuickMenuKind::Provider);
            state.quick_menu_origin_palette = true;
            let store = crate::core::config::provider_store::ProviderStore::new();
            let initial_value = crate::core::config::providers::resolve_provider_base_url(
                &provider_id,
                store.get_base_url(&provider_id).await.unwrap_or(None),
            )
            .unwrap_or_default();

            show_provider_base_url_modal(state, &provider_id, initial_value);
            crate::ui::app::logic::emit_status_text(state, 0, "Please enter Base URL...");
        }
        PaletteAction::InputProviderId(provider_type) => {
            state.quick_menu_back = Some(crate::ui::state::QuickMenuKind::Provider);
            state.quick_menu_origin_palette = true;
            state.close_palette();
            state.show_input_modal = true;
            state.input_modal_title = "Add New Provider — Enter ID".to_string();
            state.input_modal_prompt =
                "Choose a unique ID for this provider (e.g. my-lmstudio, my-ollama):".to_string();
            state.input_modal_value = String::new();
            let mut textarea = TextArea::default();
            textarea.set_cursor_line_style(ratatui::style::Style::default());
            textarea.set_placeholder_text("e.g. my-lmstudio");
            textarea.set_cursor_style(
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
            );
            state.modal_textarea = textarea;
            state.input_context = Some(crate::ui::state::palette::InputContext::AddProviderId {
                provider_type: provider_type.to_string(),
            });
        }
        PaletteAction::InputProviderName(_provider_id) => {
            // This action is not directly used; the flow is handled via InputContext transitions
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

/// Handle keyboard input for the AskUserQuestion confirmation dialog.
/// Supports option navigation (Up/Down/j/k), direct selection (1-9),
/// multi-select toggle (Space), confirm (Enter), cancel (Esc),
/// and "Other" text input (Tab to switch, then type).
async fn handle_ask_user_question_input(
    key: KeyEvent,
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Extract option info from the confirmation entry
    let (option_count, multi_select) = state
        .pending_confirmation_entry_idx
        .and_then(|idx| state.chat_history.get(idx))
        .and_then(|entry| entry.confirmation.as_ref())
        .map(|conf| match &conf.details {
            crate::types::ConfirmationDetails::AskUserQuestion {
                options,
                multi_select,
                ..
            } => (options.len(), *multi_select),
            _ => (0, false),
        })
        .unwrap_or((0, false));

    let invalidate_cache = |state: &mut ChatState| {
        if let Some(idx) = state.pending_confirmation_entry_idx {
            state.rendered_cache.remove(&idx);
        }
    };

    if state.pending_question_other_focused {
        // "Other" text input mode
        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                    })
                    .await;
                state.is_awaiting_confirmation = false;
                state.pending_tool_call_id = None;
                state.pending_confirmation_entry_idx = None;
                state.pending_question_other_focused = false;
                state.pending_other_input.clear();
                state.pending_question_selections.clear();
            }
            KeyCode::Tab => {
                // Switch back to option navigation
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                // Confirm with other input included
                submit_ask_user_question_answer(
                    state,
                    agent_tx,
                    tool_call_id,
                    option_count,
                    multi_select,
                )
                .await?;
            }
            KeyCode::Backspace => {
                state.pending_other_input.pop();
                invalidate_cache(state);
            }
            KeyCode::Char(ch) => {
                state.pending_other_input.push(ch);
                invalidate_cache(state);
            }
            _ => {}
        }
    } else {
        // Option navigation mode
        // "Other" is at index option_count (after all regular options)
        let other_idx = option_count;
        let total_items = option_count + 1; // options + Other

        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                    })
                    .await;
                state.is_awaiting_confirmation = false;
                state.pending_tool_call_id = None;
                state.pending_confirmation_entry_idx = None;
                state.pending_question_other_focused = false;
                state.pending_other_input.clear();
                state.pending_question_selections.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.pending_confirmation_choice > 0 {
                    state.pending_confirmation_choice -= 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = total_items - 1;
                }
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total_items > 0 && state.pending_confirmation_choice + 1 < total_items {
                    state.pending_confirmation_choice += 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = 0;
                }
                invalidate_cache(state);
            }
            KeyCode::Tab => {
                // Switch to "Other" input mode
                state.pending_question_other_focused = true;
                state.pending_confirmation_choice = other_idx;
                invalidate_cache(state);
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let digit = ch.to_digit(10).unwrap_or(0) as usize;
                if digit >= 1 && digit <= option_count {
                    let opt_idx = digit - 1;
                    if multi_select {
                        // Multi-select: just focus that option
                        state.pending_confirmation_choice = opt_idx;
                    } else {
                        // Single-select: select and confirm immediately
                        state.pending_confirmation_choice = opt_idx;
                        submit_ask_user_question_answer(
                            state,
                            agent_tx,
                            tool_call_id,
                            option_count,
                            multi_select,
                        )
                        .await?;
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Char(' ') if multi_select => {
                // Toggle current option in multi-select mode
                let focused = state.pending_confirmation_choice;
                if focused < option_count {
                    if let Some(pos) = state
                        .pending_question_selections
                        .iter()
                        .position(|&i| i == focused)
                    {
                        state.pending_question_selections.remove(pos);
                    } else {
                        state.pending_question_selections.push(focused);
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                if !multi_select && state.pending_confirmation_choice == other_idx {
                    // "Other" is focused — switch to text input mode instead of submitting
                    state.pending_question_other_focused = true;
                    invalidate_cache(state);
                } else if multi_select && !state.pending_question_selections.is_empty() {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                } else if !multi_select && option_count > 0 {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                }
                // If multi_select but nothing selected, ignore Enter
            }
            _ => {}
        }
    }

    Ok(())
}

/// Collect the user's answer from the AskUserQuestion UI state and submit it.
async fn submit_ask_user_question_answer(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
    option_count: usize,
    multi_select: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect answer labels from the confirmation entry options
    let answers: Vec<String> = if multi_select {
        let mut indices = state.pending_question_selections.clone();
        indices.sort();
        indices
            .iter()
            .filter_map(|&idx| {
                state
                    .pending_confirmation_entry_idx
                    .and_then(|entry_idx| state.chat_history.get(entry_idx))
                    .and_then(|entry| entry.confirmation.as_ref())
                    .and_then(|conf| match &conf.details {
                        crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                            options.get(idx).map(|o| o.label.clone())
                        }
                        _ => None,
                    })
            })
            .collect()
    } else if option_count > 0 {
        let focused = state.pending_confirmation_choice;
        state
            .pending_confirmation_entry_idx
            .and_then(|entry_idx| state.chat_history.get(entry_idx))
            .and_then(|entry| entry.confirmation.as_ref())
            .and_then(|conf| match &conf.details {
                crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                    options.get(focused).map(|o| o.label.clone())
                }
                _ => None,
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };

    let text_input = if state.pending_other_input.is_empty() {
        None
    } else {
        Some(state.pending_other_input.clone())
    };

    // Update ChatEntry outcome text
    if let Some(idx) = state.pending_confirmation_entry_idx {
        if let Some(entry) = state.chat_history.get_mut(idx) {
            if let Some(conf) = entry.confirmation.as_mut() {
                let outcome_str = if answers.is_empty() && text_input.is_some() {
                    format!("User input: {}", text_input.as_ref().unwrap())
                } else if answers.is_empty() {
                    "No selection".to_string()
                } else if answers.len() == 1 {
                    if let Some(ref ti) = text_input {
                        format!("User selected: {} (with input: {})", answers[0], ti)
                    } else {
                        format!("User selected: {}", answers[0])
                    }
                } else {
                    format!(
                        "User selected {} options: {}",
                        answers.len(),
                        answers.join(", ")
                    )
                };
                conf.outcome = Some(outcome_str);
            }
        }
        state.rendered_cache.remove(&idx);
    }

    let _ = agent_tx
        .send(AgentRequest::ConfirmTool {
            tool_call_id: tool_call_id.to_string(),
            outcome: crate::types::ToolConfirmationOutcome::UserAnswer {
                answers,
                text_input,
            },
        })
        .await;

    state.is_awaiting_confirmation = false;
    state.pending_tool_call_id = None;
    state.pending_confirmation_entry_idx = None;
    state.pending_question_other_focused = false;
    state.pending_other_input.clear();
    state.pending_question_selections.clear();

    Ok(())
}

pub async fn handle_key_event(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
    last_key_time: Option<Instant>,
) -> Result<(), Box<dyn std::error::Error>> {
    // ── Unified modal stack takes priority over everything else ──
    // Palette / MCP / Market modals consume keys first; Esc pops one level.
    if super::modal_input::handle_modal_key(state, key, agent_tx).await? {
        return Ok(());
    }

    // Handle Shift+Tab for Plan/Build toggle (Shift+Tab is normalized to BackTab
    // by the runtime event loop before reaching here)
    // 输入模态打开时 Shift+Tab 不能触发模式切换（会吞掉输入框里的按键）
    if key.code == KeyCode::BackTab && !state.show_input_modal {
        state.approval_mode = match state.approval_mode {
            ApprovalMode::Default => ApprovalMode::Plan,
            ApprovalMode::Plan => ApprovalMode::Default,
            ApprovalMode::Yolo => ApprovalMode::Default,
        };

        let _ = agent_tx
            .send(AgentRequest::SetApprovalMode(state.approval_mode.clone()))
            .await;
        return Ok(());
    }

    // Handle /clear confirmation (y/n/Enter/Esc)
    if state.show_clear_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                state.show_clear_confirmation = false;
                state.chat_history.clear();
                let _ = agent_tx
                    .send(crate::runtime::messages::AgentRequest::ResetSession)
                    .await;
                state.current_status_line = Some(
                    crate::core::i18n::t("ui.status.cleared", "对话已清除", "Conversation cleared")
                        .to_string(),
                );
                return Ok(());
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.show_clear_confirmation = false;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }

    // Handle large paste confirmation (y/n/Enter/Esc)
    if state.show_paste_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                state.show_paste_confirmation = false;
                if let Some(text) = state.pending_paste.take() {
                    super::clipboard_paste::insert_paste_block_confirmed(state, text);
                }
                return Ok(());
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.show_paste_confirmation = false;
                state.pending_paste = None;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }

    // Handle Ctrl+P for Palette
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('p')) {
        if state.is_palette_open() {
            state.close_palette();
        } else {
            state.show_help = false;
            state.show_input_modal = false;
            state.show_status_modal = false;
            close_quick_menus(state);
            state.quick_menu_back = None;
            state.quick_menu_origin_palette = false;
            state.palette_history.clear();
            state.open_palette(PaletteMode::Main);
        }
        return Ok(());
    }

    // Ctrl+O：全局切换 transcript / verbose 输出（对标 Claude Code 的 app:toggleTranscript，
    // 同为 Global 作用域）。这里必须在 handle_overlay_input 之前，否则又会退回到
    // 「只有任务面板可见时才生效」的老行为。
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('o')) {
        let on = state.toggle_transcript_mode();
        // 直接写 current_status_line：emit_status_text 只放行 i18n::status_prefixes()
        // 允许的前缀，"Verbose output: …" 会被它静默丢掉。
        state.current_status_line = Some(if on {
            "Verbose output: ON".to_string()
        } else {
            "Verbose output: OFF".to_string()
        });
        return Ok(());
    }

    // Ctrl+T：全局切换任务面板（对标 Claude Code 的 app:toggleTodos）。
    // Ctrl+T：任务面板（对标 app:toggleTodos）。Ctrl+B 不在此列 —— Claude Code 把它
    // 留给 task:background（仅前台有任务可转后台时才拦截），空闲时归输入框。
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('t')) {
        state.task_panel.toggle_visibility();
        if state.task_panel.is_visible {
            // 打开时重读任务文件，和 /tasks 一样避免显示旧快照。
            state.task_panel.reload();
        }
        return Ok(());
    }

    // Ctrl+Shift+F：全局搜索（对标 Claude Code app:globalSearch）。
    if key
        .modifiers
        .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Char('F'))
    {
        state.open_global_search();
        return Ok(());
    }

    // Ctrl+Shift+P：快速打开文件（对标 Claude Code app:quickOpen）。
    if key
        .modifiers
        .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Char('P'))
    {
        state.open_quick_open();
        return Ok(());
    }

    // Ctrl+R：历史搜索（对标 Claude Code history:search）。
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('r'))
        && !state.is_awaiting_confirmation
        && !state.is_palette_open()
    {
        state.open_history_search();
        return Ok(());
    }

    // Handle Alt+P for folded pasted input preview
    if !state.is_awaiting_confirmation
        && !state.is_palette_open()
        && !state.show_input_modal
        && key.modifiers.contains(KeyModifiers::ALT)
    {
        if let KeyCode::Char(ch) = key.code {
            if ch.eq_ignore_ascii_case(&'p') {
                let line_count = state.input_line_count;
                if line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES {
                    state.input_folded = !state.input_folded;
                    state.current_status_line = Some(if state.input_folded {
                        format!("Input folded ({} lines), press Alt+P to expand", line_count)
                    } else {
                        "Input expanded, press Alt+P to fold again".to_string()
                    });
                } else {
                    state.input_folded = false;
                    state.current_status_line = Some("Content is too short to fold".to_string());
                }
                return Ok(());
            }
            if ch.eq_ignore_ascii_case(&'t') {
                let cap = crate::core::config::models::thinking_capability(&state.current_model);
                state.thinking_effort = match cap {
                    crate::core::config::models::ThinkingCapability::Binary => {
                        // Binary models: Off ↔ Medium (On)
                        match state.thinking_effort {
                            crate::types::ThinkingEffort::Off => {
                                crate::types::ThinkingEffort::Medium
                            }
                            _ => crate::types::ThinkingEffort::Off,
                        }
                    }
                    _ => state.thinking_effort.next(),
                };
                state.current_status_line = Some(format!(
                    "Thinking: {}",
                    state.thinking_effort.display_name()
                ));
                // Persist to user settings
                let effort_str = state.thinking_effort.as_str().to_string();
                tokio::spawn(async move {
                    if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                        if let Ok(mut settings) = mgr.load_user_settings().await {
                            settings.thinking_effort = Some(effort_str);
                            let _ = mgr.save_user_settings(&settings).await;
                        }
                    }
                });
                return Ok(());
            }
            // Alt+Shift+T: Cycle theme
            if ch.eq_ignore_ascii_case(&'t') && key.modifiers.contains(KeyModifiers::SHIFT) {
                state.theme_manager.next_theme();
                let theme_name = state.theme_manager.current().name.clone();
                state.current_status_line = Some(format!("Theme: {}", theme_name));
                return Ok(());
            }
        }
    }

    // Ctrl+B 有意不绑定：Claude Code 把它留给 `task:background`（Task 作用域），且只在
    // 真有前台任务可转后台时才吃掉这个键，空闲时让它落回 readline 的 backward-char。
    // starcode 目前没有「把正在跑的前台任务转后台」的运行时能力（run_in_background 只在
    // 工具描述里许诺过，Rust 侧没有实现），所以这里同样不拦截，交给 textarea 当左移光标用。
    // 任务面板改由 Ctrl+T 切换（对标 app:toggleTodos）。

    // Handle Ctrl+C (Copy selected text / Cancel / double-press Exit)
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        // Priority 1: If there's selected text, copy to clipboard
        if state.text_selection.has_selection() {
            if let Some(selected_text) = state.get_selected_text() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(&selected_text);
                }
                state.text_selection.clear();
                state.current_status_line = Some("Copied".to_string());
                return Ok(());
            }
        }

        // Check if streaming is "stale" — cancelling grace period has expired but
        // is_streaming was never reset (e.g. Done arrived during grace window).
        let cancelling_stale = state.is_streaming
            && state
                .cancelling_since
                .map(|t| t.elapsed() > Duration::from_millis(1500))
                .unwrap_or(false);
        if cancelling_stale {
            // Force-reset the streaming state so Ctrl+C can proceed to exit logic.
            state.is_streaming = false;
            state.is_processing = false;
            state.cancelling_since = None;
        }

        if state.is_streaming || state.is_processing {
            // Cancel streaming; first Ctrl+C resets the double-press timer
            let _ = agent_tx.send(AgentRequest::Abort).await;
            state.current_status_line = Some("Cancelling...".to_string());
            state.is_processing = false;
            state.last_ctrl_c = None;
            state.cancelling_since = Some(Instant::now());
            return Ok(());
        }

        // Double-press detection: two Ctrl+C within 1.5 s exits
        let now = Instant::now();
        if let Some(prev) = state.last_ctrl_c {
            if now.duration_since(prev) <= Duration::from_millis(1500) {
                state.should_exit = true;
                return Ok(());
            }
        }
        state.last_ctrl_c = Some(now);

        if !state.input.is_empty() {
            // First press: clear input
            reset_main_textarea(state);
            crate::ui::components::command_suggestions::on_input_changed(state);
            state.current_status_line = Some("Press Ctrl+C again to exit".to_string());
        } else {
            state.current_status_line = Some("Press Ctrl+C again to exit".to_string());
        }
        return Ok(());
    }

    // Handle Ctrl+D (Exit)
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('d')) {
        if !state.task_panel.is_visible {
            if state.input.is_empty() {
                state.should_exit = true;
                return Ok(());
            }
        }
    }

    // Handle Ctrl+L (Clear screen)
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('l')) {
        state.request_clear_screen = true;
        return Ok(());
    }

    if handle_overlay_input(state, key, agent_tx).await? {
        return Ok(());
    }

    // 后台代理选择器：焦点在选择器上时独占 ↑/↓/Enter/Esc。
    // 位置刻意在 handle_overlay_input **之后** —— 任何弹窗仍然优先，选择器只跟输入框抢键。
    if state.bg_agent_selection.is_some()
        && !state.show_input_modal
        && handle_bg_agent_selector_key(state, key)
    {
        return Ok(());
    }

    // Code block copy: press 'c' when no overlay is active to copy last code block
    if key.code == KeyCode::Char('c') && key.modifiers.is_empty() {
        if state.input.is_empty() {
            if let Some(ref code) = state.last_code_block_content {
                // Copy to clipboard
                if let Ok(mut ctx) = arboard::Clipboard::new() {
                    let _ = ctx.set_text(code.clone());
                    state.push_toast(
                        &crate::core::i18n::t(
                            "ui.status.copied_code_block",
                            "已复制代码块到剪贴板",
                            "Code block copied to clipboard",
                        ),
                        crate::ui::state::store::ToastKind::Success,
                    );
                    return Ok(());
                }
            }
        }
    }

    // Vim mode intercept
    if state.vim_enabled && !state.has_overlay_active() {
        use crate::ui::vim::VimMode;
        if let KeyCode::Char(c) = key.code {
            match state.vim_state.mode {
                VimMode::Normal => {
                    state.vim_state.pending_keys.push(c);
                    let pending_keys = state.vim_state.pending_keys.clone();

                    // Try motion first
                    if let Some(motion) =
                        crate::ui::vim::motions::Motion::from_key(c, &pending_keys)
                    {
                        apply_vim_motion(state, &motion);
                        state.vim_state.reset();
                        return Ok(());
                    }

                    // Check if we need more keys (e.g., 'g', 'f', 'F', 't', 'T')
                    let needs_more = matches!(c, 'g' | 'f' | 'F' | 't' | 'T');
                    if needs_more {
                        return Ok(()); // Wait for next key
                    }

                    // Mode switches
                    match c {
                        'i' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'a' => {
                            state.vim_state.mode = VimMode::Insert;
                            state
                                .textarea
                                .move_cursor(tui_textarea::CursorMove::Forward);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'A' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::End);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'I' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'o' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::End);
                            state.textarea.insert_newline();
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'O' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.textarea.insert_newline();
                            state.textarea.move_cursor(tui_textarea::CursorMove::Up);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'v' => {
                            state.vim_state.mode = VimMode::Visual;
                            state.vim_state.reset();
                            return Ok(());
                        }
                        ':' => {
                            state.vim_state.mode = VimMode::Command;
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'x' => {
                            // Delete character under cursor
                            state.textarea.delete_next_char();
                            sync_input_from_textarea(state);
                            return Ok(());
                        }
                        'd' if pending_keys == "dd" => {
                            // Delete entire line
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.textarea.delete_line_by_end();
                            state.textarea.delete_next_char();
                            sync_input_from_textarea(state);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        _ => {
                            // Unknown key in normal mode, reset
                            state.vim_state.reset();
                            return Ok(());
                        }
                    }
                }
                VimMode::Insert => {
                    // In insert mode, fall through to normal input handling
                }
                VimMode::Visual => {
                    // Visual mode: Escape returns to Normal
                    if key.code == KeyCode::Esc {
                        state.vim_state.mode = VimMode::Normal;
                        state.vim_state.reset();
                        return Ok(());
                    }
                    // Other visual mode keys handled by motions
                    let pending_keys = state.vim_state.pending_keys.clone();
                    if let Some(motion) =
                        crate::ui::vim::motions::Motion::from_key(c, &pending_keys)
                    {
                        apply_vim_motion(state, &motion);
                        state.vim_state.reset();
                        return Ok(());
                    }
                    state.vim_state.reset();
                    return Ok(());
                }
                VimMode::Command => {
                    // Command mode: Escape returns to Normal, Enter executes
                    if key.code == KeyCode::Esc {
                        state.vim_state.mode = VimMode::Normal;
                        state.vim_state.reset();
                        return Ok(());
                    }
                    // For now, just return to normal on any key
                    state.vim_state.mode = VimMode::Normal;
                    state.vim_state.reset();
                    return Ok(());
                }
            }
        } else {
            // Non-char keys in vim modes
            match state.vim_state.mode {
                VimMode::Normal => match key.code {
                    KeyCode::Esc => {
                        state.vim_state.reset();
                        return Ok(());
                    }
                    _ => {}
                },
                VimMode::Insert => {
                    // Fall through to normal input handling for non-char keys
                }
                VimMode::Visual | VimMode::Command => {
                    if key.code == KeyCode::Esc {
                        state.vim_state.mode = VimMode::Normal;
                        state.vim_state.reset();
                        return Ok(());
                    }
                }
            }
        }
    }

    // Handle Input Modal
    if handle_input_modal(state, key, agent_tx).await? {
        return Ok(());
    }

    if let Some(action) = crate::ui::events::keymap::map_key(state, &key) {
        use crate::ui::events::keymap::UiAction;
        match action {
            UiAction::SelectPrev => {
                crate::ui::components::command_suggestions::handle_up(state);
                return Ok(());
            }
            UiAction::SelectNext => {
                crate::ui::components::command_suggestions::handle_down(state);
                return Ok(());
            }
            UiAction::AcceptSuggestion => {
                if crate::ui::components::command_suggestions::handle_enter(state) {
                    if let Some(action) = state.pending_palette_action.take() {
                        execute_palette_action(state, action, agent_tx).await?;
                    }
                    if let Some(model) = state.pending_model_change.take() {
                        state.current_model = model.clone();
                        let provider_id = state
                            .model_provider_map
                            .get(&model)
                            .cloned()
                            .or_else(|| state.current_provider_id.clone());
                        state.current_provider_id = provider_id.clone();
                        let _ = agent_tx
                            .send(AgentRequest::SetModel { model, provider_id })
                            .await;
                    }
                    return Ok(());
                }
            }
            UiAction::AcceptCompletion => {
                if crate::ui::components::command_suggestions::handle_tab(state) {
                    return Ok(());
                }
            }
        }
    }

    if handle_paste(state, key).await? {
        return Ok(());
    }

    // Handle model switching confirmation (y/n)
    if state.pending_model_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                // Confirm model switch
                if let Some(model) = state.pending_model_change.clone() {
                    state.pending_model_confirmation = false;
                    state.current_model = model.clone();
                    let provider_id = state
                        .model_provider_map
                        .get(&model)
                        .cloned()
                        .or_else(|| state.current_provider_id.clone());
                    state.current_provider_id = provider_id.clone();
                    let _ = agent_tx
                        .send(AgentRequest::SetModel {
                            model: model.clone(),
                            provider_id,
                        })
                        .await;
                    crate::ui::app::logic::emit_status_text(
                        state,
                        0,
                        &format!(
                            "已切换模型 {}，提供商 {}",
                            state.current_model,
                            state.current_provider_id.as_deref().unwrap_or("?")
                        ),
                    );
                }
                return Ok(());
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                // Cancel model switch
                state.pending_model_confirmation = false;
                state.pending_model_change = None;
                state.current_status_line = None;
                return Ok(());
            }
            _ => {
                // Ignore other keys during confirmation
                return Ok(());
            }
        }
    }

    // Handle log selector input
    if state.show_log_selector {
        match key.code {
            KeyCode::Enter => {
                // Resume selected session
                if let Some(session) = state.log_selector_state.get_selected_session() {
                    let session_id = session.id.clone();
                    state.show_log_selector = false;
                    // Send resume request
                    let _ = agent_tx
                        .send(AgentRequest::ResumeSession(session_id.clone()))
                        .await;
                    crate::ui::app::logic::emit_status_text(
                        state,
                        0,
                        &format!("Resuming session: {}", session_id),
                    );
                }
                return Ok(());
            }
            KeyCode::Esc => {
                state.show_log_selector = false;
                return Ok(());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.log_selector_state.select_prev();
                return Ok(());
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.log_selector_state.select_next();
                return Ok(());
            }
            KeyCode::Char('/') => {
                // Focus search
                state.log_selector_state.search_query.clear();
                return Ok(());
            }
            KeyCode::Backspace => {
                state.log_selector_state.search_query.pop();
                state.log_selector_state.selected_index = 0;
                return Ok(());
            }
            KeyCode::Char(c) => {
                state.log_selector_state.search_query.push(c);
                state.log_selector_state.selected_index = 0;
                return Ok(());
            }
            _ => return Ok(()),
        }
    }

    // Handle error overlay input
    if state.show_error_overlay {
        match key.code {
            KeyCode::Char('r') | KeyCode::Char('R') => {
                // Retry
                state.show_error_overlay = false;
                // Re-send last user message
                if let Some(last_user) = state
                    .chat_history
                    .iter()
                    .rev()
                    .find(|e| e.entry_type == ChatEntryType::User)
                {
                    let msg = last_user.content.clone();
                    crate::ui::app::logic::enqueue_user_message(state, msg, agent_tx).await?;
                }
                return Ok(());
            }
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => {
                state.show_error_overlay = false;
                return Ok(());
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                // Switch provider
                state.show_error_overlay = false;
                state.open_palette(PaletteMode::Provider);
                return Ok(());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.error_overlay_state.select_prev();
                return Ok(());
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.error_overlay_state.select_next();
                return Ok(());
            }
            _ => return Ok(()),
        }
    }

    match key.code {
        KeyCode::Enter => {
            // 如果正在粘贴，忽略 Enter 键（防止换行被解释为发送）
            if state.paste_in_progress {
                return Ok(());
            }

            // 粘贴结束后的短保护窗口：吞掉残留 Enter，避免误发送
            if let Some(end_time) = state.paste_end_time {
                if end_time.elapsed() < Duration::from_millis(PASTE_ENTER_GUARD_MS) {
                    return Ok(());
                }
            }

            // 启发式粘贴检测：如果按键间隔极短 (<20ms)，认为是粘贴行为，不发送消息
            // 这主要解决了不支持 Bracketed Paste 的终端（如部分 Windows 环境）粘贴多行文本时会自动发送的问题
            if let Some(last_time) = last_key_time {
                if last_time.elapsed() < Duration::from_millis(RAPID_PASTE_KEY_INTERVAL_MS) {
                    state.textarea.insert_newline();
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                    return Ok(());
                }
            }

            if !state.show_help {
                // Check for Alt+Enter or Ctrl+Enter for newline
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    state.textarea.insert_newline();
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                    return Ok(());
                }

                let raw_input = state.textarea.lines().join("\n");
                let input =
                    crate::ui::state::expand_paste_segments(&raw_input, &state.paste_segments);

                let trimmed_raw = raw_input.trim();
                if trimmed_raw == "/model" || trimmed_raw == "/models" {
                    reset_main_textarea(state);

                    if state.available_models.is_empty() {
                        state.awaiting_models = true;
                        // Trigger model list fetch
                        if let Err(_e) = agent_tx.send(AgentRequest::ListModels).await {
                            // Handle error silently or log
                        }
                    }

                    state.palette_history.clear();
                    state.open_palette(PaletteMode::Model);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                    return Ok(());
                }

                if let Some(model_name) = trimmed_raw.strip_prefix("/model ") {
                    let model_name = model_name.trim();
                    if !model_name.is_empty() {
                        reset_main_textarea(state);
                        state.current_model = model_name.to_string();
                        // 与 palette 路径同口径：优先用 model_provider_map 解析提供商
                        let provider_id = state
                            .model_provider_map
                            .get(model_name)
                            .cloned()
                            .or_else(|| state.current_provider_id.clone());
                        state.current_provider_id = provider_id.clone();
                        let _ = agent_tx
                            .send(AgentRequest::SetModel {
                                model: model_name.to_string(),
                                provider_id,
                            })
                            .await;
                        // 已获取过模型列表时做校验提示（仍允许自定义名称）
                        let known = state.available_models.is_empty()
                            || state.available_models.iter().any(|m| m == model_name);
                        let note = if known {
                            String::new()
                        } else {
                            format!(
                                " {}",
                                crate::core::i18n::t(
                                    "ui.model.not_in_list",
                                    "(不在已获取的模型列表中，仍按自定义名称使用)",
                                    "(not in fetched model list; using as custom name)",
                                )
                            )
                        };
                        crate::ui::app::logic::emit_status_text(
                            state,
                            0,
                            &format!(
                                "{}{}{}",
                                crate::core::i18n::t(
                                    "ui.model.switched_to",
                                    "已切换到模型 ",
                                    "Switched to model ",
                                ),
                                model_name,
                                note,
                            ),
                        );
                        return Ok(());
                    }
                }

                if !input.trim().is_empty() {
                    // Important: Clear textarea BEFORE enqueueing message to prevent UI glitch
                    // where Enter keeps triggering and input remains.
                    reset_main_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);

                    crate::ui::app::logic::enqueue_user_message(state, input, agent_tx).await?;
                } else {
                    // Clear empty input if any (e.g. just newlines)
                    reset_main_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                }
            } else {
                state.show_help = false;
            }
        }
        KeyCode::Esc => {
            // Handle Streaming Cancellation
            if state.is_streaming || state.is_processing {
                let _ = agent_tx.send(AgentRequest::Abort).await;
                state.current_status_line = Some("Cancelling...".to_string());
                state.is_processing = false;
                state.cancelling_since = Some(Instant::now());
                return Ok(());
            }

            // 正在看某个后台代理的详情 → Esc 先回主会话（对标详情视图顶部那句
            // "Esc to return"）。选择器自己的 Esc 在 handle_bg_agent_selector_key 里
            // 已经被消费掉，走不到这儿。
            if state.viewing_agent_task_id.is_some() {
                state.exit_teammate_view();
                return Ok(());
            }

            state.show_help = false;
            state.show_command_hints = false;
            state.command_hints.clear();
            state.show_mention_hints = false;
            state.mention_hints.clear();
            // 关闭 modal stack 栈顶（如果有）
            if let Some(closed) = state.pop_modal() {
                match closed {
                    crate::ui::state::modal::Modal::ThemePicker { prev_theme } => {
                        if let Some(prev) = prev_theme {
                            state.theme_manager.set_theme(&prev);
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            state.show_context_viz = false;
            state.show_error_overlay = false;
            state.show_log_selector = false;
            state.pending_model_confirmation = false;
            if state.show_provider_menu || state.show_session_menu {
                navigate_back_from_quick_menu(state);
            } else {
                close_quick_menus(state);
            }

            // Claude Code 风格：无任何覆盖层时，双击 Esc 清空输入并存入历史
            let overlay_open = state.has_overlay_active() || state.pending_model_confirmation;
            if !overlay_open && !state.input.trim().is_empty() {
                let now = Instant::now();
                let double = state
                    .last_esc_at
                    .map(|t| now.duration_since(t) <= Duration::from_millis(1000))
                    .unwrap_or(false);
                if double {
                    let input = state.input.trim().to_string();
                    let is_dup = state
                        .command_history
                        .front()
                        .map(|l| l == &input)
                        .unwrap_or(false);
                    if !is_dup {
                        state.command_history.push_front(input);
                        state.command_history.truncate(100);
                        crate::core::config::history_store::save_history(&state.command_history);
                    }
                    reset_main_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                    state.last_esc_at = None;
                    state.current_status_line =
                        Some("Input cleared (saved to history)".to_string());
                } else {
                    state.last_esc_at = Some(now);
                    state.current_status_line = Some("Press Esc again to clear input".to_string());
                }
            } else {
                state.last_esc_at = None;
            }
        }
        KeyCode::F(1) => {
            state.show_help = !state.show_help;
        }
        KeyCode::Char('?')
            if !state.is_palette_open() && !state.show_status_modal && !state.show_input_modal =>
        {
            // ? when input is empty: show help (like Claude Code)
            if state.textarea.lines().iter().all(|l| l.is_empty()) {
                state.show_help = !state.show_help;
            } else {
                // Otherwise insert ? normally
                push_cursor_off_sentinel(state);
                state.textarea.input(key);
                sync_input_from_textarea(state);
            }
        }
        KeyCode::Tab => {
            // Tab: toggle expand/collapse of the nearest tool entry
            if !state.is_palette_open() && !state.is_awaiting_confirmation {
                let focused_idx = find_focused_tool_entry(state);
                if let Some(idx) = focused_idx {
                    if let Some(entry) = state.chat_history.get(idx) {
                        if let Some(tc) = &entry.tool_call {
                            if state.expanded_tool_call_ids.contains(&tc.id) {
                                state.expanded_tool_call_ids.remove(&tc.id);
                            } else {
                                state.expanded_tool_call_ids.insert(tc.id.clone());
                            }
                            state.rendered_cache.remove(&idx);
                        }
                    }
                }
            }
        }
        KeyCode::PageUp => {
            let page = state.last_chat_height.saturating_sub(2) as i32;
            crate::ui::state::scroll_chat(state, -page.max(1));
        }
        KeyCode::PageDown => {
            let page = state.last_chat_height.saturating_sub(2) as i32;
            crate::ui::state::scroll_chat(state, page.max(1));
        }
        KeyCode::Home => {
            // Ctrl+Home: scroll to top of chat
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                state.scroll = 0;
                state.auto_follow = false;
            } else {
                // Home: move cursor to start of line
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                }
            }
        }
        KeyCode::End => {
            // Ctrl+End: scroll to bottom of chat (auto-follow)
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                state.auto_follow = true;
                state.show_scroll_to_bottom = false;
            } else {
                // End: move cursor to end of line
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                }
            }
        }
        KeyCode::Up => {
            let cursor_row = state.textarea.cursor().0;
            let input_is_empty = state.input.trim().is_empty();

            // 鼠标滚轮检测：连续 Up/Down 事件间隔 < 50ms 视为滚轮
            let now = Instant::now();
            let is_mouse_scroll = if let Some(pending_time) = state.pending_scroll_time {
                let elapsed = now.duration_since(pending_time).as_millis();
                if elapsed < 50 {
                    // 第二个事件快速到达，确认是鼠标滚轮
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    true
                } else {
                    // 超时，第一个事件是键盘按键，执行历史导航
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    false
                }
            } else {
                // 第一个事件，缓存为 pending
                state.pending_scroll_direction = Some(-1); // -1 = Up
                state.pending_scroll_time = Some(now);
                false
            };

            if is_mouse_scroll {
                // 鼠标滚轮：滚动聊天而非导航历史
                crate::ui::state::scroll_chat(state, -3);
            } else if cursor_row == 0 && (input_is_empty || state.history_index.is_some()) {
                // Command history navigation: Up when cursor is on first line and input is
                // empty or already in history mode
                if !state.command_history.is_empty() {
                    let idx = match state.history_index {
                        Some(i) => i.saturating_add(1),
                        None => {
                            state.history_input_snapshot = Some(state.input.clone());
                            0
                        }
                    };
                    if idx < state.command_history.len() {
                        state.history_index = Some(idx);
                        let hist_entry = state.command_history[idx].clone();
                        state.textarea.select_all();
                        state.textarea.delete_str(state.input.len());
                        state.textarea.insert_str(&hist_entry);
                        sync_input_from_textarea(state);
                        crate::ui::components::command_suggestions::on_input_changed(state);
                    }
                }
            } else if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
        }
        KeyCode::Down => {
            let cursor_row = state.textarea.cursor().0;
            let line_count = state.input_line_count;

            // 鼠标滚轮检测：连续 Up/Down 事件间隔 < 50ms 视为滚轮
            let now = Instant::now();
            let is_mouse_scroll = if let Some(pending_time) = state.pending_scroll_time {
                let elapsed = now.duration_since(pending_time).as_millis();
                if elapsed < 50 {
                    // 第二个事件快速到达，确认是鼠标滚轮
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    true
                } else {
                    // 超时，第一个事件是键盘按键，执行历史导航
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    false
                }
            } else {
                // 第一个事件，缓存为 pending
                state.pending_scroll_direction = Some(1); // 1 = Down
                state.pending_scroll_time = Some(now);
                false
            };

            if is_mouse_scroll {
                // 鼠标滚轮：滚动聊天而非导航历史
                crate::ui::state::scroll_chat(state, 3);
            } else if cursor_row + 1 >= line_count && state.history_index.is_some() {
                // Command history navigation: Down when cursor is on last line and in history mode
                let idx = state.history_index.unwrap();
                if idx == 0 {
                    state.history_index = None;
                    let snapshot = state.history_input_snapshot.take().unwrap_or_default();
                    state.textarea.select_all();
                    state.textarea.delete_str(state.input.len());
                    state.textarea.insert_str(&snapshot);
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                } else {
                    let new_idx = idx.saturating_sub(1);
                    state.history_index = Some(new_idx);
                    let hist_entry = state.command_history[new_idx].clone();
                    state.textarea.select_all();
                    state.textarea.delete_str(state.input.len());
                    state.textarea.insert_str(&hist_entry);
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                }
            } else if cursor_row + 1 >= line_count
                && state.bg_agent_selection.is_none()
                && !state.background_agent_rows().is_empty()
            {
                // ↓ 从输入框末行溢出 → 落进后台代理选择器（对标 background agent selector：
                // 「↓ manage」那句提示指的就是这个动作）。没有后台代理时保持原样，
                // 让 textarea 自己吞掉按键。
                state.bg_agent_selection = Some(0);
            } else if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+Z: Undo
            if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+Y: Kill ring 为空时退化为 Redo（textarea 默认行为）
            if state.kill_ring.is_empty() {
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                }
            } else {
                let idx = state.kill_ring.len() - 1;
                yank_from_ring(state, idx);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
            // Alt+Y: yank-pop — 轮换 kill ring 中更早的条目
            if !state.kill_ring.is_empty() {
                let len = state.kill_ring.len();
                let next = match state.kill_ring_pos {
                    Some(p) if p > 0 => p - 1,
                    Some(_) => len - 1,
                    None => len.saturating_sub(1),
                };
                yank_from_ring(state, next);
            }
        }
        KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+X: Cut selected text
            if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+U: Delete from cursor to beginning of line (Unix readline)
            let (row, col) = state.textarea.cursor();
            if let Some(line) = state.textarea.lines().get(row) {
                push_kill(state, line[..col.min(line.len())].to_string());
            }
            state.textarea.delete_line_by_head();
            sync_input_from_textarea(state);
            crate::ui::components::command_suggestions::on_input_changed(state);
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+K: Delete from cursor to end of line (Unix readline)
            let (row, col) = state.textarea.cursor();
            if let Some(line) = state.textarea.lines().get(row) {
                push_kill(state, line[col.min(line.len())..].to_string());
            }
            state.textarea.delete_line_by_end();
            sync_input_from_textarea(state);
            crate::ui::components::command_suggestions::on_input_changed(state);
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+W: Delete word backward (Unix readline)
            let (row, col) = state.textarea.cursor();
            let line_owned = state.textarea.lines().get(row).cloned();
            if let Some(line) = line_owned {
                let line = line.as_str();
                let before = &line[..col.min(line.len())];
                let trimmed = before.trim_end();
                let new_col = if trimmed.is_empty() {
                    0
                } else {
                    // Find last word boundary: skip trailing spaces, then skip word chars
                    let bytes = trimmed.as_bytes();
                    let mut i = bytes.len() - 1;
                    // Skip current char if it's a word boundary
                    if !trimmed.is_char_boundary(i) {
                        i -= 1;
                    }
                    let last_char = trimmed[i..].chars().next().unwrap_or(' ');
                    if last_char.is_alphanumeric() || last_char == '_' {
                        // Skip word chars backward
                        while i > 0 && {
                            let c = trimmed[..i].chars().last().unwrap_or(' ');
                            c.is_alphanumeric() || c == '_'
                        } {
                            i -= 1;
                        }
                    } else {
                        // Skip non-word chars backward
                        while i > 0 && {
                            let c = trimmed[..i].chars().last().unwrap_or(' ');
                            !c.is_alphanumeric() && c != '_'
                        } {
                            i -= 1;
                        }
                    }
                    i
                };
                // Rebuild line: keep chars before new_col, skip chars from new_col to col
                let killed = line[new_col..col.min(line.len())].to_string();
                push_kill(state, killed);
                let mut new_line = String::new();
                new_line.push_str(&line[..new_col]);
                if col < line.len() {
                    new_line.push_str(&line[col..]);
                }
                // Delete the entire line and reinsert
                state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                state.textarea.delete_line_by_end();
                state.textarea.insert_str(&new_line);
                // Move cursor to new_col
                state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                for _ in 0..new_col {
                    state
                        .textarea
                        .move_cursor(tui_textarea::CursorMove::Forward);
                }
                sync_input_from_textarea(state);
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
        }
        _ => {
            if !state.show_help {
                push_cursor_off_sentinel(state);
                let mut handled = state.textarea.input(key);
                // Fallback: if textarea.input() didn't handle a regular Char, insert it directly
                if !handled {
                    if let KeyCode::Char(c) = key.code {
                        if !c.is_control()
                            && (key.modifiers.is_empty()
                                || key.modifiers == crossterm::event::KeyModifiers::SHIFT)
                        {
                            state.textarea.insert_char(c);
                            handled = true;
                        }
                    }
                }
                if handled {
                    // 仅在无粘贴块时解除旧式折叠；有粘贴块时保持块内嵌显示
                    if state.input_folded && state.paste_segments.is_empty() {
                        state.input_folded = false;
                    }
                    sync_input_from_textarea(state);
                    crate::ui::components::command_suggestions::on_input_changed(state);
                }
            }
        }
    }

    Ok(())
}

/// 后台代理选择器的按键处理（对标 background agent selector）。
///
/// 返回 `true` 表示按键已被选择器消费，调用方应直接结束本次事件处理。
///
/// 「光标停在哪一行」（`bg_agent_selection`）与「正在看谁的输出」（`viewing_agent_task_id`）
/// 是分开的两件事：`Enter` 只切换后者，焦点留在原行，用户可以连着 ↑/↓ 逐个翻看；
/// `Esc` 只收起选择器，不改变当前正在看的详情。
fn handle_bg_agent_selector_key(state: &mut ChatState, key: KeyEvent) -> bool {
    // 行数每帧都可能变（后台代理还在陆续启动），先按当前快照收敛焦点
    let task_ids: Vec<String> = state
        .background_agent_rows()
        .iter()
        .map(|info| info.task_id.clone())
        .collect();
    if task_ids.is_empty() {
        state.bg_agent_selection = None;
        return false;
    }
    let last_idx = task_ids.len(); // 索引 0 是 main 行，代理从 1 开始
    let selected = state.bg_agent_selection.unwrap_or(0).min(last_idx);

    match key.code {
        KeyCode::Up => {
            if selected == 0 {
                // 已在 main 行再往上 → 焦点还给输入框
                state.bg_agent_selection = None;
            } else {
                state.bg_agent_selection = Some(selected - 1);
            }
            true
        }
        KeyCode::Down => {
            state.bg_agent_selection = Some((selected + 1).min(last_idx));
            true
        }
        KeyCode::Enter => {
            if selected == 0 {
                state.exit_teammate_view();
            } else {
                let task_id = task_ids[selected - 1].clone();
                if state.viewing_agent_task_id.as_deref() == Some(task_id.as_str()) {
                    // 再按一次 Enter 收回主会话，跟参照实现的 toggle 语义一致
                    state.exit_teammate_view();
                } else {
                    state.enter_teammate_view(&task_id);
                }
            }
            true
        }
        KeyCode::Esc => {
            state.bg_agent_selection = None;
            true
        }
        _ => false,
    }
}

async fn handle_overlay_input(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // ── 主题选择器：↑↓ 导航（实时预览）、Enter 应用、Esc 取消 ──
    if state.show_theme_picker {
        let themes = crate::ui::components::highlight::theme_picker::available_themes();
        let count = themes.len();
        let prev_index = state.selected_theme_index;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                state.selected_theme_index = if state.selected_theme_index == 0 {
                    count.saturating_sub(1)
                } else {
                    state.selected_theme_index - 1
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.selected_theme_index = (state.selected_theme_index + 1) % count.max(1);
            }
            KeyCode::Enter => {
                let name = themes
                    .get(state.selected_theme_index)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                if state.theme_manager.set_theme(&name) {
                    state.current_status_line = Some(format!("Theme: {}", name));
                }
                state.show_theme_picker = false;
                return Ok(true);
            }
            KeyCode::Esc => {
                // 取消：恢复进入 picker 前的主题
                if let Some(prev) = state.theme_picker_prev.take() {
                    state.theme_manager.set_theme(&prev);
                }
                state.show_theme_picker = false;
                return Ok(true);
            }
            _ => {}
        }
        // 实时预览：光标移动即切换主题
        if prev_index != state.selected_theme_index {
            if let Some(t) = themes.get(state.selected_theme_index) {
                state.theme_manager.set_theme(&t.name);
            }
        }
        return Ok(true);
    }

    if state.is_awaiting_confirmation {
        if let Some(id) = state.pending_tool_call_id.clone() {
            let is_ask = state
                .pending_confirmation_entry_idx
                .and_then(|idx| state.chat_history.get(idx))
                .and_then(|entry| entry.confirmation.as_ref())
                .map(|conf| {
                    matches!(
                        conf.operation_type,
                        crate::types::ConfirmationType::AskUserQuestion
                    )
                })
                .unwrap_or(false);

            if is_ask {
                handle_ask_user_question_input(key, state, agent_tx, &id).await?;
            } else {
                match key.code {
                    // Ctrl+E: 切换权限解释区 / Ctrl+D: 切换 debug 详情（Claude Code 风格）
                    KeyCode::Char('e') | KeyCode::Char('E')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        state.show_permission_explanation = !state.show_permission_explanation;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        state.show_permission_debug = !state.show_permission_debug;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('1' | 'y' | 'Y') => {
                        state.pending_confirmation_choice = 1;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('2' | 's' | 'S') => {
                        state.pending_confirmation_choice = 2;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('3' | 'a' | 'A') => {
                        state.pending_confirmation_choice = 3;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('4' | 'd' | 'D' | 'n' | 'N') => {
                        state.pending_confirmation_choice = 4;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Esc => {
                        state.pending_confirmation_choice = 4;
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome: crate::types::ToolConfirmationOutcome::Cancel,
                            })
                            .await;
                        state.is_awaiting_confirmation = false;
                        state.pending_tool_call_id = None;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                        state.pending_confirmation_entry_idx = None;
                    }
                    KeyCode::Up => {
                        if state.pending_confirmation_choice > 1 {
                            state.pending_confirmation_choice -= 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Down => {
                        if state.pending_confirmation_choice < 4 {
                            state.pending_confirmation_choice += 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Enter => {
                        let outcome = match state.pending_confirmation_choice {
                            1 => crate::types::ToolConfirmationOutcome::ProceedOnce,
                            2 => crate::types::ToolConfirmationOutcome::AllowSession,
                            3 => crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave,
                            _ => crate::types::ToolConfirmationOutcome::Cancel,
                        };
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    let outcome_str = match state.pending_confirmation_choice {
                                        1 => "Allowed (once)",
                                        2 => "Allowed (session)",
                                        3 => "Allowed (always)",
                                        _ => "Denied",
                                    };
                                    conf.outcome = Some(outcome_str.to_string());
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome,
                            })
                            .await;
                        state.is_awaiting_confirmation = false;
                        state.pending_tool_call_id = None;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.pending_confirmation_entry_idx = None;
                    }
                    _ => {}
                }
            }
        } else {
            state.is_awaiting_confirmation = false;
        }
        return Ok(true);
    }

    if state.show_status_modal {
        match key.code {
            KeyCode::Esc => {
                state.show_status_modal = false;
            }
            KeyCode::Up => {
                let count = crate::ui::components::status_modal::settings_item_count();
                state.settings_selected_index = state
                    .settings_selected_index
                    .saturating_add(count)
                    .saturating_sub(1)
                    % count;
            }
            KeyCode::Down => {
                let count = crate::ui::components::status_modal::settings_item_count();
                state.settings_selected_index = (state.settings_selected_index + 1) % count;
            }
            KeyCode::Enter => {
                // Get the action for the selected setting and execute it
                if let Some(action) =
                    crate::ui::components::status_modal::get_settings_action(state)
                {
                    state.show_status_modal = false;
                    // Re-dispatch the action through execute_palette_action
                    execute_palette_action(state, action, agent_tx).await?;
                }
            }
            _ => {}
        }
        return Ok(true);
    }

    if state.task_panel.is_visible {
        if state.task_panel.is_editing() {
            state.task_panel.handle_edit_input(key);
            return Ok(true);
        }

        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
            state
                .task_panel
                .enter_edit_mode(crate::ui::components::task_panel::EditMode::Title);
            return Ok(true);
        }

        if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
            state
                .task_panel
                .enter_edit_mode(crate::ui::components::task_panel::EditMode::Description);
            return Ok(true);
        }

        if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('v') {
            state.task_panel.cycle_view_mode();
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("View: {}", state.task_panel.view_mode),
            );
            return Ok(true);
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('j') => {
                    state.task_panel.next();
                    return Ok(true);
                }
                KeyCode::Char('k') => {
                    state.task_panel.previous();
                    return Ok(true);
                }
                KeyCode::Char(' ') => {
                    state.task_panel.toggle_status();
                    crate::ui::app::logic::emit_status_text(state, 0, "Task status toggled");
                    return Ok(true);
                }
                KeyCode::Char('s') => {
                    state.task_panel.skip_task();
                    crate::ui::app::logic::emit_status_text(state, 0, "Task skipped");
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    state.task_panel.add_new_task();
                    crate::ui::app::logic::emit_status_text(state, 0, "New task added");
                    return Ok(true);
                }
                KeyCode::Delete => {
                    state.task_panel.delete_selected();
                    crate::ui::app::logic::emit_status_text(state, 0, "Task deleted");
                    return Ok(true);
                }
                KeyCode::Up => {
                    state.task_panel.move_up();
                    return Ok(true);
                }
                KeyCode::Down => {
                    state.task_panel.move_down();
                    return Ok(true);
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Right => {
                    state.task_panel.indent();
                    return Ok(true);
                }
                KeyCode::Left => {
                    state.task_panel.outdent();
                    return Ok(true);
                }
                _ => {}
            }
        }
    }

    Ok(false)
}

async fn handle_input_modal(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !state.show_input_modal {
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Ok(mut clipboard) = Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    state.modal_textarea.insert_str(&text);
                }
            }
        }
        KeyCode::Esc => {
            state.input_context = None;
            if state.is_modal_open() {
                // 底层还有模态（如 Plugins 弹窗）：仅关闭输入框，回到该模态
                state.show_input_modal = false;
                return Ok(true);
            }
            show_palette_mode(state, state.palette_mode.clone());
        }
        KeyCode::Enter => {
            state.input_modal_value = collect_modal_input(&state.modal_textarea);

            if let Some(ctx) = state.input_context.take() {
                match ctx {
                    crate::ui::state::palette::InputContext::ProviderKey { provider_id } => {
                        let key = normalize_modal_api_key(&state.input_modal_value);

                        if !key.is_empty() {
                            let pid = provider_id.clone();
                            let store = crate::core::config::provider_store::ProviderStore::new();
                            let saved_base_url = store.get_base_url(&pid).await.unwrap_or(None);
                            let _ = store.set_api_key(&pid, &key).await;

                            if needs_manual_base_url_confirmation(&pid, saved_base_url.as_deref()) {
                                let initial_value =
                                    crate::core::config::providers::resolve_provider_base_url(
                                        &pid,
                                        saved_base_url,
                                    )
                                    .unwrap_or_default();

                                crate::ui::app::logic::emit_status_text(
                                    state,
                                    0,
                                    &format!(
                                        "Saved API Key for {}. Now confirm Base URL.",
                                        provider_id
                                    ),
                                );
                                show_provider_base_url_modal(state, &pid, initial_value);
                                return Ok(true);
                            }

                            let base_url =
                                crate::core::config::providers::resolve_provider_base_url(
                                    &pid,
                                    saved_base_url,
                                );
                            let selected_model =
                                store.get_selected_model(&pid).await.unwrap_or(None);
                            let _ = store.set_active_provider(&pid).await;
                            state.pending_model_provider = Some(pid.clone());
                            state.pending_provider_selected_model = selected_model.clone();
                            state.current_provider_id = Some(pid.clone());
                            if let Some(model) = selected_model {
                                state.current_model = model;
                            } else {
                                state.current_model.clear();
                            }

                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                &format!("已配置并切换到 {}", provider_id),
                            );
                            state.configured_providers.insert(pid.clone());

                            // Check is_openai_compatible: built-in check first, then custom provider type
                            let provider_config = store.load().await.unwrap_or_default();
                            let is_openai_compat =
                                crate::core::config::providers::provider_openai_compatible_mode(
                                    &pid,
                                )
                                .or_else(|| {
                                    provider_config.providers.get(&pid).and_then(|s| {
                                        s.r#type.as_deref().map(|t| t == "openai-compatible")
                                    })
                                });

                            let _ = agent_tx
                                .send(AgentRequest::UpdateProviderConfig {
                                    provider_id: Some(pid.clone()),
                                    api_key: Some(key),
                                    base_url,
                                    is_openai_compatible: is_openai_compat,
                                    model: state.pending_provider_selected_model.clone(),
                                })
                                .await;
                            let _ = agent_tx.send(AgentRequest::ListModels).await;

                            state.available_models.clear();
                            state.show_input_modal = false;
                            close_quick_menus(state);
                            state.palette_history.clear();
                            state.open_palette(PaletteMode::Model);
                        } else {
                            if matches!(
                                state.quick_menu_back,
                                Some(crate::ui::state::QuickMenuKind::Provider)
                            ) || state.quick_menu_origin_palette
                            {
                                navigate_back_from_quick_menu(state);
                            } else {
                                show_palette_mode(state, state.palette_mode.clone());
                            }
                        }
                    }
                    crate::ui::state::palette::InputContext::ProviderBaseUrl { provider_id } => {
                        let url = normalize_modal_base_url(&state.input_modal_value);
                        let pid = provider_id.clone();

                        let store = crate::core::config::provider_store::ProviderStore::new();
                        let _ = store.set_base_url(&pid, &url).await;
                        let base_url = crate::core::config::providers::resolve_provider_base_url(
                            &pid,
                            Some(url.clone()),
                        );
                        // Check if user has EXPLICITLY saved an API key (not from env vars)
                        let has_explicit_key =
                            store.get_api_key(&pid).await.unwrap_or(None).is_some();

                        if base_url.is_none() {
                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                &format!("Base URL is required for {}", pid),
                            );
                            show_provider_base_url_modal(state, &pid, String::new());
                            return Ok(true);
                        }

                        // Always jump to the API key modal after saving the URL, so the user
                        // has a chance to configure / confirm the key before models are loaded.
                        // The ProviderKey branch handles activation, UpdateProviderConfig and
                        // model listing once a (possibly saved) key is confirmed.
                        show_provider_api_key_modal(state, &pid, false, has_explicit_key);
                        crate::ui::app::logic::emit_status_text(
                            state,
                            0,
                            &format!("Saved Base URL for {}. Now enter API Key.", pid),
                        );
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::ContextWindow => {
                        let input = state.input_modal_value.trim().to_string();
                        let tokens = parse_context_window_str(&input);
                        if let Some(tokens) = tokens {
                            state.context_window_override = Some(tokens);
                            state.current_status_line =
                                Some(format!("Context Window: {}k", tokens / 1000));
                            tokio::spawn(async move {
                                if let Ok(mgr) =
                                    crate::core::config::settings_manager::SettingsManager::new()
                                {
                                    if let Ok(mut settings) = mgr.load_user_settings().await {
                                        settings.context_window = Some(tokens);
                                        let _ = mgr.save_user_settings(&settings).await;
                                    }
                                }
                            });
                        } else {
                            state.current_status_line =
                                Some("Invalid context window size. Use e.g. 128k, 1M".to_string());
                        }
                        state.show_input_modal = false;
                    }
                    crate::ui::state::palette::InputContext::AddProviderId { provider_type } => {
                        let provider_id = state
                            .input_modal_value
                            .trim()
                            .to_lowercase()
                            .replace(' ', "-");
                        if provider_id.is_empty() {
                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                "Provider ID cannot be empty.",
                            );
                            state.input_context =
                                Some(crate::ui::state::palette::InputContext::AddProviderId {
                                    provider_type,
                                });
                            return Ok(true);
                        }
                        // Check for conflicts with built-in providers
                        if crate::core::config::providers::get_provider_by_id(&provider_id)
                            .is_some()
                        {
                            crate::ui::app::logic::emit_status_text(state, 0, &format!("'{}' conflicts with a built-in provider. Choose a different ID.", provider_id));
                            state.input_context =
                                Some(crate::ui::state::palette::InputContext::AddProviderId {
                                    provider_type,
                                });
                            return Ok(true);
                        }
                        // Transition to name input
                        state.show_input_modal = true;
                        state.input_modal_title = "Add New Provider — Enter Name".to_string();
                        state.input_modal_prompt =
                            format!("Enter a display name for '{}':", provider_id);
                        state.input_modal_value = String::new();
                        let mut textarea = TextArea::default();
                        textarea.set_cursor_line_style(ratatui::style::Style::default());
                        textarea.set_placeholder_text("e.g. My LM Studio");
                        textarea.set_cursor_style(
                            ratatui::style::Style::default()
                                .add_modifier(ratatui::style::Modifier::REVERSED),
                        );
                        state.modal_textarea = textarea;
                        state.input_context =
                            Some(crate::ui::state::palette::InputContext::AddProviderName {
                                provider_type,
                                provider_id,
                            });
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddProviderName {
                        provider_type,
                        provider_id,
                    } => {
                        let name = state.input_modal_value.trim().to_string();
                        if name.is_empty() {
                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                "Name cannot be empty.",
                            );
                            state.input_context =
                                Some(crate::ui::state::palette::InputContext::AddProviderName {
                                    provider_type,
                                    provider_id,
                                });
                            return Ok(true);
                        }
                        // Transition to base URL input
                        state.show_input_modal = true;
                        state.input_modal_title =
                            format!("Add New Provider — Base URL for {}", provider_id);
                        state.input_modal_prompt = "Enter the API endpoint base URL:".to_string();
                        state.input_modal_value = String::new();
                        let mut textarea = TextArea::default();
                        textarea.set_cursor_line_style(ratatui::style::Style::default());
                        textarea.set_placeholder_text("e.g. http://localhost:1234/v1");
                        textarea.set_cursor_style(
                            ratatui::style::Style::default()
                                .add_modifier(ratatui::style::Modifier::REVERSED),
                        );
                        state.modal_textarea = textarea;
                        state.input_context = Some(
                            crate::ui::state::palette::InputContext::AddProviderBaseUrl {
                                provider_type,
                                provider_id,
                                name,
                            },
                        );
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddProviderBaseUrl {
                        provider_type,
                        provider_id,
                        name,
                    } => {
                        let base_url = normalize_modal_base_url(&state.input_modal_value);
                        if base_url.is_empty() {
                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                "Base URL cannot be empty.",
                            );
                            state.input_context = Some(
                                crate::ui::state::palette::InputContext::AddProviderBaseUrl {
                                    provider_type,
                                    provider_id,
                                    name,
                                },
                            );
                            return Ok(true);
                        }
                        // Transition to API key input (optional)
                        state.show_input_modal = true;
                        state.input_modal_title =
                            format!("Add New Provider — API Key for {}", provider_id);
                        state.input_modal_prompt =
                            "Enter API key (leave empty to skip):".to_string();
                        state.input_modal_value = String::new();
                        let mut textarea = TextArea::default();
                        textarea.set_cursor_line_style(ratatui::style::Style::default());
                        textarea.set_placeholder_text("Optional — press Enter to skip");
                        textarea.set_cursor_style(
                            ratatui::style::Style::default()
                                .add_modifier(ratatui::style::Modifier::REVERSED),
                        );
                        state.modal_textarea = textarea;
                        state.input_context =
                            Some(crate::ui::state::palette::InputContext::AddProviderApiKey {
                                provider_type,
                                provider_id,
                                name,
                                base_url,
                            });
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddProviderApiKey {
                        provider_type,
                        provider_id,
                        name,
                        base_url,
                    } => {
                        let api_key = normalize_modal_api_key(&state.input_modal_value);
                        let pid = provider_id.clone();
                        let store = crate::core::config::provider_store::ProviderStore::new();

                        // Save provider settings
                        let _ = store.set_base_url(&pid, &base_url).await;
                        if !api_key.is_empty() {
                            let _ = store.set_api_key(&pid, &api_key).await;
                        }

                        // Set provider type and name via a custom save
                        {
                            let mut config = store.load().await.unwrap_or_default();
                            let settings = config.providers.entry(pid.clone()).or_insert(
                                crate::core::config::models::ProviderSettings {
                                    api_key: None,
                                    base_url: None,
                                    selected_model: None,
                                    models: None,
                                    name: None,
                                    description: None,
                                    r#type: None,
                                },
                            );
                            settings.name = Some(name.clone());
                            settings.r#type = Some(provider_type.clone());
                            let _ = store.save(&config).await;
                        }

                        // Activate the provider
                        let _ = store.set_active_provider(&pid).await;
                        state.configured_providers.insert(pid.clone());
                        state.current_provider_id = Some(pid.clone());
                        state.current_model.clear();

                        // Notify agent
                        let resolved_key = crate::core::config::providers::resolve_runtime_api_key(
                            Some(&pid),
                            Some(api_key.clone()),
                        );
                        let is_openai_compat = provider_type == "openai-compatible";
                        let _ = agent_tx
                            .send(AgentRequest::UpdateProviderConfig {
                                provider_id: Some(pid.clone()),
                                api_key: resolved_key,
                                base_url: Some(base_url.clone()),
                                is_openai_compatible: Some(is_openai_compat),
                                model: None,
                            })
                            .await;
                        let _ = agent_tx.send(AgentRequest::ListModels).await;

                        state.available_models.clear();
                        state.show_input_modal = false;
                        close_quick_menus(state);
                        state.palette_history.clear();
                        state.open_palette(PaletteMode::Model);

                        crate::ui::app::logic::emit_status_text(
                            state,
                            0,
                            &format!(
                                "Created provider '{}' ({}) — now choose a model.",
                                name, pid
                            ),
                        );
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::MarketplaceSource => {
                        let source = state.input_modal_value.trim().to_string();
                        state.show_input_modal = false;

                        if source.is_empty() {
                            state.input_context = None;
                            return Ok(true);
                        }

                        // add_marketplace 需要克隆仓库，可能耗时数秒：后台执行，
                        // 完成后经 StreamMessage::PluginOpResult 回填消息并刷新
                        let cwd = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        state.plugin_op_pending = true;
                        state.plugin_message = Some(format!("Adding marketplace {}...", source));
                        let _ = agent_tx
                            .send(AgentRequest::PluginOp {
                                project_root: cwd,
                                op: crate::runtime::messages::PluginOp::AddMarketplace { source },
                            })
                            .await;
                        state.plugin_index = 0;
                        state.input_context = None;
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddWorkingDir => {
                        let path = state.input_modal_value.clone();
                        state.show_input_modal = false;
                        state.input_context = None;

                        if path.trim().is_empty() {
                            return Ok(true);
                        }

                        match crate::commands::extended::add_working_dir_to_state(state, &path) {
                            Ok(msg) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(msg).with_streaming(false),
                                );
                            }
                            Err(e) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(format!("❌ {}", e))
                                        .with_streaming(false),
                                );
                            }
                        }
                        return Ok(true);
                    }
                }
            } else {
                state.show_input_modal = false;
            }
        }
        _ => {
            state.modal_textarea.input(key);
        }
    }
    Ok(true)
}

async fn handle_paste(
    state: &mut ChatState,
    key: KeyEvent,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) || key.code != KeyCode::Char('v') {
        return Ok(false);
    }

    state.paste_in_progress = true;
    state.paste_end_time = Some(Instant::now());

    let clipboard_result = tokio::task::spawn_blocking(|| {
        if let Some((path, w, h)) = save_clipboard_image() {
            return Ok(ClipboardResult::Image(path, w, h));
        }
        if let Ok(mut clipboard) = Clipboard::new() {
            if let Ok(text) = clipboard.get_text() {
                return Ok(ClipboardResult::Text(text));
            }
        }
        Err("Clipboard empty".to_string())
    })
    .await;

    match clipboard_result {
        Ok(Ok(ClipboardResult::Image(path, w, h))) => {
            insert_image_paste_block(state, path, w, h);
            sync_input_from_textarea(state);
            crate::ui::components::command_suggestions::on_input_changed(state);
        }
        Ok(Ok(ClipboardResult::Text(text))) => {
            if let Some(file_paths) = detect_file_paths(&text) {
                insert_file_paste_block(state, file_paths);
            } else {
                insert_paste_block(state, text);
                if state.paste_segments.is_empty() {
                    maybe_auto_fold_input(state);
                }
            }
            sync_input_from_textarea(state);
            crate::ui::components::command_suggestions::on_input_changed(state);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Parse a context window size string like "128k", "1M", "2000000" into tokens.
fn parse_context_window_str(s: &str) -> Option<u32> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(num_str) = s.strip_suffix('k') {
        num_str.parse::<u32>().ok().map(|v| v * 1000)
    } else if let Some(num_str) = s.strip_suffix('m') {
        num_str.parse::<u32>().ok().map(|v| v * 1_000_000)
    } else {
        s.parse::<u32>().ok()
    }
}

/// Find the nearest ToolCall entry to the current viewport center for keyboard toggle.
/// Returns the index into chat_history, or None if no ToolCall entry is visible.
fn find_focused_tool_entry(state: &ChatState) -> Option<usize> {
    use crate::types::ChatEntryType;

    let viewport_center = state.scroll + (state.last_chat_height as usize / 2);

    // Only search for ToolCall entries (ToolResult has no tool_call field to toggle)
    let mut best_idx: Option<usize> = None;
    let mut best_distance: usize = usize::MAX;

    for (idx, entry) in state.chat_history.iter().enumerate() {
        if entry.entry_type != ChatEntryType::ToolCall {
            continue;
        }
        // Approximate: use entry index as rough position indicator
        let distance = if idx > viewport_center / 2 {
            idx.saturating_sub(viewport_center / 2)
        } else {
            (viewport_center / 2).saturating_sub(idx)
        };
        if distance < best_distance {
            best_distance = distance;
            best_idx = Some(idx);
        }
    }

    best_idx
}

/// Apply a vim motion to the textarea cursor.
fn apply_vim_motion(state: &mut ChatState, motion: &crate::ui::vim::motions::Motion) {
    use crate::ui::vim::motions::Motion;
    match motion {
        Motion::Left => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Back);
        }
        Motion::Down => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Down);
        }
        Motion::Up => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Up);
        }
        Motion::Right => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::Forward);
        }
        Motion::WordForward => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordForward);
        }
        Motion::WordBackward => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordBack);
        }
        Motion::WordEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        Motion::LineStart => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
        }
        Motion::LineEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        Motion::FileStart => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Top);
        }
        Motion::FileEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        }
        _ => {
            // Paragraph, matching bracket, find char — approximate with word movements
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordForward);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::store::AgentTaskInfo;

    fn bg_task(id: &str) -> AgentTaskInfo {
        AgentTaskInfo {
            task_id: id.to_string(),
            agent_type: "Explore".to_string(),
            description: "研究 src/hooks 目录".to_string(),
            status: crate::types::AgentTaskStatus::Running,
            tool_use_count: 3,
            tokens: 1200,
            is_async: true,
            is_resolved: false,
            is_error: false,
            last_tool_info: None,
            name: None,
            task_description: None,
            started_at: Instant::now(),
            finished_at: None,
            sub_entries: Vec::new(),
            entry_idx: 0,
        }
    }

    fn press(state: &mut ChatState, code: KeyCode) -> bool {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        handle_bg_agent_selector_key(state, key)
    }

    /// ↑/↓ 在 main 行与各代理行之间移动；在 main 行继续 ↑ 把焦点交回输入框。
    #[test]
    fn selector_keys_move_focus_and_release_to_input() {
        let mut state = ChatState::new();
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a"));
        state
            .active_agent_tasks
            .insert("b".to_string(), bg_task("b"));
        state.bg_agent_selection = Some(0);

        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(1));
        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(2));
        // 到底了不再往下
        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(2));

        for expected in [Some(1), Some(0), None] {
            assert!(press(&mut state, KeyCode::Up));
            assert_eq!(state.bg_agent_selection, expected);
        }
    }

    /// Enter 在「看主会话」与「看某个代理」之间切换，且不移动焦点行；
    /// Esc 只收起选择器，不改变正在看的详情。
    #[test]
    fn selector_enter_toggles_view_and_esc_keeps_it() {
        let mut state = ChatState::new();
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a"));
        state.bg_agent_selection = Some(1);

        assert!(press(&mut state, KeyCode::Enter));
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
        assert_eq!(state.bg_agent_selection, Some(1));

        // 同一行再按 Enter → 收回主会话
        assert!(press(&mut state, KeyCode::Enter));
        assert!(state.viewing_agent_task_id.is_none());

        // 进详情后按 Esc：选择器收起，详情保留（详情自己的 Esc 在主 Esc 分支里处理）
        assert!(press(&mut state, KeyCode::Enter));
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
        assert!(press(&mut state, KeyCode::Esc));
        assert!(state.bg_agent_selection.is_none());
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
    }

    /// 没有后台代理时选择器不该抢键，并顺手把残留焦点清掉。
    #[test]
    fn selector_yields_when_no_background_agents() {
        let mut state = ChatState::new();
        state.bg_agent_selection = Some(0);
        assert!(!press(&mut state, KeyCode::Down));
        assert!(state.bg_agent_selection.is_none());
    }
}
