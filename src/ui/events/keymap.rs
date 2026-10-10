//! 按键注册表 —— 全局绑定���**唯一定义处**。
//!
//! # 为什么要有这张表
//!
//! 在此之前，全局按键散落在 `input.rs` 的 25 层 `if` 阶梯里，每处写成
//! `if key.modifiers.contains(CONTROL) && matches!(key.code, Char('x'))`，
//! 而帮助弹窗 (`help_popup.rs`) 是**手抄**的第二份清单。两份清单已经漂移出
//! 多处不一致（Tab 被帮助说成「补全」、实际是「折叠工具」；`Ctrl+Shift+F` /
//! `Ctrl+Shift+P` / `Ctrl+R` 根本没列）。
//!
//! 现在：绑定只有 [`GlobalBinding`] 一处定义，帮助文案与「哪些键在 modal 里
//! 仍然生效」都由它生成，杜绝第三份副本。
//!
//! # 与 modal 层的关系
//!
//! 各 modal handler 的 `match key.code` **不看修饰键**，`Ctrl+C` 会被当成
//! `Char('c')` 打进过滤框。`modal_input::is_global_key` 据此把全局键从 modal 层
//! 放行 —— 那份白名单同样由本表生成（[`binding_for`]），保证两处不会各写一套。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 全局绑定。**每个键只应在此出现一次**；新增绑定请连同帮助文案一起加。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalBinding {
    /// Ctrl+P — 开关命令面板
    TogglePalette,
    /// Ctrl+O — 切换 transcript / 详细输出
    ToggleTranscript,
    /// Ctrl+T — 切换任务面板
    ToggleTasks,
    /// Ctrl+Shift+F — 全局搜索
    GlobalSearch,
    /// Ctrl+Shift+P — 快速打开文件
    QuickOpen,
    /// Ctrl+R — 历史搜索
    HistorySearch,
    /// Ctrl+C — 复制选区 / 中断生成 / 连按两次退出
    CopyOrInterrupt,
    /// Ctrl+D — 退出
    Exit,
    /// Ctrl+L — 清屏
    ClearScreen,
    /// Ctrl+V — 粘贴
    Paste,
    /// F1 — 帮助
    ToggleHelp,
    /// Shift+Tab — 切换 Plan/Build
    TogglePlanMode,
    /// Alt+P — 折叠/展开长输入
    FoldInput,
    /// Alt+T — 循环思考深度
    CycleThinking,
    /// Alt+Shift+T — 切换主题
    CycleTheme,
}

impl GlobalBinding {
    /// 帮助弹窗里展示的键名。
    pub fn label(&self) -> &'static str {
        match self {
            Self::TogglePalette => "Ctrl+P",
            Self::ToggleTranscript => "Ctrl+O",
            Self::ToggleTasks => "Ctrl+T",
            Self::GlobalSearch => "Ctrl+Shift+F",
            Self::QuickOpen => "Ctrl+Shift+P",
            Self::HistorySearch => "Ctrl+R",
            Self::CopyOrInterrupt => "Ctrl+C",
            Self::Exit => "Ctrl+D",
            Self::ClearScreen => "Ctrl+L",
            Self::Paste => "Ctrl+V",
            Self::ToggleHelp => "F1",
            Self::TogglePlanMode => "Shift+Tab",
            Self::FoldInput => "Alt+P",
            Self::CycleThinking => "Alt+T",
            Self::CycleTheme => "Alt+Shift+T",
        }
    }
}

/// 带帮助文案的注册表条目。
pub struct BindingDoc {
    pub binding: GlobalBinding,
    /// 帮助文案的中英文键，由 `i18n::t` 在渲染时填充。
    pub zh: &'static str,
    pub en: &'static str,
}

