//! agent -> UI 的流式消息处理。
//!
//! 本文件是**消息分发的骨架**：收到哪种 StreamMessage 就改哪块状态。
//! 具体实现拆在子模块：
//!
//! - [gent_task] —— 后台代理任务的状态更新
//! - [index_shift] —— 条目增删后各类索引表的整体位移
//! - [`tool_result`] —— 工具结果与正文消息

mod agent_task;
mod index_shift;
mod tool_result;

use agent_task::*;
use index_shift::*;
use tool_result::*;

#[cfg(test)]
mod tests;

/// Stream message handler — processes Agent→UI messages.
///
/// # Message Processing
///
/// This function handles all `StreamMessage` variants from the agent worker.
/// Each message type updates the UI state and may trigger a redraw.
///
/// # Performance Notes
/// - Called from the main UI loop with a time budget (8ms per batch)
/// - Heavy operations (e.g. large tool outputs) should be deferred or truncated
/// - The `rendered_cache` and `virtual_list` are invalidated on content changes
///
/// # Error Handling
/// - Errors are propagated to the UI loop which logs them
/// - The UI continues running even if a single message fails
///
use super::status_helpers::{
    format_running_tool_label, format_tool_name_for_status,
    should_suppress_redundant_result_after_confirmation, truncate_status_detail,
};
use crate::core::i18n;
use crate::runtime::messages::{
    AgentRequest, RuntimeSettingsAcknowledgement, StreamMessage, StreamStartKind,
};
use crate::types::{ChatEntry, ChatEntryType, StarToolCall, ToolResult};
use crate::ui::app::logic::{
    emit_status_text, enqueue_user_message, recover_missing_tool_results, save_tool_output,
};
use crate::ui::state::store::{
    ChatState, QueuedConfirmation, RuntimeSettingsPersistence, ToastKind,
};
use crate::ui::utils::format::{
    format_tool_call, format_tool_result, format_tool_result_with_saved_path,
};
use crate::ui::utils::text::{
    format_elapsed_for_tool, inject_elapsed_into_tool_first_line, sanitize_for_tui,
    should_save_tool_output,
};

/// 把一份模型列表灌进 UI state。
///
/// `StreamMessage::ModelsList` 和"打开面板前先读磁盘缓存"两条路径共用，
/// 避免缓存命中时和网络返回时渲染出不一致的状态。
pub(crate) fn apply_models_to_state(
    state: &mut ChatState,
    models: &[crate::types::ModelInfo],
    cache_age_secs: Option<u64>,
) {
    state.available_models = models.iter().map(|m| m.id.clone()).collect();
    state.available_models_info = models.to_vec(); // 保存完整的模型信息
    state.set_models_list_age(cache_age_secs);
    state.model_provider_map.clear();
    for m in models {
        if !m.provider.is_empty() {
            state
                .model_provider_map
                .insert(m.id.clone(), m.provider.clone());
        }
    }
    // 如果当前有模型，更新其 thinking 支持状态
    if !state.current_model.is_empty() {
        state.current_model_supports_thinking = models
            .iter()
            .find(|m| m.id == state.current_model)
            .and_then(|m| m.supports_thinking);
    }
}

/// 打开模型面板前，先用磁盘缓存把列表填上，让面板立刻有内容可显示。
///
/// 之前列表为空时 UI 先画 "Loading models..."，再等 `ListModels` 回来 ——
/// 可这条请求走的是便宜路径（`force = false`），命中磁盘缓存时几乎立刻返回，
/// 于是用户每次开面板都被闪一下假 loading，明明不需要任何网络等待。
///
/// 这里在发请求之前同步读一次磁盘缓存（纯文件读，不阻塞），面板打开即有真实
/// 列表；随后那条 `ListModels` 仍照发，拿到新数据再覆盖。
///
/// 返回是否命中。已有列表时不做任何事（用户刚拉过的新列表比旧缓存新）。
pub(crate) fn seed_models_from_disk_cache(state: &mut ChatState, provider_id: &str) -> bool {
    if !state.available_models.is_empty() {
        return false;
    }
    let Some(hit) = crate::agent::model_cache::load(provider_id) else {
        return false;
    };
    apply_models_to_state(state, &hit.models, Some(hit.age_secs));
    true
}

/// 把「磁盘缓存里这个 provider 的模型」与「用户声明的候选集」**合并**成面板列表。
///
/// # 为什么要合而不是取一个
///
/// 用户只配了 url + apikey（没配模型），provider 的候选集是空的，一切靠 API
/// 自动发现 —— 这条路上模型面板显示的是完整列表，没问题。
///
/// 但只要用户在表单里写了哪怕一个模型名，之前那段代码就把 `available_models`
/// 整个**替换**成了声明的那几个：API 明明还返回了另外十几个模型，面板上却只看得到
/// 用户写的那一个，得再点一次刷新才补得回来。用户描述的"下次再选时候它会只有
/// 这一个模型"就是这个。
///
/// 声明的候选集是「用户钦定要用这些」，API 的列表是「这家实际提供这些」——
/// 两者是**并集**，不是二选一。声明的排在前面（用户的优先级），API 发现的补在后面。
pub(crate) fn merge_declared_and_discovered_models(
    state: &mut ChatState,
    provider_id: &str,
    declared: &[String],
) {
    let cached = crate::agent::model_cache::load(provider_id);
    merge_declared_and_discovered_models_with(state, provider_id, declared, cached);
}

/// 合并逻辑本体，缓存由调用方给。
///
/// 生产路径走 [`merge_declared_and_discovered_models`]（从磁盘缓存读）；测试直接
/// 传 `load_from_path` 的结果，**不碰 `STAR_MODEL_CACHE_PATH`** —— 那是进程级
/// 环境变量，并行跑的两个用例会互相覆盖对方的缓存文件（一个用例单跑也失败）。
fn merge_declared_and_discovered_models_with(
    state: &mut ChatState,
    provider_id: &str,
    declared: &[String],
    cached: Option<crate::agent::model_cache::CachedModels>,
) {
    let mut merged: Vec<String> = declared.to_vec();
    let mut seen: std::collections::HashSet<String> = merged.iter().cloned().collect();

    // 磁盘缓存里这个 provider 曾经拉到的模型 —— 大多数情况下不用等网络。
    if let Some(hit) = cached {
        for model in &hit.models {
            if model.provider == provider_id && seen.insert(model.id.clone()) {
                merged.push(model.id.clone());
            }
        }
    }

    state.available_models = merged.clone();
    state.model_provider_map = merged
        .iter()
        .map(|model| (model.clone(), provider_id.to_string()))
        .collect();
    state.awaiting_models = false;
    state.awaiting_models_since = None;
}

/// 结束 entry 的 streaming 状态，并冻结 thinking 计时器。
/// 统一处理所有 is_streaming = false 的场景，避免遗漏 reasoning_finished_elapsed_ms。
pub(crate) fn finalize_entry_streaming(state: &mut ChatState, idx: usize) {
    if let Some(entry) = state.chat_history.get_mut(idx) {
        entry.is_streaming = Some(false);
        // 冻结 thinking 计时器：只对有 reasoning 且未冻结的 entry 设置。
        // 必须有真实请求（processing_started_at）才冻结——恢复的会话等场景
        // 没有时钟基准，冻结成 0 会把每块都显示成 "Thought for 1s"。
        if let Some(started) = state.processing_started_at {
            if entry.reasoning_content.is_some() && entry.reasoning_finished_elapsed_ms.is_none() {
                entry.reasoning_finished_elapsed_ms = Some(started.elapsed().as_millis());
            }
        }
    }
    state.rendered_cache.remove(&idx);
    state.virtual_list.mark_dirty(idx);
}

