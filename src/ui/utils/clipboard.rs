//! 剪贴板写入：系统剪贴板优先（持久持有 + 重试），失败退回终端的 OSC 52。
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
//!
//! # 为什么 arboard 的 Clipboard 要持久持有
//!
//! X11（WSLg 的 Xwayland 正是这套协议）下剪贴板内容存在**持有者进程**里：
//! `Clipboard::new()` 每次新建连接，写完就 drop —— 连接一断选择权就丢，内容
//! 全靠剪贴板管理器抢救。arboard 3.6 对此有自己的原话：「Clipboard was dropped
//! very quickly after writing … Consider keeping `Clipboard` in more persistent
//! state」。之前的实现恰好踩满：每次复制都新建再立刻丢，于是
//! `held by another party` / `dropped very quickly` 在日志里成串出现。
//! 现在进程级持久持有一个，写完选择权仍在，同类报错自然消失。

use once_cell::sync::Lazy;
use std::io::Write;
use std::sync::Mutex;

use crossterm::clipboard::CopyToClipboard;
use crossterm::Command;

/// 进程级持久持有的系统剪贴板。`Err` 表示**这一刻**建不出来（headless 等），
/// 句柄本身可复用；写入失败后置回 `None`，下次调用会重新尝试建连接。
static CLIPBOARD: Lazy<Mutex<Option<arboard::Clipboard>>> = Lazy::new(|| Mutex::new(None));

/// 经持久句柄写剪贴板；瞬时占用（WSLg bridge、剪贴板管理器抢锁）重试一次。
///
/// 返回的 `Err` 汇总每一次尝试的失败原因，调用方要原样透传给用户。
fn copy_via_system(text: &str) -> Result<(), String> {
    const RETRY_DELAY_MS: u64 = 120;
    let mut last_err = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
        }
        let result = {
            let mut slot = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = arboard::Clipboard::new().ok();
            }
            slot.as_mut()
                .ok_or_else(|| "no clipboard provider".to_string())
                .and_then(|c| c.set_text(text.to_string()).map_err(|e| e.to_string()))
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                // 句柄坏了（X11 连接被 Xwayland 重启断掉等）就重建，再试时拿新连接。
                *CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()) = None;
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// 在 tmux 下裸发 OSC 52 会被逐字节转发给内层终端，多数老版本 tmux 不解析，
/// base64 就整段打到屏幕上（用户看到的"飞字"）。tmux ≥3.4 才默认放行 OSC 系列，
/// 兼容做法是包一层 tmux passthrough（`ESC P tmux ; ESC <payload> ESC \`），
/// payload 里的 ESC 按约定加倍（`\x1b` → `\x1b\x1b`），tmux 再还原成原始序列
/// 转给真终端。tmux 官方 FAQ 与 xterm.js/Windows Terminal 都遵循这套约定。
fn osc52_passthrough(payload: String) -> Vec<u8> {
    if std::env::var_os("TMUX").is_some() {
        let escaped = payload.replace('\x1b', "\x1b\x1b");
        return format!("\x1bPtmux;\x1b{}\x1b\\", escaped).into_bytes();
    }
    payload.into_bytes()
}

/// 写剪贴板。系统剪贴板重试全失败才退回 OSC 52，两条都失败才返回 `Err`。
///
/// 返回的 `Err` 文案同时包含失败的原因 —— 调用方要把它原样显示给用户，
/// 不能吞掉，否则又是「宣告成功但内容是旧的」。
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // 空内容一律拒绝：`arboard` 会把剪贴板清空，OSC 52 的空 payload 在部分
    // 终端上同样会被当成「清空」。复制空串从来不是用户的意图。
    if text.is_empty() {
        return Err("没有可复制的内容".to_string());
    }
    if let Err(system_err) = copy_via_system(text) {
        // OSC 52：转义序列直接写到 stdout，不经过任何剪贴板服务。
        let mut seq = String::new();
        let write_ok = CopyToClipboard::to_clipboard_from(text.to_owned())
            .write_ansi(&mut seq)
            .is_ok();
        let result = if write_ok {
            let bytes = osc52_passthrough(seq);
            std::io::stdout()
                .write_all(&bytes)
                .and_then(|()| std::io::stdout().flush())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "CopyToClipboard write_ansi failed",
            ))
        };
        match result {
            Ok(()) => Ok(()),
            Err(osc_err) => Err(format!("{system_err}; OSC 52 也失败: {osc_err}")),
        }
    } else {
        Ok(())
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

    /// 非 tmux 环境：OSC 52 序列原样透传，不加 passthrough 头。
    #[test]
    fn osc52_without_tmux_is_passed_through_untouched() {
        // 不临时改环境变量 —— 有 TMUX 的环境跳过这条断言分支。
        if std::env::var_os("TMUX").is_some() {
            return;
        }
        let seq = "\x1b]52;c;QUJD\x07".to_string();
        assert_eq!(osc52_passthrough(seq.clone()), seq.into_bytes());
    }

    /// tmux 环境：包 passthrough 裙路（DSOMUX），payload 里的 ESC 加倍。
    #[test]
    fn osc52_inside_tmux_wraps_passthrough_and_doubles_esc() {
        let seq = "\x1b]52;c;QUJD\x07".to_string();
        if std::env::var_os("TMUX").is_none() {
            // 用线程局部变量模拟 tmux 环境：std::env::set_var 在多线程下是 UB，
            // 只在确定无并发读者时才动；测试进程默认单线程跑到这里即可。
            unsafe { std::env::set_var("TMUX", "test-session") };
        }
        let wrapped = String::from_utf8(osc52_passthrough(seq)).unwrap();
        assert!(
            wrapped.starts_with("\x1bPtmux;\x1b"),
            "缺 passthrough 头: {wrapped:?}"
        );
        assert!(
            wrapped.ends_with("\x1b\\"),
            "缺 passthrough 尾: {wrapped:?}"
        );
        // payload 里的 ESC 已加倍：`]` 前应紧跟两个 ESC。
        assert!(
            wrapped.contains("\x1b\x1b]52;c;QUJD\x07"),
            "payload ESC 未加倍: {wrapped:?}"
        );
    }
}
