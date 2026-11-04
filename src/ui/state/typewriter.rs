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
//! 不会因为取整变成"一帧一个字"。
//!
//! # 三条不许"块状输出"的约束
//!
//! 这个模块存在的唯一理由就是**别让正文一块一块地蹦出来**。三个地方都能让
//! 它蹦，加了三条约束把它们都按住：
//!
//! 1. **帧间隔有上限**（[`MAX_FRAME_DT`]）。`last_tick` 记的是**上一次推进**的时刻，
//!    而推进只在动画帧里发生 —— 思考阶段（没有流式输出）不推进，`last_tick`
//!    就停在几秒前。流式一开始，第一个 `dt` 是好几秒，`credit` 一次性攒够几千
//!    字符 → 整段话一帧全出来。削顶之后，"停了很久"也只多揭示 100ms 的量。
//! 2. **单帧步长有上限**（[`MAX_CHARS_PER_FRAME`]）。积压大 + 追赶倍率 6 时，
//!    `cps × factor × dt` 仍能算出很大的值。这个上限保证**任何一帧**的视觉增量
//!    都是有界的。
//! 3. **收尾不丢弃进度**。流结束时不 `flush`，而是让打字机继续把剩下的字
//!    "落针"（settle）完 —— 落针过程同样受上面两条约束。
//!
//! 另外还有一条容易漏的：**条目索引会位移**。工具结果插到 `chat_history`
//! 中间时，后面那条正在流式的消息索引 +1；打字机进度按索引存，不跟着移的话
//! `preview()` 查不到 → 返回全文 → 又是一帧蹦出一大段。见
//! [`TypewriterState::shift_indices_after_insert`]。

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Instant;

use crate::types::ChatEntry;

/// 默认揭示速率（字符/秒）。约 2 行/秒，肉眼可读又不至于让人等。
const DEFAULT_CPS: f32 = 90.0;
/// 积压超过这个字符数开始加速。
const BACKLOG_FAST: usize = 200;
/// 积压超过这个字符数全速追赶。
const BACKLOG_SPRINT: usize = 600;

/// 单帧最多揭示"这么多时间"的内容。
///
/// 帧间隔再大也不能一次蹦出一大段 —— 这是"块状输出"的头号成因。
/// 上限 100ms ≈ 6 帧 @30fps：正常帧间隔都远小于它，只有"停了很久"才被削顶。
const MAX_FRAME_DT: f32 = 0.1;

/// 单帧最多揭示的字符数（积压追赶时的硬上限）。
///
/// 保证**任何一帧**的视觉增量都是有界的 —— 也就是"不闪屏"。
const MAX_CHARS_PER_FRAME: usize = 48;