/// 从仍在执行的工具中选出最后启动的一项，保持状态行的显示标签与 ID 生命周期同步。
fn refresh_current_tool_name(state: &mut ChatState) {
    let active_id = state
        .tool_started_at
        .iter()
        .max_by(|(left_id, left_started), (right_id, right_started)| {
            left_started
                .cmp(right_started)
                .then_with(|| left_id.cmp(right_id))
        })
        .map(|(id, _)| id.as_str());

    state.current_tool_name = active_id.and_then(|id| {
        state.chat_history.iter().rev().find_map(|entry| {
            entry
                .tool_call
                .as_ref()
                .filter(|tool_call| tool_call.id == id)
                .map(|tool_call| tool_call.function.name.clone())
        })
    });
}

/// 将 worker 已确认的 runtime snapshot 作为 UI active 状态的唯一事实来源。
///
/// revision `0` 是兼容旧请求；只要 UI 已发出过 revisioned mutation，就不能让旧消息
/// 倒灌覆盖新请求。对于正常 revision，晚到的 acknowledgement 不会改写更晚的 requested
/// intent，但仍可确认对应 worker 已安装的 active snapshot。
fn apply_runtime_settings_acknowledgement(
    state: &mut ChatState,
    acknowledgement: RuntimeSettingsAcknowledgement,
) {
    let ui_revision = acknowledgement.ui_revision;
    let newest_pending_revision = state
        .pending_runtime_settings
        .back()
        .map(|mutation| mutation.ui_revision);
    let newest_ack_revision = state
        .last_runtime_settings_ack
        .as_ref()
        .map(|ack| ack.ui_revision)
        .unwrap_or(0);

    if (ui_revision == 0 && state.next_runtime_settings_revision > 1)
        || (ui_revision != 0 && ui_revision < newest_ack_revision)
    {
        return;
    }

    let has_later_pending = newest_pending_revision.is_some_and(|revision| revision > ui_revision);
    let fast_transition = state
        .pending_fast_mode_transition
        .as_ref()
        .filter(|transition| transition.ui_revision == ui_revision)
        .cloned();
    if let Some(transition) = fast_transition {
        if matches!(
            acknowledgement.outcome,
            crate::core::context_policy::RuntimeSettingOutcome::Applied
                | crate::core::context_policy::RuntimeSettingOutcome::Degraded
        ) {
            state.fast_mode = transition.enabled;
            state.fast_mode_prev_model = if transition.enabled {
                transition.previous_model
            } else {
                None
            };
        }
        state.pending_fast_mode_transition = None;
    }
    state
        .pending_runtime_settings
        .retain(|pending| pending.ui_revision > ui_revision || pending.ui_revision == 0);
    if has_later_pending {
        return;
    }

    // The worker snapshot is authoritative for active values. Once no newer UI request is
    // pending, it also becomes the displayed requested intent.
    let active = &acknowledgement.snapshot.active;
    state.current_model = active.model.model.clone();
    state.current_provider_id = active
        .model
        .provider_id
        .clone()
        .or_else(|| state.model_provider_map.get(&state.current_model).cloned());
    state.thinking_effort = active.thinking_effort.clone();
    state.context_window_override = active.context_policy.selection.as_fixed();
    state.current_model_supports_thinking = state
        .available_models_info
        .iter()
        .find(|model| {
            model.id == state.current_model
                && (state.current_provider_id.is_none()
                    || model.provider == state.current_provider_id.as_deref().unwrap_or_default())
        })
        .and_then(|model| model.supports_thinking);
    state.confirmed_runtime_settings = Some(acknowledgement.snapshot.clone());

    let persisted_by_worker = matches!(
        acknowledgement.outcome,
        crate::core::context_policy::RuntimeSettingOutcome::Applied
            | crate::core::context_policy::RuntimeSettingOutcome::Degraded
    );
    if persisted_by_worker {
        state.requested_runtime_settings = Some(acknowledgement.snapshot.requested.clone());
    } else if matches!(
        state.runtime_settings_persistence.as_ref(),
        Some(RuntimeSettingsPersistence::Pending { ui_revision: pending_revision })
            if *pending_revision == ui_revision
    ) {
        // Rejected, failed, and superseded mutations never start persistence. Do not leave a
        // local "Pending" marker that could be mistaken for an outstanding file write.
        state.runtime_settings_persistence = None;
    }
    if ui_revision >= newest_ack_revision {
        state.last_runtime_settings_ack = Some(acknowledgement.clone());
    }

    let active_context =
        crate::core::context_policy::format_context_window(active.context_policy.effective_tokens);
    let detail = acknowledgement
        .reason
        .as_deref()
        .or(active.context_policy.reason.as_deref());
    state.current_status_line = Some(match acknowledgement.outcome {
        crate::core::context_policy::RuntimeSettingOutcome::Applied => {
            format!("Runtime settings applied · context {}", active_context)
        }
        crate::core::context_policy::RuntimeSettingOutcome::Degraded => format!(
            "Runtime settings applied with reduced context {}{}",
            active_context,
            detail
                .map(|reason| format!(" · {}", reason))
                .unwrap_or_default()
        ),
        crate::core::context_policy::RuntimeSettingOutcome::Rejected
        | crate::core::context_policy::RuntimeSettingOutcome::Failed => {
            // 模型切换失败要指名道姓，不能只说 "Runtime settings not applied"。
            // 用户刚按下 Enter、切的是哪个模型、为什么没成 —— 三件事必须一次说清，
            // 否则状态栏那一行等于没说话（这正是"切不动"却查不出原因的那一段）。
            //
            // 判据用 worker 自己报回来的两份状态：`requested` 是用户要的，`active`
            // 是真正生效的。模型切换没成时前者停在用户选的那个、后者仍是原来的，
            // 两者不等即说明"没切过去的那个"就是这次失败的标的 —— 比在 UI 侧
            // 另存一份意图可靠，不用担心两边对不上。
            let requested_model = &acknowledgement.snapshot.requested.model;
            let active_model = &acknowledgement.snapshot.active.model;
            let switch_did_not_take = requested_model.model != active_model.model;
            let failed_model = if switch_did_not_take {
                Some(requested_model.model.as_str())
            } else {
                None
            };
            match failed_model {
                Some(model) => format!(
                    "{} · {}",
                    crate::core::i18n::t(
                        "ui.model.switch_failed",
                        &format!("切换模型到 {} 失败", model),
                        &format!("Switch to model {} failed", model),
                    ),
                    detail.unwrap_or("unknown reason"),
                ),
                None => format!(
                    "Runtime settings not applied{}",
                    detail
                        .map(|reason| format!(" · {}", reason))
                        .unwrap_or_default()
                ),
            }
        }
        crate::core::context_policy::RuntimeSettingOutcome::Superseded => {
            "Runtime settings request superseded by a newer update.".to_string()
        }
    });
}

fn apply_runtime_settings_persistence(
    state: &mut ChatState,
    ui_revision: u64,
    error: Option<String>,
) {
    let newest_known_revision = state
        .last_runtime_settings_ack
        .as_ref()
        .map(|ack| ack.ui_revision)
        .unwrap_or(0)
        .max(
            state
                .pending_runtime_settings
                .back()
                .map(|mutation| mutation.ui_revision)
                .unwrap_or(0),
        );
    if ui_revision == 0 || ui_revision < newest_known_revision {
        return;
    }

    state.runtime_settings_persistence = Some(match error {
        Some(reason) => RuntimeSettingsPersistence::Failed {
            ui_revision,
            reason,
        },
        None => RuntimeSettingsPersistence::Saved { ui_revision },
    });
}

use crate::ui::utils::transcript::append_transcript_event;
use std::time::Instant;
use tokio::sync::mpsc;

