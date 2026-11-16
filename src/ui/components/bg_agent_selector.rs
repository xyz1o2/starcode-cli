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

/// 有后台代理时常驻显示 main 行 + 每个代理一行。
///
/// `bg_agent_selection` 只表示键盘焦点，不决定是否渲染：焦点在输入框时
/// 仍然必须让用户看见 Explore 的实时状态；否则选择器高度为 0，整个窗口
/// 会被布局直接裁掉，用户只能在底部任务计数里看到一个模糊的 "tasks"。
pub fn selector_height(state: &ChatState) -> u16 {
    let rows = state.background_agent_rows().len();
    if rows == 0 {
        return 0;
    }
    (rows + 1) as u16
}

/// Todo 面板优先占位；终端高度不足时压缩后台代理选择器，不挤掉输入区。
pub fn selector_height_with_limit(state: &ChatState, max_height: u16) -> u16 {
    selector_height(state).min(max_height)
}

/// 渲染选择器的全部行。调用方保证高度与 [`selector_height`] 一致。
pub fn render_selector(state: &ChatState, area_width: u16) -> Vec<Line<'static>> {
    render_selector_with_limit(state, area_width, u16::MAX)
}

/// 绘制和鼠标命中共用窗口起点，焦点离开后仍保持正在查看的代理可见。
pub(crate) fn selector_window_start(state: &ChatState, max_height: u16) -> usize {
    let rows = state.background_agent_rows();
    let anchor = state.bg_agent_selection.or_else(|| {
        state.viewing_agent_task_id.as_ref().and_then(|id| {
            rows.iter()
                .position(|info| &info.task_id == id)
                .map(|index| index + 1)
        })
    });
    anchor
        .unwrap_or(0)
        .saturating_sub(max_height.saturating_sub(1) as usize)
}

