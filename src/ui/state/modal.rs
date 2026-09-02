//! Unified modal system.
//!
//! Replaces the previous scattered `show_*: bool` flags with a single modal
//! stack: at most one modal is on top, Esc pops it, the stack emptying returns
//! to the chat view. Mirrors Claude Code's dialog architecture where complex
//! dialogs are view state machines (`/mcp` → list → server-menu → tools →
//! detail; `/plugin` → tabs → nested views).
//!
//! Palette keeps its existing `palette_mode`/`palette_history` view state;
//! the stack only tracks that it is open (`Modal::Palette`).

use crate::core::mcp::MCPManager;
use crate::core::mcp::types::MCPTool;
use crate::core::mcp::load_project_mcp_config;
use crate::core::mcp::save_project_mcp_config;
use crate::ui::state::palette::PaletteMode;
use crate::ui::state::ChatState;
use std::time::Duration;

/// One entry in the modal stack. Only the top modal receives keys / renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Command palette (view state lives in `ChatState::palette_mode` etc.)
    Palette,
    /// `/mcp` manager — view state machine
    Mcp { view: McpView },
    /// `/extension market` — tabbed marketplace browser
    Market { tab: MarketTab },
    /// `/plugin` — Claude Code style plugin manager (Discover/Installed/Marketplaces/Errors)
    Plugins { tab: PluginTab },
}

/// Tabs for the plugin manager modal (mirrors Claude Code PluginSettings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTab {
    Discover,
    Installed,
    Marketplaces,
    Errors,
}

impl PluginTab {
    pub const ALL: [PluginTab; 4] = [
        PluginTab::Discover,
        PluginTab::Installed,
        PluginTab::Marketplaces,
        PluginTab::Errors,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            PluginTab::Discover => "Discover",
            PluginTab::Installed => "Installed",
            PluginTab::Marketplaces => "Marketplaces",
            PluginTab::Errors => "Errors",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            PluginTab::Discover => PluginTab::Installed,
            PluginTab::Installed => PluginTab::Marketplaces,
            PluginTab::Marketplaces => PluginTab::Errors,
            PluginTab::Errors => PluginTab::Discover,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            PluginTab::Discover => PluginTab::Errors,
            PluginTab::Installed => PluginTab::Discover,
            PluginTab::Marketplaces => PluginTab::Installed,
            PluginTab::Errors => PluginTab::Marketplaces,
        }
    }
}

/// Discover tab 行：marketplace 来源 + 可安装插件
#[derive(Debug, Clone)]
pub struct DiscoverRow {
    pub marketplace: String,
    pub plugin: crate::core::plugins::marketplace::MarketplacePlugin,
}

/// View state machine for the MCP modal (mirrors Claude Code MCPSettings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpView {
    /// All configured servers with live connection status
    List,
    /// Action menu for one server
    ServerMenu { name: String },
    /// Tool list for one server
    Tools { name: String },
    /// Tool detail (description + input schema)
    ToolDetail { name: String, index: usize },
    /// Remove confirmation
    ConfirmRemove { name: String },
}

/// Tabs for the marketplace modal (mirrors Claude Code PluginSettings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketTab {
    Browse,
    Installed,
    Sources,
}

impl MarketTab {
    pub const ALL: [MarketTab; 3] = [MarketTab::Browse, MarketTab::Installed, MarketTab::Sources];

    pub fn label(&self) -> &'static str {
        match self {
            MarketTab::Browse => "Browse",
            MarketTab::Installed => "Installed",
            MarketTab::Sources => "Sources",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            MarketTab::Browse => MarketTab::Installed,
            MarketTab::Installed => MarketTab::Sources,
            MarketTab::Sources => MarketTab::Browse,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            MarketTab::Browse => MarketTab::Sources,
            MarketTab::Installed => MarketTab::Browse,
            MarketTab::Sources => MarketTab::Installed,
        }
    }
}

/// One row in the MCP server list — merged config + live status.
#[derive(Debug, Clone)]
pub struct McpServerRow {
    pub name: String,
    pub transport: String,
    pub command: String,
    pub disabled: bool,
    pub connected: bool,
    pub tool_count: usize,
    pub error: Option<String>,
    /// 连接失败因缺少 OAuth 鉴权（对标 Claude Code 的 needs-auth 态，
    /// 显示 "Enter to auth"）
    pub needs_auth: bool,
}