pub async fn handle_stream_update(
    state: &mut ChatState,
    update: StreamMessage,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    match update {
        StreamMessage::ReloadTasks => {
            state.task_panel.reload();
        }
        StreamMessage::StatsUpdate {
            au2_compressed,
            token_usage: _,
        } => {
            state.au2_compressed = au2_compressed;
            // Context compression carries an estimate, not a concrete provider response.
            // Keep the latest provider usage and cache telemetry intact.
            append_transcript_event(
                state,
                "stats_update",
                state.active_message_id,
                serde_json::json!({
                    "au2_compressed": state.au2_compressed,
                }),
            );
        }
        StreamMessage::Start { message_id, kind } => {
            let is_fresh_user_turn = matches!(&kind, StreamStartKind::UserTurn { .. })
                && !state.response_models.contains_key(&message_id);
            if let StreamStartKind::UserTurn { model } = &kind {
                state
                    .response_models
                    .entry(message_id)
                    .or_insert_with(|| model.clone());
            }
            if is_fresh_user_turn {
                state.complete_task_message_ids.remove(&message_id);
                state.auto_continued_message_ids.remove(&message_id);
                state.response_costs.remove(&message_id);
                state.cache_warning_shown.clear();
                // 保留上一回合的 provider 用量与缓存读数作基线：新回合的真实 usage
                // 要等响应结束才回来（agent_tool_process 只在回合末发 StatsUpdate），
                // 若在这里清空，UI 只能退回字符数估算，等真实值一到就出现
                // 「~估算 ↔ 真实」的格式与数值横跳。基线在下次 TokenCount 整体替换。
                // token_count 是流内增量计数，语义不跨回合，仍归零。
                state.token_count = 0;
            }
            state
                .stream_targets
                .entry(message_id)
                .or_insert(state.chat_history.len());
            state
                .message_start_indices
                .entry(message_id)
                .or_insert(state.chat_history.len());
            state.active_message_id = Some(message_id);
            state.is_processing = true;
            state.is_streaming = true;
            state.processing_started_at = Some(std::time::Instant::now());
            state.last_token_time = Some(std::time::Instant::now());
            state.thinking_started_at = None;
            // 新一轮请求开始：清掉上一轮遗留的思考结束回顾，避免新 spinner 行闪现
            state.thinking_finished = None;
            state.current_tool_name = None;
            state.auto_follow = true; // Lock to bottom when streaming starts
            state.show_scroll_to_bottom = false;
            emit_status_text(
                state,
                message_id,
                &i18n::t(
                    "ui.status.start",
                    "Status: processing",
                    "Status: processing",
                ),
            );
            append_transcript_event(
                state,
                "start",
                Some(message_id),
                serde_json::json!({
                    "kind": match &kind {
                        StreamStartKind::UserTurn { .. } => "user_turn",
                        StreamStartKind::Operation => "operation",
                    },
                    "queued_inputs": state.pending_user_messages.len(),
                }),
            );
        }
        StreamMessage::RestoreCheckpointApplied {
            message_id,
            checkpoint_id,
            summary,
            chat_history: restored_history,
        } => {
            state.chat_history = restored_history;
            state.stream_targets.clear();
            state.message_start_indices.clear();
            state.tool_started_at.clear();
            state.tool_call_args_cache.clear();
            state.auto_follow = true;
            state
                .chat_list_state
                .select(if state.chat_history.is_empty() {
                    None
                } else {
                    Some(state.chat_history.len() - 1)
                });
            state.active_message_id = Some(message_id);
            emit_status_text(
                state,
                message_id,
                &i18n::t(
                    "ui.status.restore",
                    "Status: restored checkpoint {id}",
                    "Status: restored checkpoint {id}",
                )
                .replace("{id}", &checkpoint_id),
            );
            state.chat_history.push(
                ChatEntry::assistant(
                    i18n::t(
                        "ui.status.restore.summary",
                        "Status: restored checkpoint {id}{summary}",
                        "Status: restored checkpoint {id}{summary}",
                    )
                    .replace("{id}", &checkpoint_id)
                    .replace("{summary}", &summary),
                )
                .with_streaming(false),
            );
            append_transcript_event(
                state,
                "restore_checkpoint",
                Some(message_id),
                serde_json::json!({
                    "checkpoint_id": checkpoint_id,
                    "summary": summary,
                    "restored_entries": state.chat_history.len(),
                }),
            );
        }
        StreamMessage::Content {
            message_id,
            content,
        } => {
            handle_content_message(state, message_id, &content);
        }
        StreamMessage::TextDelta {
            message_id,
            content,
        } => {
            let content = sanitize_for_tui(&content);
            state.last_token_time = Some(std::time::Instant::now());
            if let Some(&idx0) = state.stream_targets.get(&message_id) {
                let mut idx = idx0;
                if idx < state.chat_history.len() {
                    let current_type = &state.chat_history[idx].entry_type;
                    let needs_new = *current_type == ChatEntryType::ToolCall
                        || *current_type == ChatEntryType::ToolResult
                        || *current_type == ChatEntryType::ToolConfirmation
                        || (*current_type == ChatEntryType::Assistant
                            && idx < state.chat_history.len() - 1)
                        || (*current_type == ChatEntryType::Assistant
                            && state.chat_history[idx].is_streaming != Some(true));
                    if needs_new {
                        idx = state.chat_history.len();
                        state.stream_targets.insert(message_id, idx);
                    }
                }
                if idx == state.chat_history.len() {
                    state
                        .chat_history
                        .push(ChatEntry::assistant("").with_streaming(true));
                    state.stream_targets.insert(message_id, idx);
                }
                if idx < state.chat_history.len() {
                    state.chat_history[idx].content.push_str(&content);
                    state.rendered_cache.remove(&idx);
                    state.virtual_list.mark_dirty(idx);
                    // Show scroll-to-bottom indicator when user scrolled up during streaming
                    if !state.auto_follow {
                        state.show_scroll_to_bottom = true;
                    }
                }
            }
        }
        StreamMessage::ReasoningDelta {
            message_id,
            content,
        } => {
            let content = sanitize_for_tui(&content);
            state.last_token_time = Some(std::time::Instant::now());
            if let Some(&idx0) = state.stream_targets.get(&message_id) {
                let mut idx = idx0;
                if idx < state.chat_history.len() {
                    let current_type = &state.chat_history[idx].entry_type;
                    let needs_new = *current_type == ChatEntryType::ToolCall
                        || *current_type == ChatEntryType::ToolResult
                        || *current_type == ChatEntryType::ToolConfirmation
                        || (*current_type == ChatEntryType::Assistant
                            && idx < state.chat_history.len() - 1)
                        || (*current_type == ChatEntryType::Assistant
                            && state.chat_history[idx].is_streaming != Some(true));
                    if needs_new {
                        idx = state.chat_history.len();
                        state.stream_targets.insert(message_id, idx);
                    }
                }
                if idx == state.chat_history.len() {
                    state.chat_history.push(
                        ChatEntry::assistant("")
                            .with_streaming(true)
                            .with_reasoning(""),
                    );
                    state.stream_targets.insert(message_id, idx);
                }
                if idx < state.chat_history.len() {
                    let entry = &mut state.chat_history[idx];
                    if entry.reasoning_content.is_none() {
                        entry.reasoning_content = Some(String::new());
                    }
                    if let Some(rc) = &mut entry.reasoning_content {
                        rc.push_str(&content);
                    }
                    state.rendered_cache.remove(&idx);
                    state.virtual_list.mark_dirty(idx);
                }
            }
        }
        StreamMessage::Thinking {
            message_id,
            content,
        } => {
            let content = sanitize_for_tui(&content);
            state.last_token_time = Some(std::time::Instant::now());
            if state.thinking_started_at.is_none() {
                state.thinking_started_at = Some(std::time::Instant::now());
            }
            if let Some(&idx0) = state.stream_targets.get(&message_id) {
                let mut idx = idx0;
                let mut should_create_new = false;
                if idx < state.chat_history.len() {
                    let current_type = &state.chat_history[idx].entry_type;
                    if *current_type == ChatEntryType::ToolCall
                        || *current_type == ChatEntryType::ToolResult
                        || *current_type == ChatEntryType::ToolConfirmation
                    {
                        should_create_new = true;
                    }
                    if *current_type == ChatEntryType::Assistant {
                        if idx < state.chat_history.len() - 1 {
                            should_create_new = true;
                        }
                        if state.chat_history[idx].is_streaming != Some(true) {
                            should_create_new = true;
                        }
                    }
                } else {
                    should_create_new = true;
                }
                if should_create_new {
                    idx = state.chat_history.len();
                    state.stream_targets.insert(message_id, idx);
                }
                if idx == state.chat_history.len() {
                    state.chat_history.push(
                        ChatEntry::assistant("")
                            .with_streaming(true)
                            .with_reasoning(""),
                    );
                }
                if idx < state.chat_history.len() {
                    let entry = &mut state.chat_history[idx];
                    if entry.reasoning_content.is_none() {
                        entry.reasoning_content = Some(String::new());
                    }
                    if let Some(rc) = &mut entry.reasoning_content {
                        rc.push_str(&content);
                    }
                    state.rendered_cache.remove(&idx);
                    state.virtual_list.mark_dirty(idx);
                }
            }
        }
        StreamMessage::AssistantNote {
            message_id,
            content,
        } => {
            let note = content.trim();
            if note.is_empty() {
                return Ok(());
            }
            emit_status_text(state, message_id, note);
            if note.starts_with("Warning:") || note.starts_with("Error:") {
                state
                    .chat_history
                    .push(ChatEntry::assistant(note).with_streaming(false));
            }
            append_transcript_event(
                state,
                "assistant_note",
                Some(message_id),
                serde_json::json!({
                    "content": note,
                }),
            );
        }
        StreamMessage::Trace {
            message_id,
            event,
            payload,
        } => {
            match event.as_str() {
                "model_request_preparing" => {
                    emit_status_text(
                        state,
                        message_id,
                        &i18n::t(
                            "ui.status.model_prepare",
                            "Status: preparing model request",
                            "Status: preparing model request",
                        ),
                    );
                }
                "model_request_started" => {
                    state.model_wait_started_at = Some(Instant::now());
                    let prepare_elapsed_ms = payload
                        .get("prepare_elapsed_ms")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let status = if prepare_elapsed_ms > 0 {
                        i18n::t(
                            "ui.status.model_wait",
                            "Status: local request prep finished ({ms}ms), connecting to provider",
                            "Status: local request prep finished ({ms}ms), connecting to provider",
                        )
                        .replace("{ms}", &prepare_elapsed_ms.to_string())
                    } else {
                        i18n::t(
                            "ui.status.model_wait_no_ms",
                            "Status: local request prep finished, connecting to provider",
                            "Status: local request prep finished, connecting to provider",
                        )
                    };
                    emit_status_text(state, message_id, &status);
                }
                "provider_request_started" => {
                    state.model_wait_started_at = Some(Instant::now());
                    emit_status_text(
                        state,
                        message_id,
                        &i18n::t(
                            "ui.status.provider_send",
                            "Status: sending request to provider",
                            "Status: sending request to provider",
                        ),
                    );
                }
                "provider_response_headers" => {
                    let elapsed_ms = payload
                        .get("elapsed_ms")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    state.model_wait_started_at = Some(Instant::now());
                    let status = if elapsed_ms > 0 {
                        i18n::t(
                            "ui.status.provider_headers",
                            "Status: provider responded ({ms}ms), waiting for first token",
                            "Status: provider responded ({ms}ms), waiting for first token",
                        )
                        .replace("{ms}", &elapsed_ms.to_string())
                    } else {
                        i18n::t(
                            "ui.status.provider_headers_no_ms",
                            "Status: provider responded, waiting for first token",
                            "Status: provider responded, waiting for first token",
                        )
                    };
                    emit_status_text(state, message_id, &status);
                }
                "provider_first_byte" => {
                    let elapsed_ms = payload
                        .get("elapsed_ms")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    state.model_wait_started_at = Some(Instant::now());
                    let status = if elapsed_ms > 0 {
                        i18n::t(
                            "ui.status.provider_first_byte",
                            "Status: first response bytes received ({ms}ms), waiting for first token",
                            "Status: first response bytes received ({ms}ms), waiting for first token",
                        )
                        .replace("{ms}", &elapsed_ms.to_string())
                    } else {
                        i18n::t(
                            "ui.status.provider_first_byte_no_ms",
                            "Status: first response bytes received, waiting for first token",
                            "Status: first response bytes received, waiting for first token",
                        )
                    };
                    emit_status_text(state, message_id, &status);
                }
                "model_first_chunk" => {
                    state.model_wait_started_at = None;
                    let elapsed_ms = payload
                        .get("elapsed_ms")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0);
                    let status = if elapsed_ms > 0 {
                        i18n::t(
                            "ui.status.model_ready",
                            "Status: model started responding ({ms}ms)",
                            "Status: model started responding ({ms}ms)",
                        )
                        .replace("{ms}", &elapsed_ms.to_string())
                    } else {
                        i18n::t(
                            "ui.status.model_ready_no_ms",
                            "Status: model started responding",
                            "Status: model started responding",
                        )
                    };
                    emit_status_text(state, message_id, &status);
                }
                "agent_status" => {
                    // Keep the STALL watchdog alive during agent_status events
                    // (e.g. rate-limit cooldown). Without this the watchdog
                    // clears is_processing after 30s even though the agent
                    // is still working.
                    state.last_token_time = Some(std::time::Instant::now());
                    let msg = payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !msg.is_empty() {
                        let status = i18n::t("ui.status.agent", "Status: {msg}", "Status: {msg}")
                            .replace("{msg}", msg);
                        emit_status_text(state, message_id, &status);
                    }
                }
                "auto_compact" => {
                    // 对标 Claude Code「auto-compact on/off」闪现：compact 发生时
                    // 给用户一个实时提示。payload 由 agent_loop.rs 的
                    // run_compression_check 发出。
                    if let Some(on) = payload.get("on").and_then(|value| value.as_bool()) {
                        if on {
                            state.push_toast("Auto-compact on", ToastKind::Success);
                        } else {
                            state.push_toast("Auto-compact off", ToastKind::Info);
                        }
                    }
                }
                _ => {}
            }
            append_transcript_event(state, &event, Some(message_id), payload);
        }
        StreamMessage::ToolCalls {
            message_id,
            tool_calls,
        } => {
            let _start_idx = state
                .message_start_indices
                .get(&message_id)
                .copied()
                .unwrap_or(0);
            // Get the current associated Assistant message index
            let assistant_idx = match state.stream_targets.get(&message_id).copied() {
                Some(v) => v,
                None => return Ok(()),
            };
            if tool_calls.is_empty() {
                return Ok(());
            }
            // 1. End the current Assistant message's streaming state
            let had_thinking = if let Some(entry) = state.chat_history.get(assistant_idx) {
                entry.entry_type == ChatEntryType::Assistant && entry.reasoning_content.is_some()
            } else {
                false
            };
            finalize_entry_streaming(state, assistant_idx);

            // 2. Append tool calls with individual transition messages
            // This creates: thinking → explain1 → tool1 → explain2 → tool2 → ...
            let mut _insert_pos = state.chat_history.len();
            for (i, tc) in tool_calls.iter().cloned().enumerate() {
                // Skip if this tool call already exists (e.g., re-emitted after confirmation)
                let existing_idx = state.chat_history.iter().position(|e| {
                    e.entry_type == ChatEntryType::ToolCall
                        && e.tool_call.as_ref().map(|t| t.id == tc.id).unwrap_or(false)
                });
                if let Some(idx) = existing_idx {
                    // Still update tracking state for the existing entry
                    state
                        .tool_started_at
                        .entry(tc.id.clone())
                        .or_insert_with(Instant::now);
                    state
                        .tool_call_args_cache
                        .insert(tc.id.clone(), tc.function.arguments.clone());
                    state.current_tool_name = Some(tc.function.name.clone());
                    // Mark existing entry as streaming again
                    state.chat_history[idx].is_streaming = Some(true);
                    state.rendered_cache.remove(&idx);
                    state.virtual_list.mark_dirty(idx);
                    continue;
                }
                // Add the tool call itself
                state
                    .tool_started_at
                    .entry(tc.id.clone())
                    .or_insert_with(Instant::now);
                state
                    .tool_call_args_cache
                    .insert(tc.id.clone(), tc.function.arguments.clone());
                // Track current tool name for spinner display
                state.current_tool_name = Some(tc.function.name.clone());
                state.chat_history.push(
                    ChatEntry::tool_call(format_tool_call(&tc), tc.clone()).with_streaming(true),
                );
                // 状态更新：Running {tool}
                emit_status_text(
                    state,
                    message_id,
                    &format!(
                        "Running {}",
                        format_running_tool_label(state, &tc.id, &tc.function.name),
                    ),
                );
                append_transcript_event(
                    state,
                    "tool_call",
                    Some(message_id),
                    serde_json::json!({
                        "tool_call_id": tc.id,
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }),
                );
                _insert_pos += 1;
            }
            // 3. Update stream_targets to point to the next position
            // This way, when the next Content message arrives, it will create a new Assistant Entry because idx == len
            state
                .stream_targets
                .insert(message_id, state.chat_history.len());
            // Auto-scroll to bottom
            if state.auto_follow {
                state
                    .chat_list_state
                    .select(Some(state.chat_history.len().saturating_sub(1)));
            }
        }
        StreamMessage::ToolResult {
            message_id,
            tool_call,
            tool_result,
        } => {
            handle_tool_result_message(state, message_id, tool_call, tool_result);
        }
        StreamMessage::ToolOutput {
            message_id,
            tool_call_id,
            output,
        } => {
            let output = sanitize_for_tui(&output);
            let start_idx = state
                .message_start_indices
                .get(&message_id)
                .copied()
                .unwrap_or(0);
            // Search backwards for matching tool call
            let search_end_idx = state.chat_history.len();
            let mut found: Option<usize> = None;
            for i in (start_idx..search_end_idx).rev() {
                let e = &state.chat_history[i];
                if e.entry_type == ChatEntryType::ToolCall {
                    if let Some(tc) = &e.tool_call {
                        // Check if id matches OR name matches (since we passed name as id)
                        if tc.id == tool_call_id || tc.function.name == tool_call_id {
                            found = Some(i);
                            break;
                        }
                    }
                }
            }
            if let Some(idx) = found {
                let active_tool_id = state.chat_history[idx]
                    .tool_call
                    .as_ref()
                    .map(|tool_call| tool_call.id.as_str());
                // 仅已登记工具的真实进度可以维持请求存活；迟到的输出不能掩盖完成后的停滞。
                if active_tool_id
                    .is_some_and(|active_id| state.tool_started_at.contains_key(active_id))
                {
                    state.last_token_time = Some(Instant::now());
                }
                // 更新流式状态和缓存（不再往 ToolCall 的 content 追加输出，避免与 ToolResult 重复）
                state.rendered_cache.remove(&idx);
                state.virtual_list.mark_dirty(idx);

                if let Some(tc) = state.chat_history[idx].tool_call.as_ref() {
                    let detail = output
                        .lines()
                        .rev()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or_else(|| output.trim());

                    if !detail.trim().is_empty() {
                        let tool_label =
                            format_running_tool_label(state, &tc.id, &tc.function.name);
                        let status = format!(
                            "Running {} · {}",
                            tool_label,
                            truncate_status_detail(detail.trim(), 90),
                        );
                        emit_status_text(state, message_id, &status);
                    }
                }
            }
        }
        StreamMessage::TokenCount {
            message_id,
            tokens,
            usage,
        } => {
            // 终态之后的 drain/重复 chunk 不属于新的 provider 响应，不能污染显示或再次计费。
            if state.complete_task_message_ids.contains(&message_id) {
                return Ok(());
            }

            state.token_count = tokens;
            if let Some(ref u) = usage {
                // 每个 TokenCount 带的是一次具体 provider 响应，必须整体替换。
                // 不能将工具循环中不同模型调用的 prompt/completion/cache 字段拼成一条。
                state.token_usage = Some(u.clone());
                // 当前响应未明确报告缓存计数时，不能保留上一响应的数字。
                state.cache_read_tokens = 0;
                state.cache_creation_tokens = 0;
                if u.cache_telemetry_reported {
                    state.cache_read_tokens = u.cache_read_tokens as u64;
                    state.cache_creation_tokens = u.cache_creation_tokens as u64;
                }

                // 显示的是最近一次响应；费用则必须累计同一逻辑请求的每次真实 provider 响应。
                let model = state
                    .response_models
                    .get(&message_id)
                    .map(String::as_str)
                    .unwrap_or(state.current_model.as_str());
                let response_cost = crate::ui::utils::cost::compute_response_cost(u, model);
                *state.response_costs.entry(message_id).or_insert(0.0) += response_cost;

                // 只有具备明确来源的缓存遥测才能触发提示；缺失字段的零值不表示缓存未命中。
                if u.cache_telemetry_reported {
                    let detector = crate::agent::cache_warning::CacheWarningDetector::new();
                    if let Some(warning) = detector.warning_for_usage(u) {
                        let warning_key = (message_id, warning.warning_type);
                        if state.cache_warning_shown.insert(warning_key) {
                            state.push_toast(&warning.message, ToastKind::Warning);
                        }
                    }
                }
            }
        }
        StreamMessage::Done { message_id } => {
            handle_done_message(state, agent_tx, message_id).await?
        }
        StreamMessage::Error { message_id, error } => {
            handle_error_message(state, agent_tx, message_id, error).await?
        }
        StreamMessage::ModelsList {
            models,
            cache_age_secs,
        } => {
            apply_models_to_state(state, &models, cache_age_secs);
            if let Some(pid) = state.pending_model_provider.take() {
                let remembered_model = state.pending_provider_selected_model.take();

                if let Some(model) = remembered_model
                    .filter(|model| models.iter().any(|m| m.id == *model && m.provider == pid))
                {
                    let provider_id = state
                        .model_provider_map
                        .get(&model)
                        .cloned()
                        .or_else(|| Some(pid.clone()));
                    let _ = crate::ui::events::input::request_runtime_model_change(
                        state,
                        model,
                        provider_id,
                        agent_tx,
                    )
                    .await;
                } else {
                    state.current_provider_id = Some(pid.clone());
                    state.current_model.clear();
                    emit_status_text(
                        state,
                        0,
                        &i18n::t(
                            "ui.status.provider.awaiting_model",
                            "Status: switched to {provider}; choose a model for this provider",
                            "Status: switched to {provider}; choose a model for this provider",
                        )
                        .replace("{provider}", &pid),
                    );
                }
            }
            state.awaiting_models = false;
            state.awaiting_models_since = None;
            // 自动开面板前先确认用户没有在别处输入。
            //
            // 此前只要模型列表回来了就无条件 open_palette(Model)，会把用户正填的
            // API Key 输入框（Modal::InputModal）从栈顶顶掉却不走 exit_input_modal：
            // 面板画出来了，但 handle_modal_key 见到 show_input_modal 就让位，
            // handle_input_modal 继续吞掉所有按键 —— 「面板看得见、却什么都按不动」。
            // 同理，已开着 Provider 页时也不该被异步响应换成 Model 页。
            //
            // 刻意**不**用 `KeyContext::is_exclusive()`：它把建议浮层也算成遮挡层，
            // 而打 `/model` 时命令补全浮层必然是开着的，会把正常流程一起挡掉。
            // `is_dialog()` 只认真正占屏幕的对话框：模态栈上的窗口、输入框、工具审批。
            let user_is_elsewhere =
                crate::ui::events::key_context::KeyContext::of(state).is_dialog();
            // If Palette is open in Model mode, refresh the list
            if state.is_palette_open()
                && matches!(
                    state.palette_mode,
                    crate::ui::state::palette::PaletteMode::Model
                )
            {
                state.palette_items = crate::ui::components::palette::get_model_palette_items(
                    &state.available_models,
                    &state.current_model,
                    state.awaiting_models,
                    &state.model_provider_map,
                    state.models_list_age_secs(),
                );
            }
            // Fallback: models arrived, open palette in Model mode for selection
            else if !state.available_models.is_empty() && !user_is_elsewhere {
                state.palette_history.clear();
                state.open_palette(crate::ui::state::palette::PaletteMode::Model);
                state.palette_items = crate::ui::components::palette::get_model_palette_items(
                    &state.available_models,
                    &state.current_model,
                    state.awaiting_models,
                    &state.model_provider_map,
                    state.models_list_age_secs(),
                );
                state.selected_palette_index = 0;
                state.palette_filter.clear();
            }
        }
        StreamMessage::ModelsError(err) => {
            state.awaiting_models = false;
            state.awaiting_models_since = None;
            state.chat_history.push(
                ChatEntry::assistant(
                    i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                        .replace("{error}", &err),
                )
                .with_streaming(false),
            );
            emit_status_text(
                state,
                0,
                &i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                    .replace("{error}", &err),
            );
        }
        StreamMessage::McpStatus { ready, error } => {
            state.mcp_ready = ready;
            if let Some(e) = error.as_deref() {
                emit_status_text(
                    state,
                    0,
                    &i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                        .replace("{error}", &e),
                );
            }
        }
        StreamMessage::McpServers(_) => {}
        StreamMessage::McpTools {
            server: _,
            tools: _,
        } => {}
        StreamMessage::PluginOpResult { message } => {
            // 插件市场后台操作完成：清 pending、回填消息并刷新列表。
            // 批量安装时聚合进度（对标 Claude Code 批量安装的逐项状态）：
            // 每个结果计一次进度，全部完成才刷新列表
            state.plugin_op_pending = false;
            state.plugin_loading = false;
            if state.plugin_batch_total > 0 {
                state.plugin_batch_done += 1;
                let done = state.plugin_batch_done;
                let total = state.plugin_batch_total;
                let last = message.unwrap_or_default();
                if done >= total {
                    state.plugin_batch_total = 0;
                    state.plugin_batch_done = 0;
                    state.plugin_message = Some(format!(
                        "Batch install finished ({}/{}). Last: {}",
                        done, total, last
                    ));
                } else {
                    state.plugin_message =
                        Some(format!("Installing {}/{}... ({})", done, total, last));
                    return Ok(());
                }
            } else if let Some(m) = message {
                state.plugin_message = Some(m);
            }
            state.plugin_selected.clear();
            state.reload_plugins_state().await;
        }
        StreamMessage::GlobalSearchResults {
            request_id,
            results,
            truncated,
        } => {
            // 只允许仍处于前台且请求 ID 匹配的弹窗消费结果。
            if matches!(
                state.top_modal(),
                Some(crate::ui::state::modal::Modal::GlobalSearch)
            ) {
                state
                    .global_search_state
                    .apply_results(request_id, results, truncated);
            }
        }
        StreamMessage::NoteGenerated {
            message_id: _,
            kind,
            content,
        } => {
            // /summary、/recap 旁路生成结果：作为独立助手条目展示（不进入主上下文）
            state.current_status_line = None;
            state.is_processing = false;
            let note = content.trim();
            if note.is_empty() {
                return Ok(());
            }
            let entry = if note.starts_with("⚠️") {
                note.to_string()
            } else {
                format!("**{}**\n\n{}", kind.label(), note)
            };
            state
                .chat_history
                .push(ChatEntry::assistant(entry).with_streaming(false));
        }
        StreamMessage::ConfiguredProviders(ids) => {
            state.configured_providers = ids.into_iter().collect();
            // 顺手算出"没填密钥"的那批。必须现算：key 可能来自环境变量，也可能
            // 刚在 provider 表单里补上，存成配置里的字段会立刻过期。
            //
            // 读盘失败时退化成"全都算有凭证"—— 那只是少一个标记，不能反过来把
            // 所有 provider 都标成缺密钥、让用户以为配置全坏了。
            let store = crate::core::config::provider_store::ProviderStore::new();
            state.keyless_providers = match store.load().await {
                Ok(config) => state
                    .configured_providers
                    .iter()
                    .filter(|id| {
                        !crate::core::config::providers::provider_has_usable_credentials(
                            id,
                            config.providers.get(id.as_str()),
                        )
                    })
                    .cloned()
                    .collect(),
                Err(_) => Vec::new(),
            };
        }
        StreamMessage::CurrentModelChanged { model, provider_id } => {
            // Legacy startup/provider-listing notifications cannot overwrite a newer
            // revisioned UI request or the worker-confirmed runtime snapshot.
            if state.next_runtime_settings_revision > 1
                || !state.pending_runtime_settings.is_empty()
                || state.confirmed_runtime_settings.is_some()
            {
                return Ok(());
            }
            state.current_model = model;
            state.current_provider_id =
                provider_id.or_else(|| state.model_provider_map.get(&state.current_model).cloned());
            // 从可用模型列表中查找当前模型是否支持 thinking
            state.current_model_supports_thinking = state
                .available_models_info
                .iter()
                .find(|m| m.id == state.current_model)
                .and_then(|m| m.supports_thinking);
        }
        StreamMessage::RuntimeSettingsAcknowledged(acknowledgement) => {
            apply_runtime_settings_acknowledgement(state, acknowledgement);
        }
        StreamMessage::RuntimeSettingsPersistenceCompleted { ui_revision, error } => {
            apply_runtime_settings_persistence(state, ui_revision, error);
        }
        StreamMessage::ApprovalModeChanged { mode } => {
            state.approval_mode = mode.clone();
            let mode_name = match mode {
                crate::types::ApprovalMode::Default => i18n::t("ui.mode.build", "Build", "Build"),
                crate::types::ApprovalMode::AcceptEdits => {
                    i18n::t("ui.mode.accept_edits", "Accept edits", "Accept edits")
                }
                crate::types::ApprovalMode::Plan => i18n::t("ui.mode.plan", "Plan", "Plan"),
                crate::types::ApprovalMode::Yolo => i18n::t("ui.mode.yolo", "YOLO", "YOLO"),
            };
            // 这里**不再**往 chat_history 推一条 assistant 条目。
            //
            // 之前推的那条是 `"...changed to: " + mode_name + mode_desc`，而
            // `mode_desc` 本身又以 "Build mode: " 开头，于是渲染出来是
            // `...changed to: BuildBuild mode: confirm dangerous actions; ...`
            // —— 模式名重复，且整条近 80 字符，压在欢迎抬头下面很占版面。
            //
            // 模式是持续状态而不是一次对话，底部状态栏已经一直在显示它
            // （见下面的 emit_status_text），聊天区再留一条纯属重复。
            emit_status_text(
                state,
                0,
                &i18n::t(
                    "ui.status.approval.current",
                    "Status: approval mode {name}",
                    "Status: approval mode {name}",
                )
                .replace("{name}", &mode_name),
            );
        }
        StreamMessage::UpdateGitStatus(status) => {
            state.git_status = Some(status);
        }
        StreamMessage::ToolConfirmationRequest {
            message_id,
            tool_call_id,
            originating_tool_call_id,
            confirmation,
        } => {
            let awaiting = confirmation.outcome.is_none();
            let is_ask = matches!(
                confirmation.operation_type,
                crate::types::ConfirmationType::AskUserQuestion
            );
            // 已经有一张没人回答的卡时，**不许把它的内容换成新的请求**。
            //
            // 换掉之后卡片上写的是新命令、等着回答的却是新 id，而用户是照着刚
            // 读到的那条命令按的 1 —— 于是"允许"落到一条他没看过的命令上。
            // 门控拦不住的不是命令，是"用户批准的那一条"和"实际执行的那一条"
            // 之间的对应关系。
            //
            // 但旧的请求**不能丢**：并发工具（`join_all`）可能同时发出多张卡，
            // 每张背后都有一个正在等待答复的门禁。以前把旧卡标成 superseded 就
            // 不管了，于是那几条调用永远等下去 —— 整轮卡死在 "Canoodling..."。
            // 现在把旧卡放进队列，新的这张先问；回答完再一张张接上。
            if awaiting && state.is_awaiting_confirmation {
                if let Some(idx) = state.pending_confirmation_entry_idx {
                    if idx < state.chat_history.len()
                        && state.chat_history[idx].entry_type == ChatEntryType::ToolConfirmation
                    {
                        let active_is_ask = state.chat_history[idx]
                            .confirmation
                            .as_ref()
                            .map(|c| {
                                matches!(
                                    c.operation_type,
                                    crate::types::ConfirmationType::AskUserQuestion
                                )
                            })
                            .unwrap_or(false);
                        if let Some(entry) = state.chat_history[idx].confirmation.as_mut() {
                            if entry.outcome.is_none() {
                                entry.outcome = Some(i18n::t(
                                    "ui.confirm.queued",
                                    "排队中 · 回答完当前请求后继续",
                                    "Queued — will ask after the current decision",
                                ));
                            }
                        }
                        state.chat_history[idx].is_streaming = Some(false);
                        state.rendered_cache.remove(&idx);
                        state.virtual_list.mark_dirty(idx);
                        if let Some(tool_call_id) = state.pending_tool_call_id.clone() {
                            state
                                .pending_confirmation_queue
                                .push_front(QueuedConfirmation {
                                    entry_idx: idx,
                                    message_id: state.pending_message_id.unwrap_or(message_id),
                                    tool_call_id,
                                    originating_tool_call_id: state
                                        .pending_confirmation_originating_id
                                        .clone(),
                                    is_ask: active_is_ask,
                                });
                        }
                    }
                }
            }
            let idx = state.chat_history.len();
            state.chat_history.push(
                ChatEntry::new(ChatEntryType::ToolConfirmation, String::new())
                    .with_confirmation(confirmation)
                    .with_streaming(false),
            );
            // New status update
            if awaiting {
                state.pending_confirmation_entry_idx = Some(idx);
                state.pending_message_id = Some(message_id);
                state.is_awaiting_confirmation = true;
                state.pending_tool_call_id = Some(tool_call_id);
                state.pending_confirmation_originating_id = originating_tool_call_id;
                state.confirmation_feedback_mode = false;
                state.pending_confirmation_feedback.clear();
                state.pending_confirmation_choice = if is_ask {
                    0 // 0-based: first option
                } else {
                    1 // 1-based: "Allow once" is option 1
                };
            } else if !state.is_awaiting_confirmation {
                state.pending_confirmation_entry_idx = Some(idx);
                state.pending_message_id = Some(message_id);
                state.pending_tool_call_id = None;
                state.pending_confirmation_originating_id = None;
                state.pending_confirmation_choice = 0;
            }

            // Auto-scroll to bottom to display confirmation card
            state.chat_list_state.select(Some(idx));
            if state.auto_follow {
                // Force scroll to bottom calculation in next render
            }
        }
        StreamMessage::StatusUpdate {
            message_id: _,
            status,
        } => {
            state.current_status_line = Some(status);
        }
        StreamMessage::AgentTaskUpdate {
            message_id: _,
            task_id,
            agent_type,
            description,
            status,
            tool_use_count,
            tokens,
            is_async,
            is_resolved,
            is_error,
            last_tool_info,
            name,
            task_description,
            new_sub_entries,
        } => {
            handle_agent_task_update(
                state,
                AgentTaskUpdateArgs {
                    task_id,
                    agent_type,
                    description,
                    status,
                    tool_use_count,
                    tokens,
                    is_async,
                    is_resolved,
                    is_error,
                    last_tool_info,
                    name,
                    task_description,
                    new_sub_entries,
                },
            );
        }
    }
    Ok(())
}

