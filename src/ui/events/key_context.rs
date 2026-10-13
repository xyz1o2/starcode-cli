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
//! **这不是一句口号**：原先 `handle_key_event` 里的 25 层阶梯、`has_overlay_active()`、
//! `has_dialog_active()`、`stream.rs` 的 `user_is_elsewhere` 各自推导过一次「现在是什么
//! 模式」，四份清单互不一致 —— 漏一项就表现为「弹窗开着却按键穿透」或「按键被看不见的
//! 层吞掉」。它们现在都由这里的两个派生谓词给出，见 [`KeyContext::is_exclusive`] 与
//! [`KeyContext::is_dialog`]。
//!
//! # 优先级
//!
//! [`KeyContext`] 按「离用户越近越优先」排序，与原阶梯的书写顺序一致：
//! 模态 > 覆盖层 > 面板 > 建议浮层 > vim > 输入框。

use crate::ui::state::{modal::Modal, ChatState};

/// 当前按键所处的上下文。
///
/// 优先级 = [`KeyContext::of`] 里的**判断顺序**（模态栈最优先，最后才是输入框），
/// 不是声明顺序：模态栈上的多个变体在声明上分散、在判断上是一段。
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
    /// 快捷键帮助浮层（`F1` / 空输入时的 `?`）
    ///
    /// 之前这个状态在 `has_overlay_active()` 里算作覆盖层，却不在本枚举里 ——
    /// 于是「判断模式只写一处」是假的：vim 门控按 `KeyContext` 推导时会把帮助
    /// 窗口当成普通输入，弹窗开着 vim 照样接管按键。
    Help,
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
        // 模态栈优先。输入框同样是栈上的一个 `Modal::InputModal`，不再是平行的
        // flag —— 曾经两个真相源靠手工同步，异步 ModelsList 一打穿就出现
        // 「面板画得出来、按键却全被隐形输入框吞掉」。
        if state.is_input_modal_open() {
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
        // 帮助浮层排在模态之后：draw_ui 里 help 先画、模态后画，模态在上面，
        // 判定必须与绘制顺序一致，否则顶层窗口的按键会落到下一层去。
        if state.show_help {
            return KeyContext::Help;
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

    /// 是否有遮挡层占住了键盘。
    ///
    /// 「普通输入 + vim + 常驻任务面板」都不算遮挡层：面板只吃 Ctrl/Alt 组合键与
    /// ↑/↓/Enter，普通字符仍要落到输入框（与 `has_overlay_active()` 刻意不含
    /// `task_panel.is_visible` 的口径一致）。
    ///
    /// 取代三份手写清单：`events/input.rs` 的 vim 门控与双 Esc 判定、
    /// `services/stream.rs` 的「用户是不是在别处」。注意 `TextInput` **不**是
    /// 独占状态 —— 它是基线。
    pub fn is_exclusive(&self) -> bool {
        !matches!(
            self,
            KeyContext::TextInput | KeyContext::Vim | KeyContext::TaskPanel
        )
    }

    /// 是否是「独占屏幕的对话框」：模态栈上的窗口、输入框、工具审批。
    ///
    /// 轻量浮层（建议菜单、补全提示、快捷帮助）与任务面板都不算。
    /// `services/stream.rs` 决定要不要自动弹出模型面板时用它 —— 那里**不能**用
    /// [`KeyContext::is_exclusive`]：打 `/model` 时命令补全浮层必然开着，用
    /// `is_exclusive` 会把正常流程一起挡掉。
    pub fn is_dialog(&self) -> bool {
        matches!(
            self,
            KeyContext::Modal
                | KeyContext::InputModal
                | KeyContext::Palette
                | KeyContext::ThemePicker
                | KeyContext::ReadOnlyDialog
                | KeyContext::ToolConfirmation
        )
    }

    /// 该上下文下 vim 键是否应当让位。
    ///
    /// vim 只在没有遮挡层时接管普通字符。任务面板是常驻面板而非遮挡层（只吃
    /// Ctrl/Alt 组合键与 ↑/↓/Enter），开着它时 `j`/`k`/`i` 仍要落到输入框。
    pub fn vim_enabled(&self) -> bool {
        matches!(
            self,
            KeyContext::Vim | KeyContext::TextInput | KeyContext::TaskPanel
        )
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
        // 输入框叠在别的模态之上打开时，栈顶未必是它（exit_input_modal 因此用
        // retain 而不是 pop），但按键仍归输入框。这里锁死该行为。
        let mut state = ChatState::new();
        state.enter_input_modal();
        state.open_palette(crate::ui::state::palette::PaletteMode::Main);
        assert!(state.is_input_modal_open());
        assert_eq!(KeyContext::of(&state), KeyContext::InputModal);
    }

    #[test]
    fn tool_confirmation_outranks_task_panel() {
        let mut state = ChatState::new();
        state.task_panel.is_visible = true;
        state.is_awaiting_confirmation = true;
        assert_eq!(KeyContext::of(&state), KeyContext::ToolConfirmation);
    }

    /// 帮助浮层以前不在枚举里，「模式只写一处」是假的：弹窗开着 vim 照样接管。
    #[test]
    fn help_overlay_is_a_context_and_yields_to_a_modal() {
        let mut state = ChatState::new();
        state.show_help = true;
        assert_eq!(KeyContext::of(&state), KeyContext::Help);
        assert!(KeyContext::of(&state).is_exclusive());
        assert!(!KeyContext::of(&state).vim_enabled());
        assert!(!KeyContext::of(&state).is_dialog());

        // 模态画在帮助之上，判定顺序必须跟着绘制顺序。
        state.open_palette(crate::ui::state::palette::PaletteMode::Main);
        assert_eq!(KeyContext::of(&state), KeyContext::Palette);
    }

    #[test]
    fn only_real_dialogs_count_as_dialog() {
        let mut state = ChatState::new();
        assert!(!KeyContext::of(&state).is_dialog());

        // 打 /model 时命令补全浮层必然开着，不能算对话框。
        state.show_command_hints = true;
        assert!(!KeyContext::of(&state).is_dialog());
        assert!(KeyContext::of(&state).is_exclusive());

        state.show_command_hints = false;
        state.enter_input_modal();
        assert!(KeyContext::of(&state).is_dialog());
    }

    #[test]
    fn base_contexts_are_not_exclusive() {
        let mut state = ChatState::new();
        assert!(!KeyContext::of(&state).is_exclusive());

        state.vim_enabled = true;
        assert!(!KeyContext::of(&state).is_exclusive());

        state.task_panel.is_visible = true;
        assert!(!KeyContext::of(&state).is_exclusive());
    }
}