/// 传输层 401 错误是否为 OAuth 鉴权需求（对应 transport.rs 的
/// "MCP OAuth required" 错误文案）
fn is_oauth_required_error(err: &str) -> bool {
    err.contains("OAuth required") || err.contains("needs-auth")
}

/// 从 OAuth 错误信息中提取授权 URL（transport 会把 Auth URL 拼进错误）
pub fn extract_oauth_url(err: &str) -> Option<String> {
    let idx = err.find("Auth URL: ")?;
    let url = err[idx + "Auth URL: ".len()..].trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

/// Pending install/uninstall confirmation in the marketplace modal.
#[derive(Debug, Clone)]
pub struct MarketConfirm {
    pub name: String,
    pub install: bool,
}

/// Pending confirmation in the plugins modal（安装 Discover 项 / 卸载插件 / 移除 marketplace）。
#[derive(Debug, Clone)]
pub struct PluginConfirm {
    /// install: <source>|<name>；uninstall: 插件名；remove-marketplace: marketplace 名
    pub kind: PluginConfirmKind,
}

#[derive(Debug, Clone)]
pub enum PluginConfirmKind {
    /// 安装确认（scope 已选定）
    Install {
        plugin: crate::core::plugins::marketplace::MarketplacePlugin,
        scope: String,
    },
    /// 安装范围选择（对标 Claude Code 的 scope 菜单：u=user / p=project）
    InstallScope {
        plugin: crate::core::plugins::marketplace::MarketplacePlugin,
    },
    Uninstall {
        name: String,
    },
    RemoveMarketplace {
        name: String,
    },
}

// ============ ChatState helpers: stack ops ============

impl ChatState {
    // ---- generic stack ----

    pub fn push_modal(&mut self, modal: Modal) {
        self.modal_stack.push(modal);
    }

    pub fn pop_modal(&mut self) -> Option<Modal> {
        self.modal_stack.pop()
    }

    pub fn top_modal(&self) -> Option<&Modal> {
        self.modal_stack.last()
    }

    pub fn top_modal_mut(&mut self) -> Option<&mut Modal> {
        self.modal_stack.last_mut()
    }

    pub fn close_all_modals(&mut self) {
        self.modal_stack.clear();
    }

    pub fn is_modal_open(&self) -> bool {
        !self.modal_stack.is_empty()
    }

    // ---- palette (replaces the old `show_palette: bool`) ----

    pub fn is_palette_open(&self) -> bool {
        self.modal_stack
            .iter()
            .any(|m| matches!(m, Modal::Palette))
    }

    /// Close any open palette instance.
    pub fn close_palette(&mut self) {
        self.modal_stack.retain(|m| !matches!(m, Modal::Palette));
    }

    /// (Re)open the palette at `mode`: resets items/filter/selection.
    pub fn open_palette(&mut self, mode: PaletteMode) {
        self.close_palette();
        let items = crate::ui::components::palette::get_items(&mode, self);
        self.palette_mode = mode;
        self.palette_items = items;
        self.selected_palette_index = 0;
        self.palette_filter.clear();
        self.input_modal_value.clear();
        self.modal_stack.push(Modal::Palette);
    }

    /// Navigate within the open palette: push current mode onto history.
    pub fn push_palette_mode(&mut self, mode: PaletteMode) {
        self.palette_history.push(self.palette_mode.clone());
        let items = crate::ui::components::palette::get_items(&mode, self);
        self.palette_mode = mode;
        self.palette_items = items;
        self.selected_palette_index = 0;
        self.palette_filter.clear();
    }

    // ---- MCP modal ----

    pub fn open_mcp_modal(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::Mcp { view: McpView::List });
        self.mcp_modal_index = 0;
        self.mcp_modal_action_msg = None;
    }

    // ---- Market modal ----

    pub async fn open_market_modal(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::Market {
            tab: MarketTab::Browse,
        });
        self.market_index = 0;
        self.market_query.clear();
        self.market_confirm = None;
        self.market_message = None;
        self.reload_market_entries().await;
    }

    // ---- Plugins modal (Claude Code style /plugin) ----

    pub async fn open_plugins_modal(
        &mut self,
        agent_tx: &tokio::sync::mpsc::Sender<crate::runtime::messages::AgentRequest>,
    ) {
        use crate::core::plugins::marketplace as mp;
        use crate::runtime::messages::{AgentRequest, PluginOp};

        self.close_all_modals();
        self.modal_stack.push(Modal::Plugins {
            tab: PluginTab::Discover,
        });
        self.plugin_index = 0;
        self.plugin_search.clear();
        self.plugin_selected.clear();
        self.plugin_detail = None;
        self.plugin_batch_total = 0;
        self.plugin_batch_done = 0;
        self.plugin_confirm = None;
        self.plugin_message = None;
        self.plugin_op_pending = false;

        // 对标 Claude Code 启动检查：首次打开时自动注册官方默认 marketplace。
        // 只做本地快速检查（读配置文件），实际 clone 在后台任务执行，
        // 避免阻塞 UI 事件循环导致 /plugin 回车卡顿。
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let has_default = mp::load_marketplaces(&cwd)
            .await
            .map(|list| list.iter().any(|m| m.name == mp::DEFAULT_MARKETPLACE_NAME))
            .unwrap_or(false);
        if !has_default {
            self.plugin_message = Some(format!(
                "Registering default marketplace '{}'...",
                mp::DEFAULT_MARKETPLACE_NAME
            ));
            self.plugin_op_pending = true;
            let _ = agent_tx
                .send(AgentRequest::PluginOp {
                    project_root: cwd,
                    op: PluginOp::EnsureDefaultMarketplace,
                })
                .await;
        }

        self.reload_plugins_state().await;
    }

    /// Discover tab 实时过滤：按 `plugin_search`（不区分大小写）匹配
    /// 插件名 / marketplace / 描述，返回命中的 `plugin_discover` 下标。
    /// 渲染与按键导航共用，保证两边看到同一份过滤结果。
    pub fn filtered_discover_indices(&self) -> Vec<usize> {
        let q = self.plugin_search.trim().to_lowercase();
        if q.is_empty() {
            return (0..self.plugin_discover.len()).collect();
        }
        self.plugin_discover
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.plugin.name.to_lowercase().contains(&q)
                    || r.marketplace.to_lowercase().contains(&q)
                    || r.plugin.description.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 重新加载 Plugins 弹窗当前 tab 的数据。
    pub async fn reload_plugins_state(&mut self) {
        use crate::core::plugins::marketplace as mp;

        let tab = match self.top_modal() {
            Some(Modal::Plugins { tab }) => *tab,
            _ => PluginTab::Discover,
        };
        self.plugin_loading = true;
        self.plugin_confirm = None;
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        match tab {
            PluginTab::Discover => {
                self.plugin_discover.clear();
                let marketplaces = mp::load_marketplaces(&cwd).await.unwrap_or_default();
                for m in &marketplaces {
                    match mp::list_marketplace_plugins(&cwd, m).await {
                        Ok(plugins) => {
                            for p in plugins {
                                self.plugin_discover.push(DiscoverRow {
                                    marketplace: m.name.clone(),
                                    plugin: p,
                                });
                            }
                        }
                        Err(e) => self.plugin_errors_hint = Some(e),
                    }
                }
                // 对标 Claude Code DiscoverPlugins：只列未安装的插件，
                // 并按名称排序（无安装量数据时官方即字母序）
                let installed: std::collections::HashSet<String> =
                    crate::core::plugins::resolve_installed_plugins(&cwd)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| p.entry.name)
                        .collect();
                self.plugin_discover
                    .retain(|row| !installed.contains(&row.plugin.name));
                self.plugin_discover
                    .sort_by(|a, b| a.plugin.name.cmp(&b.plugin.name));
                self.plugin_index = self
                    .plugin_index
                    .min(self.plugin_discover.len().saturating_sub(1));
            }
            PluginTab::Installed => {
                self.plugin_installed = crate::core::plugins::resolve_installed_plugins(&cwd)
                    .await
                    .unwrap_or_default();
                self.plugin_index = self
                    .plugin_index
                    .min(self.plugin_installed.len().saturating_sub(1));
            }
            PluginTab::Marketplaces => {
                self.plugin_marketplaces = mp::load_marketplaces(&cwd).await.unwrap_or_default();
                self.plugin_marketplace_counts.clear();
                for m in &self.plugin_marketplaces {
                    let count = mp::list_marketplace_plugins(&cwd, m)
                        .await
                        .map(|v| v.len())
                        .unwrap_or(0);
                    self.plugin_marketplace_counts
                        .insert(m.name.clone(), count);
                }
                // 行 0 = "Add marketplace…"，其余为 marketplace 行
                self.plugin_index = self
                    .plugin_index
                    .min(self.plugin_marketplaces.len());
            }
            PluginTab::Errors => {
                self.plugin_errors.clear();
                if let Ok(plugins) = crate::core::plugins::resolve_installed_plugins(&cwd).await {
                    for p in plugins {
                        if let Some(err) = &p.runtime_error {
                            self.plugin_errors
                                .push((p.entry.name.clone(), err.clone()));
                        }
                    }
                }
                self.plugin_index = self.plugin_index.min(self.plugin_errors.len().saturating_sub(1));
            }
        }
        self.plugin_loading = false;
    }

    /// Rebuild the marketplace listing for the current tab (local, fast).
    ///
    /// Installed 数据合并两个来源：extensions 注册表（`/extension install`）
    /// 与 plugins 系统（`/plugin install`），后者经 `resolve_installed_plugins`
    /// 异步读取。启停/卸载时按 `market_plugin_names` 分流到对应 API。
    pub async fn reload_market_entries(&mut self) {
        use crate::core::extensions::marketplace::Marketplace;
        use crate::core::extensions::registry::ExtensionRegistryManager;
        use crate::core::extensions::types::{ExtensionType, MarketplaceEntry};

        let tab = match self.top_modal() {
            Some(Modal::Market { tab }) => *tab,
            _ => MarketTab::Browse,
        };
        self.market_loading = true;

        // 快照：已安装（注册表）+ 启用状态
        let registry_entries = ExtensionRegistryManager::new().list_all();
        for e in &registry_entries {
            self.market_installed_names.insert(e.name.clone());
            self.market_enabled_map.insert(e.name.clone(), e.enabled);
        }

        match tab {
            MarketTab::Browse => {
                let marketplace = Marketplace::new();
                let mut entries = if self.market_query.trim().is_empty() {
                    marketplace.list_all()
                } else {
                    marketplace.search(self.market_query.trim())
                };
                entries.sort_by(|a, b| b.featured.cmp(&a.featured).then(b.downloads.cmp(&a.downloads)));
                self.market_entries = entries;
            }
            MarketTab::Installed => {
                let marketplace = Marketplace::new();
                let all = marketplace.list_all();
                let mut rows = registry_entries
                    .iter()
                    .map(|e| {
                        all.iter()
                            .find(|m| m.name == e.name)
                            .cloned()
                            .unwrap_or_else(|| MarketplaceEntry {
                                name: e.name.clone(),
                                version: e.version.clone(),
                                description: format!(
                                    "Installed {} (source: {})",
                                    type_word(&e.extension_type),
                                    e.source
                                ),
                                author: String::new(),
                                extension_type: e.extension_type.clone(),
                                source: e.source.clone(),
                                tags: Vec::new(),
                                downloads: 0,
                                stars: 0,
                                featured: false,
                            })
                    })
                    .collect::<Vec<_>>();

                // 追加 plugins 系统安装的插件
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                if let Ok(plugins) = crate::core::plugins::resolve_installed_plugins(&cwd).await {
                    for p in plugins {
                        self.market_installed_names.insert(p.entry.name.clone());
                        self.market_enabled_map
                            .insert(p.entry.name.clone(), p.entry.enabled);
                        self.market_plugin_names.insert(p.entry.name.clone());
                        rows.push(MarketplaceEntry {
                            name: p.entry.name.clone(),
                            version: p
                                .runtime_manifest
                                .as_ref()
                                .and_then(|r| r.version.clone())
                                .unwrap_or_else(|| "-".to_string()),
                            description: p
                                .runtime_manifest
                                .as_ref()
                                .and_then(|r| r.description.clone())
                                .filter(|d| !d.trim().is_empty())
                                .unwrap_or_else(|| format!("plugin · source: {}", p.entry.source)),
                            author: String::new(),
                            extension_type: ExtensionType::Plugin,
                            source: p.entry.source.clone(),
                            tags: Vec::new(),
                            downloads: 0,
                            stars: 0,
                            featured: false,
                        });
                    }
                }
                self.market_entries = rows;
            }
            MarketTab::Sources => {
                self.market_entries.clear();
            }
        }
        self.market_loading = false;
    }

    /// Marketplace entry names already installed (for Browse tab markers).
    pub fn installed_extension_names(&self) -> std::collections::HashSet<String> {
        use crate::core::extensions::registry::ExtensionRegistryManager;
        ExtensionRegistryManager::new()
            .list_all()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }
}