/// 同一批并发 Agent 的归组时间窗。
///
/// 对标 Claude Code 把同一条 assistant 消息里的多个 Agent tool_use 合并成
/// 一个 group 渲染；这里用「相邻启动」近似「同一批并发」。
const AGENT_GROUP_WINDOW_MS: i64 = 3000;

/// 一次 Agent 进度更新的全部字段（原先是 12 个位置参数）
pub(crate) struct AgentTaskUpdateArgs {
    pub task_id: String,
    pub agent_type: String,
    pub description: String,
    pub status: crate::types::AgentTaskStatus,
    pub tool_use_count: u32,
    pub tokens: u32,
    pub is_async: bool,
    pub is_resolved: bool,
    pub is_error: bool,
    pub last_tool_info: Option<String>,
    pub name: Option<String>,
    pub task_description: Option<String>,
    pub new_sub_entries: Vec<crate::types::ChatEntry>,
}

async fn handle_done_message(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    message_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if state.complete_task_message_ids.contains(&message_id) {
        return Ok(());
    }

    crate::utils::logging::append_debug_log_line(&format!(
        "[DEBUG] StreamHandler: Received Done message (message_id={})",
        message_id
    ));
    recover_missing_tool_results(state, message_id, "stream_done");
    let assistant_idx = state.stream_targets.get(&message_id).copied();
    // 这一轮正常收尾，Esc 重发的判据随之作废。
    state.last_turn_failed = false;

    let cancelling_graceful = state
        .cancelling_since
        .map(|t| t.elapsed() < std::time::Duration::from_millis(1500))
        .unwrap_or(false);

    if let Some(idx) = assistant_idx {
        if idx < state.chat_history.len()
            && state.chat_history[idx].entry_type == ChatEntryType::Assistant
            && !cancelling_graceful
        {
            finalize_entry_streaming(state, idx);
        }
    }
    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);
    let has_tools = assistant_idx
        .map(|ai| {
            let end = ai.min(state.chat_history.len());
            (start_idx, end)
        })
        .map(|(s, e)| {
            state.chat_history[s..e].iter().any(|ce| {
                ce.entry_type == ChatEntryType::ToolCall
                    || ce.entry_type == ChatEntryType::ToolResult
            })
        })
        .unwrap_or(false);
    let assistant_empty = assistant_idx
        .and_then(|idx| state.chat_history.get(idx))
        .map(|ce| ce.content.trim().is_empty())
        .unwrap_or(true);
    // Auto-continue keeps the same message ID, so this logical request retains its cost and model.
    let can_auto_continue = state.auto_continue_enabled
        && state.auto_continue_remaining > 0
        && has_tools
        && !state.auto_continued_message_ids.contains(&message_id);
    if can_auto_continue && !cancelling_graceful {
        state.auto_continue_remaining = state.auto_continue_remaining.saturating_sub(1);
        state.auto_continued_message_ids.insert(message_id);
        if let Some(idx) = assistant_idx {
            if idx < state.chat_history.len() {
                state.chat_history[idx].is_streaming = Some(true);
            }
        }
        state.is_processing = true;
        state.is_streaming = true;
        state.active_message_id = Some(message_id);

        let continue_msg = i18n::t(
            "ui.status.continue",
            "Status: continuing final response",
            "Status: continuing final response",
        );
        state
            .chat_history
            .push(ChatEntry::assistant(format!("⟳ {}", continue_msg)).with_streaming(false));
        emit_status_text(state, message_id, &continue_msg);
        append_transcript_event(
            state,
            "auto_continue",
            Some(message_id),
            serde_json::json!({
                "remaining": state.auto_continue_remaining,
                "has_tools": has_tools,
                "assistant_empty": assistant_empty,
            }),
        );
        let _ = agent_tx
            .send(AgentRequest::SendMessage {
                message_id,
                message: i18n::t(
                    "agent.auto_continue.prompt",
                    "Continue working. If the task is complete, provide a concise summary. If not, use the available tools to continue making progress.",
                    "Continue working. If the task is complete, provide a concise summary. If not, use the available tools to continue making progress.",
                ),
            })
            .await;
        return Ok(());
    }

    if let Some(idx) = assistant_idx {
        if idx < state.chat_history.len() {
            if !cancelling_graceful {
                state.chat_history[idx].is_streaming = Some(false);
            }
            state.rendered_cache.remove(&idx);
            state.last_rendered_stream_key.remove(&idx);
            state.virtual_list.mark_dirty(idx);
        }
    }
    settle_response_cost(state, message_id, assistant_idx);
    finish_terminal_stream(state, message_id, cancelling_graceful);
    if !cancelling_graceful {
        state.current_status_line = Some("✓ Done".to_string());
    }

    if let Some(idx) = assistant_idx.filter(|&idx| idx < state.chat_history.len()) {
        let content = &state.chat_history[idx].content;
        state.last_code_block_content = extract_last_code_block(content);
    }

    append_transcript_event(
        state,
        "done",
        Some(message_id),
        serde_json::json!({
            "auto_continued": false,
        }),
    );
    dequeue_pending_user_input(state, agent_tx).await
}

