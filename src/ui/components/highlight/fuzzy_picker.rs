/// FuzzyPicker 通用搜索壳（对标 Claude Code FuzzyPicker<T>）。
///
/// 提供三个搜索对话框共用的渲染逻辑：
/// - 响应式布局（预览在右侧/底部）
/// - 搜索输入框
/// - 结果列表（带滚动指示器）
/// - 预览区
/// - Byline 快捷键提示
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// 预览位置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPosition {
    /// 预览在底部
    Bottom,
    /// 预览在右侧
    Right,
}

/// FuzzyPicker 布局区域
pub struct PickerAreas {
    pub search: Rect,
    pub list: Rect,
    pub preview: Rect,
}

/// 计算 FuzzyPicker 布局（对标 CCB FuzzyPicker 布局逻辑）。
///
/// - `preview_threshold`: 预览在右侧的最小终端宽度
/// - `preview_height`: 预览在底部时的高度
///
/// 返回 (PickerAreas, PreviewPosition, content_area)。
/// content_area 是去除 Pane 分割线后的实际内容区域。
pub fn compute_layout(
    area: Rect,
    preview_threshold: u16,
    preview_height: u16,
) -> (PickerAreas, PreviewPosition, Rect) {
    let preview_on_right = area.width >= preview_threshold;

    // 对标 CCB Pane: paddingTop=1 (空白行) + 分割线 + paddingX=2
    // 分割线占用 1 行，上方空白 1 行，所以内容从 y+2 开始
    let content_area = Rect {
        x: area.x + 2,
        y: area.y + 2,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(3),
    };

    if preview_on_right {
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(5)])
            .split(content_area);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(vertical[1]);
        (
            PickerAreas {
                search: vertical[0],
                list: horizontal[0],
                preview: horizontal[1],
            },
            PreviewPosition::Right,
            content_area,
        )
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(preview_height),
            ])
            .split(content_area);
        (
            PickerAreas {
                search: chunks[0],
                list: chunks[1],
                preview: chunks[2],
            },
            PreviewPosition::Bottom,
            content_area,
        )
    }
}

/// 渲染 Pane 分割线（对标 CCB Pane Divider）。
///
/// 在 area 顶部渲染全宽 `─` 水平线。
pub fn render_pane_divider(f: &mut Frame, area: Rect, color: Color) {
    // 分割线在 area.y + 1 位置（上方留一行空白）
    let divider_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: 1,
    };
    let line = "─".repeat(area.width as usize);
    let divider = Paragraph::new(Line::from(Span::styled(line, Style::default().fg(color))));
    f.render_widget(divider, divider_area);
}

