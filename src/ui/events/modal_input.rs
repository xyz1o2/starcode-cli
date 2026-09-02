//! Unified modal key dispatch.
//!
//! `handle_modal_key` is called FIRST from `handle_key_event`; when a modal is
//! on the stack it consumes the key. Esc pops one view/modal level, matching
//! Claude Code's "Esc goes back one view, closes at the outermost" rule.

use crate::runtime::messages::AgentRequest;
use crate::ui::state::modal::{
    load_mcp_server_rows, load_mcp_tools, set_mcp_server_disabled, MarketTab, McpView,
    PluginConfirmKind, PluginTab, Modal,
};
use crate::ui::state::ChatState;
use tokio::sync::mpsc;

type Err = Box<dyn std::error::Error>;

/// Returns `Ok(true)` when the key was consumed by a modal.
pub async fn handle_modal_key(
    state: &mut ChatState,
    key: crossterm::event::KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Err> {
    let Some(top) = state.top_modal().cloned() else {
        return Ok(false);
    };

    // 输入模态（老体系）优先：由 input.rs 的 input-modal 分支处理
    if state.show_input_modal {
        return Ok(false);
    }

    match top {
        Modal::Palette => handle_palette(state, key, agent_tx).await,
        Modal::Mcp { view } => handle_mcp(state, key, view, agent_tx).await,
        Modal::Market { tab } => handle_market(state, key, tab, agent_tx).await,
        Modal::Plugins { tab } => handle_plugins(state, key, tab, agent_tx).await,
    }
}

async fn handle_palette(
    state: &mut ChatState,
    key: crossterm::event::KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Err> {
    use crossterm::event::KeyCode::*;

    let query = state.palette_filter.trim().to_lowercase();
    let search_items =
        crate::ui::components::palette::get_search_items(&state.palette_mode, state, &query);
    let items_len = search_items
        .iter()
        .filter(|item| crate::ui::components::palette::palette_item_matches_query(item, &query))
        .count();

    match key.code {
        Esc => {
            if let Some(prev_mode) = state.palette_history.pop() {
                let items =
                    crate::ui::components::palette::get_items(&prev_mode, state);
                state.palette_mode = prev_mode;
                state.palette_items = items;
                state.selected_palette_index = 0;
                state.palette_filter.clear();
            } else {
                state.close_palette();
            }
        }
        Up => {
            if items_len > 0 {
                if state.selected_palette_index > 0 {
                    state.selected_palette_index -= 1;
                } else {
                    state.selected_palette_index = items_len - 1;
                }
            }
        }
        Down => {
            if state.is_awaiting_confirmation {
                if state.pending_confirmation_choice < 4 {
                    state.pending_confirmation_choice += 1;
                }
                return Ok(true);
            }
            if items_len > 0 {
                if state.selected_palette_index < items_len - 1 {
                    state.selected_palette_index += 1;
                } else {
                    state.selected_palette_index = 0;
                }
            }
        }
        PageUp => {
            if items_len > 0 {
                state.selected_palette_index = state.selected_palette_index.saturating_sub(10);
            }
        }
        PageDown => {
            if items_len > 0 {
                state.selected_palette_index =
                    (state.selected_palette_index + 10).min(items_len - 1);
            }
        }
        Enter => {
            let filtered_items: Vec<_> = search_items
                .iter()
                .filter(|item| {
                    crate::ui::components::palette::palette_item_matches_query(item, &query)
                })
                .collect();

            if let Some(selected_item) = filtered_items.get(state.selected_palette_index) {
                let action = selected_item.action.clone();
                super::input::execute_palette_action(state, action, agent_tx).await?;
            }
        }
        Tab | BackTab => {
            if items_len > 0 {
                if state.selected_palette_index < items_len - 1 {
                    state.selected_palette_index += 1;
                } else {
                    state.selected_palette_index = 0;
                }
            }
        }
        Backspace => {
            state.palette_filter.pop();
            state.selected_palette_index = 0;
        }
        Char(c) if c.is_control() => {
            if c == '\t' && items_len > 0 {
                if state.selected_palette_index < items_len - 1 {
                    state.selected_palette_index += 1;
                } else {
                    state.selected_palette_index = 0;
                }
            }
        }
        Char(c) => {
            if state.palette_mode == crate::ui::state::PaletteMode::Provider && c == 'e' {
                let filtered_items: Vec<_> = search_items
                    .iter()
                    .filter(|item| {
                        crate::ui::components::palette::palette_item_matches_query(item, &query)
                    })
                    .collect();

                if let Some(selected_item) = filtered_items.get(state.selected_palette_index) {
                    let provider_id = match &selected_item.action {
                        crate::ui::state::PaletteAction::SelectProvider(pid) => Some(pid.clone()),
                        crate::ui::state::PaletteAction::InputApiKey(pid) => Some(pid.clone()),
                        crate::ui::state::PaletteAction::InputBaseUrl(pid) => Some(pid.clone()),
                        crate::ui::state::PaletteAction::Navigate(
                            crate::ui::state::PaletteMode::ProviderOptions(pid),
                        ) => Some(pid.clone()),
                        _ => None,
                    };

                    if let Some(pid) = provider_id {
                        let store = crate::core::config::provider_store::ProviderStore::new();
                        let has_saved_key =
                            store.get_api_key(&pid).await.unwrap_or(None).is_some();
                        super::input::show_provider_api_key_modal(state, &pid, true, has_saved_key);
                        return Ok(true);
                    }
                }
            }

            state.palette_filter.push(c);
            state.selected_palette_index = 0;
        }
        _ => {}
    }
    Ok(true)
}

async fn handle_mcp(
    state: &mut ChatState,
    key: crossterm::event::KeyEvent,
    view: McpView,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Err> {
    use crossterm::event::KeyCode::*;

    match view {
        McpView::List => match key.code {
            Esc => {
                state.pop_modal();
            }
            Up | Left => {
                if state.mcp_modal_index > 0 {
                    state.mcp_modal_index -= 1;
                } else if !state.mcp_modal_servers.is_empty() {
                    state.mcp_modal_index = state.mcp_modal_servers.len() - 1;
                }
            }
            Down | Right | Tab => {
                if !state.mcp_modal_servers.is_empty() {
                    state.mcp_modal_index = (state.mcp_modal_index + 1) % state.mcp_modal_servers.len();
                }
            }
            Char('r') | Char('R') => {
                state.mcp_modal_action_msg = None;
                load_mcp_server_rows(state).await;
            }
            Char('n') | Char('N') => {
                state.close_all_modals();
                if !state.input.is_empty() {
                    state.input.push(' ');
                }
                state.input.push_str("/mcp add ");
                crate::ui::components::command_suggestions::on_input_changed(state);
            }
            Enter => {
                if let Some(row) = state.mcp_modal_servers.get(state.mcp_modal_index).cloned() {
                    state.mcp_modal_menu_index = 0;
                    state.mcp_modal_action_msg = None;
                    state.push_modal(Modal::Mcp {
                        view: McpView::ServerMenu { name: row.name },
                    });
                }
            }
            _ => {}
        },

        McpView::ServerMenu { name } => {
            let row = state
                .mcp_modal_servers
                .iter()
                .find(|r| r.name == name)
                .cloned();
            // needs-auth 服务器多一项 Authenticate（0=tools 1=reconnect 2=toggle
            // [3=auth] [remove] [back]）
            let has_auth = row.as_ref().map(|r| r.needs_auth).unwrap_or(false);
            let auth_idx = if has_auth { Some(3usize) } else { None };
            let remove_idx = if has_auth { 4 } else { 3 };
            let back_idx = if has_auth { 5 } else { 4 };
            let menu_len = back_idx + 1;
            match key.code {
                Esc => {
                    state.pop_modal();
                }
                Up => {
                    if state.mcp_modal_menu_index > 0 {
                        state.mcp_modal_menu_index -= 1;
                    } else {
                        state.mcp_modal_menu_index = menu_len - 1;
                    }
                }
                Down => {
                    state.mcp_modal_menu_index = (state.mcp_modal_menu_index + 1) % menu_len;
                }
                Enter => match state.mcp_modal_menu_index {
                    0 => {
                        // View tools
                        state.mcp_modal_error = None;
                        load_mcp_tools(state, &name).await;
                        state.push_modal(Modal::Mcp {
                            view: McpView::Tools { name },
                        });
                    }
                    1 => {
                        // Reconnect
                        state.mcp_modal_action_msg = Some(format!("Reconnecting {}…", name));
                        let _ = agent_tx.send(AgentRequest::McpRefresh).await;
                        load_mcp_server_rows(state).await;
                        state.mcp_modal_action_msg =
                            Some(format!("Reconnected (reloaded) {}", name));
                    }
                    2 => {
                        // Enable/Disable toggle
                        let disabled = row.map(|r| r.disabled).unwrap_or(false);
                        match set_mcp_server_disabled(&name, !disabled).await {
                            Ok(()) => {
                                state.mcp_modal_action_msg = Some(if disabled {
                                    format!("Enabled {}", name)
                                } else {
                                    format!("Disabled {}", name)
                                });
                                let _ = agent_tx.send(AgentRequest::McpRefresh).await;
                                load_mcp_server_rows(state).await;
                            }
                            Err(e) => state.mcp_modal_action_msg = Some(format!("Error: {}", e)),
                        }
                    }
                    idx if Some(idx) == auth_idx => {
                        // Authenticate：从错误信息提取授权 URL 并打开浏览器
                        // （对标 Claude Code "Enter to auth"）
                        let url = row
                            .as_ref()
                            .and_then(|r| r.error.as_deref())
                            .and_then(crate::ui::state::modal::extract_oauth_url);
                        match url {
                            Some(u) => match open_url_in_browser(&u).await {
                                Ok(()) => {
                                    state.mcp_modal_action_msg =
                                        Some(format!("Opening auth page: {}", u))
                                }
                                Err(e) => {
                                    state.mcp_modal_action_msg =
                                        Some(format!("Failed to open browser: {} — {}", e, u))
                                }
                            },
                            None => {
                                state.mcp_modal_action_msg =
                                    Some("No auth URL available. Reconnect first.".to_string());
                            }
                        }
                    }
                    idx if idx == remove_idx => {
                        state.push_modal(Modal::Mcp {
                            view: McpView::ConfirmRemove { name },
                        });
                    }
                    _ => {
                        state.pop_modal();
                    }
                },
                _ => {}
            }
        }

        McpView::Tools { .. } => match key.code {
            Esc => {
                state.pop_modal();
                state.mcp_modal_error = None;
            }
            Up => {
                if state.mcp_modal_index > 0 {
                    state.mcp_modal_index -= 1;
                } else if !state.mcp_modal_tools.is_empty() {
                    state.mcp_modal_index = state.mcp_modal_tools.len() - 1;
                }
            }
            Down | Tab => {
                if !state.mcp_modal_tools.is_empty() {
                    state.mcp_modal_index = (state.mcp_modal_index + 1) % state.mcp_modal_tools.len();
                }
            }
            Enter => {
                let name = match view {
                    McpView::Tools { name } => name,
                    _ => String::new(),
                };
                let idx = state.mcp_modal_index;
                state.push_modal(Modal::Mcp {
                    view: McpView::ToolDetail { name, index: idx },
                });
            }
            _ => {}
        },

        McpView::ToolDetail { .. } => {
            if key.code == Esc {
                state.pop_modal();
            }
        }

        McpView::ConfirmRemove { name } => match key.code {
            Esc => {
                state.pop_modal();
            }
            Enter => {
                match crate::commands::mcp::remove_mcp_server(&name).await {
                    Ok(msg) => state.mcp_modal_action_msg = Some(msg),
                    Err(e) => state.mcp_modal_action_msg = Some(format!("Error: {}", e)),
                }
                let _ = agent_tx.send(AgentRequest::McpRefresh).await;
                load_mcp_server_rows(state).await;
                state.pop_modal(); // close ConfirmRemove
            }
            _ => {}
        },
    }
    Ok(true)
}

async fn handle_market(
    state: &mut ChatState,
    key: crossterm::event::KeyEvent,
    tab: MarketTab,
    _agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Err> {
    use crossterm::event::{KeyCode::*, KeyModifiers};

    // Confirm overlay takes priority
    if state.market_confirm.is_some() {
        match key.code {
            Esc => state.market_confirm = None,
            Enter => {
                let confirm = state.market_confirm.take().unwrap();
                let is_plugin = state.market_plugin_names.contains(&confirm.name);
                let message = if confirm.install {
                    match crate::core::extensions::marketplace::Marketplace::new()
                        .install(&confirm.name)
                        .await
                    {
                        Ok(r) => r.message,
                        Err(e) => format!("Error: {}", e),
                    }
                } else if is_plugin {
                    uninstall_plugin(state, &confirm.name).await
                } else {
                    match crate::core::extensions::marketplace::Marketplace::new()
                        .uninstall(&confirm.name)
                    {
                        Ok(r) => r.message,
                        Err(e) => format!("Error: {}", e),
                    }
                };
                state.market_message = Some(message);
                state.reload_market_entries().await;
            }
            _ => {}
        }
        return Ok(true);
    }

    match key.code {
        Esc => {
            state.market_query.clear();
            state.pop_modal();
        }
        Tab | Right => {
            let new_tab = tab.next();
            state.pop_modal();
            state.push_modal(Modal::Market { tab: new_tab });
            state.market_index = 0;
            state.reload_market_entries().await;
        }
        BackTab | Left => {
            let new_tab = tab.prev();
            state.pop_modal();
            state.push_modal(Modal::Market { tab: new_tab });
            state.market_index = 0;
            state.reload_market_entries().await;
        }
        Up => {
            if state.market_index > 0 {
                state.market_index -= 1;
            } else if !state.market_entries.is_empty() {
                state.market_index = state.market_entries.len() - 1;
            }
        }
        Down => {
            if !state.market_entries.is_empty() {
                state.market_index = (state.market_index + 1) % state.market_entries.len();
            }
        }
        Backspace => {
            if matches!(state.top_modal(), Some(Modal::Market { tab: MarketTab::Browse })) && !state.market_query.is_empty() {
                state.market_query.pop();
                state.market_index = 0;
                state.reload_market_entries().await;
            }
        }
        Char('/') => {
            // focus search: '/' is the search prefix itself, no-op marker
        }
        Char('u') | Char('U') if matches!(tab, MarketTab::Installed) => {
            if let Some(entry) = state.market_entries.get(state.market_index) {
                state.market_confirm = Some(crate::ui::state::modal::MarketConfirm {
                    name: entry.name.clone(),
                    install: false,
                });
            }
        }
        Enter => match tab {
            MarketTab::Browse => {
                if let Some(entry) = state.market_entries.get(state.market_index) {
                    let name = entry.name.clone();
                    state.market_confirm = Some(crate::ui::state::modal::MarketConfirm {
                        name,
                        install: true,
                    });
                }
            }
            MarketTab::Installed => {
                // toggle enable/disable（按来源分流：plugins 系统 / extensions 注册表）
                if let Some(entry) = state.market_entries.get(state.market_index).cloned() {
                    let name = entry.name.clone();
                    if state.market_plugin_names.contains(&name) {
                        let current = state
                            .market_enabled_map
                            .get(&name)
                            .copied()
                            .unwrap_or(true);
                        let cwd = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        match crate::core::plugins::set_plugin_enabled(&cwd, &name, !current).await
                        {
                            Ok(Some(_)) => {
                                state.market_message = Some(if current {
                                    format!("Disabled plugin {}", name)
                                } else {
                                    format!("Enabled plugin {}", name)
                                });
                            }
                            Ok(None) => {
                                state.market_message = Some(format!("Plugin not found: {}", name));
                            }
                            Err(e) => state.market_message = Some(format!("Error: {}", e)),
                        }
                    } else {
                        use crate::core::extensions::registry::ExtensionRegistryManager;
                        let current = state
                            .market_enabled_map
                            .get(&name)
                            .copied()
                            .unwrap_or(true);
                        match ExtensionRegistryManager::new().set_enabled(&name, !current) {
                            Ok(()) => {
                                state.market_message = Some(if current {
                                    format!("Disabled {}", name)
                                } else {
                                    format!("Enabled {}", name)
                                });
                            }
                            Err(e) => state.market_message = Some(format!("Error: {}", e)),
                        }
                    }
                    state.reload_market_entries().await;
                }
            }
            MarketTab::Sources => {}
        },
        Char(c) => {
            if matches!(state.top_modal(), Some(Modal::Market { tab: MarketTab::Browse }))
                && !c.is_control()
            {
                state.market_query.push(c);
                state.market_index = 0;
                state.reload_market_entries().await;
            }
        }
        _ => {}
    }
    let _ = KeyModifiers::NONE;
    Ok(true)
}

/// 通过 plugins 系统卸载插件（`U` 键 / 确认回车），返回结果消息。
async fn uninstall_plugin(state: &mut ChatState, name: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match crate::core::plugins::remove_plugin(&cwd, name).await {
        Ok(true) => format!("Uninstalled plugin {}", name),
        Ok(false) => format!("Plugin not found: {}", name),
        Err(e) => format!("Error: {}", e),
    }
}

/// 跨平台打开 URL（MCP OAuth 授权页等）。
/// Windows: `cmd /c start`（CREATE_NO_WINDOW 防闪控制台）；
/// macOS: `open`；Linux: `xdg-open`。
async fn open_url_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        tokio::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .creation_flags(0x0800_0000)
            .status()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        tokio::process::Command::new("open")
            .arg(url)
            .status()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        tokio::process::Command::new("xdg-open")
            .arg(url)
            .status()
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// 发送后台安装操作（scope 已定）：设 pending 提示并经 PluginOp 通道执行。
async fn send_install_op(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    plugin: crate::core::plugins::marketplace::MarketplacePlugin,
    scope: &str,
) {
    state.plugin_op_pending = true;
    state.plugin_batch_total = 1;
    state.plugin_batch_done = 0;
    state.plugin_message = Some(format!("Installing plugin {} ({})...", plugin.name, scope));
    state.plugin_detail = None;
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let _ = agent_tx
        .send(AgentRequest::PluginOp {
            project_root: cwd,
            op: crate::runtime::messages::PluginOp::InstallPlugin {
                plugin,
                scope: scope.to_string(),
            },
        })
        .await;
}

/// `/plugin` 管理弹窗：Discover / Installed / Marketplaces / Errors 四 tab。
async fn handle_plugins(
    state: &mut ChatState,
    key: crossterm::event::KeyEvent,
    tab: PluginTab,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Err> {
    use crate::runtime::messages::PluginOp;
    use crossterm::event::KeyCode::*;

    // 确认浮层优先
    if let Some(confirm) = state.plugin_confirm.clone() {
        // scope 选择浮层（对标 Claude Code scope 菜单）：u=user / p=project
        if let PluginConfirmKind::InstallScope { plugin } = &confirm.kind {
            match key.code {
                Esc => state.plugin_confirm = None,
                Char('u') | Char('U') => {
                    state.plugin_confirm = None;
                    send_install_op(state, agent_tx, plugin.clone(), "user").await;
                }
                Char('p') | Char('P') => {
                    state.plugin_confirm = None;
                    send_install_op(state, agent_tx, plugin.clone(), "project").await;
                }
                _ => {}
            }
            return Ok(true);
        }

        match key.code {
            Esc => state.plugin_confirm = None,
            Enter => {
                state.plugin_confirm = None;
                // 安装/移除 marketplace 涉及 git clone 或删目录，放后台执行，
                // 完成后经 StreamMessage::PluginOpResult 回填消息并刷新列表
                match confirm.kind {
                    PluginConfirmKind::Install { plugin, scope } => {
                        send_install_op(state, agent_tx, plugin, &scope).await;
                    }
                    PluginConfirmKind::InstallScope { .. } => unreachable!(),
                    PluginConfirmKind::RemoveMarketplace { name } => {
                        state.plugin_op_pending = true;
                        state.plugin_message =
                            Some(format!("Removing marketplace {}...", name));
                        let cwd = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        let _ = agent_tx
                            .send(AgentRequest::PluginOp {
                                project_root: cwd,
                                op: PluginOp::RemoveMarketplace { name },
                            })
                            .await;
                    }
                    PluginConfirmKind::Uninstall { name } => {
                        let message = uninstall_plugin(state, &name).await;
                        state.plugin_message = Some(message);
                        state.reload_plugins_state().await;
                    }
                }
            }
            _ => {}
        }
        return Ok(true);
    }

    // 插件详情页按键（对标 Claude Code 详情视图）：Esc 返回，Enter/i 安装
    if let Some((mp_name, plugin)) = state.plugin_detail.clone() {
        match key.code {
            Esc => state.plugin_detail = None,
            Enter | Char('i') | Char('I') => {
                state.plugin_confirm =
                    Some(crate::ui::state::modal::PluginConfirm {
                        kind: PluginConfirmKind::InstallScope {
                            plugin: plugin.clone(),
                        },
                    });
            }
            _ => {}
        }
        let _ = mp_name;
        return Ok(true);
    }

    let discover_indices = matches!(tab, PluginTab::Discover)
        .then(|| state.filtered_discover_indices());
    let list_len = match tab {
        // Discover 用过滤后的数量（搜索框实时过滤）
        PluginTab::Discover => discover_indices.as_ref().map_or(0, |v| v.len()),
        PluginTab::Installed => state.plugin_installed.len(),
        // 行 0 = Add marketplace…
        PluginTab::Marketplaces => state.plugin_marketplaces.len() + 1,
        PluginTab::Errors => state.plugin_errors.len(),
    };

    match key.code {
        Esc => {
            // 对标 Claude Code：搜索模式下 Esc 先清搜索，再按才退出弹窗
            if matches!(tab, PluginTab::Discover) && !state.plugin_search.is_empty() {
                state.plugin_search.clear();
                state.plugin_index = 0;
            } else {
                state.plugin_message = None;
                state.pop_modal();
            }
        }
        Tab => {
            let new_tab = tab.next();
            if let Some(Modal::Plugins { tab: t }) = state.top_modal_mut() {
                *t = new_tab;
            }
            state.plugin_index = 0;
            // 后台操作进行中时保留提示消息（如 "Registering..." / "Installing..."）
            if !state.plugin_op_pending {
                state.plugin_message = None;
            }
            state.reload_plugins_state().await;
        }
        BackTab => {
            let new_tab = tab.prev();
            if let Some(Modal::Plugins { tab: t }) = state.top_modal_mut() {
                *t = new_tab;
            }
            state.plugin_index = 0;
            if !state.plugin_op_pending {
                state.plugin_message = None;
            }
            state.reload_plugins_state().await;
        }
        Up => {
            if list_len > 0 {
                if state.plugin_index > 0 {
                    state.plugin_index -= 1;
                } else {
                    state.plugin_index = list_len - 1;
                }
            }
        }
        Down => {
            if list_len > 0 {
                state.plugin_index = (state.plugin_index + 1) % list_len;
            }
        }
        // Discover tab：Space 勾选/取消当前行（对标 Claude Code 批量安装多选）
        Char(' ') if matches!(tab, PluginTab::Discover) => {
            let indices = state.filtered_discover_indices();
            if let Some(&di) = indices.get(state.plugin_index) {
                if let Some(row) = state.plugin_discover.get(di) {
                    let name = row.plugin.name.clone();
                    if !state.plugin_selected.remove(&name) {
                        state.plugin_selected.insert(name);
                    }
                }
            }
        }
        // Discover tab：i 批量安装已勾选插件（逐个走后台任务）；无勾选时
        // 落入下方 Char(c) 搜索臂作为普通输入
        Char('i') | Char('I')
            if matches!(tab, PluginTab::Discover) && !state.plugin_selected.is_empty() =>
        {
            use crate::runtime::messages::PluginOp;
            let plugins: Vec<_> = state
                .plugin_discover
                .iter()
                .filter(|r| state.plugin_selected.contains(&r.plugin.name))
                .map(|r| r.plugin.clone())
                .collect();
            if !plugins.is_empty() {
                state.plugin_op_pending = true;
                state.plugin_batch_total = plugins.len();
                state.plugin_batch_done = 0;
                state.plugin_message =
                    Some(format!("Installing 0/{} plugins...", plugins.len()));
                let cwd =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                for p in plugins {
                    let _ = agent_tx
                        .send(AgentRequest::PluginOp {
                            project_root: cwd.clone(),
                            op: PluginOp::InstallPlugin {
                                plugin: p,
                                scope: "project".to_string(),
                            },
                        })
                        .await;
                }
                state.plugin_selected.clear();
                state.plugin_index = 0;
            }
        }
        // Discover tab：直接输入即搜索（实时过滤），Backspace 删字符
        Char(c) if matches!(tab, PluginTab::Discover) => {
            state.plugin_search.push(c);
            state.plugin_index = 0;
        }
        Backspace if matches!(tab, PluginTab::Discover) => {
            state.plugin_search.pop();
            state.plugin_index = 0;
        }
        Char('a') | Char('A') if matches!(tab, PluginTab::Marketplaces) => {
            open_marketplace_source_input(state);
        }
        // Marketplaces tab：u 更新所选 marketplace（官方走 GCS，其他重 clone）
        Char('u') | Char('U') if matches!(tab, PluginTab::Marketplaces) => {
            if state.plugin_index > 0 {
                if let Some(m) = state
                    .plugin_marketplaces
                    .get(state.plugin_index - 1)
                    .cloned()
                {
                    state.plugin_op_pending = true;
                    state.plugin_message = Some(format!("Updating {}...", m.name));
                    let cwd = std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let _ = agent_tx
                        .send(AgentRequest::PluginOp {
                            project_root: cwd,
                            op: PluginOp::UpdateMarketplace { name: m.name },
                        })
                        .await;
                }
            }
        }
        // Errors tab：r 重试（刷新重查，瞬态错误重试后即自愈）
        Char('r') | Char('R') if matches!(tab, PluginTab::Errors) => {
            state.plugin_message = Some("Retrying...".to_string());
            state.plugin_errors_hint = None;
            state.reload_plugins_state().await;
        }
        Enter => match tab {
            PluginTab::Discover => {
                // 对标 Claude Code：Enter 进插件详情页（版本/作者/安装入口）
                let indices = state.filtered_discover_indices();
                if let Some(&di) = indices.get(state.plugin_index) {
                    if let Some(row) = state.plugin_discover.get(di).cloned() {
                        state.plugin_detail = Some((row.marketplace, row.plugin));
                    }
                }
            }
            PluginTab::Installed => {
                if let Some(p) = state.plugin_installed.get(state.plugin_index).cloned() {
                    let current = p.entry.enabled;
                    let cwd = std::env::current_dir()
                        .unwrap_or_else(|_| std::path::PathBuf::from("."));
                    match crate::core::plugins::set_plugin_enabled(&cwd, &p.entry.name, !current)
                        .await
                    {
                        Ok(Some(_)) => {
                            state.plugin_message = Some(if current {
                                format!("Disabled plugin {}", p.entry.name)
                            } else {
                                format!("Enabled plugin {}", p.entry.name)
                            });
                        }
                        Ok(None) => {
                            state.plugin_message =
                                Some(format!("Plugin not found: {}", p.entry.name));
                        }
                        Err(e) => state.plugin_message = Some(format!("Error: {}", e)),
                    }
                    state.reload_plugins_state().await;
                }
            }
            PluginTab::Marketplaces => {
                if state.plugin_index == 0 {
                    open_marketplace_source_input(state);
                } else if let Some(m) = state
                    .plugin_marketplaces
                    .get(state.plugin_index - 1)
                    .cloned()
                {
                    state.plugin_confirm = Some(crate::ui::state::modal::PluginConfirm {
                        kind: PluginConfirmKind::RemoveMarketplace { name: m.name },
                    });
                }
            }
            PluginTab::Errors => {}
        },
        _ => {}
    }
    Ok(true)
}

/// 打开输入模态录入 marketplace 来源（git URL / owner/repo / 本地路径）。
fn open_marketplace_source_input(state: &mut ChatState) {
    state.show_status_modal = false;
    state.show_input_modal = true;
    state.input_modal_title = "Add Marketplace".to_string();
    state.input_modal_prompt =
        "Marketplace source (git URL, owner/repo, or local path):".to_string();
    state.input_modal_value.clear();
    let mut textarea = tui_textarea::TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_placeholder_text("https://github.com/owner/repo or local path");
    textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );
    state.modal_textarea = textarea;
    state.input_context = Some(crate::ui::state::palette::InputContext::MarketplaceSource);
}
