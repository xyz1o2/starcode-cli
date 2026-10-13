//! 剪贴板写入：系统剪贴板优先，失败退回终端的 OSC 52。
//!
//! # 为什么需要两条路
//!
//! `arboard` 写的是**本进程所在机器**的剪贴板。在 SSH 会话里它写的是远端机器，
//! 在 WSL 里它写的是发行版那一侧（通常没有 X/Wayland），在 headless 下它根本
//! 建不出来。这三种环境里"已复制"的提示照常显示，用户粘出来却是上次的旧内容。
//!
//! OSC 52 走的是终端转义序列：文本由**本地终端程序**接收并放进它自己的剪贴板，
//! 因此在 SSH/WSL 下恰好是那条能用的路。crossterm 0.29 自带
//! [`crossterm::clipboard::CopyToClipboard`]，仓库此前从未引用。

use crossterm::clipboard::CopyToClipboard;

/// 写剪贴板。两步都失败才返回 `Err`。
///
/// 返回的 `Err` 文案同时包含两次失败的原因 —— 调用方要把它原样显示给用户，
/// 不能吞掉，否则又是「宣告成功但内容是旧的」。
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // 空内容一律拒绝：`arboard` 会把剪贴板清空，OSC 52 的空 payload 在部分
    // 终端上同样会被当成「清空」。复制空串从来不是用户的意图。
    if text.is_empty() {
        return Err("没有可复制的内容".to_string());
    }
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text.to_string())) {
        Ok(()) => Ok(()),
        Err(system_err) => {
            // OSC 52：转义序列直接写到 stdout，不经过任何剪贴板服务。
            let result = crossterm::execute!(
                std::io::stdout(),
                CopyToClipboard::to_clipboard_from(text.to_owned())
            );
            match result {
                Ok(()) => Ok(()),
                Err(osc_err) => Err(format!("{system_err}; OSC 52 也失败: {osc_err}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空串不许进剪贴板 —— `arboard` 会清空它，OSC 52 的空 payload 部分终端也
    /// 当成清空，而「复制空内容」从来不是用户的意图。
    #[test]
    fn empty_text_is_rejected_before_touching_the_clipboard() {
        let err = copy_to_clipboard("").expect_err("空内容应当被拒绝");
        assert!(err.contains("没有可复制的内容"), "实际错误：{err}");
    }
}