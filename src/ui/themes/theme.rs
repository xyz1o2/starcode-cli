use ratatui::style::Color;

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
            true
        } else {
            false
        }
    }

    pub fn list_themes(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.name.as_str()).collect()
    }

    pub fn next_theme(&mut self) {
        self.current_index = (self.current_index + 1) % self.themes.len();
    }
}
