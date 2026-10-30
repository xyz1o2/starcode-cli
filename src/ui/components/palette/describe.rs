//! 命令面板里各条目的副标题文本。
//!
//! 独立成文件是因为这些描述同时被「条目构造」和「面板摘要」两处用，
//! 而它们与条目本身无关 —— 挪动描述不该牵动条目表。

use crate::ui::state::ChatState;
use crate::ui::utils::status::{
    approval_mode_label, current_model_id, current_provider_display, current_provider_id,
    status_summary,
};

pub(super) fn model_summary_description(state: Option<&ChatState>) -> String {
    state
        .and_then(current_model_id)
        .map(|model| format!("Current: {}", model))
        .unwrap_or_else(|| "Switch the current model".to_string())
}

pub(super) fn provider_summary_description(state: Option<&ChatState>) -> String {
    let configured = state
        .map(|state| state.configured_providers.len())
        .unwrap_or(0);
    if let Some(state) = state {
        if current_provider_id(state).is_some() {
            let provider = current_provider_display(state);
            return format!("Current: {} · {} configured", provider, configured.max(1));
        }
    }

    if configured > 0 {
        format!("{} configured provider(s)", configured)
    } else {
        "Connect or switch provider".to_string()
    }
}

pub(super) fn status_overview_description(state: Option<&ChatState>) -> String {
    state
        .map(status_summary)
        .unwrap_or_else(|| "Review model, provider and runtime status".to_string())
}

pub(super) fn approval_mode_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", approval_mode_label(&state.approval_mode)))
        .unwrap_or_else(|| "Switch Auto, Plan or Yolo".to_string())
}

pub(super) fn thinking_effort_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", state.thinking_effort.label()))
        .unwrap_or_else(|| "Set thinking/reasoning effort level".to_string())
}

pub(super) fn theme_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current: {}", state.theme_manager.current().name))
        .unwrap_or_else(|| "Switch color theme".to_string())
}
