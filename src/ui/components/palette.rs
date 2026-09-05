use crate::core::config::providers::{
    get_provider_by_id, provider_requires_manual_base_url, ProviderCategory, ProviderMetadata,
    ALL_PROVIDERS,
};
use crate::core::i18n;
use crate::ui::state::{ChatState, PaletteAction, PaletteItem, PaletteMode};
use crate::ui::utils::status::{
    approval_mode_label, current_model_id, current_provider_display, current_provider_id,
    status_summary,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::collections::HashSet;

pub fn palette_item_matches_query(item: &PaletteItem, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }

    if item.label.to_lowercase().contains(query_lower) {
        return true;
    }

    if item.description.to_lowercase().contains(query_lower) {
        return true;
    }

    if let Some(category) = &item.category {
        if category.to_lowercase().contains(query_lower) {
            return true;
        }
    }

    false
}

pub fn get_items(mode: &PaletteMode, state: &ChatState) -> Vec<PaletteItem> {
    match mode {
        PaletteMode::Main => get_main_palette_items_for_state(Some(state)),
        PaletteMode::Session => get_session_palette_items(),
        PaletteMode::System => get_system_palette_items(),
        PaletteMode::Agent => get_agent_palette_items(),
        PaletteMode::Provider => get_provider_palette_items(&state.configured_providers),
        PaletteMode::ProviderPopular => get_provider_popular_items(&state.configured_providers),
        PaletteMode::ProviderOther => get_provider_other_items(&state.configured_providers),
        PaletteMode::ProviderLocal => get_provider_local_items(&state.configured_providers),
        PaletteMode::ProviderOptions(pid) => {
            get_provider_options_items(pid, &state.configured_providers)
        }
        PaletteMode::AddProvider => get_add_provider_items(),
        PaletteMode::AddProviderId(provider_type) => get_add_provider_id_items(provider_type),
        PaletteMode::Memory => get_memory_palette_items(),
        PaletteMode::AgentMode => get_agent_mode_palette_items(),
        PaletteMode::ThinkingEffort => {
            get_thinking_effort_palette_items(&state.thinking_effort, &state.current_model)
        }
        PaletteMode::ContextWindow => {
            get_context_window_palette_items(state.context_window_override)
        }
        PaletteMode::Theme => get_theme_palette_items(state),
        PaletteMode::Model => get_model_palette_items(
            &state.available_models,
            &state.current_model,
            state.awaiting_models,
            &state.model_provider_map,
            state.models_list_age_secs(),
        ),
        PaletteMode::Project => get_project_palette_items(),
        PaletteMode::Integrations => get_integrations_palette_items(),
        PaletteMode::McpManage => get_mcp_manage_palette_items(),
        PaletteMode::Help => get_help_palette_items(),
        PaletteMode::Language => get_language_palette_items(),
        PaletteMode::OutputStyle => get_output_style_palette_items(),
        PaletteMode::Git => get_git_palette_items(),
        _ => vec![],
    }
}

pub fn get_search_items(mode: &PaletteMode, state: &ChatState, query: &str) -> Vec<PaletteItem> {
    if query.trim().is_empty() {
        return get_items(mode, state);
    }

    match mode {
        PaletteMode::Main => get_global_search_palette_items(state),
        PaletteMode::Provider => get_items(mode, state),
        _ => get_items(mode, state),
    }
}

pub fn palette_title(mode: &PaletteMode, query: &str) -> String {
    let scope = if !query.trim().is_empty() && matches!(mode, PaletteMode::Main) {
        "Search All"
    } else {
        match mode {
            PaletteMode::Main => "Actions",
            PaletteMode::Provider => "Providers",
            PaletteMode::Session => "Session",
            PaletteMode::System => "Settings",
            PaletteMode::Agent => "Agent",
            PaletteMode::Memory => "Memory",
            PaletteMode::Git => "Git",
            PaletteMode::Mcp => "MCP",
            PaletteMode::Project => "Project",
            PaletteMode::Integrations => "Integrations",
            PaletteMode::McpManage => "MCP",
            PaletteMode::Help => "Help",
            PaletteMode::Model => "Models",
            PaletteMode::AgentMode => "Agent Mode",
            PaletteMode::ThinkingEffort => "Thinking",
            PaletteMode::ContextWindow => "Context Window",
            PaletteMode::Theme => "Theme",
            PaletteMode::ProviderPopular => "Popular Providers",
            PaletteMode::ProviderOther => "Regional Providers",
            PaletteMode::ProviderLocal => "Local Providers",
            PaletteMode::ProviderOptions(_) => "Provider Setup",
            PaletteMode::AddProvider => "Add Provider",
            PaletteMode::AddProviderId(_) => "Add Provider",
            PaletteMode::Language => "Language",
            PaletteMode::OutputStyle => "Output Style",
        }
    };

    format!(" {} (Ctrl+P) ", scope)
}

pub fn palette_placeholder(mode: &PaletteMode) -> &'static str {
    match mode {
        PaletteMode::Main => "Search actions, models, providers",
        PaletteMode::Provider => "Search providers by name or endpoint type",
        PaletteMode::Model => "Search models",
        PaletteMode::Language => "Select UI language",
        PaletteMode::System => "Search settings, language, diagnostics",
        PaletteMode::Memory => "Search memory actions",
        PaletteMode::Session => "Search session actions",
        _ => "Search commands",
    }
}

fn get_global_search_palette_items(state: &ChatState) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    push_unique_actionable_items(
        &mut items,
        &mut seen,
        get_main_palette_items_for_state(Some(state))
            .into_iter()
            .filter(|item| !matches!(item.action, PaletteAction::Navigate(_)))
            .collect(),
    );
    push_unique_actionable_items(&mut items, &mut seen, get_session_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_project_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_integrations_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_memory_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_system_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_help_palette_items());
    push_unique_actionable_items(&mut items, &mut seen, get_agent_mode_palette_items());
    push_unique_actionable_items(
        &mut items,
        &mut seen,
        get_all_provider_search_items(&state.configured_providers),
    );

    if !state.available_models.is_empty() {
        push_unique_actionable_items(
            &mut items,
            &mut seen,
            get_model_palette_items(
                &state.available_models,
                &state.current_model,
                state.awaiting_models,
                &state.model_provider_map,
                state.models_list_age_secs(),
            ),
        );
    }

    items
}

fn get_all_provider_search_items(configured: &HashSet<String>) -> Vec<PaletteItem> {
    let builtin_ids: HashSet<&str> = ALL_PROVIDERS.iter().map(|p| p.id).collect();
    let mut items: Vec<PaletteItem> = ALL_PROVIDERS
        .iter()
        .map(|provider| build_provider_item_flat(provider, configured))
        .collect();

    // Add custom providers
    for id in configured
        .iter()
        .filter(|id| !builtin_ids.contains(id.as_str()))
    {
        items.push(PaletteItem {
            id: format!("provider_{}", id),
            label: format!("{} ✓", id),
            description: "Custom provider".to_string(),
            category: Some("Custom".to_string()),
            action: PaletteAction::Navigate(PaletteMode::ProviderOptions(id.clone())),
        });
    }

    items
}

fn is_actionable_item(item: &PaletteItem) -> bool {
    !matches!(item.action, PaletteAction::Back)
}

fn push_unique_actionable_items(
    target: &mut Vec<PaletteItem>,
    seen: &mut HashSet<String>,
    items: Vec<PaletteItem>,
) {
    for item in items.into_iter().filter(is_actionable_item) {
        let key = format!(
            "{}|{}|{}",
            item.label.to_lowercase(),
            item.description.to_lowercase(),
            palette_action_key(&item.action)
        );
        if seen.insert(key) {
            target.push(item);
        }
    }
}