/// 一条流式消息的显示进度。
#[derive(Debug)]
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

    /// 在 `pos` 处插入条目后，把进度索引整体后移。
    ///
    /// 工具结果插在工具调用后面（`handle_tool_result_message` 走的就是这条路），
    /// 后面那条正在流式的消息索引 +1。进度不跟着移的话，`preview()` 查不到这条
    /// 的进度 → **返回全文** → 整段话一帧蹦出来 —— 就是"块状输出"。
    pub fn shift_indices_after_insert(&mut self, pos: usize) {
        self.shown = self
            .shown
            .iter()
            .map(|(idx, shown)| (if *idx >= pos { idx + 1 } else { *idx }, *shown))
            .collect();
    }

    /// 移除 `pos` 处的条目后，把进度索引整体前移。
    pub fn shift_indices_after_removal(&mut self, pos: usize) {
        self.shown = self
            .shown
            .drain()
            .filter_map(|(idx, shown)| match idx.cmp(&pos) {
                std::cmp::Ordering::Less => Some((idx, shown)),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some((idx - 1, shown)),
            })
            .collect();
    }

    /// 还有没有"没铺完"的正文。
    ///
    /// 有的话事件循环就得继续跑动画帧 —— 否则收尾阶段的"落针"没人推进，
    /// 屏幕上会永远少一截。
    pub fn has_pending_reveal(&self, chat_history: &[ChatEntry]) -> bool {
        self.shown.iter().any(|(idx, shown)| {
            chat_history
                .get(*idx)
                .is_some_and(|entry| *shown < entry.content.chars().count())
        })
    }

    /// 清空所有进度（`/clear`、会话切换、重置渲染缓存时调用）。
    pub fn reset(&mut self) {
        self.shown.clear();
        self.credit = 0.0;
        self.last_tick = None;
    }

    /// 推进一帧。返回 `true` 表示有内容被揭示，调用方需要重绘。
    ///
    /// 推进两类条目：
    /// 1. **还在流式的** —— 目标是它的全文长度；
    /// 2. **已经结束、但还没铺完的** —— 让它"落针"。
    ///    流结束时不再直接丢弃进度（那会让剩下的字整块弹出），
    ///    而是继续按同样的速率铺完，铺完才丢掉进度记录。
    pub fn advance(&mut self, chat_history: &[ChatEntry]) -> bool {
        if !self.enabled {
            return false;
        }

        let now = Instant::now();
        // 帧间隔削顶：停了很久之后第一次推进，也只多揭示 100ms 的量。
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(0.0)
            .min(MAX_FRAME_DT);
        self.last_tick = Some(now);

        // 目标集合：还在流式的 + 已经结束但还没铺完的。
        let mut targets: Vec<(usize, usize)> = Vec::new();
        for (idx, entry) in chat_history.iter().enumerate() {
            let total = entry.content.chars().count();
            if entry.is_streaming == Some(true) {
                targets.push((idx, total));
            } else if let Some(&shown) = self.shown.get(&idx) {
                if shown < total {
                    targets.push((idx, total));
                }
            }
        }

        // 已经铺完、且不再流式的条目：进度可以丢掉（预览本来就是全文）。
        self.shown
            .retain(|idx, shown| match chat_history.get(*idx) {
                Some(entry) => {
                    entry.is_streaming == Some(true) || *shown < entry.content.chars().count()
                }
                None => false,
            });

        if targets.is_empty() {
            self.credit = 0.0;
            return false;
        }

        let mut progressed = false;
        for (idx, total) in targets {
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

            self.credit += self.cps * factor * dt;
            // 单帧步长有上限：任何一帧的视觉增量都是有界的。
            let step = (self.credit.floor() as usize).clamp(1, MAX_CHARS_PER_FRAME);
            self.credit = (self.credit - step as f32).max(0.0);
            *shown = (*shown + step).min(total);
            progressed = true;
        }

        // 有进度才保留余量，避免空闲帧不断累积成一次"瞬移"。
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
    use crate::ui::state::ChatState;

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
        assert!(tw.advance(&state.chat_history));
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
            tw.advance(&state.chat_history);
            let preview = tw.preview(idx, content).into_owned();
            assert!(
                content.starts_with(&preview),
                "预览必须是原文前缀: {preview:?}"
            );
        }
    }

    /// 回归：流结束后**不**立刻丢弃进度 —— 剩下的字要按速率"落针"。
    ///
    /// 以前流一结束就 `flush`，剩余文字整块弹出（屏幕上最后一段"啪"一下出现）。
    /// 现在进度保留，继续按同样的速率铺完；铺完之后预览自然回到全文。
    #[test]
    fn preview_returns_full_text_after_stream_ends() {
        let mut state = state_with_streaming("short");
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.advance(&state.chat_history);
        state.chat_history[idx].is_streaming = Some(false);

        // 落针阶段：预览仍是前缀，不是全文。
        tw.advance(&state.chat_history);
        let preview = tw.preview(idx, "short").into_owned();
        assert!(
            "short".starts_with(&preview),
            "落针阶段预览必须是前缀：{preview:?}"
        );

        // 铺完之后回到全文。
        let mut tw2 = TypewriterState::new();
        tw2.cps = 1000.0;
        for _ in 0..5 {
            tw2.advance(&state.chat_history);
        }
        assert_eq!(
            tw2.preview(idx, "short"),
            Cow::Borrowed(&"short".to_string())
        );
    }

    /// 大积压要加速追赶，但**单帧步长有上限**。
    ///
    /// 上限是"不闪屏"的硬保证：任何一帧的视觉增量都是有界的。
    #[test]
    fn large_backlog_is_caught_up_faster() {
        let long = "x".repeat(4000);
        let state = state_with_streaming(&long);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 30.0;
        tw.advance(&state.chat_history);
        let first = tw.shown(idx).unwrap();
        // 一个正常帧（30fps ≈ 33ms）
        tw.last_tick = Some(Instant::now() - std::time::Duration::from_millis(33));
        tw.advance(&state.chat_history);
        let second = tw.shown(idx).unwrap();
        let advanced = second - first;
        assert!(advanced > 1, "大积压必须加速追赶，实际只推进了 {advanced}");
        assert!(
            advanced <= MAX_CHARS_PER_FRAME,
            "单帧步长必须有上限（否则又是一次'一块一块'）：{advanced}"
        );
    }

    #[test]
    fn disabled_typewriter_is_a_no_op() {
        let state = state_with_streaming("abc");
        let mut tw = TypewriterState::new();
        tw.enabled = false;
        assert!(!tw.advance(&state.chat_history));
        assert_eq!(tw.preview(1, "abc"), Cow::Borrowed("abc"));
    }

    /// 大积压要加速追赶，但**单帧步长有上限**。
    /// 回归：帧间隔再大，单帧也不能蹦出一大段。
    ///
    /// 打字机只在动画帧里推进，`last_tick` 记的是**上一次推进**的时刻。
    /// 思考阶段（没有流式输出）不推进 → `last_tick` 停在几秒前 →
    /// 流式一开始第一个 `dt` 是好几秒 → 整段话一帧全出来。
    #[test]
    fn a_long_gap_does_not_release_a_big_block_at_once() {
        let long = "x".repeat(4000);
        let state = state_with_streaming(&long);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 90.0;
        // 模拟"停了很久才收到第一块内容"：`last_tick` 是 10 秒前。
        tw.last_tick = Some(Instant::now() - std::time::Duration::from_secs(10));
        tw.advance(&state.chat_history);
        let shown = tw.shown(idx).expect("应有进度");
        assert!(
            shown <= MAX_CHARS_PER_FRAME,
            "单帧揭示 {shown} 个字符，超出上限 {MAX_CHARS_PER_FRAME} —— 这就是'一块一块'"
        );
    }

    /// 回归：收尾时不该把剩下的字整块弹出来。
    ///
    /// 流结束 → 进度进入"落针"阶段，按同样的速率铺完；铺完之前预览仍是前缀。
    #[test]
    fn the_tail_settles_instead_of_popping_in() {
        let content = "y".repeat(500);
        let mut state = state_with_streaming(&content);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 90.0;
        tw.advance(&state.chat_history);
        let before = tw.shown(idx).unwrap();

        // 流结束（不再 streaming），但进度还在。
        state.chat_history[idx].is_streaming = Some(false);
        tw.advance(&state.chat_history);
        let after = tw.shown(idx).expect("落针阶段应保留进度");
        assert!(after > before, "落针阶段应继续推进：{before} -> {after}");
        assert!(after < 500, "不能一次全铺完：{after}");
        let preview = tw.preview(idx, &content).into_owned();
        assert_eq!(
            preview.chars().count(),
            after,
            "预览长度应等于进度（否则尾部会整块跳出）"
        );
    }

    /// 回归：进度落完后自动回到全文（不能永远截着一截）。
    #[test]
    fn a_settled_entry_returns_the_full_text() {
        let content = "z".repeat(30);
        let mut state = state_with_streaming(&content);
        let idx = state.chat_history.len() - 1;
        let mut tw = TypewriterState::new();
        tw.cps = 1000.0;
        state.chat_history[idx].is_streaming = Some(false);
        for _ in 0..10 {
            tw.advance(&state.chat_history);
        }
        assert_eq!(tw.preview(idx, &content), Cow::Borrowed(&content));
    }

    /// 端到端：模拟"流式回答 + 中间插入工具结果"，记录每帧揭示的字符数。
    ///
    /// 帧间增量有界 = 不闪屏。旧实现里，工具结果插入后进度索引没跟着位移，
    /// 下一帧直接返回全文 —— 这一帧的增量就是整段的长度。
    #[test]
    fn reveal_steps_stay_bounded_across_a_tool_call() {
        let mut state = ChatState::test_fixture();
        state.chat_history.clear();

        // 索引 0：工具调用；索引 1：正在流式的助手消息
        // （真实布局就是这样：工具结果插在工具调用后面，流式消息因此往后挪）
        state.chat_history.push(ChatEntry::tool_call(
            "call-1",
            crate::types::StarToolCall {
                id: "call-1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                },
            },
        ));
        state.chat_history.push(
            ChatEntry::assistant("第一段回答，解释这个项目怎么组织的。".to_string())
                .with_streaming(true),
        );

        let mut tw = TypewriterState::new();
        tw.cps = 90.0;

        let mut steps: Vec<usize> = Vec::new();
        let mut last_shown = 0usize;

        // 阶段 1：流式内容一点点到达（每帧 33ms ≈ 30fps）
        for _ in 0..12 {
            state.chat_history[1]
                .content
                .push_str("又来了一些内容，继续写。");
            tw.last_tick = Some(Instant::now() - std::time::Duration::from_millis(33));
            tw.advance(&state.chat_history);
            let shown = tw.shown(1).unwrap_or(0);
            steps.push(shown.saturating_sub(last_shown));
            last_shown = shown;
        }

        // 阶段 2：工具结果插到索引 1 → 流式消息索引变成 2
        state.chat_history.insert(
            1,
            ChatEntry::tool_result(
                "out",
                crate::types::StarToolCall {
                    id: "call-1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: "Read".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                crate::types::ToolResult {
                    success: true,
                    output: Some("out".to_string()),
                    error: None,
                    data: None,
                    llm_content: None,
                },
            ),
        );
        tw.shift_indices_after_insert(1);

        // 阶段 3：继续流式（索引已变成 2）
        for _ in 0..12 {
            state.chat_history[2].content.push_str("工具之后的正文。");
            tw.last_tick = Some(Instant::now() - std::time::Duration::from_millis(33));
            tw.advance(&state.chat_history);
            let shown = tw.shown(2).unwrap_or(0);
            steps.push(shown.saturating_sub(last_shown));
            last_shown = shown;
        }

        let max_step = steps.iter().copied().max().unwrap_or(0);
        println!("每帧揭示字符数: {steps:?}");
        assert!(
            max_step <= MAX_CHARS_PER_FRAME,
            "单帧最多揭示 {MAX_CHARS_PER_FRAME} 字，实际 {max_step} —— 这就是'一块一块'"
        );
        assert!(
            last_shown > 0,
            "插入工具结果后进度必须还在（否则整段会一帧蹦出）"
        );
    }
}
