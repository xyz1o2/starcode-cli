//! 用户中断提示行（对标 CC 的 `Interrupted · What should Claude do instead?`）。
//!
//! 这不是组件，而是**往 `chat_history` 推一条 assistant 条目**的中断标记：CC 在中断后
//! 会在回复末尾留一行暗色小字，告诉用户"这一轮停了，已输出的内容还在，想改方向直接说"。
//! 缺了它，用户按完 Esc 只看到输入框重新可用，不知道刚才那半句话到底算不算数。
//!
//! 为什么原来是死代码：本文件从头到尾都在渲染 `Line`，而渲染路径拿不到"用户刚
//! 按了 Esc"这个时机（时机在事件循环的宽限期里，见 `ui/app/runtime.rs`）。

use crate::types::ChatEntry;

/// 中断提示文案。中文/英文按当前界面语言走 i18n。
pub fn interrupted_notice_text() -> String {
    crate::core::i18n::t(
        "ui.interrupted.notice",
        "已中断 · 已输出的内容保留。想换方向直接说。",
        "Interrupted · What should I do instead?",
    )
}

/// 往聊天区推一条中断提示。
///
/// `partial_kept` 为 false 时（模型一个字都没输出就被打断）不推 —— 空提示只会
/// 让用户以为发生了什么。
pub fn push_interrupted_notice(state: &mut crate::ui::state::ChatState, partial_kept: bool) {
    if !partial_kept {
        return;
    }
    state
        .chat_history
        .push(ChatEntry::assistant(interrupted_notice_text()).with_streaming(false));
    state.rendered_cache.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_is_pushed_only_when_something_was_streamed() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        let before = state.chat_history.len();

        push_interrupted_notice(&mut state, false);
        assert_eq!(
            state.chat_history.len(),
            before,
            "一个字都没输出就中断时不该留一条空提示"
        );

        push_interrupted_notice(&mut state, true);
        assert_eq!(state.chat_history.len(), before + 1);
        let text = &state.chat_history.last().unwrap().content;
        // 文案随界面语言走，测试不钉语言
        assert!(
            text.contains("Interrupted") || text.contains("中断"),
            "中断提示行应说明这一轮被停下，实际 {text:?}"
        );
    }
}