fn palette_action_key(action: &PaletteAction) -> String {
    match action {
        PaletteAction::Navigate(mode) => format!("navigate:{}", palette_mode_key(mode)),
        PaletteAction::ShowStatus => "show_status".to_string(),
        PaletteAction::ShowModelMenu => "show_model_menu".to_string(),
        PaletteAction::ShowProviderMenu => "show_provider_menu".to_string(),
        PaletteAction::ShowSessionMenu => "show_session_menu".to_string(),
        PaletteAction::ExecuteCommand(cmd) => format!("execute:{}", cmd),
        PaletteAction::TypeCommand(cmd) => format!("type:{}", cmd),
        PaletteAction::SelectProvider(provider_id) => format!("provider:{}", provider_id),
        PaletteAction::ToggleFeature(feature) => format!("toggle:{}", feature),
        PaletteAction::InputApiKey(provider_id) => format!("api_key:{}", provider_id),
        PaletteAction::InputBaseUrl(provider_id) => format!("base_url:{}", provider_id),
        PaletteAction::Back => "back".to_string(),
        PaletteAction::SetModel(model) => format!("model:{}", model),
        PaletteAction::SetAgentMode(mode) => format!("agent_mode:{}", mode),
        PaletteAction::SetThinkingEffort(level) => format!("thinking_effort:{}", level),
        PaletteAction::SetContextWindow(size) => format!("context_window:{}", size),
        PaletteAction::SetTheme(theme) => format!("theme:{}", theme),
        PaletteAction::SetOutputStyle(style) => format!("output_style:{}", style),
        PaletteAction::ShowLogSelector => "show_log_selector".to_string(),
        PaletteAction::ShowContextViz => "show_context_viz".to_string(),
        PaletteAction::ToggleVimMode => "toggle_vim_mode".to_string(),
        PaletteAction::ToggleUiVerbose => "toggle_ui_verbose".to_string(),
        PaletteAction::CreatePr => "create_pr".to_string(),
        PaletteAction::ToggleColorblindMode => "toggle_colorblind_mode".to_string(),
        PaletteAction::InputProviderId(provider_type) => {
            format!("add_provider_id:{}", provider_type)
        }
        PaletteAction::InputProviderName(provider_id) => {
            format!("add_provider_name:{}", provider_id)
        }
        PaletteAction::InputModelName => "input_model_name".to_string(),
        PaletteAction::RefreshModels => "refresh_models".to_string(),
        PaletteAction::OpenMcpModal => "open_mcp_modal".to_string(),
        PaletteAction::OpenMarketModal => "open_market_modal".to_string(),
    }
}

fn palette_mode_key(mode: &PaletteMode) -> String {
    match mode {
        PaletteMode::Main => "main".to_string(),
        PaletteMode::Provider => "provider".to_string(),
        PaletteMode::Session => "session".to_string(),
        PaletteMode::System => "system".to_string(),
        PaletteMode::Agent => "agent".to_string(),
        PaletteMode::Memory => "memory".to_string(),
        PaletteMode::Git => "git".to_string(),
        PaletteMode::Mcp => "mcp".to_string(),
        PaletteMode::Project => "project".to_string(),
        PaletteMode::Integrations => "integrations".to_string(),
        PaletteMode::McpManage => "mcp_manage".to_string(),
        PaletteMode::Help => "help".to_string(),
        PaletteMode::Model => "model".to_string(),
        PaletteMode::AgentMode => "agent_mode".to_string(),
        PaletteMode::ThinkingEffort => "thinking_effort".to_string(),
        PaletteMode::ContextWindow => "context_window".to_string(),
        PaletteMode::Theme => "theme".to_string(),
        PaletteMode::ProviderPopular => "provider_popular".to_string(),
        PaletteMode::ProviderOther => "provider_other".to_string(),
        PaletteMode::ProviderLocal => "provider_local".to_string(),
        PaletteMode::ProviderOptions(provider_id) => format!("provider_options:{}", provider_id),
        PaletteMode::Language => "language".to_string(),
        PaletteMode::OutputStyle => "output_style".to_string(),
        PaletteMode::AddProvider => "add_provider".to_string(),
        PaletteMode::AddProviderId(provider_type) => format!("add_provider_id:{}", provider_type),
    }
}

fn model_summary_description(state: Option<&ChatState>) -> String {
    state
        .and_then(current_model_id)
        .map(|model| format!("Current: {}", model))
        .unwrap_or_else(|| "Switch the current model".to_string())
}

fn provider_summary_description(state: Option<&ChatState>) -> String {
    let configured = state
        .map(|state| state.configured_providers.len())
        .unwrap_or(0);
    if let Some(state) = state {
        if current_provider_id(state).is_some() {
            let provider = current_provider_display(state);
            return format!("Current: {} · {} configured", provider, configured.max(1));
        }
    }

    if configured > 0 {
        format!("{} configured provider(s)", configured)
    } else {
        "Connect or switch provider".to_string()
    }
}

fn status_overview_description(state: Option<&ChatState>) -> String {
    state
        .map(status_summary)
        .unwrap_or_else(|| "Review model, provider and runtime status".to_string())
}

fn approval_mode_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", approval_mode_label(&state.approval_mode)))
        .unwrap_or_else(|| "Switch Auto, Plan or Yolo".to_string())
}

fn thinking_effort_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", state.thinking_effort.label()))
        .unwrap_or_else(|| "Set thinking/reasoning effort level".to_string())
}

fn theme_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", state.theme_manager.current().name))
        .unwrap_or_else(|| "Switch color theme".to_string())
}

pub fn get_main_palette_items() -> Vec<PaletteItem> {
    get_main_palette_items_for_state(None)
}

fn get_main_palette_items_for_state(state: Option<&ChatState>) -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "status_overview".to_string(),
            label: i18n::t("palette.label.status", "Status", "Status"),
            description: status_overview_description(state),
            category: Some(i18n::t("palette.cat.overview", "Overview", "Overview")),
            action: PaletteAction::ShowStatus,
        },
        PaletteItem {
            id: "switch_model".to_string(),
            label: i18n::t("palette.label.model", "Model", "Model"),
            description: model_summary_description(state),
            category: Some(i18n::t("palette.cat.ai", "AI", "AI")),
            action: PaletteAction::ShowModelMenu,
        },
        PaletteItem {
            id: "provider_menu".to_string(),
            label: i18n::t("palette.label.providers", "Providers", "Providers"),
            description: provider_summary_description(state),
            category: Some(i18n::t("palette.cat.ai", "AI", "AI")),
            action: PaletteAction::Navigate(PaletteMode::Provider),
        },
        PaletteItem {
            id: "settings_overview".to_string(),
            label: i18n::t(
                "palette.label.settings_overview",
                "Settings Overview",
                "Settings Overview",
            ),
            description: i18n::t(
                "palette.desc.settings_overview",
                "Config files, language and provider paths",
                "Config files, language and provider paths",
            ),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::ExecuteCommand("/settings".to_string()),
        },
        PaletteItem {
            id: "approval_mode".to_string(),
            label: i18n::t(
                "palette.label.approval_mode",
                "Approval Mode",
                "Approval Mode",
            ),
            description: approval_mode_description(state),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::Navigate(PaletteMode::AgentMode),
        },
        PaletteItem {
            id: "thinking_effort".to_string(),
            label: i18n::t("palette.label.thinking_effort", "Thinking", "Thinking"),
            description: thinking_effort_description(state),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::Navigate(PaletteMode::ThinkingEffort),
        },
        PaletteItem {
            id: "theme".to_string(),
            label: i18n::t("palette.label.theme", "Theme", "Theme"),
            description: theme_description(state),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::Navigate(PaletteMode::Theme),
        },
        PaletteItem {
            id: "vim_mode".to_string(),
            label: i18n::t("palette.label.vim_mode", "Vim Mode", "Vim Mode"),
            description: if state.map_or(false, |s| s.vim_enabled) {
                i18n::t(
                    "palette.desc.vim_on",
                    "Enabled — toggle off",
                    "Enabled — toggle off",
                )
            } else {
                i18n::t(
                    "palette.desc.vim_off",
                    "Disabled — toggle on",
                    "Disabled — toggle on",
                )
            },
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::ToggleVimMode,
        },
        PaletteItem {
            id: "ui_verbose".to_string(),
            label: "Verbose UI".to_string(),
            description: if state.map_or(false, |s| s.ui_verbose) {
                "Enabled — full paths, untruncated commands".to_string()
            } else {
                "Disabled — toggle to show full details".to_string()
            },
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::ToggleUiVerbose,
        },
        PaletteItem {
            id: "colorblind_mode".to_string(),
            label: i18n::t(
                "palette.label.colorblind",
                "Colorblind Mode",
                "Colorblind Mode",
            ),
            description: if state.map_or(false, |s| s.colorblind_mode) {
                i18n::t(
                    "palette.desc.colorblind_on",
                    "Enabled — toggle off",
                    "Enabled — toggle off",
                )
            } else {
                i18n::t(
                    "palette.desc.colorblind_off",
                    "Disabled — add shape indicators",
                    "Disabled — add shape indicators",
                )
            },
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::ToggleColorblindMode,
        },
        PaletteItem {
            id: "system_doctor".to_string(),
            label: i18n::t(
                "palette.label.system_doctor",
                "System Doctor",
                "System Doctor",
            ),
            description: i18n::t(
                "palette.desc.system_doctor",
                "Run diagnostics",
                "Run diagnostics",
            ),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::ExecuteCommand("/doctor".to_string()),
        },
        PaletteItem {
            id: "session_menu".to_string(),
            label: i18n::t("palette.label.sessions", "Sessions", "Sessions"),
            description: i18n::t(
                "palette.desc.sessions",
                "New, switch, save and delete sessions",
                "New, switch, save and delete sessions",
            ),
            category: Some(i18n::t("palette.cat.workspace", "Workspace", "Workspace")),
            action: PaletteAction::Navigate(PaletteMode::Session),
        },
        PaletteItem {
            id: "project_menu".to_string(),
            label: i18n::t("palette.label.project", "Project", "Project"),
            description: i18n::t(
                "palette.desc.project",
                "Init, Restore, Git Status",
                "Init, Restore, Git Status",
            ),
            category: Some(i18n::t("palette.cat.workspace", "Workspace", "Workspace")),
            action: PaletteAction::Navigate(PaletteMode::Project),
        },
        PaletteItem {
            id: "integrations_menu".to_string(),
            label: i18n::t("palette.label.integrations", "Integrations", "Integrations"),
            description: i18n::t(
                "palette.desc.integrations",
                "MCP Servers, Tools",
                "MCP Servers, Tools",
            ),
            category: Some(i18n::t("palette.cat.workspace", "Workspace", "Workspace")),
            action: PaletteAction::Navigate(PaletteMode::Integrations),
        },
        PaletteItem {
            id: "memory_menu".to_string(),
            label: i18n::t("palette.label.memory", "Memory", "Memory"),
            description: i18n::t(
                "palette.desc.memory",
                "Show, add, refresh",
                "Show, add, refresh",
            ),
            category: Some(i18n::t("palette.cat.workspace", "Workspace", "Workspace")),
            action: PaletteAction::Navigate(PaletteMode::Memory),
        },
        PaletteItem {
            id: "help_menu".to_string(),
            label: i18n::t("palette.label.help", "Help", "Help"),
            description: i18n::t(
                "palette.desc.help",
                "Shortcuts and common commands",
                "Shortcuts and common commands",
            ),
            category: Some(i18n::t("palette.cat.support", "Support", "Support")),
            action: PaletteAction::Navigate(PaletteMode::Help),
        },
        PaletteItem {
            id: "context_viz".to_string(),
            label: i18n::t("palette.label.context", "Context Window", "Context Window"),
            description: i18n::t(
                "palette.desc.context",
                "View token usage breakdown",
                "View token usage breakdown",
            ),
            category: Some(i18n::t(
                "palette.cat.diagnostics",
                "Diagnostics",
                "Diagnostics",
            )),
            action: PaletteAction::ShowContextViz,
        },
        PaletteItem {
            id: "output_style".to_string(),
            label: i18n::t("palette.label.output_style", "Output Style", "Output Style"),
            description: i18n::t(
                "palette.desc.output_style",
                "Change response formatting style",
                "Change response formatting style",
            ),
            category: Some(i18n::t("palette.cat.settings", "Settings", "Settings")),
            action: PaletteAction::Navigate(PaletteMode::OutputStyle),
        },
        PaletteItem {
            id: "log_selector".to_string(),
            label: i18n::t(
                "palette.label.log_selector",
                "Session Browser",
                "Session Browser",
            ),
            description: i18n::t(
                "palette.desc.log_selector",
                "Browse and resume past sessions",
                "Browse and resume past sessions",
            ),
            category: Some(i18n::t("palette.cat.workspace", "Workspace", "Workspace")),
            action: PaletteAction::ShowLogSelector,
        },
    ]
}

