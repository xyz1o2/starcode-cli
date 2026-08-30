use crate::ui::state::ChatState;
use crate::ui::utils::status::{
    approval_mode_label, current_model_display, current_provider_display,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Returns the number of setting items (for navigation bounds).
pub fn settings_item_count() -> usize {
    SETTING_ITEMS.len()
}

/// A setting item in the interactive settings panel.
struct SettingItem {
    id: &'static str,
    label: &'static str,
    get_value: fn(&ChatState) -> String,
    description: &'static str,
}

const SETTING_ITEMS: &[SettingItem] = &[
    SettingItem {
        id: "model",
        label: "Model",
        get_value: |s| current_model_display(s),
        description: "Switch the AI model used for responses",
    },
    SettingItem {
        id: "provider",
        label: "Provider",
        get_value: |s| current_provider_display(s),
        description: "Switch the AI provider (OpenAI, Anthropic, etc.)",
    },
    SettingItem {
        id: "approval",
        label: "Approval Mode",
        get_value: |s| approval_mode_label(&s.approval_mode).to_string(),
        description: "Default: confirm dangerous ops\nPlan: require approval for all edits\nYolo: auto-approve everything",
    },
    SettingItem {
        id: "thinking",
        label: "Thinking Effort",
        get_value: |s| {
            let cap = crate::core::config::models::thinking_capability(&s.current_model);
            match cap {
                crate::core::config::models::ThinkingCapability::Binary => {
                    match s.thinking_effort {
                        crate::types::ThinkingEffort::Off => "Off".to_string(),
                        _ => "On".to_string(),
                    }
                }
                _ => format!("{:?}", s.thinking_effort),
            }
        },
        description: "Controls reasoning depth.\nGranular models: Off, Low, Medium, High\nBinary models: Off, On\nVaries by model capability",
    },
    SettingItem {
        id: "context_window",
        label: "Context Window",
        get_value: |s| {
            if let Some(override_val) = s.context_window_override {
                format!("{}k (custom)", override_val / 1000)
            } else {
                "auto".to_string()
            }
        },
        description: "Override the model's context window size\nAuto: detect from model/API\nCustom: set manually (e.g. 128k, 200k, 1M)",
    },
    SettingItem {
        id: "theme",
        label: "Theme",
        get_value: |_| "Current".to_string(),
        description: "Change the UI color theme",
    },
    SettingItem {
        id: "output_style",
        label: "Output Style",
        get_value: |s| {
            crate::core::config::settings_manager::SettingsManager::new()
                .ok()
                .and_then(|_| Some("default".to_string()))
                .unwrap_or_else(|| "default".to_string())
        },
        description: "Default: standard output\nConcise: minimal explanations\nVerbose: detailed output",
    },
    SettingItem {
        id: "vim_mode",
        label: "Vim Mode",
        get_value: |s| if s.vim_enabled { "ON".to_string() } else { "OFF".to_string() },
        description: "Toggle vim keybindings for the input area",
    },
    SettingItem {
        id: "language",
        label: "Language",
        get_value: |_| "auto".to_string(),
        description: "Change the UI language (en-US, zh-CN, auto)",
    },
    SettingItem {
        id: "colorblind",
        label: "Colorblind Mode",
        get_value: |s| if s.colorblind_mode { "ON".to_string() } else { "OFF".to_string() },
        description: "Add shape indicators alongside colors for accessibility",
    },
];

fn build_setting_lines(state: &ChatState, selected: usize) -> Vec<ListItem<'static>> {
    SETTING_ITEMS
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let value = (item.get_value)(state);
            let is_selected = i == selected;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let line = Line::from(vec![
                Span::styled(format!("{:<18}", item.label), style),
                Span::styled(value, style.fg(if is_selected { Color::Black } else { Color::Gray })),
            ]);
            ListItem::new(line)
        })
        .collect()
}

fn build_detail_lines(state: &ChatState, selected: usize) -> Vec<Line<'static>> {
    if selected >= SETTING_ITEMS.len() {
        return vec![];
    }
    let item = &SETTING_ITEMS[selected];
    let value = (item.get_value)(state);
    vec![
        Line::from(Span::styled(
            item.label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            format!("Current: {}", value),
            Style::default().fg(Color::White),
        )),
        Line::default(),
        Line::from(Span::styled(
            item.description,
            Style::default().fg(Color::Gray),
        )),
        Line::default(),
        Line::from(Span::styled(
            "Enter to change | Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

pub fn render_status_modal(f: &mut Frame, area: Rect, state: &ChatState) {
    if !state.show_status_modal {
        return;
    }

    let area = centered_rect(70, 50, area);
    let block = Block::default()
        .title(" Settings ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(Clear, area);
    f.render_widget(block.clone(), area);

    let inner_area = block.inner(area);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(inner_area);

    // Left panel: settings list
    let items = build_setting_lines(state, state.settings_selected_index);
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray));

    let mut list_state = ListState::default();
    list_state.select(Some(state.settings_selected_index));
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    // Right panel: detail view
    let detail_lines = build_detail_lines(state, state.settings_selected_index);
    let detail = Paragraph::new(detail_lines).block(Block::default());
    f.render_widget(detail, chunks[1]);
}

/// Returns the palette action for the currently selected setting, if any.
pub fn get_settings_action(state: &ChatState) -> Option<crate::ui::state::palette::PaletteAction> {
    if state.settings_selected_index >= SETTING_ITEMS.len() {
        return None;
    }
    let item = &SETTING_ITEMS[state.settings_selected_index];
    match item.id {
        "model" => Some(crate::ui::state::palette::PaletteAction::ShowModelMenu),
        "provider" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::Provider,
        )),
        "approval" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::AgentMode,
        )),
        "thinking" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::ThinkingEffort,
        )),
        "context_window" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::ContextWindow,
        )),
        "theme" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::Theme,
        )),
        "output_style" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::OutputStyle,
        )),
        "vim_mode" => Some(crate::ui::state::palette::PaletteAction::ToggleVimMode),
        "colorblind" => Some(crate::ui::state::palette::PaletteAction::ToggleColorblindMode),
        "language" => Some(crate::ui::state::palette::PaletteAction::Navigate(
            crate::ui::state::palette::PaletteMode::Language,
        )),
        _ => None,
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
