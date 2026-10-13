//! TUI 渲染测试的公共设施。
//!
//! 整个模块是 `#[cfg(test)]` 的，不会进入发布构建。三块东西：
//!
//! 1. [`snapshot`] —— 把 `Buffer` 序列化成带颜色的可 diff 文本，并做基线断言。
//! 2. [`ChatState::test_fixture`] —— 一份与机器无关的 `ChatState`。
//! 3. 组件级约定：组件测试住在组件文件自己的 `#[cfg(test)] mod tests` 里，
//!    快照断言只负责"画出来 → 和基线比"，逻辑断言照旧单独写。
//!
//! ## 为什么基线必须是带颜色的
//!
//! 之前 240 个 `ui::` 测试里只有 2 个真的画进 `TestBackend`，两个都是"扫一遍
//! 缓冲区找子串"式的，既给不出布局基线，也看不出颜色变化。这里补上的正是
//! 缺的那一层：改一个字符、改一段布局、改一种颜色，diff 都会指出来。

pub mod snapshot;

pub use snapshot::{assert_render_named, capture, capture_lines, serialize_buffer};
// `#[macro_export]` 的宏落在 crate 根，转一道手让组件测试从 test_support 统一取用。
pub use crate::{assert_lines_render, assert_render};

use crate::ui::state::ChatState;

impl ChatState {
    /// 快照测试用的确定性状态。
    ///
    /// `ChatState::new()` 会读机器上的真实数据，直接拿来当基线，同一份代码在
    /// 不同机器、不同环境下会渲染出不同的东西，基线就没法当回归护栏。这里把
    /// 每一条"读外部世界"的来源都按测试需要钉死。
    ///
    /// | 字段 | `new()` 里的来源 | 这里钉成 |
    /// |---|---|---|
    /// | `command_history` | `history_store::load_history()` 读 `~/.star/history.json` | 空 |
    /// | `transcript_enabled` | `STAR_TRANSCRIPT` 环境变量（且 `OnceLock` 记忆） | `false` |
    /// | `transcript_path` | `STAR_TRANSCRIPT_PATH` 环境变量 | `None` |
    /// | `transcript_run_id` | `Utc::now()` + 进程 id | `run-test` |
    /// | i18n 词典 | `<repo>/i18n/*.json` > `~/.starcode/i18n/` > `<project>/.star/i18n/` | 空（退回调用点写死的英文） |
    ///
    /// 主题不用钉：`ThemeManager::new()` 只压入 13 个硬编码预设，不读文件。
    ///
    /// ## 仍然不确定的：时钟
    ///
    /// `session_started_at` 是 `Instant::now()`，钉不到绝对值。渲染耗时的地方
    /// （状态栏的 uptime、转圈计时）因此**天生拍不出稳定基线**，测这些组件要么
    /// 先给时钟开个缝，要么退回断言具体字段而不是整帧。
    pub fn test_fixture() -> Self {
        crate::core::i18n::reset_for_tests();

        let mut state = Self::new();

        // new() 读的用户数据 —— 与机器/账号绑定，会让基线不可复现
        state.command_history.clear();
        state.transcript_enabled = false;
        state.transcript_path = None;
        state.transcript_run_id = "run-test".to_string();

        state
    }
}
