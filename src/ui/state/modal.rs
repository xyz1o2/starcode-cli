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

use crate::core::mcp::load_project_mcp_config;
use crate::core::mcp::save_project_mcp_config;
use crate::core::mcp::types::MCPTool;
use crate::core::mcp::MCPManager;
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
    // ── 从 show_* bool 迁移来的对话框 ──
    /// 全局搜索（Ctrl+Shift+F）
    GlobalSearch,
    /// 快速打开文件（Ctrl+Shift+P）
    QuickOpen,
    /// 历史搜索（Ctrl+R）
    HistorySearch,
    /// 主题选择器
    ThemePicker {
        /// 进入前的主题名（Esc 取消时恢复）
        prev_theme: Option<String>,
    },
    /// 用量统计
    UsageStats,
    /// 导出对话框
    Export,
    /// 压缩状态
    CompressionStatus,
    /// 上下文可视化
    ContextViz,
    /// 错误覆盖层
    ErrorOverlay,
    /// 日志选择器
    LogSelector,
    /// 通用输入对话框（API Key、重命名等）
    InputModal,
    /// 状态模态（provider/session 选择）
    StatusModal,
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

/// 通用输入框的边框样式。
///
/// 在 [`ChatState::enter_input_modal`] 里设置一次即可 —— 此前 `render_input_modal`
/// 每帧都重设一次，纯粹是「渲染顺手改状态」。
pub fn modal_textarea_block() -> ratatui::widgets::Block<'static> {
    use ratatui::style::{Color, Style};
    use ratatui::widgets::{Block, BorderType, Borders};

    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::White))
}

impl ChatState {
    // ---- generic stack ----

    pub fn push_modal(&mut self, modal: Modal) {
        self.modal_stack.push(modal);
    }

    pub fn pop_modal(&mut self) -> Option<Modal> {
        let modal = self.modal_stack.pop();
        if matches!(modal, Some(Modal::GlobalSearch)) {
            self.global_search_state.cancel_active_request();
        }
        if matches!(modal, Some(Modal::QuickOpen)) {
            self.quick_open_state.cancel_search();
        }
        modal
    }

    pub fn top_modal(&self) -> Option<&Modal> {
        self.modal_stack.last()
    }

    pub fn top_modal_mut(&mut self) -> Option<&mut Modal> {
        self.modal_stack.last_mut()
    }

    /// 输入对话框是否打开。
    ///
    /// **模态栈是唯一真相源** —— 此前还有一个平行的 `show_input_modal` flag，
    /// 靠 [`ChatState::enter_input_modal`] / [`ChatState::exit_input_modal`] 手工
    /// 同步两边。异步 stream 消息（`ServicesList` 回来后自动开模型面板）一打穿就
    /// 出现「面板画得出来、按键却全被隐形输入框吞掉」。
    ///
    /// 用 `any` 而非只看栈顶：输入框是叠在别的模态**之上**打开的，栈顶未必是它
    /// （见 [`ChatState::exit_input_modal`] 里用 `retain` 而不是 `pop` 的原因）。
    /// 渲染与按键分发各自按栈顶走，这个谓词只回答「有没有开着」。
    pub fn is_input_modal_open(&self) -> bool {
        self.modal_stack
            .iter()
            .any(|modal| matches!(modal, Modal::InputModal))
    }

    pub fn close_all_modals(&mut self) {
        if self
            .modal_stack
            .iter()
            .any(|modal| matches!(modal, Modal::GlobalSearch))
        {
            self.global_search_state.cancel_active_request();
        }
        if self
            .modal_stack
            .iter()
            .any(|modal| matches!(modal, Modal::QuickOpen))
        {
            self.quick_open_state.cancel_search();
        }
        self.modal_stack.clear();
        // 后台代理选择器也是一个按键层。不清的话，开一次 modal 再 Esc 之后，
        // ↑/↓/Enter 仍然被选择器分支接走，输入框失去方向键。
        self.bg_agent_selection = None;
    }