/// 渲染受高度限制的选择器，固定 main 行并跟随当前焦点。
pub fn render_selector_with_limit(
    state: &ChatState,
    area_width: u16,
    max_height: u16,
) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
    let selected = state.bg_agent_selection;
    let all_rows = state.background_agent_rows();
    if max_height == 0 {
        return Vec::new();
    }
    let visible_count = max_height.saturating_sub(1) as usize;
    let start = selector_window_start(state, max_height);
    let rows = all_rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_count)
        .collect::<Vec<_>>();

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
    let main_fg = if main_focused || main_active {
        theme.foreground
    } else {
        theme.subtle
    };
    let main_mod = if main_focused || main_active {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };
    let viewing_label = crate::core::i18n::t1("ui.agent.viewing");
    let main_label = if main_active {
        format!("main [{viewing_label}]")
    } else {
        "main".to_string()
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
            main_label.clone(),
            Style::default().fg(main_fg).add_modifier(main_mod),
        ),
    ];
    // 操作提示只挂在 main 行右侧，避免每行都重复一遍（文案对齐参考实现文档）
    let hint = "↑/↓ to select · Enter to view";
    let used = crate::ui::utils::render::display_width("❯ ")
        + crate::ui::utils::render::display_width(if main_active { "● " } else { "○ " })
        + crate::ui::utils::render::display_width(&main_label);
    let hint_width = crate::ui::utils::render::display_width(hint);
    if (area_width as usize) > used + hint_width + 2 {
        let pad = area_width as usize - used - hint_width - 1;
        main_spans.push(Span::styled(" ".repeat(pad), Style::default()));
        main_spans.push(Span::styled(hint, Style::default().fg(theme.inactive)));
    }
    lines.push(Line::from(main_spans));

    // ── 各后台代理行 ──
    for (row_index, info) in rows {
        let idx = row_index + 1;
        let focused = selected == Some(idx);
        let viewing = state.viewing_agent_task_id.as_deref() == Some(info.task_id.as_str());

        // 右侧统计先算出来，剩下的宽度才给描述
        let stats = format!(
            "{} · ↓ {}",
            super::agent_group_render::format_duration(info.elapsed()),
            format_tokens(info.tokens),
        );
        let name = info.name.as_deref().unwrap_or(info.agent_type.as_str());
        let label = if viewing {
            format!("{name} [{viewing_label}]")
        } else {
            name.to_string()
        };
        let desc = info
            .task_description
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(info.description.as_str());

        // 状态标记必须反映任务状态，而不是当前查看对象：之前用
        // `viewing_agent_task_id` 决定 ●/○，导致所有未打开详情的运行中
        // Explore 都显示成空心圆，看起来像没启动。查看状态改用行高亮表达。
        let (status_mark, status_color) = match info.status {
            AgentTaskStatus::Running => ("●", theme.success),
            AgentTaskStatus::Completed => ("✓", theme.success),
            AgentTaskStatus::Failed | AgentTaskStatus::Rejected => ("×", theme.error),
            AgentTaskStatus::Background => ("·", theme.info),
        };

        // 2(❯) + 2(●) + label + 2 空格 + desc + 空隙 + stats。
        // 所有宽度都按终端列数计算，不能用 chars().count() 处理中文/emoji。
        let fixed = crate::ui::utils::render::display_width("❯ ")
            + crate::ui::utils::render::display_width(&format!("{} ", status_mark))
            + crate::ui::utils::render::display_width(&label)
            + crate::ui::utils::render::display_width("  ")
            + crate::ui::utils::render::display_width(&stats)
            + 2;
        let desc_width = (area_width as usize).saturating_sub(fixed);
        let desc_shown = truncate_to_display_width(desc, desc_width);

        let row_fg = if focused || viewing {
            theme.foreground
        } else {
            theme.subtle
        };
        let mut row_mod = if focused || viewing {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        if viewing && !focused {
            row_mod |= Modifier::UNDERLINED;
        }
        let mut spans = vec![
            Span::styled(
                if focused { "❯ " } else { "  " },
                Style::default().fg(theme.suggestion),
            ),
            Span::styled(
                format!("{} ", status_mark),
                Style::default().fg(status_color),
            ),
            Span::styled(
                label.to_string(),
                Style::default().fg(row_fg).add_modifier(row_mod),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(desc_shown.clone(), Style::default().fg(theme.inactive)),
        ];

        // 右对齐统计。状态图标的宽度按实际字符计算，不能假定所有图标都相同。
        let status_prefix = format!("{} ", status_mark);
        let left_width = crate::ui::utils::render::display_width("❯ ")
            + crate::ui::utils::render::display_width(&status_prefix)
            + crate::ui::utils::render::display_width(&label)
            + crate::ui::utils::render::display_width("  ")
            + crate::ui::utils::render::display_width(&desc_shown);
        let stats_width = crate::ui::utils::render::display_width(&stats);
        if (area_width as usize) > left_width + stats_width + 1 {
            let pad = area_width as usize - left_width - stats_width - 1;
            spans.push(Span::styled(" ".repeat(pad), Style::default()));
        } else {
            spans.push(Span::styled(" ", Style::default()));
        }
        spans.push(Span::styled(stats, Style::default().fg(theme.inactive)));

        lines.push(Line::from(spans));
    }

    lines
}

/// `10.9k tokens` / `1.2M tokens` 风格的 token 计数（与 agent_task_render 一致）
fn format_tokens(count: u32) -> String {
    let scaled = if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    };
    format!("{} tokens", scaled)
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
        let mut state = ChatState::test_fixture();
        for i in 0..n {
            let id = format!("t{i}");
            state
                .active_agent_tasks
                .insert(id.clone(), bg_task(&id, desc));
        }
        state
    }

    /// 有后台代理时即使焦点在输入框也必须显示；没有代理才隐藏。
    #[test]
    fn selector_stays_visible_while_input_has_focus() {
        let mut state = state_with(2, "research");
        assert_eq!(selector_height(&state), 3, "main 行 + 2 个代理");

        state.bg_agent_selection = Some(0);
        assert_eq!(
            selector_height(&state),
            3,
            "focus only changes highlighting"
        );

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

    /// 短终端同时存在 Todo 和代理任务时，代理列表收缩但 main 行保留。
    #[test]
    fn selector_respects_available_height() {
        let mut state = state_with(3, "研究任务");
        assert_eq!(selector_height_with_limit(&state, 2), 2);
        assert_eq!(render_selector_with_limit(&state, 80, 2).len(), 2);
        state.bg_agent_selection = Some(3);
        let lines = render_selector_with_limit(&state, 80, 2);
        let rendered: String = lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(
            rendered.starts_with("❯ "),
            "selected row must remain visible: {rendered}"
        );
        assert_eq!(selector_height_with_limit(&state, 0), 0);
        assert!(render_selector_with_limit(&state, 80, 0).is_empty());
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

    #[test]
    fn viewed_agent_is_labeled_after_keyboard_focus_returns_to_input() {
        let mut state = state_with(3, "检查组件");
        state.viewing_agent_task_id = Some("t2".into());
        state.bg_agent_selection = None;
        assert_eq!(selector_window_start(&state, 2), 2);
        let lines = render_selector_with_limit(&state, 100, 2);
        let main = crate::ui::utils::text::line_plain_text(&lines[0]);
        let agent = crate::ui::utils::text::line_plain_text(&lines[1]);
        let label = crate::core::i18n::t1("ui.agent.viewing");
        assert!(!main.contains(&label));
        assert!(agent.contains(&label), "{agent}");
        assert!(
            agent.contains("●"),
            "running status remains independent: {agent}"
        );
        let info = state.active_agent_tasks.get_mut("t2").unwrap();
        info.finished_at = Some(info.started_at + std::time::Duration::from_secs(3));
        let lines = render_selector_with_limit(&state, 100, 2);
        crate::ui::test_support::assert_lines_render!("agent_selector_viewing", 100, 2, &lines);
        state.viewing_agent_task_id = None;
        let lines = render_selector_with_limit(&state, 100, 2);
        assert_eq!(selector_window_start(&state, 2), 0);
        assert!(crate::ui::utils::text::line_plain_text(&lines[0]).contains(&label));
        assert!(!crate::ui::utils::text::line_plain_text(&lines[1]).contains(&label));
    }

    #[test]
    fn running_state_mark_is_visible_without_opening_agent() {
        let state = state_with(1, "research");
        let lines = render_selector(&state, 80);
        let rendered: String = lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            rendered.contains("●"),
            "running agents must look active: {rendered}"
        );
        assert!(
            !rendered.contains("○"),
            "viewing must not control running marker"
        );
    }
}
