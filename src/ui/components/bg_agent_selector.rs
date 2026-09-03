//! 后台代理选择器 —— 对标 Claude Code 的 background agent selector。
//!
//! 参照 `study_or_copy_projects/claude-code-main/docs/features/background-agent-selector.md`：
//! 输入框下方一块可上下移动的列表，第一行是主会话，其余每行一个后台代理，
//! `Enter` 在「看主会话」与「看某个代理的输出」之间切换。
//!
//! ```text
//!   ○ main                                 ↑/↓ select · Enter view
//!   ● Explore  Research src/hooks           23s · ↓ 10.9k tokens
//!   ○ Explore  Research src/components      22s · ↓  9.5k tokens
//! ```
//!
//! 数据来源单一：[`ChatState::background_agent_rows`]。本模块只负责把它画出来，
//! 不持有任何状态。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::types::AgentTaskStatus;
use crate::ui::state::ChatState;
use crate::ui::utils::render::truncate_to_display_width;

/// main 行 + 每个后台代理各一行；`bg_agent_selection` 为 `None`（焦点在输入框）时不显示。
pub fn selector_height(state: &ChatState) -> u16 {
    if state.bg_agent_selection.is_none() {
        return 0;
    }
    let rows = state.background_agent_rows().len();
    if rows == 0 {
        return 0;
    }
    (rows + 1) as u16
}