pub fn get_session_palette_items() -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.session", "Session", "Session"));
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: i18n::t("palette.back.label", ".. Back", ".. Back"),
            description: i18n::t(
                "palette.back.desc.main",
                "Return to main menu",
                "Return to main menu",
            ),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "new_session".to_string(),
            label: i18n::t("palette.label.new_session", "New Session", "New Session"),
            description: i18n::t(
                "palette.desc.new_session",
                "Start a new session",
                "Start a new session",
            ),
            category: cat.clone(),
            action: PaletteAction::ExecuteCommand("/clear".to_string()),
        },
        PaletteItem {
            id: "switch_session".to_string(),
            label: i18n::t(
                "palette.label.switch_session",
                "Switch Session",
                "Switch Session",
            ),
            description: i18n::t(
                "palette.desc.switch_session",
                "Browse and resume a saved session",
                "Browse and resume a saved session",
            ),
            category: cat.clone(),
            action: PaletteAction::ShowSessionMenu,
        },
        PaletteItem {
            id: "save_session".to_string(),
            label: i18n::t("palette.label.save_session", "Save Session", "Save Session"),
            description: i18n::t(
                "palette.desc.save_session",
                "Save checkpoint (/chat save)",
                "Save checkpoint (/chat save)",
            ),
            category: cat.clone(),
            action: PaletteAction::TypeCommand("/chat save ".to_string()),
        },
        PaletteItem {
            id: "delete_session".to_string(),
            label: i18n::t(
                "palette.label.delete_session",
                "Delete Session",
                "Delete Session",
            ),
            description: i18n::t(
                "palette.desc.delete_session",
                "Delete a saved session",
                "Delete a saved session",
            ),
            category: cat.clone(),
            action: PaletteAction::TypeCommand("/chat delete ".to_string()),
        },
        PaletteItem {
            id: "resume_latest".to_string(),
            label: i18n::t(
                "palette.label.resume_latest",
                "Resume Latest",
                "Resume Latest",
            ),
            description: i18n::t(
                "palette.desc.resume_latest",
                "Restore the most recent saved session",
                "Restore the most recent saved session",
            ),
            category: cat,
            action: PaletteAction::ExecuteCommand("/resume".to_string()),
        },
    ]
}

pub fn get_system_palette_items() -> Vec<PaletteItem> {
    let cat_settings = Some(i18n::t("palette.cat.settings", "Settings", "Settings"));
    let cat_diag = Some(i18n::t(
        "palette.cat.diagnostics",
        "Diagnostics",
        "Diagnostics",
    ));
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: i18n::t("palette.back.label", ".. Back", ".. Back"),
            description: i18n::t(
                "palette.back.desc.main",
                "Return to main menu",
                "Return to main menu",
            ),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "settings_help".to_string(),
            label: i18n::t(
                "palette.label.settings_overview",
                "Settings Overview",
                "Settings Overview",
            ),
            description: i18n::t(
                "palette.desc.settings_show",
                "Show current model, provider and config paths",
                "Show current model, provider and config paths",
            ),
            category: cat_settings.clone(),
            action: PaletteAction::ExecuteCommand("/settings".to_string()),
        },
        PaletteItem {
            id: "approval_mode".to_string(),
            label: i18n::t(
                "palette.label.approval_mode",
                "Approval Mode",
                "Approval Mode",
            ),
            description: i18n::t(
                "palette.desc.approval_mode",
                "Switch Auto, Plan or Yolo",
                "Switch Auto, Plan or Yolo",
            ),
            category: cat_settings.clone(),
            action: PaletteAction::Navigate(PaletteMode::AgentMode),
        },
        PaletteItem {
            id: "system_doctor".to_string(),
            label: i18n::t(
                "palette.label.system_doctor",
                "System Doctor",
                "System Doctor",
            ),
            description: i18n::t(
                "palette.desc.system_doctor_run",
                "Run system diagnostics (/doctor)",
                "Run system diagnostics (/doctor)",
            ),
            category: cat_diag,
            action: PaletteAction::ExecuteCommand("/doctor".to_string()),
        },
        PaletteItem {
            id: "language_settings".to_string(),
            label: i18n::t("palette.label.ui_language", "UI Language", "UI Language"),
            description: format!(
                "{}: {} — {}",
                i18n::t("palette.desc.lang_current", "Current", "Current"),
                i18n::current_language().as_code(),
                i18n::t(
                    "palette.desc.lang_switch",
                    "Switch interface language",
                    "Switch interface language"
                ),
            ),
            category: cat_settings,
            action: PaletteAction::Navigate(PaletteMode::Language),
        },
    ]
}

pub fn get_language_palette_items() -> Vec<PaletteItem> {
    let current = i18n::current_language();
    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.back.desc.settings",
            "Return to settings",
            "Return to settings",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    for (code, label) in i18n::available_languages() {
        let resolved = i18n::resolve_ui_language(Some(code));
        let active = resolved == current;
        items.push(PaletteItem {
            id: format!("lang_{}", code),
            label: format!("{}{}", if active { "✓ " } else { "  " }, label),
            description: format!(
                "{} {} ({})",
                i18n::t("palette.desc.lang_to", "Switch to", "Switch to"),
                label,
                code
            ),
            category: Some(i18n::t("palette.cat.language", "Language", "Language")),
            action: PaletteAction::ExecuteCommand(format!("/lang {}", code)),
        });
    }

    items
}