// ============ Async data loaders (called from async key handlers) ============

/// Load the MCP server snapshot (config + live status) for the modal.
///
/// Live discovery is bounded per-server so a dead server cannot hang the UI;
/// on timeout the row is marked disconnected with a timeout error.
pub async fn load_mcp_server_rows(state: &mut ChatState) {
    state.mcp_modal_loading = true;
    state.mcp_modal_error = None;

    let mut rows: Vec<McpServerRow> = Vec::new();

    match load_project_mcp_config().await {
        Ok(cfg) => {
            let mut names: Vec<&String> = cfg.mcp_servers.keys().collect();
            names.sort();

            let manager = MCPManager::new();
            // Best effort: initialize only spawns configured processes; errors
            // surface per-server below via list_tools failures.
            let _ = manager.initialize_mcp_servers().await;

            for name in names {
                let server = match cfg.mcp_servers.get(name) {
                    Some(s) => s.clone(),
                    None => continue,
                };
                let disabled = server.disabled.unwrap_or(false);
                let transport = server
                    .transport_type
                    .clone()
                    .unwrap_or_else(|| {
                        if server.url.is_some() {
                            "http".to_string()
                        } else {
                            "stdio".to_string()
                        }
                    });
                let command = match &server.url {
                    Some(url) => url.clone(),
                    None => server.command.clone().unwrap_or_else(|| "-".to_string()),
                };

                if disabled {
                    rows.push(McpServerRow {
                        name: name.clone(),
                        transport,
                        command,
                        disabled: true,
                        connected: false,
                        tool_count: 0,
                        error: None,
                        needs_auth: false,
                    });
                    continue;
                }

                match tokio::time::timeout(Duration::from_secs(3), manager.list_tools(name)).await {
                    Ok(Ok(tools)) => rows.push(McpServerRow {
                        name: name.clone(),
                        transport,
                        command,
                        disabled: false,
                        connected: true,
                        tool_count: tools.len(),
                        error: None,
                        needs_auth: false,
                    }),
                    Ok(Err(e)) => {
                        let err = e.to_string();
                        let needs_auth = is_oauth_required_error(&err);
                        rows.push(McpServerRow {
                            name: name.clone(),
                            transport,
                            command,
                            disabled: false,
                            connected: false,
                            tool_count: 0,
                            error: Some(truncate(&err, 48)),
                            needs_auth,
                        })
                    }
                    Err(_) => rows.push(McpServerRow {
                        name: name.clone(),
                        transport,
                        command,
                        disabled: false,
                        connected: false,
                        tool_count: 0,
                        error: Some("timeout".to_string()),
                        needs_auth: false,
                    }),
                }
            }
        }
        Err(e) => {
            state.mcp_modal_error = Some(truncate(&e.to_string(), 96));
        }
    }

    state.mcp_modal_servers = rows;
    state.mcp_modal_index = state.mcp_modal_index.min(state.mcp_modal_servers.len().saturating_sub(1));
    state.mcp_modal_loading = false;
}

