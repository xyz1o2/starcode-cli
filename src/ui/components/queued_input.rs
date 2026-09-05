//! 排队输入面板 —— 对标 Claude Code 的 `PromptInputQueuedCommands`。
//!
//! 参照 `study_or_copy_projects/claude-code-main/src/components/PromptInput/PromptInputQueuedCommands.tsx`
//! 与 `src/context/QueuedMessageContext.tsx`：agent 还在跑时用户继续敲的需求，
//! 不该从屏幕上凭空消失，而应该以「暗色的用户消息」原样停在输入框正上方
//! （参考实现是 `paddingX={2}` 的一块，排队消息按 `subtle` 上色），
//! 等本轮结束再依次发出。
//!
//! ```text
//!   > fix the failing test in agent_llm.rs
//!   > then update the docs
//!   2 queued · ↑ to edit
//! ```
//!
//! 数据来源单一：[`ChatState::pending_user_messages`] —— 也就是真正会被发出去的
//! 那个队列。屏幕上看到的就是待发的，不存在第二份「展示用」副本会和它走岔。

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::core::i18n;
use crate::ui::state::ChatState;
use crate::ui::utils::render::{truncate_to_display_width, wrap_text_to_width};

/// 队列正文最多占的行数：一次粘贴几十行需求时不能把输入框顶出屏幕。
/// 被截掉的部分由末行的 `…` 和页脚里的真实条数交代。
const MAX_BODY_ROWS: usize = 6;

/// 与聊天区里的用户消息保持一致的提示符，见 `message_render.rs` 的 `USER_PREFIX`。
const PREFIX: &str = "> ";

/// 左侧留白，对标参考实现的 `paddingX={2}`；也让排队消息和聊天区里
/// 顶格的已发消息一眼可分。
const LEFT_PAD: &str = "  ";

/// 面板高度；`0` 表示队列为空、不占位（和 task panel 一样按需撑开）。
pub fn queued_panel_height(state: &ChatState, width: u16) -> u16 {
    build_lines(state, width).len() as u16
}

/// 渲染面板全部行。调用方保证高度与 [`queued_panel_height`] 一致。
pub fn render_queued_panel(state: &ChatState, width: u16) -> Vec<Line<'static>> {
    build_lines(state, width)
}

fn build_lines(state: &ChatState, width: u16) -> Vec<Line<'static>> {
    let total = state.pending_user_messages.len();
    if total == 0 {
        return Vec::new();
    }

    let theme = state.theme_manager.current();
    // 暗色 = 「还没发出去」。已发出的用户消息用 user_fg，这里刻意拉开对比。
    let body_style = Style::default().fg(theme.subtle);
    let footer_style = Style::default().fg(theme.inactive);

    let indent = PREFIX.chars().count(); // ASCII，2 列
    let pad = LEFT_PAD.chars().count();
    let body_width = (width as usize).saturating_sub(pad + indent);

    let mut body: Vec<Line<'static>> = Vec::new();
    let mut truncated = false;
    for msg in state.pending_user_messages.iter() {
        let remaining = MAX_BODY_ROWS.saturating_sub(body.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        let rows = wrap_queued_message(msg, body_width);
        let take = rows.len().min(remaining);
        if take < rows.len() {
            truncated = true;
        }
        for (idx, row) in rows.into_iter().take(take).enumerate() {
            let lead = if idx == 0 {
                Span::styled(PREFIX, body_style)
            } else {
                Span::raw(" ".repeat(indent))
            };
            body.push(Line::from(vec![
                Span::raw(LEFT_PAD),
                lead,
                Span::styled(row, body_style),
            ]));
        }
        if truncated {
            break;
        }
    }

    let mut lines = body;
    if truncated {
        lines.push(Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled("…", footer_style),
        ]));
    }

    // 页脚给的是队列真实条数，所以正文被截断也不会误导。
    let footer = i18n::t(
        "ui.queued.footer",
        &format!("{} 条排队中 · ↑ 取回编辑", total),
        &format!("{} queued · ↑ to edit", total),
    );
    lines.push(Line::from(vec![
        Span::raw(LEFT_PAD),
        Span::styled(
            truncate_to_display_width(&footer, (width as usize).saturating_sub(pad)),
            footer_style,
        ),
    ]));

    lines
}