pub fn get_agent_palette_items() -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.agent", "Agent", "Agent"));
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: i18n::t("palette.back.label", ".. Back", ".. Back"),
            description: i18n::t(
                "palette.back.desc.main",
                "Return to main menu",
                "Return to main menu",
            ),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "agent_mode".to_string(),
            label: i18n::t("palette.label.agent_mode", "Agent Mode", "Agent Mode"),
            description: i18n::t(
                "palette.desc.agent_mode",
                "Switch agent mode (Plan/Auto/Yolo)",
                "Switch agent mode (Plan/Auto/Yolo)",
            ),
            category: cat.clone(),
            action: PaletteAction::Navigate(PaletteMode::AgentMode),
        },
        PaletteItem {
            id: "list_tools".to_string(),
            label: i18n::t("palette.label.list_tools", "List Tools", "List Tools"),
            description: i18n::t(
                "palette.desc.list_tools",
                "Show available tools",
                "Show available tools",
            ),
            category: cat.clone(),
            action: PaletteAction::ExecuteCommand("/tools".to_string()),
        },
        PaletteItem {
            id: "mcp_status".to_string(),
            label: i18n::t("palette.label.mcp_status", "MCP Status", "MCP Status"),
            description: i18n::t(
                "palette.desc.mcp_status",
                "Check MCP server status",
                "Check MCP server status",
            ),
            category: cat,
            action: PaletteAction::ExecuteCommand("/mcp status".to_string()),
        },
    ]
}

pub fn get_memory_palette_items() -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.memory", "Memory", "Memory"));
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: i18n::t("palette.back.label", ".. Back", ".. Back"),
            description: i18n::t(
                "palette.back.desc.prev",
                "Return to previous menu",
                "Return to previous menu",
            ),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "memory_stats".to_string(),
            label: i18n::t("palette.label.show_memory", "Show Memory", "Show Memory"),
            description: i18n::t(
                "palette.desc.show_memory",
                "Open current project memory",
                "Open current project memory",
            ),
            category: cat.clone(),
            action: PaletteAction::ExecuteCommand("/memory show".to_string()),
        },
        PaletteItem {
            id: "compact_memory".to_string(),
            label: i18n::t("palette.label.add_memory", "Add Memory", "Add Memory"),
            description: i18n::t(
                "palette.desc.add_memory",
                "Append a new memory entry",
                "Append a new memory entry",
            ),
            category: cat.clone(),
            action: PaletteAction::TypeCommand("/memory add ".to_string()),
        },
        PaletteItem {
            id: "clear_memory".to_string(),
            label: i18n::t(
                "palette.label.refresh_memory",
                "Refresh Memory",
                "Refresh Memory",
            ),
            description: i18n::t(
                "palette.desc.refresh_memory",
                "Reload memory from file",
                "Reload memory from file",
            ),
            category: cat,
            action: PaletteAction::ExecuteCommand("/memory refresh".to_string()),
        },
    ]
}

pub fn get_agent_mode_palette_items() -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.mode", "Mode", "Mode"));
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: i18n::t("palette.back.label", ".. Back", ".. Back"),
            description: i18n::t(
                "palette.back.desc.prev",
                "Return to previous menu",
                "Return to previous menu",
            ),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "mode_default".to_string(),
            label: i18n::t("palette.label.mode_auto", "Auto", "Auto"),
            description: i18n::t(
                "palette.desc.mode_auto",
                "Ask for confirmation on risky actions",
                "Ask for confirmation on risky actions",
            ),
            category: cat.clone(),
            action: PaletteAction::SetAgentMode("default".to_string()),
        },
        PaletteItem {
            id: "mode_plan".to_string(),
            label: i18n::t("palette.label.mode_plan", "Plan Mode", "Plan Mode"),
            description: i18n::t(
                "palette.desc.mode_plan",
                "Research first, avoid write actions",
                "Research first, avoid write actions",
            ),
            category: cat.clone(),
            action: PaletteAction::SetAgentMode("plan".to_string()),
        },
        PaletteItem {
            id: "mode_yolo".to_string(),
            label: i18n::t("palette.label.mode_yolo", "YOLO Mode", "YOLO Mode"),
            description: i18n::t(
                "palette.desc.mode_yolo",
                "Auto-approve all actions (dangerous)",
                "Auto-approve all actions (dangerous)",
            ),
            category: cat,
            action: PaletteAction::SetAgentMode("yolo".to_string()),
        },
    ]
}

pub fn get_thinking_effort_palette_items(
    current: &crate::types::ThinkingEffort,
    model_name: &str,
) -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.thinking", "Thinking", "Thinking"));
    let cap = crate::core::config::models::thinking_capability(model_name);

    let current_id = match current {
        crate::types::ThinkingEffort::Off => "thinking_off",
        crate::types::ThinkingEffort::Low => "thinking_low",
        crate::types::ThinkingEffort::Medium => "thinking_medium",
        crate::types::ThinkingEffort::High => "thinking_high",
    };
    // 选中标记不能再用 "●" —— 那正是 High 档的符号，`High ●` 读起来像两个东西。
    // 档位前缀直接用状态栏那套符号，用户在面板里选一次就认得状态栏上那一格了。
    let check = |id: &str| if id == current_id { "  (current)" } else { "" };
    let sym = |e: crate::types::ThinkingEffort| e.symbol();

    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.desc.back.main",
            "Return to previous menu",
            "Return to previous menu",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    // Always show Off
    items.push(PaletteItem {
        id: "thinking_off".to_string(),
        label: format!(
            "{} Off{}",
            sym(crate::types::ThinkingEffort::Off),
            check("thinking_off")
        ),
        description: i18n::t(
            "palette.desc.thinking_off",
            "Disable thinking/reasoning",
            "Disable thinking/reasoning",
        ),
        category: cat.clone(),
        action: PaletteAction::SetThinkingEffort("off".to_string()),
    });

    match cap {
        crate::core::config::models::ThinkingCapability::Granular => {
            // Full granular support: Low, Medium, High
            items.push(PaletteItem {
                id: "thinking_low".to_string(),
                label: format!(
                    "{} Low{}",
                    sym(crate::types::ThinkingEffort::Low),
                    check("thinking_low")
                ),
                description: i18n::t(
                    "palette.desc.thinking_low",
                    "Light thinking for simple tasks",
                    "Light thinking for simple tasks",
                ),
                category: cat.clone(),
                action: PaletteAction::SetThinkingEffort("low".to_string()),
            });
            items.push(PaletteItem {
                id: "thinking_medium".to_string(),
                label: format!(
                    "{} Medium{}",
                    sym(crate::types::ThinkingEffort::Medium),
                    check("thinking_medium")
                ),
                description: i18n::t(
                    "palette.desc.thinking_medium",
                    "Balanced thinking (recommended)",
                    "Balanced thinking (recommended)",
                ),
                category: cat.clone(),
                action: PaletteAction::SetThinkingEffort("medium".to_string()),
            });
            items.push(PaletteItem {
                id: "thinking_high".to_string(),
                label: format!(
                    "{} High{}",
                    sym(crate::types::ThinkingEffort::High),
                    check("thinking_high")
                ),
                description: i18n::t(
                    "palette.desc.thinking_high",
                    "Deep thinking for complex tasks",
                    "Deep thinking for complex tasks",
                ),
                category: cat,
                action: PaletteAction::SetThinkingEffort("high".to_string()),
            });
        }
        crate::core::config::models::ThinkingCapability::Binary => {
            // Binary support: just On (mapped to Medium internally)
            items.push(PaletteItem {
                id: "thinking_medium".to_string(),
                label: format!(
                    "{} On{}",
                    sym(crate::types::ThinkingEffort::Medium),
                    check("thinking_medium")
                ),
                description: i18n::t(
                    "palette.desc.thinking_on",
                    "Enable thinking/reasoning",
                    "Enable thinking/reasoning",
                ),
                category: cat,
                action: PaletteAction::SetThinkingEffort("medium".to_string()),
            });
        }
        crate::core::config::models::ThinkingCapability::None => {
            // No thinking support — show info only
            items.push(PaletteItem {
                id: "thinking_unavailable".to_string(),
                label: i18n::t(
                    "palette.thinking.unavailable",
                    "Not supported by this model",
                    "Not supported by this model",
                ),
                description: i18n::t(
                    "palette.desc.thinking_unavailable",
                    "Current model does not support thinking/reasoning",
                    "Current model does not support thinking/reasoning",
                ),
                category: cat,
                action: PaletteAction::SetThinkingEffort("off".to_string()),
            });
        }
    }

    items
}

