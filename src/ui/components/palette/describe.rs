//! 命令面板里各条目的副标题文本。
//!
//! 独立成文件是因为这些描述同时被「条目构造」和「面板摘要」两处用，
//! 而它们与条目本身无关 —— 挪动描述不该牵动条目表。

use crate::ui::state::ChatState;
use crate::ui::utils::status::{
    approval_mode_label, current_model_id, current_provider_display, current_provider_id,
};

pub(super) fn model_summary_description(state: Option<&ChatState>) -> String {
    state
        .and_then(current_model_id)
        .map(|model| format!("Current · {}", model))
        .unwrap_or_else(|| "Switch the current model".to_string())
}

pub(super) fn provider_summary_description(state: Option<&ChatState>) -> String {
    let configured = state
        .map(|state| state.configured_providers.len())
        .unwrap_or(0);
    if let Some(state) = state {
        if current_provider_id(state).is_some() {
            let provider = current_provider_display(state);
            return format!("Current · {} · {} configured", provider, configured.max(1));
        }
    }

    if configured > 0 {
        format!("{} configured", configured)
    } else {
        "Connect or switch provider".to_string()
    }
}

/// 「Status」条目的副标题。
///
/// 只说**当前模型** —— 不复述状态栏那四段摘要（模型 · provider · 审批模式 ·
/// 语言）。那四段是给状态栏那条整行用的；塞进面板的一行里，四十列宽的地方
/// 放不下，末尾会被切掉半个词（"Not set · Not set · Default · en"）。
///
/// 状态栏本来就在屏幕下方常驻着，用户要看完整状态随时能看；面板这一行要
/// 回答的只是"现在在用什么模型"，其余的点了 Status 就有。
pub(super) fn status_overview_description(state: Option<&ChatState>) -> String {
    state
        .and_then(current_model_id)
        .map(|model| format!("Current · {}", model))
        .unwrap_or_else(|| "No model set".to_string())
}

pub(super) fn approval_mode_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current · {}", approval_mode_label(&state.approval_mode)))
        .unwrap_or_else(|| "Switch Auto, Default, Plan or Bypass permissions".to_string())
}

pub(super) fn thinking_effort_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current · {}", state.thinking_effort.label()))
        .unwrap_or_else(|| "Set thinking/reasoning effort level".to_string())
}

pub(super) fn theme_description(state: Option<&ChatState>) -> String {
    state
        .map(|state| format!("Current · {}", state.theme_manager.current().name))
        .unwrap_or_else(|| "Switch color theme".to_string())
}
