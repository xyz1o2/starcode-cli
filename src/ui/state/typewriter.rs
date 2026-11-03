//! 打字机显示层（typewriter effect）。
//!
//! # 为什么它是一层“显示状态”，而不是改 `entry.content`
//!
//! 流式消息的 `entry.content` 是**数据**：它要进会话存档、要给复制/搜索/上下文
//! 统计用、要能整段重渲染。打字机只是“屏幕上这一刻先露出多少”的**表现**。把已
//! 显示的长度单独记在 `ChatState` 里，两者就不会互相污染：
//!
//! - 复制走 `rendered_cache` / `entry.content`，永远拿到全文；
//! - 会话恢复、`/compact`、steering 切分都看不到打字机状态；
//! - 关掉开关时（`STAR_TYPEWRITER=0`）行为与没有这一层完全一致。
//!
//! # 节奏
//!
//! 按**字符**推进，不按字节 —— CJK、emoji 不会被切在 UTF-8 中间。每帧的推进量
//! 由经过时间 × 速率算出，并带一个分数余量（`credit`），所以 30fps 下也是平滑的，
//! 不会因为取整变成“一帧一个字”。
//!
//! 流式输出本身可能比打字机快得多。积压（backlog）越大，揭示速率越高，超过阈值
//! 直接按倍数追赶 —— 打字机永远只是“把已经收到的内容铺开”，它不是限速器，不能
//! 让模型已经在下一段了屏幕上还卡在上上段。
//!
//! 条目不再流式时（`is_streaming != Some(true)`）立刻丢弃对应进度，渲染回到全文。

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Instant;

use crate::ui::state::ChatState;

/// 默认揭示速率（字符/秒）。约 2 行/秒，肉眼可读又不至于让人等。
const DEFAULT_CPS: f32 = 90.0;
/// 积压超过这个字符数开始加速。
const BACKLOG_FAST: usize = 200;
/// 积压超过这个字符数全速追赶。
const BACKLOG_SPRINT: usize = 600;

/// 一条流式消息的显示进度。
#[derive(Debug, Default)]
pub struct TypewriterState {
    /// 开关（`STAR_TYPEWRITER=0` 关闭）。
    pub enabled: bool,
    /// 揭示速率（字符/秒）。
    pub cps: f32,
    /// 每个条目已经揭示的**字符**数。
    shown: HashMap<usize, usize>,
    /// 未满一个字符的推进余量，保证低帧率下也按时间公平推进。
    credit: f32,
    /// 上一次推进时刻；`None` 表示刚开始或在空闲。
    last_tick: Option<Instant>,
}

impl TypewriterState {
    pub fn new() -> Self {
        let enabled = !matches!(
            std::env::var("STAR_TYPEWRITER").as_deref(),
            Ok("0") | Ok("off") | Ok("false")
        );
        let cps = std::env::var("STAR_TYPEWRITER_CPS")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(DEFAULT_CPS);
        Self {
            enabled,
            cps,
            shown: HashMap::new(),
            credit: 0.0,
            last_tick: None,
        }
    }

    /// 已揭示的字符数（无进度时返回 `None`）。
    pub fn shown(&self, idx: usize) -> Option<usize> {
        self.shown.get(&idx).copied()
    }