pub fn get_context_window_palette_items(current_override: Option<u32>) -> Vec<PaletteItem> {
    let cat = Some(i18n::t(
        "palette.cat.context_window",
        "Context Window",
        "Context Window",
    ));
    let presets: &[(&str, u32, &str)] = &[
        ("ctx_auto", 0, "Auto (detect from model)"),
        ("ctx_128k", 128, "128k"),
        ("ctx_200k", 200, "200k"),
        ("ctx_256k", 256, "256k"),
        ("ctx_512k", 512, "512k"),
        ("ctx_1m", 1000, "1M"),
        ("ctx_2m", 2000, "2M"),
    ];
    let current_id = match current_override {
        None => "ctx_auto",
        Some(v) => {
            let k = v / 1000;
            presets
                .iter()
                .find(|(_, val, _)| *val == k)
                .map(|(id, _, _)| *id)
                .unwrap_or("ctx_custom")
        }
    };
    let check = |id: &str| if id == current_id { " ●" } else { "" };

    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.desc.back.main",
            "Return to previous menu",
            "Return to previous menu",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    for (id, _val, label) in presets {
        items.push(PaletteItem {
            id: id.to_string(),
            label: format!("{}{}", label, check(id)),
            description: if *id == "ctx_auto" {
                i18n::t(
                    "palette.desc.ctx_auto",
                    "Use model's default context window",
                    "Use model's default context window",
                )
            } else {
                format!("Set context window to {}", label)
            },
            category: cat.clone(),
            action: if *id == "ctx_auto" {
                PaletteAction::SetContextWindow("auto".to_string())
            } else {
                PaletteAction::SetContextWindow(format!("{}k", _val))
            },
        });
    }

    // Custom input option
    let custom_label = if current_id == "ctx_custom" {
        if let Some(v) = current_override {
            format!("Custom: {}k ●", v / 1000)
        } else {
            "Custom...".to_string()
        }
    } else {
        "Custom...".to_string()
    };
    items.push(PaletteItem {
        id: "ctx_custom".to_string(),
        label: custom_label,
        description: i18n::t(
            "palette.desc.ctx_custom",
            "Enter a custom context window size (e.g. 128k, 1M)",
            "Enter a custom context window size (e.g. 128k, 1M)",
        ),
        category: cat,
        action: PaletteAction::SetContextWindow("custom".to_string()),
    });

    items
}

pub fn get_theme_palette_items(state: &ChatState) -> Vec<PaletteItem> {
    let cat = Some(i18n::t("palette.cat.theme", "Theme", "Theme"));
    let current_name = &state.theme_manager.current().name;
    let check = |name: &str| if name == current_name { " ●" } else { "" };

    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.desc.back.main",
            "Return to previous menu",
            "Return to previous menu",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    for theme_name in state.theme_manager.list_themes() {
        let display_name = theme_name.to_string();
        items.push(PaletteItem {
            id: format!("theme_{}", theme_name),
            label: format!("{}{}", display_name, check(theme_name)),
            description: format!("Switch to {} theme", display_name),
            category: cat.clone(),
            action: PaletteAction::SetTheme(theme_name.to_string()),
        });
    }

    items
}

pub fn get_provider_palette_items(
    configured: &std::collections::HashSet<String>,
) -> Vec<PaletteItem> {
    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.desc.back.main",
            "Return to main menu",
            "Return to main menu",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    // Built-in providers
    let builtin_ids: std::collections::HashSet<&str> = ALL_PROVIDERS.iter().map(|p| p.id).collect();
    for provider in ALL_PROVIDERS.iter() {
        items.push(build_provider_item_flat(provider, configured));
    }

    // Custom providers (configured but not built-in)
    let mut custom_ids: Vec<&String> = configured
        .iter()
        .filter(|id| !builtin_ids.contains(id.as_str()))
        .collect();
    custom_ids.sort();
    for id in custom_ids {
        let is_active = false; // Will be determined by the palette rendering
        let label = format!("{} ✓", id);
        items.push(PaletteItem {
            id: format!("provider_{}", id),
            label,
            description: "Custom provider — click to configure".to_string(),
            category: Some("Custom".to_string()),
            action: PaletteAction::Navigate(PaletteMode::ProviderOptions(id.clone())),
        });
    }

    // Add new provider entry
    items.push(PaletteItem {
        id: "add_provider".to_string(),
        label: "➕ Add New Provider".to_string(),
        description: "Create a custom OpenAI/Anthropic compatible endpoint".to_string(),
        category: Some("Custom".to_string()),
        action: PaletteAction::Navigate(PaletteMode::AddProvider),
    });

    items
}

fn build_provider_item_flat(
    provider: &ProviderMetadata,
    configured: &HashSet<String>,
) -> PaletteItem {
    let is_configured = configured.contains(provider.id);
    let uses_manual_base_url = provider_requires_manual_base_url(provider.id);

    let label = if is_configured {
        format!("{} ✓", provider.name)
    } else {
        provider.name.to_string()
    };

    let action = if uses_manual_base_url {
        PaletteAction::Navigate(PaletteMode::ProviderOptions(provider.id.to_string()))
    } else {
        PaletteAction::InputApiKey(provider.id.to_string())
    };

    PaletteItem {
        id: format!("provider_{}", provider.id),
        label,
        description: String::new(),
        category: None,
        action,
    }
}

pub fn get_provider_popular_items(configured: &HashSet<String>) -> Vec<PaletteItem> {
    get_provider_category_items(ProviderCategory::Popular, "Popular", configured)
}

pub fn get_provider_local_items(configured: &HashSet<String>) -> Vec<PaletteItem> {
    get_provider_category_items(ProviderCategory::Local, "Local", configured)
}

pub fn get_provider_other_items(configured: &HashSet<String>) -> Vec<PaletteItem> {
    get_provider_category_items(ProviderCategory::Chinese, "Regional", configured)
}

fn get_provider_category_items(
    category: ProviderCategory,
    category_label: &str,
    configured: &HashSet<String>,
) -> Vec<PaletteItem> {
    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: i18n::t("palette.back.label", ".. Back", ".. Back"),
        description: i18n::t(
            "palette.back.desc.providers",
            "Return to providers",
            "Return to providers",
        ),
        category: None,
        action: PaletteAction::Back,
    }];

    for provider in ALL_PROVIDERS.iter().filter(|p| p.category == category) {
        items.push(build_provider_item(provider, category_label, configured));
    }

    items
}

pub fn get_add_provider_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: ".. Back".to_string(),
            description: "Return to providers".to_string(),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "add_openai_compatible".to_string(),
            label: "OpenAI Compatible".to_string(),
            description: "Custom OpenAI-compatible endpoint (LM Studio, Ollama, vLLM, etc.)"
                .to_string(),
            category: Some("Choose Type".to_string()),
            action: PaletteAction::Navigate(PaletteMode::AddProviderId(
                "openai-compatible".to_string(),
            )),
        },
        PaletteItem {
            id: "add_anthropic_compatible".to_string(),
            label: "Anthropic Compatible".to_string(),
            description: "Custom Anthropic-compatible endpoint (Claude /v1/messages)".to_string(),
            category: Some("Choose Type".to_string()),
            action: PaletteAction::Navigate(PaletteMode::AddProviderId(
                "anthropic-compatible".to_string(),
            )),
        },
    ]
}

pub fn get_add_provider_id_items(provider_type: &str) -> Vec<PaletteItem> {
    let type_label = if provider_type == "anthropic-compatible" {
        "Anthropic Compatible"
    } else {
        "OpenAI Compatible"
    };
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: ".. Back".to_string(),
            description: "Return to add provider".to_string(),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "input_provider_id".to_string(),
            label: format!("Enter a unique ID for your {} provider", type_label),
            description: "e.g. my-lmstudio, my-ollama, work-api".to_string(),
            category: Some("Provider ID".to_string()),
            action: PaletteAction::InputProviderId(provider_type.to_string()),
        },
    ]
}