/// 渲染搜索输入框（对标 CCB SearchBox）。
///
/// CCB SearchBox 特点:
/// - 前缀字符 `⌖` (U+2316)
/// - 圆角边框 (borderStyle="round")
/// - 聚焦时边框为 suggestion 颜色
/// - placeholder 首字符反色显示
pub fn render_search_input(f: &mut Frame, area: Rect, title: &str, query: &str, placeholder: &str) {
    let input_block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    // 对标 CCB SearchBox: 前缀 `⌖` + 内容
    let input_line = if query.is_empty() {
        // placeholder 首字符反色（对标 CCB）
        let mut spans = vec![Span::styled("⌖ ", Style::default().fg(Color::DarkGray))];
        if !placeholder.is_empty() {
            let chars: Vec<char> = placeholder.chars().collect();
            spans.push(Span::styled(
                chars[0].to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            ));
            if chars.len() > 1 {
                spans.push(Span::styled(
                    chars[1..].iter().collect::<String>(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        Line::from(spans)
    } else {
        Line::from(vec![
            Span::styled("⌖ ", Style::default().fg(Color::DarkGray)),
            Span::styled(query.to_string(), Style::default().fg(Color::White)),
        ])
    };

    let input = Paragraph::new(input_line).block(input_block);
    f.render_widget(input, area);
}

/// 计算可见窗口（对标 CCB FuzzyPicker windowing）。
///
/// 返回 (window_start, visible_count)。
pub fn compute_window(
    selected_index: usize,
    total: usize,
    available_height: u16,
) -> (usize, usize) {
    let visible_count = available_height.saturating_sub(2).max(2) as usize;
    let window_start = if selected_index >= visible_count {
        selected_index - visible_count + 1
    } else {
        0
    };
    (window_start.min(total.saturating_sub(1)), visible_count)
}

/// 渲染滚动指示器前缀（对标 CCB ListItem 滚动指示器）。
///
/// 返回 (indicator_span, actual_index)。
pub fn scroll_indicator(
    i: usize,
    actual_idx: usize,
    selected_index: usize,
    visible_count: usize,
    has_above: bool,
    has_below: bool,
) -> Span<'static> {
    let is_focused = actual_idx == selected_index;
    if is_focused {
        Span::styled("▶ ", Style::default().fg(Color::Yellow))
    } else if i == 0 && has_above {
        Span::styled("↑ ", Style::default().fg(Color::DarkGray))
    } else if i == visible_count.saturating_sub(1) && has_below {
        Span::styled("↓ ", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("  ")
    }
}

/// 渲染 Byline 快捷键提示（对标 CCB FuzzyPicker byline）。
///
/// 使用 ` · ` (middot) 分隔，支持 compact 模式（columns < 120 时缩短标签）。
pub fn render_byline(f: &mut Frame, area: Rect, hints: &[(&str, &str)]) {
    let byline_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    // 对标 CCB compact 模式: columns < 120 时缩短标签
    let compact = area.width < 120;
    let mut spans = Vec::new();
    for (i, (key, action)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        // compact 模式下缩短 action 标签
        let action_display = if compact {
            match *action {
                "navigate" => "nav",
                "mention" => "ment",
                "insert path" => "path",
                _ => action,
            }
        } else {
            action
        };
        spans.push(Span::raw(format!(" {}", action_display)));
    }
    let byline = Paragraph::new(Line::from(spans));
    f.render_widget(byline, byline_area);
}

/// 渲染空状态消息（对标 CCB emptyMessage）。
pub fn render_empty_state(f: &mut Frame, area: Rect, block: Block<'_>, message: &str) {
    let empty = Paragraph::new(message)
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(empty, area);
}

/// 渲染带滚动指示器的列表（对标 CCB List + ListItem）。
///
/// `render_item_fn` 接收 (item_ref, is_focused) 返回 Line。
pub fn render_scrolling_list<T>(
    f: &mut Frame,
    area: Rect,
    block: Block<'_>,
    items: &[T],
    selected_index: usize,
    render_item_fn: impl Fn(&T, bool) -> Line<'static>,
) {
    if items.is_empty() {
        return;
    }

    let (window_start, visible_count) = compute_window(selected_index, items.len(), area.height);
    let window_end = (window_start + visible_count).min(items.len());
    let has_above = window_start > 0;
    let has_below = window_end < items.len();

    let list_items: Vec<ListItem> = items[window_start..window_end]
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let actual_idx = window_start + i;
            let is_focused = actual_idx == selected_index;
            let indicator = scroll_indicator(
                i,
                actual_idx,
                selected_index,
                visible_count,
                has_above,
                has_below,
            );
            let content = render_item_fn(item, is_focused);
            let mut spans = vec![indicator];
            spans.extend(content.spans);
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(list_items).block(block).highlight_style(
        Style::default()
            .bg(Color::Rgb(40, 40, 60))
            .add_modifier(Modifier::BOLD),
    );

    let mut list_state = ListState::default();
    list_state.select(Some(selected_index - window_start));
    f.render_stateful_widget(list, area, &mut list_state);
}

/// 渲染 matchLabel（对标 CCB matchLabel — 空结果时传 ' ' 保留行高防跳）。
pub fn format_match_label(
    count: usize,
    truncated: bool,
    is_searching: bool,
    label: &str,
) -> String {
    if count == 0 {
        " ".to_string()
    } else {
        format!(
            "{}{} {}{}",
            count,
            if truncated { "+" } else { "" },
            label,
            if is_searching { "…" } else { "" }
        )
    }
}
