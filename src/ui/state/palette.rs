#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteMode {
    Main,
    /// `/` 打开的命令面板。与 `Main` 共用同一个窗口、配色和选中行为，
    /// 只是条目来源换成 slash 命令（`commands::system::command_palette_items`）。
    Command,
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
    /// 分组标题。`None` = 不参与分组，渲染时平铺且不带任何标题行。
    ///
    /// 导航类条目（`.. Back`）和面板级操作项都用 `None`：给它们套一个
    /// "Other" 之类的兜底标题，会凭空造出一个语义为空的分组层级，
    /// 让用户找一个真正的条目时先穿过无意义的标题。
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
    /// 打开表单编辑一个已存在的 provider（内置或自定义均可）：预填当前值，
    /// 提交时 upsert 到同一个 id。
    EditProvider(String),
    Back,
    // New actions
    /// 选中一个模型。
    ///
    /// `provider_id` 是这个选项**自带的**归属，来自当前列表的
    /// `model_provider_map`（哪份列表给出的条目，归属就是那份列表说的，不猜）。
    /// `None` 表示当前列表没给出归属 —— 此时才回退到 `resolve_provider_for_model`
    /// 那套兜底推导。
    ///
    /// 为什么必须带：同一个模型名可以被多个 provider 提供，而 provider 决定请求
    /// 发往哪个 base_url。以前这里只带模型名，按下 Enter 再回头猜归属，猜错就
    /// 静默失败。`palette_action_key` 也因此用 `model + provider` 做去重键 ——
    /// 只用模型名的话，两个 provider 的同名条目会被去重成一个，另一个永远选不到。
    SetModel {
        model: String,
        provider_id: Option<String>,
    },
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
    /// 开关鼠标捕获。开：滚轮是真滚轮事件；关：终端原生选区恢复。
    ToggleMouseCapture,
    ToggleUiVerbose,
    CreatePr,
    ToggleColorblindMode,
    /// 打开"新增 provider"单面板表单（url / key / model 一次填完）
    OpenProviderForm,
    /// 删除一个自定义 provider（内置 provider 不可删）
    DeleteProvider(String),
    /// 清空某个 provider 已保存的 API Key。
    ///
    /// 为什么必须是独立动作、而不能靠"编辑时把 Key 留空"：留空在编辑模式里**已经有
    /// 含义**了 —— 它是「保持原 key」（见 `submit_provider_form`），因为密钥从不往
    /// 屏幕上画、不预填。所以没有任何输入能表达"我要清掉它"。
    ///
    /// 内置 provider 尤其需要：它们没有 Delete 入口（删掉整项配置没有意义），于是
    /// "清空 key"曾经是唯一做不了的操作 —— key 过期了、想切回环境变量、或者纯粹想
    /// 确认它到底存不存在，都只能去手改 `user-settings.json`。
    ClearProviderKey(String),
    /// 显式刷新模型列表（跳过缓存，扇出到所有已配置 provider）
    RefreshModels,
    OpenMcpModal,    // open the unified /mcp manager modal
    OpenMarketModal, // open the unified extension marketplace modal
}

#[derive(Debug, Clone)]
pub enum InputContext {
    ContextWindow,
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
///
/// 填完字段 Tab 到底部的操作行（`on_actions`）：`‹ Back` 取消回上一层，
/// `Save ›` 提交。`action_index` 是操作行里的选中项。
///
/// `editing_id` 为 `Some` 时是编辑模式：五个字段预填该 provider 的现值，
/// 提交时 upsert 回同一个 id；为 `None` 是新增，id 由名称 / URL host 派生。
#[derive(Debug, Clone)]
pub struct ProviderFormState {
    /// 与 [`PROVIDER_FORM_FIELDS`] 下标对齐的字段值
    pub values: Vec<String>,
    /// 当前焦点的字段下标
    pub active_field: usize,
    /// 焦点是否在底部的 Back/Save 操作行上
    pub on_actions: bool,
    /// 操作行选中项：0 = Back，1 = Save
    pub action_index: usize,
    /// 编辑模式：正在改的 provider id。`None` 是新增。
    pub editing_id: Option<String>,
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
            active_field: 0,
            on_actions: false,
            // 默认落在 Save 上：填完五个字段后通常就是要保存
            action_index: 1,
            editing_id: None,
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

/// API Key 字段的提示：新增时空着是"不要 key"，编辑时空着是"保持原 key"
/// （密钥不预填到表单里，见 `open_provider_form`）。
pub fn api_key_field_hint(editing: bool) -> &'static str {
    if editing {
        "Leave empty to keep the current key"
    } else {
        "Leave empty to skip"
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
        "Models",
        "Comma / newline separated · first is the default · leave empty to auto-fetch",
        true,
    ),
];