pub fn get_provider_options_items(
    provider_id: &str,
    configured: &HashSet<String>,
) -> Vec<PaletteItem> {
    let is_configured = configured.contains(provider_id);
    let metadata = get_provider_by_id(provider_id);
    let uses_manual_base_url = provider_requires_manual_base_url(provider_id);
    let requires_api_key = metadata
        .map(|provider| provider.requires_api_key)
        .unwrap_or(true);
    let base_url_hint = if uses_manual_base_url {
        "Required first for custom or local endpoints".to_string()
    } else {
        "Override the built-in endpoint URL".to_string()
    };
    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: ".. Back".to_string(),
        description: "Return to list".to_string(),
        category: None,
        action: PaletteAction::Back,
    }];

    let (primary_label, primary_description, primary_action) = if is_configured {
        (
            "Use This Provider".to_string(),
            "Activate this provider and choose a model".to_string(),
            PaletteAction::SelectProvider(provider_id.to_string()),
        )
    } else if uses_manual_base_url {
        (
            if requires_api_key {
                "Set Base URL First".to_string()
            } else {
                "Set Base URL & Choose Model".to_string()
            },
            if requires_api_key {
                "Save endpoint URL, then continue to API key and model selection".to_string()
            } else {
                "Save endpoint URL, then continue directly to model selection".to_string()
            },
            PaletteAction::InputBaseUrl(provider_id.to_string()),
        )
    } else if !requires_api_key {
        (
            "Use This Provider".to_string(),
            "Activate this provider and choose a model".to_string(),
            PaletteAction::SelectProvider(provider_id.to_string()),
        )
    } else {
        (
            "Set API Key & Choose Model".to_string(),
            "Save API key and continue to model selection".to_string(),
            PaletteAction::InputApiKey(provider_id.to_string()),
        )
    };

    items.push(PaletteItem {
        id: "use_provider".to_string(),
        label: primary_label,
        description: primary_description,
        category: Some("Connect".to_string()),
        action: primary_action,
    });

    let api_key_item = PaletteItem {
        id: "input_key".to_string(),
        label: if requires_api_key {
            "Set API Key".to_string()
        } else {
            "Set API Key (Optional)".to_string()
        },
        description: if requires_api_key {
            "Save API key for this provider".to_string()
        } else {
            "Only needed when your local gateway expects a key".to_string()
        },
        category: Some("Config".to_string()),
        action: PaletteAction::InputApiKey(provider_id.to_string()),
    };

    let base_url_item = PaletteItem {
        id: "input_url".to_string(),
        label: "Set Base URL".to_string(),
        description: base_url_hint,
        category: Some("Config".to_string()),
        action: PaletteAction::InputBaseUrl(provider_id.to_string()),
    };

    if uses_manual_base_url {
        items.push(base_url_item);
        items.push(api_key_item);
    } else {
        items.push(api_key_item);
        items.push(base_url_item);
    }

    items.push(PaletteItem {
        id: "provider_doctor".to_string(),
        label: "Diagnose Effective Config".to_string(),
        description: "Show provider, model, key source and config file paths".to_string(),
        category: Some("Config".to_string()),
        action: PaletteAction::ExecuteCommand("/provider doctor".to_string()),
    });

    items
}

fn build_provider_item(
    provider: &ProviderMetadata,
    category_label: &str,
    configured: &HashSet<String>,
) -> PaletteItem {
    let is_configured = configured.contains(provider.id);
    let uses_manual_base_url = provider_requires_manual_base_url(provider.id);
    let label = if is_configured {
        format!("{} ✓", provider.name)
    } else {
        provider.name.to_string()
    };

    let description = if uses_manual_base_url {
        if is_configured {
            if provider.requires_api_key {
                format!(
                    "{} - ready, endpoint and API key configurable",
                    provider.description
                )
            } else {
                format!("{} - ready, endpoint configurable", provider.description)
            }
        } else if provider.requires_api_key {
            format!(
                "{} - custom endpoint, URL first then API key",
                provider.description
            )
        } else {
            format!(
                "{} - local endpoint, set URL then choose model",
                provider.description
            )
        }
    } else if is_configured {
        if provider.requires_api_key {
            format!(
                "{} - ready, review settings or replace API key",
                provider.description
            )
        } else {
            format!(
                "{} - ready, review settings or choose model",
                provider.description
            )
        }
    } else {
        format!("{} - paste API key only", provider.description)
    };

    let action = if uses_manual_base_url || is_configured {
        PaletteAction::Navigate(PaletteMode::ProviderOptions(provider.id.to_string()))
    } else {
        PaletteAction::InputApiKey(provider.id.to_string())
    };

    PaletteItem {
        id: format!("provider_{}", provider.id),
        label,
        description,
        category: Some(category_label.to_string()),
        action,
    }
}

pub fn get_provider_quick_items(configured: &HashSet<String>) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    for (category, category_label) in [
        (ProviderCategory::Popular, "Popular"),
        (ProviderCategory::Local, "Local"),
        (ProviderCategory::Chinese, "Regional"),
    ] {
        for provider in ALL_PROVIDERS
            .iter()
            .filter(|provider| provider.category == category)
        {
            items.push(build_provider_item(provider, category_label, configured));
        }
    }
    items
}

/// 把缓存年龄写成人话（"12m ago"）。`None` 表示这个会话里还没拿到过列表。
/// `/model list` 也用它，保证两处措辞一致。
pub(crate) fn format_cache_age(age_secs: Option<u64>) -> Option<String> {
    let secs = age_secs?;
    Some(match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    })
}

/// 手动输入模型名 + 显式刷新。
///
/// 这两项放在列表最前面（紧跟 `.. Back`）是有意的：模型多的中转站一屏放不下，
/// 放在末尾等于用户找不到。用户要的就是"要么自己敲模型名，要么明确让它去拉一次"。
fn push_model_entry_options(
    items: &mut Vec<PaletteItem>,
    current: &str,
    is_loading: bool,
    cache_age_secs: Option<u64>,
) {
    items.push(PaletteItem {
        id: "type_model".to_string(),
        label: "⌨ Enter model name...".to_string(),
        description: if current.is_empty() {
            "Type any model ID your provider accepts — no network call".to_string()
        } else {
            format!("Type any model ID (prefilled with {})", current)
        },
        category: Some("Manual".to_string()),
        action: PaletteAction::InputModelName,
    });

    let (label, description) = if is_loading {
        (
            "⟳ Fetching model list...".to_string(),
            "Querying every configured provider".to_string(),
        )
    } else {
        let hint = match format_cache_age(cache_age_secs) {
            Some(age) => format!("List cached {} · queries every configured provider", age),
            None => "Queries every configured provider (can take a few seconds)".to_string(),
        };
        ("⟳ Fetch model list from API".to_string(), hint)
    };
    items.push(PaletteItem {
        id: "refresh_models".to_string(),
        label,
        description,
        category: Some("Manual".to_string()),
        action: PaletteAction::RefreshModels,
    });
}

pub fn get_model_palette_items(
    models: &[String],
    current: &str,
    is_loading: bool,
    model_provider_map: &std::collections::HashMap<String, String>,
    cache_age_secs: Option<u64>,
) -> Vec<PaletteItem> {
    let mut items = vec![PaletteItem {
        id: "back".to_string(),
        label: ".. Back".to_string(),
        description: "Return to main menu".to_string(),
        category: None,
        action: PaletteAction::Back,
    }];

    push_model_entry_options(&mut items, current, is_loading, cache_age_secs);

    if models.is_empty() {
        if is_loading {
            items.push(PaletteItem {
                id: "loading".to_string(),
                label: "⏳ Loading models...".to_string(),
                description: "Fetching from remote API".to_string(),
                category: None,
                action: PaletteAction::Back,
            });
            return items;
        }
        if !current.is_empty() {
            items.push(PaletteItem {
                id: format!("model_{}", current),
                label: format!("{} (current)", current),
                description: "Using configured model".to_string(),
                category: Some("Models".to_string()),
                action: PaletteAction::SetModel(current.to_string()),
            });
        }
        return items;
    }

    // Group models by provider
    let mut provider_models: std::collections::BTreeMap<String, Vec<&String>> =
        std::collections::BTreeMap::new();

    for model in models {
        let provider = model_provider_map.get(model).cloned().unwrap_or_else(|| {
            // Infer provider from model name
            infer_provider_from_model(model)
        });
        provider_models.entry(provider).or_default().push(model);
    }

    // Add provider-grouped models
    for (provider, provider_model_list) in &provider_models {
        let provider_label = format_provider_name(provider);
        for model in provider_model_list {
            let label = if model.as_str() == current {
                format!("{} (Current)", model)
            } else {
                model.to_string()
            };

            items.push(PaletteItem {
                id: format!("model_{}", model),
                label,
                description: format!("Provider: {}", provider_label),
                category: Some(provider_label.clone()),
                action: PaletteAction::SetModel(model.to_string()),
            });
        }
    }

    items
}

