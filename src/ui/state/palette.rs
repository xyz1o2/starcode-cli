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
    InputProviderId(String),   // provider_type
    InputProviderName(String), // provider_id (after ID is entered)
    /// 手动输入模型名：打开输入框，不发任何网络请求
    InputModelName,
    /// 显式刷新模型列表（跳过缓存，扇出到所有已配置 provider）
    RefreshModels,
    OpenMcpModal,    // open the unified /mcp manager modal
    OpenMarketModal, // open the unified extension marketplace modal
}

#[derive(Debug, Clone)]
pub enum InputContext {
    ProviderKey {
        provider_id: String,
    },
    ProviderBaseUrl {
        provider_id: String,
    },
    ContextWindow,
    /// 手动录入模型名（不校验、不联网，直接切过去）
    ModelName,
    AddProviderId {
        provider_type: String,
    },
    AddProviderName {
        provider_type: String,
        provider_id: String,
    },
    AddProviderBaseUrl {
        provider_type: String,
        provider_id: String,
        name: String,
    },
    AddProviderApiKey {
        provider_type: String,
        provider_id: String,
        name: String,
        base_url: String,
    },
    /// Plugins 弹窗：录入 marketplace 来源（git URL / owner/repo / 本地路径）
    MarketplaceSource,
    /// /add-dir 无参数：录入要追加的工作目录路径
    AddWorkingDir,
}

impl Default for PaletteMode {
    fn default() -> Self {
        Self::Main
    }
}
