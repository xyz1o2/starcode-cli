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
        ApprovalMode::AcceptEdits => "Accept edits",
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

/// 当前生效的 provider id，供状态栏 / Settings 面板 / 命令行提示共用。
///
/// 顺序：worker 确认的 active snapshot → `current_provider_id` → 当前模型在
/// `model_provider_map` 里的归属 → 磁盘缓存的跨桶归属 → `pending_model_provider`。
///
/// # 为什么最后两步不能省
///
/// `model_provider_map` 只装**当前那份列表**的映射（`apply_models_to_state` 每次
/// 都 `clear()` 再重建），所以一个属于别的网关的模型名在这里查不到。而真正决定
/// 请求发给谁的是 [`crate::ui::events::input::resolve_provider_for_model`]，它多走
/// 一步 `model_cache::find_provider_for_model` 全桶扫描。两侧顺序不一致的直接后果
/// 就是截图里那个画面：Model 有值、Provider 写着 `Not set`，用户无从判断模型到底
/// 被发去了哪一家。
///
/// 跨桶那一查要读磁盘，所以只在前面都没命中时才做。
pub fn current_provider_id(state: &ChatState) -> Option<String> {
    state
        .confirmed_runtime_settings
        .as_ref()
        .and_then(|snapshot| snapshot.active.model.provider_id.as_deref())
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
            // 归属和"切换请求会用哪个 provider"必须同源，否则面板写着 Provider A
            // 而请求实际发去了 B。两边都走 `resolve_provider_for_model`，里面已经
            // 过滤掉了已删除的 provider（`ChatState::live_providers`）。
            current_model_id(state).and_then(|model| {
                crate::ui::events::input::resolve_provider_for_model(state, model)
            })
        })
        // Provider provisioning is not an active runtime transition. Keep it only as a
        // display fallback before the worker has supplied an active provider snapshot.
        .or_else(|| {
            state
                .pending_model_provider
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string)
        })
        .filter(|provider_id| !provider_id.trim().is_empty())
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