/// Infer provider from model name prefix
fn infer_provider_from_model(model: &str) -> String {
    let model_lower = model.to_lowercase();
    if model_lower.starts_with("gpt-")
        || model_lower.starts_with("o1-")
        || model_lower.starts_with("o3-")
    {
        "openai".to_string()
    } else if model_lower.starts_with("claude-") {
        "anthropic".to_string()
    } else if model_lower.starts_with("gemini-") || model_lower.starts_with("palm-") {
        "google".to_string()
    } else if model_lower.starts_with("deepseek-") {
        "deepseek".to_string()
    } else if model_lower.starts_with("qwen-") || model_lower.starts_with("qwq-") {
        "alibaba".to_string()
    } else if model_lower.starts_with("glm-") || model_lower.starts_with("chatglm-") {
        "zhipu".to_string()
    } else if model_lower.starts_with("moonshot-") || model_lower.starts_with("kimi-") {
        "moonshot".to_string()
    } else if model_lower.starts_with("doubao-") || model_lower.starts_with("bytedance-") {
        "bytedance".to_string()
    } else if model_lower.starts_with("mistral-") || model_lower.starts_with("mixtral-") {
        "mistral".to_string()
    } else if model_lower.starts_with("llama-") || model_lower.starts_with("codellama-") {
        "meta".to_string()
    } else if model_lower.starts_with("yi-") {
        "01ai".to_string()
    } else if model_lower.starts_with("internlm-") {
        "internlm".to_string()
    } else if model_lower.starts_with("phi-") {
        "microsoft".to_string()
    } else if model_lower.starts_with("amazon-") || model_lower.starts_with("titan-") {
        "amazon".to_string()
    } else if model_lower.starts_with("command-") || model_lower.starts_with("c4ai-") {
        "cohere".to_string()
    } else if model_lower.contains("grok") {
        "xai".to_string()
    } else {
        "other".to_string()
    }
}

/// Format provider name for display
fn format_provider_name(provider: &str) -> String {
    match provider {
        "openai" => "OpenAI".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "google" => "Google".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "alibaba" => "Alibaba (Qwen)".to_string(),
        "zhipu" => "Zhipu (GLM)".to_string(),
        "moonshot" => "Moonshot".to_string(),
        "bytedance" => "ByteDance".to_string(),
        "mistral" => "Mistral".to_string(),
        "meta" => "Meta".to_string(),
        "01ai" => "01.AI (Yi)".to_string(),
        "internlm" => "InternLM".to_string(),
        "microsoft" => "Microsoft".to_string(),
        "amazon" => "Amazon".to_string(),
        "cohere" => "Cohere".to_string(),
        "xai" => "xAI".to_string(),
        "other" => "Other".to_string(),
        _ => provider.to_string(),
    }
}

pub fn get_session_quick_items(
    sessions: &[crate::utils::session_manager::SessionSummary],
) -> Vec<PaletteItem> {
    if sessions.is_empty() {
        return vec![PaletteItem {
            id: "session_empty".to_string(),
            label: "No saved sessions".to_string(),
            description: "Use /chat save to create one".to_string(),
            category: Some("Session".to_string()),
            action: PaletteAction::Back,
        }];
    }

    sessions
        .iter()
        .map(|session| PaletteItem {
            id: format!("session_{}", session.id),
            label: session.title.clone(),
            description: session.subtitle.clone(),
            category: Some("Session".to_string()),
            action: PaletteAction::ExecuteCommand(format!("/chat resume {}", session.id)),
        })
        .collect()
}

pub fn get_project_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: ".. Back".to_string(),
            description: "Return to main menu".to_string(),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "init_project".to_string(),
            label: "Initialize Project".to_string(),
            description: "Analyze and create STARCODE.md (/init)".to_string(),
            category: Some("Project".to_string()),
            action: PaletteAction::ExecuteCommand("/init".to_string()),
        },
        PaletteItem {
            id: "file_status".to_string(),
            label: "File Status (Git)".to_string(),
            description: "Show changed files".to_string(),
            category: Some("Project".to_string()),
            action: PaletteAction::ExecuteCommand("/git status".to_string()),
        },
        PaletteItem {
            id: "restore_file".to_string(),
            label: "Restore File".to_string(),
            description: "Restore file from checkpoint (/restore)".to_string(),
            category: Some("Project".to_string()),
            action: PaletteAction::TypeCommand("/restore ".to_string()),
        },
        PaletteItem {
            id: "project_stats".to_string(),
            label: "Project Stats".to_string(),
            description: "Show session statistics (/stats)".to_string(),
            category: Some("Project".to_string()),
            action: PaletteAction::ExecuteCommand("/stats".to_string()),
        },
        PaletteItem {
            id: "git_operations".to_string(),
            label: i18n::t("palette.label.git_ops", "Git Operations", "Git Operations"),
            description: i18n::t(
                "palette.desc.git_ops",
                "Status, Diff, Create PR, Log",
                "Status, Diff, Create PR, Log",
            ),
            category: Some("Project".to_string()),
            action: PaletteAction::Navigate(PaletteMode::Git),
        },
    ]
}

pub fn get_integrations_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: ".. Back".to_string(),
            description: "Return to main menu".to_string(),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "manage_mcp".to_string(),
            label: "Manage MCP Servers".to_string(),
            description: "Servers, tools, enable/disable, reconnect".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::OpenMcpModal,
        },
        PaletteItem {
            id: "marketplace".to_string(),
            label: "Extension Marketplace".to_string(),
            description: "Browse & install skills, plugins, MCP servers".to_string(),
            category: Some("Marketplace".to_string()),
            action: PaletteAction::OpenMarketModal,
        },
        PaletteItem {
            id: "list_tools".to_string(),
            label: "List Tools".to_string(),
            description: "Show all available tools".to_string(),
            category: Some("Tools".to_string()),
            action: PaletteAction::ExecuteCommand("/tools".to_string()),
        },
        PaletteItem {
            id: "tool_desc".to_string(),
            label: "Show Tool Descriptions".to_string(),
            description: "Toggle detailed tool info".to_string(),
            category: Some("Tools".to_string()),
            action: PaletteAction::ExecuteCommand("/tools desc".to_string()),
        },
    ]
}

pub fn get_mcp_manage_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "back".to_string(),
            label: ".. Back".to_string(),
            description: "Return to Integrations".to_string(),
            category: None,
            action: PaletteAction::Back,
        },
        PaletteItem {
            id: "mcp_manager".to_string(),
            label: "Open MCP Manager".to_string(),
            description: "Interactive server manager (status, tools, toggle)".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::OpenMcpModal,
        },
        PaletteItem {
            id: "mcp_list".to_string(),
            label: "List Servers".to_string(),
            description: "Show connected MCP servers".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::ExecuteCommand("/mcp list".to_string()),
        },
        PaletteItem {
            id: "mcp_status".to_string(),
            label: "Check Status".to_string(),
            description: "Check connection status".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::ExecuteCommand("/mcp status".to_string()),
        },
        PaletteItem {
            id: "mcp_add".to_string(),
            label: "Add Server".to_string(),
            description: "Connect new MCP server".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::TypeCommand("/mcp add ".to_string()),
        },
        PaletteItem {
            id: "mcp_refresh".to_string(),
            label: "Refresh Servers".to_string(),
            description: "Reload all MCP connections".to_string(),
            category: Some("MCP".to_string()),
            action: PaletteAction::ExecuteCommand("/mcp refresh".to_string()),
        },
    ]
}

