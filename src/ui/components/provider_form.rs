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

    let block = Block::default()
        .title(" Add New Provider ")
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

        let is_active = state.provider_form.active_field == index;
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
        label_line.push(Span::styled(*hint, Style::default().fg(Color::DarkGray)));
        f.render_widget(Paragraph::new(Line::from(label_line)), label_area);

        if *is_text {
            // 文本字段：高亮活动字段的边框
            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if is_active {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }));
            state.modal_textarea.set_block(input_block);
            f.render_widget(&state.modal_textarea, input_area);
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

    let mut footer_index = PROVIDER_FORM_FIELDS.len() * 2;
    // 空行已经占了一个 chunk，快捷键提示从下一个开始
    footer_index += 1;
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Tab/↑↓", Style::default().fg(Color::DarkGray)),
            Span::styled(" next field", Style::default().fg(Color::Gray)),
            Span::styled("  ←/→", Style::default().fg(Color::DarkGray)),
            Span::styled(" switch type", Style::default().fg(Color::Gray)),
            Span::styled("  Enter", Style::default().fg(Color::DarkGray)),
            Span::styled(" save", Style::default().fg(Color::Gray)),
            Span::styled("  Esc", Style::default().fg(Color::DarkGray)),
            Span::styled(" cancel", Style::default().fg(Color::Gray)),
        ])),
        chunks[footer_index],
    );

    if let Some(error) = state.provider_form.error.clone() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                error,
                Style::default().fg(Color::Red),
            )])),
            chunks[footer_index + 1],
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