/// 帮助弹窗「控制」一节的内容，**由注册表生成**。
///
/// 新增全局绑定只需在 [`GLOBAL_BINDINGS`] 里加一行，帮助弹窗自动跟上。
pub const GLOBAL_BINDINGS: &[BindingDoc] = &[
    BindingDoc {
        binding: GlobalBinding::TogglePalette,
        zh: "打开命令面板",
        en: "Open command palette",
    },
    BindingDoc {
        binding: GlobalBinding::ToggleTasks,
        zh: "切换任务面板",
        en: "Toggle tasks",
    },
    BindingDoc {
        binding: GlobalBinding::ToggleTranscript,
        zh: "切换详细输出（transcript）",
        en: "Verbose output (transcript)",
    },
    BindingDoc {
        binding: GlobalBinding::GlobalSearch,
        zh: "全局搜索代码",
        en: "Search across files",
    },
    BindingDoc {
        binding: GlobalBinding::QuickOpen,
        zh: "快速打开文件",
        en: "Quick open file",
    },
    BindingDoc {
        binding: GlobalBinding::HistorySearch,
        zh: "搜索历史输入",
        en: "Search input history",
    },
    BindingDoc {
        binding: GlobalBinding::CopyOrInterrupt,
        zh: "复制选区 / 中断生成 / 清空输入（连按两次退出）",
        en: "Copy selection / interrupt / clear input (press twice to exit)",
    },
    BindingDoc {
        binding: GlobalBinding::ClearScreen,
        zh: "清屏",
        en: "Clear screen",
    },
    BindingDoc {
        binding: GlobalBinding::Paste,
        zh: "粘贴剪贴板内容",
        en: "Paste from clipboard",
    },
    BindingDoc {
        binding: GlobalBinding::ToggleHelp,
        zh: "显示/隐藏本帮助",
        en: "Toggle this help",
    },
    BindingDoc {
        binding: GlobalBinding::Exit,
        zh: "退出程序",
        en: "Exit",
    },
    BindingDoc {
        binding: GlobalBinding::TogglePlanMode,
        zh: "切换 Plan/Build",
        en: "Toggle Plan/Build",
    },
    BindingDoc {
        binding: GlobalBinding::FoldInput,
        zh: "折叠/展开长输入",
        en: "Fold/unfold long input",
    },
    BindingDoc {
        binding: GlobalBinding::CycleThinking,
        zh: "切换思考深度（◌/○/◐/●）",
        en: "Cycle thinking depth (◌/○/◐/●)",
    },
    BindingDoc {
        binding: GlobalBinding::CycleTheme,
        zh: "切换主题",
        en: "Cycle theme",
    },
];

/// 把一个按键事件映射到全局绑定。
///
/// `None` 表示该键不是全局绑定，交给当前 modal / 输入框处理。
pub fn binding_for(key: &KeyEvent) -> Option<GlobalBinding> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    if ctrl && alt {
        return None;
    }

    if ctrl {
        let b = match key.code {
            KeyCode::Char('p') if !shift => GlobalBinding::TogglePalette,
            KeyCode::Char('o') => GlobalBinding::ToggleTranscript,
            KeyCode::Char('t') => GlobalBinding::ToggleTasks,
            KeyCode::Char('f') if shift => GlobalBinding::GlobalSearch,
            KeyCode::Char('p') if shift => GlobalBinding::QuickOpen,
            KeyCode::Char('r') => GlobalBinding::HistorySearch,
            KeyCode::Char('c') => GlobalBinding::CopyOrInterrupt,
            KeyCode::Char('d') => GlobalBinding::Exit,
            KeyCode::Char('l') => GlobalBinding::ClearScreen,
            KeyCode::Char('v') => GlobalBinding::Paste,
            _ => return None,
        };
        return Some(b);
    }

    if alt {
        return match key.code {
            KeyCode::Char('p') | KeyCode::Char('P') => Some(GlobalBinding::FoldInput),
            // Alt+T 循环思考深度；Alt+Shift+T 切主题。两者共用 Alt+T 前缀，
            // 用 shift 位区分（此前是靠 `ch.is_uppercase()` 判断字符大小写，
            // 对非字母键和部分终端的 shift 映射并不可靠）。
            KeyCode::Char('t') | KeyCode::Char('T') => Some(if shift {
                GlobalBinding::CycleTheme
            } else {
                GlobalBinding::CycleThinking
            }),
            _ => None,
        };
    }

    match key.code {
        KeyCode::F(1) => Some(GlobalBinding::ToggleHelp),
        KeyCode::BackTab => Some(GlobalBinding::TogglePlanMode),
        _ => None,
    }
}

/// 在 modal 打开时仍然必须生效的全局键。
///
/// modal handler 的 `match key.code` 不看修饰键，也没有「按下的键已被消费」
/// 的概念，任何没显式匹配的键都会被它当成输入字符或直接丢弃。这里由注册表
/// 推导出一张放行名单 —— 与 [`binding_for`] 同源，不会再各写一套而漂移。
///
/// 刻意**不含** `Esc` / `↑` / `↓` / `Enter`：那是 modal 自己的语义。
///
/// [`state/modal.rs`](crate::ui::state::modal) 里那些只读展示型对话框
/// （UsageStats / Export / CompressionStatus / ContextViz）也会放行这批键，
/// 它们内部没有任何交互，放行无副作用。
pub fn is_global_key(key: &KeyEvent) -> bool {
    binding_for(key).is_some()
}

