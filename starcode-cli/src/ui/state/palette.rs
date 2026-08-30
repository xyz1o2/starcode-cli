#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteMode {
    Main,
    Provider,
    Session,
    System,
    Agent,
    Memory,
    Git,
    Mcp,
    // New modes
    Project,
    Integrations,
    McpManage,
    Help,
    Model,
    AgentMode,
    ThinkingEffort,
    ContextWindow,
    Theme,
    ProviderPopular,
    ProviderOther,
    ProviderLocal,
    ProviderOptions(String),
    AddProvider,
    AddProviderId(String), // provider_type: "openai-compatible" or "anthropic-compatible"
    Language,
    OutputStyle,
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub category: Option<String>,
    pub action: PaletteAction,
}

#[derive(Debug, Clone)]
pub enum PaletteAction {
    Navigate(PaletteMode),
    ShowStatus,
    ShowModelMenu,
    ShowProviderMenu,
    ShowSessionMenu,
    ExecuteCommand(String), // Immediately execute
    TypeCommand(String),    // Type into input box (with trailing space if needed)
    SelectProvider(String),
    ToggleFeature(String),
    InputApiKey(String),  // provider_id
    InputBaseUrl(String), // provider_id
    Back,
    // New actions
    SetModel(String),
    SetAgentMode(String),
    SetThinkingEffort(String),
    SetContextWindow(String),
    SetTheme(String),
    SetOutputStyle(String),
    ShowLogSelector,
    ShowContextViz,
    ToggleVimMode,
    ToggleUiVerbose,
    CreatePr,
    ToggleColorblindMode,
    InputProviderId(String),    // provider_type
    InputProviderName(String),  // provider_id (after ID is entered)
}

#[derive(Debug, Clone)]
pub enum InputContext {
    ProviderKey { provider_id: String },
    ProviderBaseUrl { provider_id: String },
    ContextWindow,
    AddProviderId { provider_type: String },
    AddProviderName { provider_type: String, provider_id: String },
    AddProviderBaseUrl { provider_type: String, provider_id: String, name: String },
    AddProviderApiKey { provider_type: String, provider_id: String, name: String, base_url: String },
}

impl Default for PaletteMode {
    fn default() -> Self {
        Self::Main
    }
}