/// 把一条排队消息折成若干显示行；多行输入逐行折，空行保留（用户自己敲的分段）。
fn wrap_queued_message(msg: &str, body_width: usize) -> Vec<String> {
    if body_width == 0 {
        return vec![String::new()];
    }
    let clean = crate::ui::utils::text::sanitize_for_tui(msg);
    let mut rows: Vec<String> = Vec::new();
    for line in clean.lines() {
        if line.is_empty() {
            rows.push(String::new());
        } else {
            rows.extend(wrap_text_to_width(line, body_width));
        }
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(queued: &[&str]) -> ChatState {
        let mut state = ChatState::new();
        for msg in queued {
            state.pending_user_messages.push_back(msg.to_string());
        }
        state
    }

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn empty_queue_takes_no_space() {
        let state = state_with(&[]);
        assert_eq!(queued_panel_height(&state, 80), 0);
        assert!(render_queued_panel(&state, 80).is_empty());
    }

    #[test]
    fn queued_message_is_visible_with_user_prompt_prefix() {
        // 回归：排队的需求以前只在状态栏留一句 "⏳ 1 pending"，正文完全看不见
        let state = state_with(&["fix the failing test"]);
        let rows = plain(&render_queued_panel(&state, 80));
        assert_eq!(rows.len(), 2, "一条消息 + 一行页脚: {:?}", rows);
        assert_eq!(rows[0], "  > fix the failing test");
        assert!(rows[1].contains("1 queued"), "页脚缺少条数: {:?}", rows[1]);
    }

    #[test]
    fn every_queued_message_gets_its_own_prompt_row() {
        let state = state_with(&["first", "second", "third"]);
        let rows = plain(&render_queued_panel(&state, 80));
        assert_eq!(rows[0], "  > first");
        assert_eq!(rows[1], "  > second");
        assert_eq!(rows[2], "  > third");
        assert!(rows[3].contains("3 queued"));
    }

    #[test]
    fn multiline_message_keeps_continuation_indent() {
        let state = state_with(&["line one\nline two"]);
        let rows = plain(&render_queued_panel(&state, 80));
        assert_eq!(rows[0], "  > line one");
        assert_eq!(rows[1], "    line two");
    }

    #[test]
    fn body_is_capped_but_footer_reports_the_real_count() {
        let queued: Vec<String> = (0..12).map(|i| format!("req {}", i)).collect();
        let refs: Vec<&str> = queued.iter().map(|s| s.as_str()).collect();
        let state = state_with(&refs);
        let lines = render_queued_panel(&state, 80);
        // 正文 ≤ MAX_BODY_ROWS，加 "…" 和页脚各一行
        assert!(
            lines.len() <= MAX_BODY_ROWS + 2,
            "面板过高: {}",
            lines.len()
        );
        let rows = plain(&lines);
        assert_eq!(rows[MAX_BODY_ROWS].trim(), "…");
        assert!(rows.last().unwrap().contains("12 queued"));
    }

    #[test]
    fn height_matches_rendered_line_count() {
        // 高度是布局用的，和实际画出来的行数必须一致，否则输入框会被挤掉一行
        for queued in [
            vec!["one"],
            vec!["one", "two"],
            vec!["a very long requirement that will certainly need to wrap at this width"],
        ] {
            let state = state_with(&queued);
            for width in [24u16, 40, 80] {
                assert_eq!(
                    queued_panel_height(&state, width) as usize,
                    render_queued_panel(&state, width).len(),
                    "queued={:?} width={}",
                    queued,
                    width
                );
            }
        }
    }

    #[test]
    fn narrow_terminal_does_not_panic() {
        let state = state_with(&["something long enough to wrap"]);
        for width in 0u16..=6 {
            let _ = render_queued_panel(&state, width);
        }
    }
}