    /// 这一条当前应显示的文本前缀。
    ///
    /// 没有进度、或打字机关闭时原样返回 `content`（借用，不分配）。
    pub fn preview<'a>(&self, idx: usize, content: &'a str) -> Cow<'a, str> {
        if !self.enabled {
            return Cow::Borrowed(content);
        }
        let Some(&shown) = self.shown.get(&idx) else {
            return Cow::Borrowed(content);
        };
        match content.char_indices().nth(shown) {
            Some((byte, _)) => Cow::Borrowed(&content[..byte]),
            None => Cow::Borrowed(content),
        }
    }

    /// 丢弃一个条目的进度：它的下一帧按全文渲染。
    pub fn flush(&mut self, idx: usize) {
        self.shown.remove(&idx);
    }

    /// 清空所有进度（`/clear`、会话切换、重置渲染缓存时调用）。
    pub fn reset(&mut self) {
        self.shown.clear();
        self.credit = 0.0;
        self.last_tick = None;
    }

    /// 推进一帧。返回 `true` 表示有内容被揭示，调用方需要重绘。
    ///
    /// 只关心**仍在流式**的条目：它们的内容长度是目标，`shown` 是当前进度。
    pub fn advance(&mut self, state: &ChatState) -> bool {
        if !self.enabled {
            return false;
        }

        let now = Instant::now();
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0);
        self.last_tick = Some(now);

        // 条目集合先收窄：已结束的流把所有进度丢掉，屏幕上直接是全文。
        let streaming: Vec<(usize, usize)> = state
            .chat_history
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_streaming == Some(true))
            .map(|(idx, e)| (idx, e.content.chars().count()))
            .collect();

        self.shown.retain(|idx, _| streaming.iter().any(|(i, _)| i == idx));

        if streaming.is_empty() {
            self.credit = 0.0;
            return false;
        }

        let mut progressed = false;
        for (idx, total) in streaming {
            let shown = self.shown.entry(idx).or_insert(0);
            if *shown >= total {
                continue;
            }
            let backlog = total - *shown;
            let factor = if backlog > BACKLOG_SPRINT {
                6.0
            } else if backlog > BACKLOG_FAST {
                2.0
            } else {
                1.0
            };

            // 每帧至少推进 1 个字符：dt 为 0（同一毫秒内的连续 tick）时不能原地不动。
            self.credit += self.cps * factor * dt;
            let step = self.credit.floor().max(1.0) as usize;
            self.credit -= (step.max(1) - 1) as f32;
            *shown = (*shown + step).min(total);
            progressed = true;
        }

        // 有进度才保留余量，避免空闲帧不断累积成一次“瞬移”。
        if !progressed {
            self.credit = 0.0;
        }
        progressed
    }
}

impl Default for TypewriterState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatEntry;

    fn state_with_streaming(content: &str) -> ChatState {
        let mut state = ChatState::test_fixture();
        state
            .chat_history
            .push(ChatEntry::assistant(content.to_string()).with_streaming(true));
        state
    }

    #[test]
    fn first_tick_reveals_only_a_prefix() {
        let state = state_with_streaming("hello world, this is a long streamed answer");
        let mut tw = TypewriterState::new();
        tw.cps = 10.0;
        assert!(tw.advance(&state));
        let idx = state.chat_history.len() - 1;
        let shown = tw.shown(idx).expect("进步应被记录");
        assert!(shown > 0 && shown < state.chat_history[idx].content.chars().count());
        assert!(tw.preview(idx, &state.chat_history[idx].content).len() < 32);
    }

    #[test]
    fn preview_never_splits_a_utf8_char() {
        let content = "中文流式输出，混排 emoji 🙂 也要能正确截断";
        let state = state_with_streaming(content);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 3.0;
        // 反复推进，每一步的预览都必须是合法 UTF-8 前缀
        for _ in 0..40 {
            tw.advance(&state);
            let preview = tw.preview(idx, content).into_owned();
            assert!(content.starts_with(&preview), "预览必须是原文前缀: {preview:?}");
        }
    }

    #[test]
    fn preview_returns_full_text_after_stream_ends() {
        let mut state = state_with_streaming("short");
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.advance(&state);
        state.chat_history[idx].is_streaming = Some(false);
        tw.advance(&state);
        assert!(tw.shown(idx).is_none(), "流结束后进度必须丢弃");
        assert_eq!(tw.preview(idx, "short"), Cow::Borrowed("short"));
    }

    #[test]
    fn disabled_typewriter_is_a_no_op() {
        let state = state_with_streaming("abc");
        let mut tw = TypewriterState::new();
        tw.enabled = false;
        assert!(!tw.advance(&state));
        assert_eq!(tw.preview(1, "abc"), Cow::Borrowed("abc"));
    }

    #[test]
    fn large_backlog_is_caught_up_faster() {
        let long = "x".repeat(4000);
        let state = state_with_streaming(&long);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 30.0;
        tw.advance(&state);
        let first = tw.shown(idx).unwrap();
        // 900ms 后，积压 4000 字符走 sprint 档位
        tw.last_tick = Some(Instant::now() - std::time::Duration::from_millis(900));
        tw.advance(&state);
        let second = tw.shown(idx).unwrap();
        assert!(
            second - first > 100,
            "大积压必须加速追赶，实际只推进了 {}",
            second - first
        );
    }
}
