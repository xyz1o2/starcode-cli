//! 按键上下文 —— 「当前处于哪个模式」的**唯一真相源**。
//!
//! # 为什么要有这一层
//!
//! 改动前，`handle_key_event` 是一个 25 层的顺序 `if` 阶梯，每层自带守卫条件：
//!
//! ```text
//! handle_modal_key → Shift+Tab → /clear 确认 → 粘贴确认 → Ctrl+P → …
//!   → handle_overlay_input → bg-agent 选择器 → vim → handle_input_modal
//!   → keymap → handle_paste → 模型确认 → LogSelector → ErrorOverlay → 主 match
//! ```
//!
//! 「现在是什么模式」这个判断被**重复推导**了十几次，且各层清单互不一致：
//! `has_overlay_active()` 漏掉 6 层、modal handler 漏掉 12 个全局键、任务面板
//! 漏掉 modal 守卫。漏一项就表现为「弹窗开着却按键穿透」或「按键被看不见的层吞掉」。
//!
//! 现在模式只在这里算一次（[`KeyContext::of`]），各层用 [`KeyContext`] 查询而不是
//! 各写一份 `if state.show_xxx || state.modal_stack...`。新增模式只改这一处。
//!
//! # 优先级
//!
//! [`KeyContext`] 按「离用户越近越优先」排序，与原阶梯的书写顺序一致：
//! 模态 > 覆盖层 > 面板 > 建议浮层 > vim > 输入框。

use crate::ui::state::{modal::Modal, ChatState};

/// 当前按键所处的上下文。按 [`KeyContext::of`] 返回值的**声明顺序**即优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// 模态对话框独占（palette / mcp / plugins / 搜索 / 只读展示窗 …）
    Modal,
    /// 输入对话框（API Key 等）—— 历史遗留的第二真相源，与 Modal.InputModal 并存
    InputModal,
    /// 行内确认：工具审批、AskUserQuestion、权限说明
    ToolConfirmation,
    /// 行内确认：`/clear` 与大段粘贴的「确定吗」
    InlineConfirm,
    /// 模型切换 y/n 确认
    ModelConfirm,
    /// 主题选择器（Enter 应用 / Esc 回滚）
    ThemePicker,
    /// 只读展示型对话框（Settings 状态、LogSelector、ErrorOverlay …）
    ReadOnlyDialog,
    /// 后台代理选择器：↑/↓/Enter/Esc
    BgAgentSelector,
    /// 命令面板（`Ctrl+P`）—— Modal 的一个特例，单独列出便于守卫
    Palette,
    /// 任务面板（常驻，`Ctrl+T`）
    TaskPanel,
    /// 建议浮层：provider / session 菜单、命令与 @ 补全
    SuggestionOverlay,
    /// vim
    Vim,
    /// 主输入框（默认）
    TextInput,
}

impl KeyContext {
    /// 由状态推导当前上下文。**这是唯一该写模式判断的地方。**
    pub fn of(state: &ChatState) -> KeyContext {
        // 模态栈优先。`show_input_modal` 与 `Modal::InputModal` 是两个真相源，
        // 异步路径（stream.rs 的 ModelsList）可能让它们不同步 —— 这里以 flag 为准，
        // 与 modal_input.rs 的分发口径保持一致。
        if state.show_input_modal {
            return KeyContext::InputModal;
        }
        match state.top_modal() {
            Some(Modal::Palette) => return KeyContext::Palette,
            Some(Modal::ThemePicker { .. }) => return KeyContext::ThemePicker,
            Some(
                Modal::UsageStats
                | Modal::Export
                | Modal::CompressionStatus
                | Modal::ContextViz
                | Modal::StatusModal
                | Modal::LogSelector
                | Modal::ErrorOverlay,
            ) => return KeyContext::ReadOnlyDialog,
            Some(_) => return KeyContext::Modal,
            None => {}
        }
        if state.is_awaiting_confirmation {
            return KeyContext::ToolConfirmation;
        }
        if state.show_clear_confirmation || state.show_paste_confirmation {
            return KeyContext::InlineConfirm;
        }
        if state.pending_model_confirmation {
            return KeyContext::ModelConfirm;
        }
        if state.bg_agent_selection.is_some() {
            return KeyContext::BgAgentSelector;
        }
        if state.task_panel.is_visible {
            return KeyContext::TaskPanel;
        }
        if state.show_provider_menu
            || state.show_session_menu
            || state.show_mention_hints
            || state.show_command_hints
        {
            return KeyContext::SuggestionOverlay;
        }
        if state.vim_enabled {
            return KeyContext::Vim;
        }
        KeyContext::TextInput
    }

    /// 该上下文是否完全独占键盘 —— 打开时全局键之外的键都不该漏到底层输入框。
    pub fn is_exclusive(&self) -> bool {
        !matches!(self, KeyContext::TaskPanel | KeyContext::Vim)
    }

    /// 该上下文下 vim 键是否应当让位。
    ///
    /// vim 只在没有遮挡层时接管普通字符。任务面板是常驻面板而非遮挡层（只吃
    /// Ctrl/Alt 组合键与 ↑/↓/Enter），开着它时 `j`/`k`/`i` 仍要落到输入框。
    pub fn vim_enabled(&self) -> bool {
        matches!(self, KeyContext::Vim | KeyContext::TextInput | KeyContext::TaskPanel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_state_is_text_input() {
        let state = ChatState::new();
        assert_eq!(KeyContext::of(&state), KeyContext::TextInput);
    }

    #[test]
    fn panel_is_not_exclusive_so_plain_chars_still_reach_input() {
        let mut state = ChatState::new();
        state.task_panel.is_visible = true;
        let ctx = KeyContext::of(&state);
        assert_eq!(ctx, KeyContext::TaskPanel);
        assert!(!ctx.is_exclusive(), "任务面板不应完全独占键盘");
        assert!(ctx.vim_enabled(), "开着任务面板时 vim 仍应可用");
    }

    #[test]
    fn palette_is_its_own_context_and_is_exclusive() {
        let mut state = ChatState::new();
        state.open_palette(crate::ui::state::palette::PaletteMode::Main);
        let ctx = KeyContext::of(&state);
        assert_eq!(ctx, KeyContext::Palette);
        assert!(ctx.is_exclusive());
        assert!(!ctx.vim_enabled(), "面板打开时 vim 必须让位");
    }

    #[test]
    fn input_modal_wins_over_modal_stack() {
        // 异步 ModelsList 曾经把 Palette 压在 InputModal 之上，两个真相源不同步。
        // 分发口径以 show_input_modal 为准，这里锁死该行为。
        let mut state = ChatState::new();
        state.enter_input_modal();
        state.open_palette(crate::ui::state::palette::PaletteMode::Main);
        assert_eq!(KeyContext::of(&state), KeyContext::InputModal);
    }

    #[test]
    fn tool_confirmation_outranks_task_panel() {
        let mut state = ChatState::new();
        state.task_panel.is_visible = true;
        state.is_awaiting_confirmation = true;
        assert_eq!(KeyContext::of(&state), KeyContext::ToolConfirmation);
    }
}