    pub fn is_modal_open(&self) -> bool {
        !self.modal_stack.is_empty()
    }

    // ---- palette (replaces the old `show_palette: bool`) ----

    pub fn is_palette_open(&self) -> bool {
        self.modal_stack.iter().any(|m| matches!(m, Modal::Palette))
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
        self.modal_stack.push(Modal::Mcp {
            view: McpView::List,
        });
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

    // ---- 从 show_* bool 迁移来的对话框 ----

    /// 全局搜索
    pub fn open_global_search(&mut self) {
        self.close_all_modals();
        self.global_search_state =
            crate::ui::components::highlight::search::GlobalSearchState::new();
        self.modal_stack.push(Modal::GlobalSearch);
    }

    /// 快速打开
    pub fn open_quick_open(&mut self) {
        self.close_all_modals();
        self.quick_open_state.reset();
        self.modal_stack.push(Modal::QuickOpen);
    }

    /// 历史搜索（打开时预填充全部历史条目）
    pub fn open_history_search(&mut self) {
        self.close_all_modals();
        self.history_search_state =
            crate::ui::components::highlight::history::HistorySearchState::new();
        // 预填充历史数据
        let history_tuples: Vec<(String, String)> = self
            .chat_history
            .iter()
            .filter(|e| {
                matches!(
                    e.entry_type,
                    crate::types::ChatEntryType::User | crate::types::ChatEntryType::Assistant
                )
            })
            .map(|e| {
                let role = match e.entry_type {
                    crate::types::ChatEntryType::User => "user".to_string(),
                    _ => "assistant".to_string(),
                };
                (role, e.content.clone())
            })
            .collect();
        self.history_search_state.results =
            crate::ui::components::highlight::history::search_history(&history_tuples, "");
        self.modal_stack.push(Modal::HistorySearch);
    }

    /// 主题选择器
    pub fn open_theme_picker(&mut self) {
        self.close_all_modals();
        let prev = self.theme_manager.current().name.clone();
        self.selected_theme_index = 0;
        self.modal_stack.push(Modal::ThemePicker {
            prev_theme: Some(prev),
        });
    }

    /// 用量统计
    pub fn open_usage_stats(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::UsageStats);
    }