/// 终态状态仅允许结算一次；错误和完成路径共享清理逻辑。
fn finish_terminal_stream(state: &mut ChatState, message_id: u64, cancelling_graceful: bool) {
    // 回合结束（正常 Done 或错误）后，排队中的确认请求不可能再被回答；
    // 留着它们只会让下一轮收到新请求时把它们当成有效队列项。
    state.discard_pending_confirmation_queue(&i18n::t(
        "ui.confirm.cancelled_turn",
        "已取消 · 回合已结束",
        "Cancelled — the turn ended",
    ));
    state.is_processing = false;
    state.current_tool_name = None;
    state.end_thinking();
    state.last_token_time = None;
    if !cancelling_graceful {
        state.is_streaming = false;
    }
    state.model_wait_started_at = None;
    state.processing_started_at = None;
    state.active_message_id = Some(message_id);
    state.complete_task_message_ids.insert(message_id);
    state.stream_targets.remove(&message_id);
    state.message_start_indices.remove(&message_id);
}

async fn dequeue_pending_user_input(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !state.is_awaiting_confirmation {
        if let Some(next_input) = state.pending_user_messages.pop_front() {
            enqueue_user_message(state, next_input, agent_tx).await?;
        }
    }
    Ok(())
}

async fn handle_error_message(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    message_id: u64,
    error: String,
) -> Result<(), Box<dyn std::error::Error>> {
    if state.complete_task_message_ids.contains(&message_id) {
        return Ok(());
    }

    recover_missing_tool_results(state, message_id, &error);
    let assistant_idx = state.stream_targets.get(&message_id).copied();
    let cancelling_graceful = state
        .cancelling_since
        .map(|t| t.elapsed() < std::time::Duration::from_millis(1500))
        .unwrap_or(false);

    if !cancelling_graceful {
        if let Some(idx) = assistant_idx {
            if idx < state.chat_history.len()
                && state.chat_history[idx].entry_type == ChatEntryType::Assistant
            {
                finalize_entry_streaming(state, idx);
            }
        }
    }

    let error_type = crate::ui::components::error_overlay::classify_error(&error);
    // 每一类错误都弹浮层，不只"可重试"的那几类。
    //
    // 以前 Auth / Quota / Provider 错误只弹一条 3 秒 toast 就消失，于是 401/403
    // 之后界面上**什么都不剩**：没有浮层的 `r`（唯一能重发的入口），也没有别的
    // 重试键（`/retry` 不存在，Esc 与重试无关）。而这恰恰是最需要"改完 provider
    // 再试一次"的错误 —— 浮层上的 `s` 直接开 provider 面板，`r` 直接重发。
    state.error_overlay_state = crate::ui::components::error_overlay::ErrorOverlayState {
        error_message: error.clone(),
        error_type: error_type.clone(),
        retry_count: 0,
        max_retries: 10,
        is_retrying: false,
        // 不可重试的错误（401/403、余额耗尽）默认高亮"切换提供商"：原地重试
        // 只会复现同一个错，先把 key / 网关改对才是有用的下一步。
        selected_action: if crate::ui::components::error_overlay::is_retryable(&error_type) {
            crate::ui::components::error_overlay::ErrorAction::Retry
        } else {
            crate::ui::components::error_overlay::ErrorAction::SwitchProvider
        },
    };
    state.open_error_overlay();

    // 状态栏只放一行精简信息：标签 + 下一步提示；原始错误全文另进 chat_history
    // （可换行查看）。按 "Original error:" 显式切开，即便上游把换行吞了，也不会
    // 让状态栏拖出一长串 "…then retry.Original error: …"。
    let concise = error.split("Original error:").next().unwrap_or(&error);
    let first_line = concise.lines().next().unwrap_or(concise).trim().to_string();
    emit_status_text(
        state,
        message_id,
        &i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
            .replace("{error}", &first_line),
    );
    state.chat_history.push(
        ChatEntry::assistant(
            i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                .replace("{error}", error.trim()),
        )
        .with_streaming(false),
    );
    let error_idx = state.chat_history.len() - 1;
    settle_response_cost(state, message_id, Some(error_idx));
    finish_terminal_stream(state, message_id, cancelling_graceful);
    // Esc 重发的判据。见 `ChatState::last_turn_failed`。
    state.last_turn_failed = true;
    // 错误可能正好落在一个待确认的工具上。不清掉 `is_awaiting_confirmation` 的
    // 话，`enqueue_user_message` 会一直往 `pending_user_messages` 里排队，而
    // `dequeue_pending_user_input` 又拒绝出队 —— 之后每一条消息都被静默吞掉。
    state.clear_pending_confirmation();

    append_transcript_event(
        state,
        "error",
        Some(message_id),
        serde_json::json!({
            "error": error,
        }),
    );
    dequeue_pending_user_input(state, agent_tx).await
}