pub fn get_help_palette_items() -> Vec<PaletteItem> {
    let mut items = vec![
        PaletteItem {
            id: "help_keys".to_string(),
            label: "Keyboard Shortcuts".to_string(),
            description: "Open the keybindings popup (same as F1)".to_string(),
            category: Some("Help".to_string()),
            action: PaletteAction::ExecuteCommand("/help keys".to_string()),
        },
        PaletteItem {
            id: "about_cmd".to_string(),
            label: "About".to_string(),
            description: "Version and build info".to_string(),
            category: Some("Help".to_string()),
            action: PaletteAction::ExecuteCommand("/about".to_string()),
        },
    ];

    // 全部命令按分类列出：选中即把命令填入输入框（不直接执行，方便补参数）
    for cmd in crate::commands::system::ALL_COMMANDS {
        let aliases = if cmd.alt_names.is_empty() {
            String::new()
        } else {
            format!(" (alias: {})", cmd.alt_names.join(", "))
        };
        let subs = if cmd.sub_commands.is_empty() {
            String::new()
        } else {
            format!(
                " — subcommands: {}",
                cmd.sub_commands
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        items.push(PaletteItem {
            id: format!("cmd_{}", cmd.name),
            label: format!("/{}", cmd.name),
            description: format!("{}{}{}", cmd.description, aliases, subs),
            category: Some(cmd.category.to_string()),
            action: PaletteAction::TypeCommand(format!("/{} ", cmd.name)),
        });
    }

    items
}

pub fn render_palette(f: &mut Frame, area: Rect, state: &mut ChatState) {
    if !state.is_palette_open() {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(palette_title(&state.palette_mode, &state.palette_filter));

    let area = centered_rect(70, 60, area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
        .split(area);

    let query = state.palette_filter.trim();
    let query_lower = query.to_lowercase();
    let is_empty = query.is_empty();

    let placeholder = palette_placeholder(&state.palette_mode);
    let input_text = if is_empty {
        placeholder
    } else {
        state.palette_filter.as_str()
    };
    let input_style = if is_empty {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let input = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(input_text.to_string(), input_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Search "),
    );
    f.render_widget(input, chunks[0]);

    let search_items = get_search_items(&state.palette_mode, state, query);
    let filtered: Vec<&PaletteItem> = search_items
        .iter()
        .filter(|item| palette_item_matches_query(item, &query_lower))
        .collect();

    let grouped = !query.is_empty() || matches!(state.palette_mode, PaletteMode::Model);

    let mut list_items: Vec<ListItem> = Vec::new();
    let mut selectable_indices: Vec<usize> = Vec::new();

    if filtered.is_empty() {
        list_items.push(ListItem::new(Line::from(Span::styled(
            "No results",
            Style::default().fg(Color::DarkGray),
        ))));
    } else if grouped {
        let mut groups: Vec<(String, Vec<&PaletteItem>)> = Vec::new();

        for item in filtered.iter() {
            let category = item.category.clone().unwrap_or_else(|| "Other".to_string());

            if let Some((_, items)) = groups.iter_mut().find(|(name, _)| name == &category) {
                items.push(*item);
            } else {
                groups.push((category, vec![*item]));
            }
        }

        for (category, items) in groups {
            list_items.push(ListItem::new(Line::from(Span::styled(
                category,
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ))));

            for item in items {
                let mut spans = Vec::new();
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    item.label.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                if !item.description.is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        item.description.clone(),
                        Style::default().fg(Color::Gray),
                    ));
                }

                list_items.push(ListItem::new(Line::from(spans)));
                selectable_indices.push(list_items.len() - 1);
            }
        }
    } else {
        for item in filtered.iter() {
            let mut spans = Vec::new();
            spans.push(Span::styled(
                item.label.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
            if !item.description.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    item.description.clone(),
                    Style::default().fg(Color::Gray),
                ));
            }

            list_items.push(ListItem::new(Line::from(spans)));
            selectable_indices.push(list_items.len() - 1);
        }
    }

    let mut list_state = ListState::default();
    let filtered_len = filtered.len();
    if filtered_len > 0 {
        if state.selected_palette_index >= filtered_len {
            state.selected_palette_index = filtered_len - 1;
        }
        let selected_index = selectable_indices[state.selected_palette_index];
        list_state.select(Some(selected_index));
    }

    let list = List::new(list_items)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, chunks[1], &mut list_state);
}

fn get_output_style_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "style_default".to_string(),
            label: i18n::t("palette.output_style.default", "Default", "Default"),
            description: i18n::t(
                "palette.output_style.default_desc",
                "Standard output formatting",
                "Standard output formatting",
            ),
            category: None,
            action: PaletteAction::SetOutputStyle("default".to_string()),
        },
        PaletteItem {
            id: "style_concise".to_string(),
            label: i18n::t("palette.output_style.concise", "Concise", "Concise"),
            description: i18n::t(
                "palette.output_style.concise_desc",
                "Minimal output, fewer explanations",
                "Minimal output, fewer explanations",
            ),
            category: None,
            action: PaletteAction::SetOutputStyle("concise".to_string()),
        },
        PaletteItem {
            id: "style_verbose".to_string(),
            label: i18n::t("palette.output_style.verbose", "Verbose", "Verbose"),
            description: i18n::t(
                "palette.output_style.verbose_desc",
                "Detailed output with full explanations",
                "Detailed output with full explanations",
            ),
            category: None,
            action: PaletteAction::SetOutputStyle("verbose".to_string()),
        },
    ]
}

fn get_git_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "git_status".to_string(),
            label: i18n::t("palette.git.status", "Git Status", "Git Status"),
            description: i18n::t(
                "palette.git.status_desc",
                "Show current git status",
                "Show current git status",
            ),
            category: None,
            action: PaletteAction::ExecuteCommand("/git status".to_string()),
        },
        PaletteItem {
            id: "git_diff".to_string(),
            label: i18n::t("palette.git.diff", "Git Diff", "Git Diff"),
            description: i18n::t(
                "palette.git.diff_desc",
                "Show staged and unstaged changes",
                "Show staged and unstaged changes",
            ),
            category: None,
            action: PaletteAction::ExecuteCommand("/git diff".to_string()),
        },
        PaletteItem {
            id: "create_pr".to_string(),
            label: i18n::t("palette.git.create_pr", "Create PR", "Create PR"),
            description: i18n::t(
                "palette.git.create_pr_desc",
                "Create a pull request with AI assistance",
                "Create a pull request with AI assistance",
            ),
            category: None,
            action: PaletteAction::CreatePr,
        },
        PaletteItem {
            id: "git_log".to_string(),
            label: i18n::t("palette.git.log", "Git Log", "Git Log"),
            description: i18n::t(
                "palette.git.log_desc",
                "Show recent commit history",
                "Show recent commit history",
            ),
            category: None,
            action: PaletteAction::ExecuteCommand("/git log".to_string()),
        },
    ]
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1]);

    layout[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[PaletteItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    /// 手动输入和显式刷新必须在列表最前面：中转站能返回几百个模型，
    /// 放在末尾等于用户翻不到。
    #[test]
    fn manual_entry_and_refresh_come_before_the_models() {
        let models: Vec<String> = (0..40).map(|i| format!("model-{}", i)).collect();
        let items = get_model_palette_items(
            &models,
            "model-0",
            false,
            &std::collections::HashMap::new(),
            None,
        );

        let labels = ids(&items);
        assert_eq!(labels[0], ".. Back");
        assert!(labels[1].contains("Enter model name"), "{:?}", labels[1]);
        assert!(labels[2].contains("Fetch model list"), "{:?}", labels[2]);
        assert_eq!(items.len(), 3 + models.len());
    }

    /// 空列表时同样给出两个出口 —— 拉不到模型不该是死路。
    #[test]
    fn an_empty_list_still_offers_both_ways_forward() {
        let items = get_model_palette_items(
            &[],
            "claude-opus-5",
            false,
            &std::collections::HashMap::new(),
            None,
        );

        let labels = ids(&items);
        assert!(labels.iter().any(|l| l.contains("Enter model name")));
        assert!(labels.iter().any(|l| l.contains("Fetch model list")));
        // 当前模型即使不在列表里也要能选回来
        assert!(labels.iter().any(|l| l.starts_with("claude-opus-5")));
    }

    #[test]
    fn refresh_item_reports_progress_while_fetching() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            true,
            &std::collections::HashMap::new(),
            None,
        );
        let refresh = items
            .iter()
            .find(|i| i.id == "refresh_models")
            .expect("refresh item should always be present");
        assert!(refresh.label.contains("Fetching"), "{}", refresh.label);
    }

    #[test]
    fn cache_age_shows_up_in_the_refresh_hint() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            false,
            &std::collections::HashMap::new(),
            Some(7_200),
        );
        let refresh = items.iter().find(|i| i.id == "refresh_models").unwrap();
        assert!(
            refresh.description.contains("2h ago"),
            "{}",
            refresh.description
        );
    }

    #[test]
    fn cache_age_is_written_in_the_biggest_fitting_unit() {
        assert_eq!(format_cache_age(None), None);
        assert_eq!(format_cache_age(Some(0)).unwrap(), "just now");
        assert_eq!(format_cache_age(Some(59)).unwrap(), "just now");
        assert_eq!(format_cache_age(Some(60)).unwrap(), "1m ago");
        assert_eq!(format_cache_age(Some(3_599)).unwrap(), "59m ago");
        assert_eq!(format_cache_age(Some(3_600)).unwrap(), "1h ago");
        assert_eq!(format_cache_age(Some(86_400)).unwrap(), "1d ago");
    }

    /// 主面板搜索列表里两个入口都要有，且 key 不能撞（撞了会被去重掉一个）。
    #[test]
    fn the_two_entry_points_have_distinct_dedup_keys() {
        let items = get_model_palette_items(
            &["a".to_string()],
            "a",
            false,
            &std::collections::HashMap::new(),
            None,
        );
        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        push_unique_actionable_items(&mut merged, &mut seen, items);

        assert!(merged.iter().any(|i| i.id == "type_model"));
        assert!(merged.iter().any(|i| i.id == "refresh_models"));
    }
}