// ───────────────────────── 建议浮层的动作 ─────────────────────────

/// 建议浮层（provider / session 菜单、命令与 @ 补全）的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    SelectPrev,
    SelectNext,
    AcceptSuggestion,
    AcceptCompletion,
}

/// 建议浮层的按键映射。
///
/// 与全局注册表是两回事：这些键只在建议浮层可见时才有意义。
pub fn map_key(state: &crate::ui::state::ChatState, key: &KeyEvent) -> Option<UiAction> {
    let overlay = state.show_provider_menu
        || state.show_session_menu
        || state.show_mention_hints
        || state.show_command_hints;
    match key.code {
        KeyCode::Up if overlay => Some(UiAction::SelectPrev),
        KeyCode::Down if overlay => Some(UiAction::SelectNext),
        KeyCode::Enter if overlay => Some(UiAction::AcceptSuggestion),
        KeyCode::Tab | KeyCode::Char('\t')
            if state.show_mention_hints || state.show_command_hints =>
        {
            Some(UiAction::AcceptCompletion)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_char_is_not_a_global_binding() {
        assert_eq!(binding_for(&key(KeyCode::Char('a'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn ctrl_shift_f_is_global_search_not_plain_f() {
        assert_eq!(
            binding_for(&key(KeyCode::Char('f'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)),
            Some(GlobalBinding::GlobalSearch)
        );
        // 普通 Ctrl+F 不绑定（留给终端/输入框）
        assert_eq!(
            binding_for(&key(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn ctrl_p_and_ctrl_shift_p_are_distinct() {
        assert_eq!(
            binding_for(&key(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Some(GlobalBinding::TogglePalette)
        );
        assert_eq!(
            binding_for(&key(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(GlobalBinding::QuickOpen)
        );
    }

    #[test]
    fn alt_t_splits_by_shift_not_by_char_case() {
        assert_eq!(
            binding_for(&key(KeyCode::Char('t'), KeyModifiers::ALT)),
            Some(GlobalBinding::CycleThinking)
        );
        assert_eq!(
            binding_for(&key(KeyCode::Char('T'), KeyModifiers::ALT)),
            Some(GlobalBinding::CycleThinking),
            "大写 T 不应被当成切主题"
        );
        assert_eq!(
            binding_for(&key(
                KeyCode::Char('t'),
                KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            Some(GlobalBinding::CycleTheme)
        );
    }

    #[test]
    fn modal_navigation_keys_are_not_global() {
        // Esc / 方向键 / Enter 必须留给 modal 自己
        for code in [
            KeyCode::Esc,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Backspace,
        ] {
            assert_eq!(
                binding_for(&key(code, KeyModifiers::NONE)),
                None,
                "{code:?} 不该是全局键"
            );
        }
    }

    #[test]
    fn registry_labels_are_unique() {
        let mut labels: Vec<&str> = GLOBAL_BINDINGS.iter().map(|b| b.binding.label()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(before, labels.len(), "绑定键名重复");
    }

    #[test]
    fn registry_covers_every_documented_binding() {
        // 帮助文案必须覆盖全部已定义绑定，否则又会出现「有键没文档」的漂移
        let mut in_registry: Vec<GlobalBinding> = GLOBAL_BINDINGS.iter().map(|b| b.binding).collect();
        in_registry.sort_by_key(|b| b.label());
        let mut declared = vec![
            GlobalBinding::TogglePalette,
            GlobalBinding::ToggleTranscript,
            GlobalBinding::ToggleTasks,
            GlobalBinding::GlobalSearch,
            GlobalBinding::QuickOpen,
            GlobalBinding::HistorySearch,
            GlobalBinding::CopyOrInterrupt,
            GlobalBinding::Exit,
            GlobalBinding::ClearScreen,
            GlobalBinding::Paste,
            GlobalBinding::ToggleHelp,
            GlobalBinding::TogglePlanMode,
            GlobalBinding::FoldInput,
            GlobalBinding::CycleThinking,
            GlobalBinding::CycleTheme,
        ];
        declared.sort_by_key(|b| b.label());
        assert_eq!(in_registry, declared);
    }
}