/// 渲染选择器的全部行。调用方保证高度与 [`selector_height`] 一致。
pub fn render_selector(state: &ChatState, area_width: u16) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
    let selected = state.bg_agent_selection;
    let rows = state.background_agent_rows();

    let mut lines = Vec::with_capacity(rows.len() + 1);

    // ── main 行（索引 0）──
    // 「当前在看主会话」= 没有 viewing_agent_task_id；圆点反映的是这个，而不是光标位置。
    let main_focused = selected == Some(0);
    let main_active = state.viewing_agent_task_id.is_none();
    let main_dot = if main_active {
        theme.success
    } else {
        theme.inactive
    };
    let main_fg = if main_focused {
        theme.foreground
    } else {
        theme.subtle
    };
    let main_mod = if main_focused {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let mut main_spans = vec![
        Span::styled(
            if main_focused { "❯ " } else { "  " },
            Style::default().fg(theme.suggestion),
        ),
        Span::styled(
            if main_active { "● " } else { "○ " },
            Style::default().fg(main_dot),
        ),
        Span::styled(
            "main".to_string(),
            Style::default().fg(main_fg).add_modifier(main_mod),
        ),
    ];
    // 操作提示只挂在 main 行右侧，避免每行都重复一遍
    let hint = "↑/↓ select · Enter view · Esc close";
    let used = 4 + 4 + "main".len();
    if (area_width as usize) > used + hint.len() + 2 {
        let pad = area_width as usize - used - hint.len() - 1;
        main_spans.push(Span::styled(" ".repeat(pad), Style::default()));
        main_spans.push(Span::styled(hint, Style::default().fg(theme.inactive)));
    }
    lines.push(Line::from(main_spans));

    // ── 各后台代理行 ──
    for (i, info) in rows.iter().enumerate() {
        let idx = i + 1;
        let focused = selected == Some(idx);
        let viewing = state.viewing_agent_task_id.as_deref() == Some(info.task_id.as_str());

        // 右侧统计先算出来，剩下的宽度才给描述
        let stats = format!(
            "{} · ↓ {}",
            super::agent_group_render::format_duration(info.elapsed()),
            format_tokens(info.tokens),
        );
        let label = info.name.as_deref().unwrap_or(info.agent_type.as_str());
        let desc = info
            .task_description
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(info.description.as_str());

        // 2(❯) + 2(●) + label + 2 空格 + desc + 空隙 + stats
        let fixed = 4 + label.chars().count() + 2 + stats.chars().count() + 2;
        let desc_width = (area_width as usize).saturating_sub(fixed);
        let desc_shown = truncate_to_display_width(desc, desc_width);

        let dot_color = match info.status {
            AgentTaskStatus::Running => theme.warning,
            AgentTaskStatus::Completed => theme.success,
            AgentTaskStatus::Failed | AgentTaskStatus::Rejected => theme.error,
            AgentTaskStatus::Background => theme.info,
        };

        let row_fg = if focused {
            theme.foreground
        } else {
            theme.subtle
        };
        let row_mod = if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        let mut spans = vec![
            Span::styled(
                if focused { "❯ " } else { "  " },
                Style::default().fg(theme.suggestion),
            ),
            Span::styled(
                if viewing { "● " } else { "○ " },
                Style::default().fg(dot_color),
            ),
            Span::styled(
                label.to_string(),
                Style::default().fg(row_fg).add_modifier(row_mod),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(desc_shown.clone(), Style::default().fg(theme.inactive)),
        ];

        // 右对齐统计
        let left_width = 4 + label.chars().count() + 2 + desc_shown.chars().count();
        if (area_width as usize) > left_width + stats.chars().count() + 1 {
            let pad = area_width as usize - left_width - stats.chars().count() - 1;
            spans.push(Span::styled(" ".repeat(pad), Style::default()));
        } else {
            spans.push(Span::styled(" ", Style::default()));
        }
        spans.push(Span::styled(stats, Style::default().fg(theme.inactive)));

        lines.push(Line::from(spans));
    }

    lines
}

/// `10.9k` / `1.2M` 风格的 token 计数（与 agent_task_render 一致）
fn format_tokens(count: u32) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        format!("{} tokens", count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::store::AgentTaskInfo;

    fn bg_task(id: &str, desc: &str) -> AgentTaskInfo {
        AgentTaskInfo {
            task_id: id.to_string(),
            agent_type: "Explore".to_string(),
            description: desc.to_string(),
            status: AgentTaskStatus::Running,
            tool_use_count: 4,
            tokens: 10_900,
            is_async: true,
            is_resolved: true,
            is_error: false,
            last_tool_info: None,
            name: None,
            task_description: None,
            started_at: std::time::Instant::now(),
            finished_at: None,
            sub_entries: Vec::new(),
            entry_idx: 0,
        }
    }

    fn state_with(n: usize, desc: &str) -> ChatState {
        let mut state = ChatState::new();
        for i in 0..n {
            let id = format!("t{i}");
            state
                .active_agent_tasks
                .insert(id.clone(), bg_task(&id, desc));
        }
        state
    }

    /// 焦点不在选择器上、或者没有后台代理时高度必须是 0（面板整块隐藏）
    #[test]
    fn height_is_zero_unless_focused_with_rows() {
        let mut state = state_with(2, "research");
        assert_eq!(selector_height(&state), 0, "focus in input → hidden");

        state.bg_agent_selection = Some(0);
        assert_eq!(selector_height(&state), 3, "main 行 + 2 个代理");

        state.active_agent_tasks.clear();
        assert_eq!(selector_height(&state), 0, "no rows → hidden");
    }

    /// 高度与实际渲染行数必须一致，否则面板会被裁掉或留空行
    #[test]
    fn height_matches_rendered_line_count() {
        for n in 1..=3 {
            let mut state = state_with(n, "research src/hooks");
            state.bg_agent_selection = Some(0);
            assert_eq!(
                selector_height(&state) as usize,
                render_selector(&state, 80).len(),
                "n = {n}"
            );
        }
    }

    /// 回归：中文描述 + 任意窄宽度都不能 panic（字节切片实现在这里必崩）
    #[test]
    fn render_survives_cjk_description_at_every_width() {
        let mut state = state_with(2, "研究 src/hooks 目录下的组件实现 🎉");
        state.bg_agent_selection = Some(1);
        state.viewing_agent_task_id = Some("t0".to_string());
        for width in 0..=120u16 {
            let lines = render_selector(&state, width);
            assert_eq!(lines.len(), 3, "width = {width}");
        }
    }
}
