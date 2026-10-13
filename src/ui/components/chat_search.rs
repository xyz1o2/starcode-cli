//! 对话内搜索面板（Ctrl+F，对标 Claude Code 的 conversation search）。
//!
//! 刻意**不进 modal 栈**：它是一条贴在聊天区顶部的横条，不像居中对话框那样
//! 夺走整个键盘。面板打开期间主输入框仍然能打字，所以它只在自己的那几行里
//! 吃掉按键（分发在 `ui/events/input.rs::handle_chat_search_key`）。
//!
//! 高亮打在本组件里，命中坐标由 `ChatSearchState` 提供 —— 那里的行号是
//! `render_chat_lines` 的输出下标，与这里读到的 `lines` 天然对齐。

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::core::i18n;
use crate::ui::state::ChatState;

/// 面板高度恒为 1 行：与 CC 一致，且不挤压聊天区（挤压会让正在看的历史跳位）。
pub const SEARCH_BAR_HEIGHT: u16 = 1;

/// 面板占掉的高度。布局层要拿它给聊天区让位。
pub fn search_bar_height(state: &ChatState) -> u16 {
    if state.chat_search.active {
        SEARCH_BAR_HEIGHT
    } else {
        0
    }
}

/// 渲染搜索横条。
pub fn render_chat_search_bar(f: &mut Frame, state: &ChatState, area: Rect) {
    if area.height == 0 {
        return;
    }
    let search = &state.chat_search;
    let mut spans = vec![Span::styled(
        " / ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    spans.push(Span::styled(
        search.query.clone(),
        Style::default().fg(Color::White),
    ));
    // 光标：插在 query 的 cursor 位置，宽度按显示宽度算（CJK 占两列）
    spans.push(Span::styled(
        "▏",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::SLOW_BLINK),
    ));

    if !search.query.is_empty() {
        let (label, color) = if search.matches.is_empty() {
            (
                i18n::t("ui.search.no_match", "无匹配", "no matches"),
                Color::Red,
            )
        } else {
            (
                format!("{}/{}", search.selected + 1, search.matches.len()),
                Color::DarkGray,
            )
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(label, Style::default().fg(color)));
    }

    spans.push(Span::styled(
        i18n::t(
            "ui.search.hints",
            "  Enter 下一个 · Shift+Enter 上一个 · Esc 关闭",
            "  Enter next · Shift+Enter prev · Esc close",
        ),
        Style::default().fg(Color::DarkGray),
    ));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 把命中的那一行标出来。
///
/// 只标**当前**命中而不是全部：全部标记在长回复里会变成一片高亮，反而看不出
/// 自己在哪。CC 也是"跳到哪标哪"。
pub fn highlight_current_match(
    f: &mut Frame,
    lines: &[Line<'static>],
    area: Rect,
    scroll: usize,
    match_row: usize,
) {
    let Some(row) = match_row.checked_sub(scroll) else {
        return;
    };
    let Some(line) = lines.get(row) else {
        return;
    };
    if row >= area.height as usize {
        return;
    }

    let target_y = area.y + row as u16;
    let theme = crate::ui::themes::theme::Theme::default_dark();
    let styled: Vec<Span<'static>> = line
        .spans
        .iter()
        .map(|s| s.style.bg(Color::Indexed(58)).fg(theme.background))
        .zip(line.spans.iter().map(|s| s.content.clone()))
        .map(|(style, content)| Span::styled(content, style))
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(styled)),
        Rect {
            y: target_y,
            height: 1,
            ..area
        },
    );
}
