use ratatui::style::{Color, Modifier, Style};

/// **没有主题上下文时**的次要文字兜底样式。
///
/// `src/ui` 里有一批纯工具函数（`utils/render.rs`、`components/common/*`、
/// `components/highlight/*`、几个 modal）根本不接收 `&Theme`，它们以前直接写
/// `Style::default().fg(Color::DarkGray)`。这里给它一个名字：**次要文字只有
/// 这一处定义**，将来要把这些函数也接上主题时，只需要把调用点的
/// `muted_style()` 换成 `theme.subtle_style()`，颜色本身不用再改。
pub fn muted_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// [`muted_style`] 的更弱一档（对应以前的 `DarkGray + DIM`）。
pub fn muted_dim_style() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM)
}

impl Theme {
    /// 次要文字（辅助说明、提示行）：比正文弱一档。
    ///
    /// 以前这类文字一半写死 `Color::DarkGray`（不跟随主题）、一半取
    /// `theme.subtle` —— 同一语义两套颜色来源。统一从这里取色。
    pub fn subtle_style(&self) -> Style {
        Style::default().fg(self.subtle)
    }

    /// 更弱的文字：在 [`Theme::subtle_style`] 基础上再压一档。
    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.subtle).add_modifier(Modifier::DIM)
    }
}

#[derive(Clone)]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub foreground: Color,
    pub primary: Color,
    pub secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,
    pub border: Color,
    pub highlight: Color,
    pub comment: Color,
    pub string: Color,
    pub keyword: Color,
    pub function: Color,

    // Shimmer 效果颜色（用于 spinner 和状态栏的闪烁）
    pub primary_shimmer: Color,
    pub secondary_shimmer: Color,
    pub warning_shimmer: Color,
    pub error_shimmer: Color,

    // Diff 颜色
    pub diff_added: Color,
    pub diff_removed: Color,
    pub diff_added_dimmed: Color,
    pub diff_removed_dimmed: Color,
    pub diff_added_word: Color,
    pub diff_removed_word: Color,

    // Agent 颜色（用于多代理场景）
    pub agent_red: Color,
    pub agent_blue: Color,
    pub agent_green: Color,
    pub agent_yellow: Color,
    pub agent_purple: Color,
    pub agent_orange: Color,

    // UI 元素颜色
    pub user_message_bg: Color,
    pub selection_bg: Color,
    pub inactive: Color,
    pub subtle: Color,
    pub suggestion: Color,

    // Thinking 相关颜色
    pub thinking_fg: Color,
    pub thinking_bg: Color,

    // Tool 相关颜色
    pub tool_fg: Color,
    pub tool_bg: Color,
    pub tool_success: Color,
    pub tool_error: Color,
    pub tool_border: Color,

    // 用户消息颜色
    pub user_fg: Color,
    pub user_bg: Color,

    // Assistant 消息颜色
    pub assistant_fg: Color,
    pub assistant_bg: Color,

    // 状态栏颜色
    pub status_fg: Color,
    pub status_bg: Color,

    // 输入框颜色
    pub input_fg: Color,
    pub input_bg: Color,
    pub input_border: Color,

    // 代码块颜色
    pub code_fg: Color,
    pub code_bg: Color,

    // 链接颜色
    pub link_fg: Color,
}

pub struct ThemeManager {
    themes: Vec<Theme>,
    current_index: usize,
}

impl ThemeManager {
    pub fn new() -> Self {
        let mut manager = Self {
            themes: Vec::new(),
            current_index: 0,
        };
        manager.load_presets();
        manager
    }

    fn load_presets(&mut self) {
        self.themes.push(Theme::default_dark());
        self.themes.push(Theme::default_light());
        self.themes.push(Theme::monokai());
        self.themes.push(Theme::dracula());
        self.themes.push(Theme::solarized_dark());
        self.themes.push(Theme::solarized_light());
        self.themes.push(Theme::catppuccin_mocha());
        self.themes.push(Theme::tokyo_night());
        self.themes.push(Theme::gruvbox_dark());
        self.themes.push(Theme::nord());
        self.themes.push(Theme::one_dark());
        self.themes.push(Theme::claude_code());
        self.themes.push(Theme::high_contrast());
    }

    pub fn current(&self) -> &Theme {
        &self.themes[self.current_index]
    }

    pub fn set_theme(&mut self, name: &str) -> bool {
        if let Some(index) = self.themes.iter().position(|t| t.name == name) {
            self.current_index = index;
            publish_current_theme(&self.themes[self.current_index]);
            true
        } else {
            false
        }
    }

    /// 主题名 → 颜色的直接查询（不需要 ThemeManager 实例）。
    ///
    /// 深层渲染工具函数（如 `build_user_body_block`）拿不到 `ChatState`，
    /// 从这里取当前主题的字段。进程里真正的 `ThemeManager` 在 `ChatState`
    /// 上，`set_theme` 时会把当前主题同步发布到这里。
    pub fn current_theme() -> std::sync::Arc<Theme> {
        shared_current_theme()
    }

    pub fn list_themes(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.name.as_str()).collect()
    }

    pub fn next_theme(&mut self) {
        self.current_index = (self.current_index + 1) % self.themes.len();
        publish_current_theme(&self.themes[self.current_index]);
    }
}

/// 当前主题的进程级快照 —— `set_theme` / `next_theme` 时发布。
///
/// 背景：主题此前只活在 `ChatState.theme_manager` 里，而一些**深层渲染工具
/// 函数**（`build_user_body_block` 等）不在组件调用链上、拿不到 `state`。
/// 每当 `ThemeManager` 切换主题就覆盖这把锁，工具函数读最近一帧的主题。
/// 测试专用构造（`new_for_test` 一类）不经过 `set_theme` 时读起来是
/// `default_dark`，与首套预设一致。
static CURRENT_THEME: std::sync::OnceLock<std::sync::RwLock<std::sync::Arc<Theme>>> =
    std::sync::OnceLock::new();

fn shared_current_theme() -> std::sync::Arc<Theme> {
    let lock = CURRENT_THEME
        .get_or_init(|| std::sync::RwLock::new(std::sync::Arc::new(Theme::default_dark())));
    lock.read()
        .map(|t| t.clone())
        .unwrap_or_else(|_| std::sync::Arc::new(Theme::default_dark()))
}

fn publish_current_theme(theme: &Theme) {
    let lock = CURRENT_THEME
        .get_or_init(|| std::sync::RwLock::new(std::sync::Arc::new(Theme::default_dark())));
    if let Ok(mut slot) = lock.write() {
        *slot = std::sync::Arc::new(theme.clone());
    }
}
