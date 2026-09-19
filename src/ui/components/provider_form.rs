//! "新增 provider" 单面板表单。
//!
//! 原来加一个自定义 provider 要走四个串行模态框（类型 → ID → 名称 → Base URL →
//! API Key），其中 ID 和名称本质是同一个东西。这里合并成一个面板：Type / Name /
//! Base URL / API Key / Model 五行一次填完，provider ID 由名称（或 URL host）派生。
//!
//! # 渲染
//!
//! 沿用 `Modal::InputModal` 的开关，不引入新 Modal 变体——`enter_input_modal` /
//! `exit_input_modal`、Ctrl+V 粘贴、Esc 退出那一套就都能白送。所有字段共用
//! `state.modal_textarea` 当活动字段编辑器，切换字段时内容由
//! `ProviderFormState` 里的 `values` 搬进搬出（按键处理在
//! `ui::events::input::handle_input_modal`）。
//!
//! Type 字段（下标 0）不是自由文本，不渲染 textarea，只显示当前选项和 `◀ ▶` 提示。
//! 非活动的文本字段同样不渲染 textarea——textarea 只有一份内容，全画上去会让
//! 所有框显示同一段文字。它们画 `values` 里已经存好的值（空字段画 hint，和
//! textarea 的 placeholder 对齐，不然焦点移开时提示一闪一闪）。
//!
//! 面板底部是操作行：`‹ Back` 取消回上一层，`Save ›` 提交，←/→ 选、Enter 定。
//! Tab/↓ 从末字段进操作行，再按继续循环回 Type。

use crate::ui::state::palette::{ProviderFormState, PROVIDER_FORM_FIELDS, PROVIDER_FORM_TYPES};
use crate::ui::state::ChatState;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

/// 表单是不是在编辑状态（`InputContext::ProviderForm` 且输入模态开着）。
pub fn is_provider_form_active(state: &ChatState) -> bool {
    state.show_input_modal
        && matches!(
            state.input_context,
            Some(crate::ui::state::palette::InputContext::ProviderForm)
        )
}