/// Load the tool list for one server into the modal state.
pub async fn load_mcp_tools(state: &mut ChatState, server: &str) {
    state.mcp_modal_loading = true;
    state.mcp_modal_tools.clear();

    let manager = MCPManager::new();
    let _ = manager.initialize_mcp_servers().await;

    let tools: Vec<MCPTool> = match tokio::time::timeout(
        Duration::from_secs(4),
        manager.list_tools(server),
    )
    .await
    {
        Ok(Ok(tools)) => tools,
        Ok(Err(e)) => {
            state.mcp_modal_error = Some(truncate(&format!("{}: {}", server, e), 96));
            state.mcp_modal_loading = false;
            return;
        }
        Err(_) => {
            state.mcp_modal_error = Some(format!("{}: timeout", server));
            state.mcp_modal_loading = false;
            return;
        }
    };

    state.mcp_modal_tools = tools;
    state.mcp_modal_index = 0;
    state.mcp_modal_loading = false;
}

/// Toggle a server's `disabled` flag in the project MCP config.
pub async fn set_mcp_server_disabled(name: &str, disabled: bool) -> Result<(), String> {
    let mut cfg = load_project_mcp_config()
        .await
        .map_err(|e| e.to_string())?;
    match cfg.mcp_servers.get_mut(name) {
        Some(server) => {
            server.disabled = Some(disabled);
        }
        None => return Err(format!("server not found: {}", name)),
    }
    save_project_mcp_config(&cfg)
        .await
        .map_err(|e| e.to_string())
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{}…", cut)
    }
}

fn type_word(t: &crate::core::extensions::types::ExtensionType) -> &'static str {
    match t {
        crate::core::extensions::types::ExtensionType::Skill => "skill",
        crate::core::extensions::types::ExtensionType::Plugin => "plugin",
        crate::core::extensions::types::ExtensionType::Mcp => "mcp",
    }
}
