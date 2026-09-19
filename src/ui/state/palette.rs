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
    /// 删除自定义 provider 的二次确认页（内容是 provider_id）
    ProviderDelete(String),
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
    SetThinkingEffort(crate::types::ThinkingEffort),
    SetContextWindow(crate::core::context_policy::ContextWindowSelection),
    /// 打开精确 context capacity 的输入框；解析由 ContextWindowSelection 共用实现负责。
    InputContextWindow,
    SetTheme(String),
    SetOutputStyle(String),
    ShowLogSelector,
    ShowContextViz,
    ToggleVimMode,
    ToggleUiVerbose,
    CreatePr,
    ToggleColorblindMode,
    /// 打开"新增 provider"单面板表单（url / key / model 一次填完）
    OpenProviderForm,
    /// 删除一个自定义 provider（内置 provider 不可删）
    DeleteProvider(String),
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
    /// 单面板"新增 provider"表单。数据在 `ChatState::provider_form` 里，
    /// 这里只是 input-modal 走哪种提交逻辑的标记。
    ProviderForm,
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

/// "新增 provider" 单面板表单的编辑状态。
///
/// 五个字段共用一个 `ChatState::modal_textarea` 当活动字段编辑器：切字段时
/// 把当前内容存回 `values[active]`，再拿 `values[next]` 重建 textarea。
/// Type 字段（下标 0）是选项而不是自由文本，它的值存在 `values[0]` 里，
/// 由 `provider_type()` 解析，textarea 不渲染。
#[derive(Debug, Clone)]
pub struct ProviderFormState {
    /// 与 [`PROVIDER_FORM_FIELDS`] 下标对齐的字段值
    pub values: Vec<String>,
    /// 当前焦点的字段下标
    pub active_field: usize,
    /// 校验错误，显示在面板底部
    pub error: Option<String>,
}

impl ProviderFormState {
    pub fn new() -> Self {
        Self {
            values: vec![
                PROVIDER_FORM_TYPES[0].to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ],
            active_field: 1,
            error: None,
        }
    }

    /// `values[0]` 存的是类型标签，转成配置里持久化的 `r#type` 值。
    pub fn provider_type(&self) -> &'static str {
        match self.values.first().map(String::as_str) {
            Some(ANTHROPIC_COMPATIBLE_LABEL) => "anthropic-compatible",
            _ => "openai-compatible",
        }
    }

    pub fn name(&self) -> &str {
        self.values.get(1).map(String::as_str).unwrap_or("")
    }

    pub fn base_url(&self) -> &str {
        self.values.get(2).map(String::as_str).unwrap_or("")
    }

    pub fn api_key(&self) -> &str {
        self.values.get(3).map(String::as_str).unwrap_or("")
    }

    pub fn model(&self) -> &str {
        self.values.get(4).map(String::as_str).unwrap_or("")
    }

    /// ←/→ 切换类型；循环。只在 Type 字段获得焦点时调用。
    pub fn cycle_type(&mut self, forward: bool) {
        // `provider_type()` 返回的已经是类型串（"anthropic-compatible" 之类），
        // 不能再当标签喂给 `provider_type_of_label`——它只认 "Anthropic Compatible"
        // 这种标签，匹配不上就 fallback 成 openai-compatible，选中 Anthropic 后
        // ←/→ 就卡住不动。
        let current = self.provider_type();
        let idx = PROVIDER_FORM_TYPES
            .iter()
            .position(|label| provider_type_of_label(label) == current)
            .unwrap_or(0);
        let len = PROVIDER_FORM_TYPES.len();
        let next = if forward {
            (idx + 1) % len
        } else {
            (idx + len - 1) % len
        };
        self.values[0] = PROVIDER_FORM_TYPES[next].to_string();
    }
}

impl Default for ProviderFormState {
    fn default() -> Self {
        Self::new()
    }
}

/// Type 字段可选项（标签顺序就是 ←/→ 循环的顺序）
pub const PROVIDER_FORM_TYPES: &[&str] = &["OpenAI Compatible", "Anthropic Compatible"];
pub const OPENAI_COMPATIBLE_LABEL: &str = "OpenAI Compatible";
pub const ANTHROPIC_COMPATIBLE_LABEL: &str = "Anthropic Compatible";

fn provider_type_of_label(label: &str) -> &'static str {
    match label {
        ANTHROPIC_COMPATIBLE_LABEL => "anthropic-compatible",
        _ => "openai-compatible",
    }
}

/// 表单字段（下标与 `ProviderFormState::values` 对齐）。渲染和按键处理都读它，
/// 两边不用各维护一份硬编码列表。
pub const PROVIDER_FORM_FIELDS: &[(&str, &str, bool)] = &[
    ("Type", "API protocol type", false),
    (
        "Name",
        "Display name (optional — derived from URL host if empty)",
        true,
    ),
    (
        "Base URL",
        "Endpoint URL, e.g. http://localhost:1234/v1",
        true,
    ),
    ("API Key", "Leave empty to skip", true),
    (
        "Model",
        "Leave empty to auto-fetch the model list (cached)",
        true,
    ),
];