pub fn render_provider_form(f: &mut Frame, area: Rect, state: &mut ChatState) {
    if !is_provider_form_active(state) {
        return;
    }

    let area = centered_rect(72, 62, area);

    let title = if state.provider_form.editing_id.is_some() {
        " Edit Provider "
    } else {
        " Add New Provider "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(Clear, area);
    f.render_widget(block.clone(), area);

    let inner = block.inner(area);

    // 每个字段两行（标签 + 输入框），外加底部提示和错误行
    let mut constraints: Vec<Constraint> = Vec::new();
    for _ in 0..PROVIDER_FORM_FIELDS.len() {
        constraints.push(Constraint::Length(1)); // 标签
        constraints.push(Constraint::Length(3)); // 输入框
    }
    constraints.push(Constraint::Length(1)); // 空行
    constraints.push(Constraint::Length(3)); // 操作行（‹ Back / Save ›）
    constraints.push(Constraint::Length(1)); // 快捷键提示
    if state.provider_form.error.is_some() {
        constraints.push(Constraint::Length(1)); // 错误
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (index, (label, hint, is_text)) in PROVIDER_FORM_FIELDS.iter().enumerate() {
        let label_area = chunks[index * 2];
        let input_area = chunks[index * 2 + 1];

        // 操作行聚焦时没有任何字段是活动的（textarea 不渲染），否则末字段
        // 会和操作行同时显示成选中态
        let is_active =
            !state.provider_form.on_actions && state.provider_form.active_field == index;
        let editing = state.provider_form.editing_id.is_some();
        // API Key 的提示在编辑模式下含义不同（空着 = 保持原 key）
        let hint = if *label == "API Key" {
            crate::ui::state::palette::api_key_field_hint(editing)
        } else {
            *hint
        };
        let value = state
            .provider_form
            .values
            .get(index)
            .map(String::as_str)
            .unwrap_or("");

        // 标签行：`▸ Name   hint`
        let mut label_line = vec![Span::styled(
            if is_active { "▸ " } else { "  " },
            Style::default().fg(if is_active {
                Color::Cyan
            } else {
                Color::DarkGray
            }),
        )];
        label_line.push(Span::styled(
            *label,
            Style::default()
                .fg(if is_active { Color::White } else { Color::Gray })
                .add_modifier(if is_active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        label_line.push(Span::raw("  "));
        label_line.push(Span::styled(hint, Style::default().fg(Color::DarkGray)));
        f.render_widget(Paragraph::new(Line::from(label_line)), label_area);

        if *is_text {
            // textarea 全表单共用一份，里面只有活动字段的内容。非活动字段要是也
            // 画 textarea，四个文本框会同时显示同一段文字（联动 bug），所以它们
            // 直接画 `values` 里存好的值。
            if is_active {
                let input_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                state.modal_textarea.set_block(input_block);
                f.render_widget(&state.modal_textarea, input_area);
            } else {
                let input_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray));
                // 空字段显示 hint，和活动字段 textarea 的 placeholder 一致；
                // 不然焦点移开时框里提示突然消失，看起来像样式在闪
                let is_placeholder = value.is_empty();
                f.render_widget(
                    Paragraph::new(if is_placeholder { hint } else { value }).style(
                        Style::default().fg(if is_placeholder {
                            Color::DarkGray
                        } else {
                            Color::Gray
                        }),
                    ),
                    input_area,
                );
            }
        } else {
            // 选项字段：不接管 textarea，直接画当前值 + ◀ ▶
            let row = Line::from(vec![
                Span::styled(
                    "◀ ",
                    Style::default().fg(if is_active {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    value,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " ▶",
                    Style::default().fg(if is_active {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]);
            f.render_widget(Paragraph::new(row).alignment(Alignment::Center), input_area);
        }
    }

    // 空行占了一个 chunk，操作行从下一个开始
    let footer_index = PROVIDER_FORM_FIELDS.len() * 2 + 1;

    // 操作行：‹ Back（取消回上一层）/ Save ›（提交）
    let action_area = chunks[footer_index];
    let box_width = action_area.width.saturating_sub(3) / 2;
    let action_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(box_width),
            Constraint::Length(1), // 两个按钮之间的间隙
            Constraint::Length(box_width),
            Constraint::Min(0),
        ])
        .split(action_area);
    for (index, label) in ["‹ Back", "Save ›"].iter().enumerate() {
        let selected = state.provider_form.on_actions && state.provider_form.action_index == index;
        let action_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if selected {
                Color::Cyan
            } else {
                Color::DarkGray
            }));
        f.render_widget(
            Paragraph::new(*label)
                .block(action_block)
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(if selected { Color::White } else { Color::Gray })
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            action_chunks[index * 2],
        );
    }

    // 快捷键提示按焦点位置变：操作行上 ←/→ 是"选择"、Enter 是"确认"，
    // Type 上 Enter 是"下一字段"，文本字段上才是"保存"
    let hint_parts: &[(&str, &str)] = if state.provider_form.on_actions {
        &[
            (" Tab/↑↓", " back to fields"),
            ("  ←/→", " choose"),
            ("  Enter", " confirm"),
            ("  Esc", " cancel"),
        ]
    } else if state.provider_form.active_field == 0 {
        &[
            (" Tab/↑↓", " next field"),
            ("  ←/→", " switch type"),
            ("  Enter", " next field"),
            ("  Esc", " cancel"),
        ]
    } else {
        &[
            (" Tab/↑↓", " next field"),
            ("  Enter", " save"),
            ("  Esc", " cancel"),
        ]
    };
    let mut hint_spans = Vec::new();
    for (key_part, label_part) in hint_parts {
        hint_spans.push(Span::styled(
            *key_part,
            Style::default().fg(Color::DarkGray),
        ));
        hint_spans.push(Span::styled(*label_part, Style::default().fg(Color::Gray)));
    }
    f.render_widget(
        Paragraph::new(Line::from(hint_spans)),
        chunks[footer_index + 1],
    );

    if let Some(error) = state.provider_form.error.clone() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                error,
                Style::default().fg(Color::Red),
            )])),
            chunks[footer_index + 2],
        );
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_field_defaults_to_openai_compatible() {
        let form = ProviderFormState::new();
        assert_eq!(form.provider_type(), "openai-compatible");
        assert_eq!(form.values[0], "OpenAI Compatible");
    }

    #[test]
    fn form_opens_focused_on_type_field() {
        // 打开表单先落在 Type（下标 0）：用户得先选协议类型，再 Tab 往下填，
        // 不要一上来就默认替他选好 OpenAI Compatible。
        let form = ProviderFormState::new();
        assert_eq!(form.active_field, 0);
    }

    #[test]
    fn cycling_type_wraps_both_ways() {
        let mut form = ProviderFormState::new();
        form.cycle_type(true);
        assert_eq!(form.provider_type(), "anthropic-compatible");
        form.cycle_type(true);
        assert_eq!(form.provider_type(), "openai-compatible");
        // 反向也循环
        form.cycle_type(false);
        assert_eq!(form.provider_type(), "anthropic-compatible");
    }

    #[test]
    fn accessors_read_the_right_slots() {
        let mut form = ProviderFormState::new();
        form.values[1] = "My LM Studio".to_string();
        form.values[2] = "http://localhost:1234/v1".to_string();
        form.values[3] = "sk-test".to_string();
        form.values[4] = "qwen2.5-coder".to_string();
        assert_eq!(form.name(), "My LM Studio");
        assert_eq!(form.base_url(), "http://localhost:1234/v1");
        assert_eq!(form.api_key(), "sk-test");
        assert_eq!(form.model(), "qwen2.5-coder");
    }

    #[test]
    fn form_state_and_field_table_stay_in_sync() {
        // values 与 PROVIDER_FORM_FIELDS 下标对齐：长度不一致时 render 里的
        // `values.get(index)` 会读到 None，字段就变成空白。
        assert_eq!(
            ProviderFormState::new().values.len(),
            PROVIDER_FORM_FIELDS.len()
        );
        // 只有 Type 一个字段不是自由文本
        assert_eq!(
            PROVIDER_FORM_FIELDS
                .iter()
                .filter(|(_, _, is_text)| !is_text)
                .count(),
            1
        );
        // 类型选项固定两个，←/→ 才有地方循环
        assert_eq!(PROVIDER_FORM_TYPES.len(), 2);
    }
}