/// Extract the last fenced code block from markdown content.
/// Returns the code block content (without fences) if found.

#[cfg(test)]
mod cache_merge_tests {
    use super::merge_declared_and_discovered_models_with;
    use crate::ui::state::store::ChatState;

    /// 面板列表必须是「声明的候选」与「API 发现的」的**并集**。
    ///
    /// 用户只配了 url + apikey 时候选集是空的，一切靠 API 自动发现；但只要
    /// 他在表单里写了哪怕一个模型名，之前那段代码就把整个列表**替换**成了声明
    /// 的那几个 —— API 明明还返回了另外十几个模型，面板上却只看得到一个，
    /// 得再点一次刷新才补得回来。
    ///
    /// 缓存从临时文件**直接读进函数**，不碰 `STAR_MODEL_CACHE_PATH`：那是进程级
    /// 环境变量，并行跑的两个用例会互相覆盖对方的缓存文件（单跑第二个也失败）。
    #[test]
    fn the_panel_lists_declared_and_discovered_models_together() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("model-cache.json");
        crate::agent::model_cache::save_to_path(
            &cache,
            "e2e",
            &[
                crate::types::ModelInfo::new("discovered-a", "e2e"),
                crate::types::ModelInfo::new("discovered-b", "e2e"),
            ],
        );
        let cached = crate::agent::model_cache::load_from_path(&cache, "e2e");

