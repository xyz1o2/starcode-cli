use crate::core::config::providers::get_provider_by_id;
use crate::types::ApprovalMode;
use crate::ui::state::ChatState;

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub fn approval_mode_label(mode: &ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::Default => "Auto",
        ApprovalMode::Plan => "Plan",
        ApprovalMode::Yolo => "Yolo",
    }
}

pub fn current_model_id(state: &ChatState) -> Option<&str> {
    non_empty(&state.current_model)
}

pub fn current_model_display(state: &ChatState) -> String {
    current_model_id(state).unwrap_or("Not set").to_string()
}

pub fn describe_provider_id(provider_id: &str) -> String {
    if let Some(provider) = get_provider_by_id(provider_id) {
        format!("{} ({})", provider.name, provider_id)
    } else {
        provider_id.to_string()
    }
}

pub fn current_provider_id(state: &ChatState) -> Option<String> {
    state
        .pending_model_provider
        .as_deref()
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| {
            state
                .current_provider_id
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string)
        })
        .or_else(|| {
            current_model_id(state)
                .and_then(|model| state.model_provider_map.get(model))
                .and_then(|provider_id| non_empty(provider_id))
                .map(str::to_string)
        })
}

pub fn current_provider_display(state: &ChatState) -> String {
    current_provider_id(state)
        .map(|provider_id| describe_provider_id(&provider_id))
        .unwrap_or_else(|| "Not set".to_string())
}

/// Returns the active UI language code for display in the status bar (e.g. "zh-CN" or "en").
pub fn current_language_display() -> &'static str {
    crate::core::i18n::current_language().as_code()
}

pub fn status_summary(state: &ChatState) -> String {
    format!(
        "{} · {} · {} · {}",
        current_model_display(state),
        current_provider_display(state),
        approval_mode_label(&state.approval_mode),
        current_language_display()
    )
}