    /// 导出对话框
    pub fn open_export_dialog(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::Export);
    }

    /// 压缩状态
    pub fn open_compression_status(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::CompressionStatus);
    }

    /// 上下文可视化
    pub fn open_context_viz(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::ContextViz);
    }

    /// 错误覆盖层
    pub fn open_error_overlay(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::ErrorOverlay);
    }

    /// 日志选择器
    pub fn open_log_selector(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::LogSelector);
    }

    /// 通用输入对话框
    pub fn open_input_modal(&mut self, title: String, prompt: String) {
        self.close_all_modals();
        self.input_modal_title = title;
        self.input_modal_prompt = prompt;
        self.input_modal_value.clear();
        let mut textarea = tui_textarea::TextArea::default();
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_cursor_style(
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        );
        self.modal_textarea = textarea;
        self.enter_input_modal();
    }

    /// 标记输入对话框为打开状态 —— 把 [`Modal::InputModal`] 压栈。
    ///
    /// 栈是唯一真相源：渲染看栈顶（`ui/app/mod.rs` 的 `draw_ui`），按键分发看
    /// [`ChatState::is_input_modal_open`]，两边读的是同一个状态。调用方只负责填
    /// title / prompt / textarea。
    pub fn enter_input_modal(&mut self) {
        if !self.is_input_modal_open() {
            self.modal_stack.push(Modal::InputModal);
        }
        // 输入框的边框样式在这里设一次，而不是每帧在渲染里 `set_block` ——
        // 渲染过程不该改状态（`render_input_modal` 因此只需要 `&ChatState`）。
        self.modal_textarea.set_block(modal_textarea_block());
    }

    /// 关闭输入对话框：把栈里的 `InputModal` 摘掉。
    ///
    /// 用 `retain` 而不是 `pop`：输入框未必在栈顶（底下可能压着 Plugins
    /// 之类的模态），pop 会把别人的模态误关掉。
    pub fn exit_input_modal(&mut self) {
        self.modal_stack.retain(|m| !matches!(m, Modal::InputModal));
    }

    /// 打开 provider 快捷菜单。
    ///
    /// 必须走这个方法而不是直接置 `show_provider_menu`：两个快捷菜单是互斥的，
    /// 直接置位会留下「两个同时 true」的非法状态，此后 ↑/↓/Enter 该听谁的只能靠
    /// `command_suggestions` 里一份没有文档的优先级顺序来裁决。
    pub fn open_provider_menu(&mut self) {
        self.show_session_menu = false;
        self.show_provider_menu = true;
        self.selected_provider_index = 0;
    }

    /// 打开 session 快捷菜单（同 [`ChatState::open_provider_menu`]，互斥）。
    pub fn open_session_menu(&mut self) {
        self.show_provider_menu = false;
        self.show_session_menu = true;
    }

    /// 关闭两个快捷菜单。
    pub fn close_quick_menus(&mut self) {
        self.show_provider_menu = false;
        self.show_session_menu = false;
    }

    /// 状态模态
    ///
    /// 只压栈即可：渲染（`app/mod.rs`）与按键（`modal_input::handle_modal_key`）
    /// 都以 `modal_stack` 为准。曾经还有一个只被赋 `false`、却又在
    /// `render_status_modal` 里当门控的 `show_status_modal` flag，症状是
    /// 「按 Ctrl+P → Status 之后什么窗口都没出现，但按键全被吃掉」。
    pub fn open_status_modal(&mut self) {
        self.close_all_modals();
        self.modal_stack.push(Modal::StatusModal);
    }

    // ── 模式判断不在这里 ──
    //
    // 这里曾有一份手维护的「有没有覆盖层」清单（`has_overlay_active`）和一份死掉的
    // `has_dialog_active`。两处真实调用点（vim 门控、双 Esc、任务面板守卫）都已改用
    // [`crate::ui::events::key_context::KeyContext`]，它们还各有 `is_exclusive()`
    // / `is_dialog()` 两个派生谓词。清单留在 ChatState 上只会成为第 N+1 份需要同步
    // 维护的模式定义 —— 新增模式请改 `KeyContext::of`，不要在这里加字段。

    // ---- Discover tab 实时过滤：按 `plugin_search`（不区分大小写）匹配
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
                    self.plugin_marketplace_counts.insert(m.name.clone(), count);
                }
                // 行 0 = "Add marketplace…"，其余为 marketplace 行
                self.plugin_index = self.plugin_index.min(self.plugin_marketplaces.len());
            }
            PluginTab::Errors => {
                self.plugin_errors.clear();
                if let Ok(plugins) = crate::core::plugins::resolve_installed_plugins(&cwd).await {
                    for p in plugins {
                        if let Some(err) = &p.runtime_error {
                            self.plugin_errors.push((p.entry.name.clone(), err.clone()));
                        }
                    }
                }
                self.plugin_index = self
                    .plugin_index
                    .min(self.plugin_errors.len().saturating_sub(1));
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
                entries.sort_by(|a, b| {
                    b.featured
                        .cmp(&a.featured)
                        .then(b.downloads.cmp(&a.downloads))
                });
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
                let transport = server.transport_type.clone().unwrap_or_else(|| {
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
    state.mcp_modal_index = state
        .mcp_modal_index
        .min(state.mcp_modal_servers.len().saturating_sub(1));
    state.mcp_modal_loading = false;
}

/// Load the tool list for one server into the modal state.
pub async fn load_mcp_tools(state: &mut ChatState, server: &str) {
    state.mcp_modal_loading = true;
    state.mcp_modal_tools.clear();

    let manager = MCPManager::new();
    let _ = manager.initialize_mcp_servers().await;

    let tools: Vec<MCPTool> =
        match tokio::time::timeout(Duration::from_secs(4), manager.list_tools(server)).await {
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
    let mut cfg = load_project_mcp_config().await.map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_global_search_cancels_active_request() {
        let mut state = ChatState::new();
        state.open_global_search();
        let token = state.global_search_state.begin_request(7);

        assert_eq!(state.pop_modal(), Some(Modal::GlobalSearch));
        assert!(token.is_cancelled());
        assert!(!state.global_search_state.is_searching);
    }

    #[test]
    fn close_all_modals_cancels_global_search_below_another_modal() {
        let mut state = ChatState::new();
        state.open_global_search();
        let token = state.global_search_state.begin_request(8);
        state.push_modal(Modal::Palette);

        state.close_all_modals();

        assert!(state.modal_stack.is_empty());
        assert!(token.is_cancelled());
        assert!(!state.global_search_state.is_searching);
    }

    #[test]
    fn quick_open_close_and_reopen_invalidates_searches() {
        let mut state = ChatState::new();
        state.open_quick_open();
        let opened_generation = state.quick_open_state.search_generation;

        assert_eq!(state.pop_modal(), Some(Modal::QuickOpen));
        let closed_generation = state.quick_open_state.search_generation;
        assert!(closed_generation > opened_generation);

        state.open_quick_open();
        assert!(state.quick_open_state.search_generation > closed_generation);
        assert!(matches!(state.top_modal(), Some(Modal::QuickOpen)));
    }

    #[test]
    fn close_all_modals_invalidates_quick_open_below_another_modal() {
        let mut state = ChatState::new();
        state.open_quick_open();
        let generation = state.quick_open_state.search_generation;
        state.push_modal(Modal::Palette);

        state.close_all_modals();

        assert!(state.modal_stack.is_empty());
        assert!(state.quick_open_state.search_generation > generation);
    }

    /// 输入框的「开没开」只有模态栈一个真相源。平行的 `show_input_modal` flag 已删除
    /// —— 靠 `enter`/`exit` 手工同步两个状态时，异步 ModelsList 一打穿就是
    /// 「面板看得见、按键却全被隐形输入框吞掉」。
    #[test]
    fn input_modal_openness_is_derived_from_the_stack() {
        let mut state = ChatState::new();
        assert!(!state.is_input_modal_open());

        state.enter_input_modal();
        assert!(state.is_input_modal_open());
        // 重复 enter 不会叠两个（provider 表单有多条路径会调它）
        state.enter_input_modal();
        assert_eq!(
            state.modal_stack.iter().filter(|m| **m == Modal::InputModal).count(),
            1
        );

        // 输入框叠在别的模态之上：按键仍归输入框，但栈顶是后来者。
        state.push_modal(Modal::Palette);
        assert_eq!(state.top_modal(), Some(&Modal::Palette));
        assert!(state.is_input_modal_open());

        // exit 用 retain：不能把底下别人的模态一起带走
        state.exit_input_modal();
        assert!(!state.is_input_modal_open());
        assert_eq!(state.top_modal(), Some(&Modal::Palette));

        state.close_all_modals();
        assert!(state.modal_stack.is_empty());
        assert!(!state.is_input_modal_open());
    }

    /// 状态弹窗只由栈表达：曾经有一个只被赋 `false` 的 `show_status_modal` flag，
    /// 渲染侧拿它当门控、按键侧照常消费，于是「看不见的窗口把按键全吃掉」。
    #[test]
    fn status_modal_lifecycle_is_the_stack() {
        let mut state = ChatState::new();
        state.open_status_modal();
        assert_eq!(state.top_modal(), Some(&Modal::StatusModal));

        state.pop_modal();
        assert!(state.modal_stack.is_empty());
    }
}