        let mut state = ChatState::new();
        merge_declared_and_discovered_models_with(
            &mut state,
            "e2e",
            &["declared".to_string()],
            cached,
        );

        assert!(
            state.available_models.contains(&"declared".to_string()),
            "声明的候选必须在列表里: {:?}",
            state.available_models
        );
        assert!(
            state.available_models.contains(&"discovered-a".to_string()),
            "API 发现的模型也必须在列表里: {:?}",
            state.available_models
        );
        assert!(
            state.available_models.contains(&"discovered-b".to_string()),
            "API 发现的模型也必须在列表里: {:?}",
            state.available_models
        );
        // 声明的排前面 —— 那是用户钦定的优先级。
        assert_eq!(
            state.available_models.first().map(String::as_str),
            Some("declared"),
            "声明的候选必须排在最前"
        );
    }

    /// 别家 provider 的模型不能混进来 —— 缓存是按 provider 分桶的。
    #[test]
    fn only_this_providers_discovered_models_are_merged_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("model-cache.json");
        crate::agent::model_cache::save_to_path(
            &cache,
            "other",
            &[crate::types::ModelInfo::new("someone-elses-model", "other")],
        );
        let cached = crate::agent::model_cache::load_from_path(&cache, "e2e");

        let mut state = ChatState::new();
        merge_declared_and_discovered_models_with(&mut state, "e2e", &[], cached);

        assert!(
            !state
                .available_models
                .contains(&"someone-elses-model".to_string()),
            "别家 provider 的模型不该进来: {:?}",
            state.available_models
        );
    }
}
