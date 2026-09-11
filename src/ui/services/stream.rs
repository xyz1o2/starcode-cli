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
use crate::runtime::messages::{AgentRequest, StreamMessage, StreamStartKind};
use crate::types::{ChatEntry, ChatEntryType, StarToolCall, ToolResult};
use crate::ui::app::logic::{
    emit_status_text, enqueue_user_message, recover_missing_tool_results, save_tool_output,
};
use crate::ui::state::store::{ChatState, ToastKind};
use crate::ui::utils::format::{
    format_tool_call, format_tool_result, format_tool_result_with_saved_path,
};
use crate::ui::utils::text::{
    format_elapsed_for_tool, inject_elapsed_into_tool_first_line, sanitize_for_tui,
    should_save_tool_output,
};

/// 结束 entry 的 streaming 状态，并冻结 thinking 计时器。
/// 统一处理所有 is_streaming = false 的场景，避免遗漏 reasoning_finished_elapsed_ms。
fn finalize_entry_streaming(state: &mut ChatState, idx: usize) {
    if let Some(entry) = state.chat_history.get_mut(idx) {
        entry.is_streaming = Some(false);
        // 冻结 thinking 计时器：只对有 reasoning 且未冻结的 entry 设置
        if entry.reasoning_content.is_some() && entry.reasoning_finished_elapsed_ms.is_none() {
            let frozen = state
                .processing_started_at
                .map(|t| t.elapsed().as_millis())
                .unwrap_or(0);
            entry.reasoning_finished_elapsed_ms = Some(frozen);
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
                // 不让上一回合的 provider 用量或缓存读数停留到新回合。
                state.token_count = 0;
                state.token_usage = None;
                state.cache_read_tokens = 0;
                state.cache_creation_tokens = 0;
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
            state.available_models = models.iter().map(|m| m.id.clone()).collect();
            state.available_models_info = models.clone(); // 保存完整的模型信息
            state.set_models_list_age(cache_age_secs);
            state.model_provider_map.clear();
            for m in &models {
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
            if let Some(pid) = state.pending_model_provider.take() {
                let remembered_model = state.pending_provider_selected_model.take();

                if let Some(model) = remembered_model
                    .filter(|model| models.iter().any(|m| m.id == *model && m.provider == pid))
                {
                    state.current_model = model.clone();
                    let provider_id = state
                        .model_provider_map
                        .get(&model)
                        .cloned()
                        .or_else(|| Some(pid.clone()));
                    state.current_provider_id = provider_id.clone();
                    let _ = agent_tx
                        .send(AgentRequest::SetModel {
                            model: model.clone(),
                            provider_id,
                        })
                        .await;
                    emit_status_text(
                        state,
                        0,
                        &i18n::t(
                            "ui.status.model.changed",
                            "Status: model switched to {model}",
                            "Status: model switched to {model}",
                        )
                        .replace("{model}", &state.current_model),
                    );
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
            else if !state.available_models.is_empty() {
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
        }
        StreamMessage::CurrentModelChanged { model, provider_id } => {
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
        StreamMessage::ApprovalModeChanged { mode } => {
            state.approval_mode = mode.clone();
            let mode_name = match mode {
                crate::types::ApprovalMode::Default => i18n::t("ui.mode.build", "Build", "Build"),
                crate::types::ApprovalMode::Plan => i18n::t("ui.mode.plan", "Plan", "Plan"),
                crate::types::ApprovalMode::Yolo => i18n::t("ui.mode.yolo", "YOLO", "YOLO"),
            };
            let mode_desc = match mode {
                crate::types::ApprovalMode::Default => i18n::t(
                    "ui.mode.build.desc",
                    "Build mode: confirm dangerous actions; manage tasks with Ctrl+T or /tasks",
                    "Build mode: confirm dangerous actions; manage tasks with Ctrl+T or /tasks",
                ),
                crate::types::ApprovalMode::Plan => i18n::t(
                    "ui.mode.plan.desc",
                    "Plan mode: read-only research; manage plans with /tasks or Ctrl+T",
                    "Plan mode: read-only research; manage plans with /tasks or Ctrl+T",
                ),
                crate::types::ApprovalMode::Yolo => i18n::t(
                    "ui.mode.yolo.desc",
                    "YOLO mode: all actions auto-run (dangerous!)",
                    "YOLO mode: all actions auto-run (dangerous!)",
                ),
            };
            state.chat_history.push(
                ChatEntry::assistant(format!(
                    "{}{}{}",
                    i18n::t(
                        "ui.approval.mode_changed.label",
                        "Status: Approval mode changed to: ",
                        "Status: Approval mode changed to: ",
                    ),
                    mode_name,
                    mode_desc
                ))
                .with_streaming(false),
            );
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
            confirmation,
        } => {
            let awaiting = confirmation.outcome.is_none();
            let is_ask = matches!(
                confirmation.operation_type,
                crate::types::ConfirmationType::AskUserQuestion
            );
            if awaiting && state.is_awaiting_confirmation {
                if let Some(idx) = state.pending_confirmation_entry_idx {
                    if idx < state.chat_history.len()
                        && state.chat_history[idx].entry_type == ChatEntryType::ToolConfirmation
                    {
                        state.chat_history[idx].confirmation = Some(confirmation);
                        state.chat_history[idx].is_streaming = Some(false);
                        state.pending_message_id = Some(message_id);
                        state.pending_tool_call_id = Some(tool_call_id);
                        state.confirmation_feedback_mode = false;
                        state.pending_confirmation_feedback.clear();
                        if state.pending_confirmation_choice == 0 {
                            state.pending_confirmation_choice = 1;
                        }
                        state.rendered_cache.remove(&idx);
                        state.chat_list_state.select(Some(idx));
                        if state.auto_follow {
                            // Force scroll to bottom calculation in next render
                        }
                        return Ok(());
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

/// 处理 Agent 任务更新：创建或更新 chat_history 中的 AgentTask / AgentGroup 条目
///
/// 条目归属只在该 task 的**第一条**更新时决定，之后一直复用
/// `AgentTaskInfo::entry_idx`。否则每条进度更新都会重新走一遍归属判断，
/// 把同一个 agent 反复追加进 `agent_task_ids`。
fn handle_agent_task_update(state: &mut ChatState, args: AgentTaskUpdateArgs) {
    use crate::types::ChatEntryType;

    let prior = state.active_agent_tasks.get(&args.task_id);
    let started_at = prior
        .map(|i| i.started_at)
        .unwrap_or_else(std::time::Instant::now);
    // 首次进入**终态**时冻结耗时，之后不再随渲染增长。
    // `Background` 不算终态 —— 它只表示 Agent 工具交回了控制权，任务还在后台跑；
    // 早先按 `is_resolved` 判断会在启动那一刻就把 finished_at 钉住，
    // 于是后台代理的耗时永远显示 0s。
    let is_terminal = matches!(
        args.status,
        crate::types::AgentTaskStatus::Completed
            | crate::types::AgentTaskStatus::Failed
            | crate::types::AgentTaskStatus::Rejected
    );
    let finished_at = prior
        .and_then(|i| i.finished_at)
        .or_else(|| is_terminal.then(std::time::Instant::now));
    let mut sub_entries = prior.map(|i| i.sub_entries.clone()).unwrap_or_default();
    sub_entries.extend(args.new_sub_entries.iter().cloned());
    // 进度 chunk 未携带这些字段时沿用旧值，避免状态行闪回 "Initializing…"
    let last_tool_info = args
        .last_tool_info
        .clone()
        .or_else(|| prior.and_then(|i| i.last_tool_info.clone()));
    let name = args
        .name
        .clone()
        .or_else(|| prior.and_then(|i| i.name.clone()));
    let task_description = args
        .task_description
        .clone()
        .or_else(|| prior.and_then(|i| i.task_description.clone()));

    let entry_idx = match prior.map(|i| i.entry_idx) {
        Some(idx) if idx < state.chat_history.len() => idx,
        _ => attach_agent_task_entry(state, &args),
    };

    // 独立 AgentTask 条目从 entry 自身取数渲染；AgentGroup 从
    // active_agent_tasks 取数，无需回写。
    let is_standalone = state
        .chat_history
        .get(entry_idx)
        .map(|e| e.entry_type == ChatEntryType::AgentTask)
        .unwrap_or(false);
    if is_standalone {
        if let Some(entry) = state.chat_history.get_mut(entry_idx) {
            entry.agent_description = Some(args.description.clone());
            entry.agent_status = Some(args.status.clone());
            entry.agent_tool_use_count = Some(args.tool_use_count);
            entry.agent_tokens = Some(args.tokens);
            entry.agent_is_resolved = Some(args.is_resolved);
            entry.agent_is_error = Some(args.is_error);
            entry.agent_is_async = Some(args.is_async);
            entry.agent_last_tool_info = last_tool_info.clone();
            entry.agent_name = name.clone();
            entry.agent_task_description = task_description.clone();
            if !sub_entries.is_empty() {
                entry.agent_sub_entries = Some(sub_entries.clone());
            }
        }
    }
    state.rendered_cache.remove(&entry_idx);
    state.virtual_list.mark_dirty(entry_idx);

    state.active_agent_tasks.insert(
        args.task_id.clone(),
        crate::ui::state::store::AgentTaskInfo {
            task_id: args.task_id,
            agent_type: args.agent_type,
            description: args.description,
            status: args.status,
            tool_use_count: args.tool_use_count,
            tokens: args.tokens,
            is_async: args.is_async,
            is_resolved: args.is_resolved,
            is_error: args.is_error,
            last_tool_info,
            name,
            task_description,
            started_at,
            finished_at,
            sub_entries,
            entry_idx,
        },
    );

    // 自动跟随到底部
    if state.auto_follow {
        state.scroll = state
            .virtual_list
            .total_lines()
            .saturating_sub(state.last_chat_height as usize);
    }
}

/// 决定一个**新** Agent 落在哪个 chat_history 条目上，返回该条目索引。
///
/// - 时间窗内已有 AgentGroup → 加入该组
/// - 时间窗内是独立 AgentTask → 原地升级为 AgentGroup，两个 agent 一起渲染
/// - 否则 → 新建独立 AgentTask 条目
fn attach_agent_task_entry(state: &mut ChatState, args: &AgentTaskUpdateArgs) -> usize {
    use crate::types::{ChatEntry, ChatEntryType};

    let now = chrono::Utc::now();
    let recent = state.chat_history.iter().rposition(|e| {
        matches!(
            e.entry_type,
            ChatEntryType::AgentTask | ChatEntryType::AgentGroup
        ) && now.signed_duration_since(e.timestamp).num_milliseconds() < AGENT_GROUP_WINDOW_MS
    });

    let Some(idx) = recent else {
        let mut entry = ChatEntry::agent_task(&args.task_id, &args.agent_type);
        entry.agent_description = Some(args.description.clone());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.virtual_list.mark_dirty(idx);
        return idx;
    };

    if state.chat_history[idx].entry_type == ChatEntryType::AgentGroup {
        if let Some(ids) = state.chat_history[idx].agent_task_ids.as_mut() {
            if !ids.contains(&args.task_id) {
                ids.push(args.task_id.clone());
            }
        }
    } else {
        // 独立 AgentTask 遇到并发的第二个 agent → 升级成组
        let mut ids: Vec<String> = state.chat_history[idx]
            .agent_task_id
            .clone()
            .into_iter()
            .collect();
        ids.push(args.task_id.clone());
        state.chat_history[idx] = ChatEntry::agent_group(ids.clone());
        // 组内既有成员的 entry_idx 仍指向同一索引，无需迁移，
        // 但要确保它们不会再被当作独立条目回写。
        for id in ids {
            if let Some(info) = state.active_agent_tasks.get_mut(&id) {
                info.entry_idx = idx;
            }
        }
    }

    state.rendered_cache.remove(&idx);
    state.virtual_list.mark_dirty(idx);
    idx
}

fn settle_response_cost(state: &mut ChatState, message_id: u64, assistant_idx: Option<usize>) {
    let response_cost = state.response_costs.remove(&message_id).unwrap_or(0.0);
    state.response_models.remove(&message_id);
    if response_cost == 0.0 {
        return;
    }

    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);
    let entry_idx = assistant_idx.and_then(|idx| {
        if state
            .chat_history
            .get(idx)
            .is_some_and(|entry| entry.entry_type == ChatEntryType::Assistant)
        {
            return Some(idx);
        }

        let end_idx = idx.saturating_add(1).min(state.chat_history.len());
        state.chat_history[start_idx.min(end_idx)..end_idx]
            .iter()
            .rposition(|entry| entry.entry_type == ChatEntryType::Assistant)
            .map(|offset| start_idx.min(end_idx) + offset)
    });

    state.total_cost += response_cost;
    if let Some(idx) = entry_idx {
        let entry = &mut state.chat_history[idx];
        entry.cost = Some(entry.cost.unwrap_or(0.0) + response_cost);
    }
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
    state.is_processing = false;
    state.current_tool_name = None;
    state.thinking_started_at = None;
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
    if crate::ui::components::error_overlay::is_retryable(&error_type) {
        state.error_overlay_state = crate::ui::components::error_overlay::ErrorOverlayState {
            error_message: error.clone(),
            error_type: error_type.clone(),
            retry_count: 0,
            max_retries: 10,
            is_retrying: false,
            selected_action: crate::ui::components::error_overlay::ErrorAction::Retry,
        };
        state.open_error_overlay();
    } else {
        state.push_toast(
            &crate::ui::components::error_overlay::error_type_title(&error_type),
            ToastKind::Error,
        );
    }

    let first_line = error.lines().next().unwrap_or(&error).trim().to_string();
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

/// 在 pos 处向 chat_history 中插入条目后，修正所有按索引的缓存/映射（>=pos 的 +1）。
/// 否则中插会让 rendered_cache、流式键、消息起止索引等全部错位。
fn shift_index_caches_after_insert(state: &mut ChatState, pos: usize) {
    state.virtual_list.insert_at(pos);
    // 选区按 entry 索引存储，中插后索引失效，直接清除
    state.text_selection.clear();
    if state.last_item_heights.len() >= pos {
        state.last_item_heights.insert(pos, 0);
    } else {
        state.last_item_heights.push(0);
    }

    // rendered_cache: HashMap<usize, (u16, Vec<Line>)>
    let remixed: std::collections::HashMap<usize, _> = state
        .rendered_cache
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.rendered_cache = remixed;

    // last_rendered_stream_key
    let remixed: std::collections::HashMap<usize, _> = state
        .last_rendered_stream_key
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.last_rendered_stream_key = remixed;

    // streaming_height_floor
    let remixed: std::collections::HashMap<usize, _> = state
        .streaming_height_floor
        .drain()
        .map(|(k, v)| (if k >= pos { k + 1 } else { k }, v))
        .collect();
    state.streaming_height_floor = remixed;

    // 按消息 id 存的索引值
    for v in state.message_start_indices.values_mut() {
        if *v >= pos {
            *v += 1;
        }
    }
    for v in state.stream_targets.values_mut() {
        if *v >= pos {
            *v += 1;
        }
    }
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx >= pos {
            *idx += 1;
        }
    }
}

fn shift_indexed_cache_after_removal<T>(
    cache: &mut std::collections::HashMap<usize, T>,
    pos: usize,
) {
    *cache = cache
        .drain()
        .filter_map(|(idx, value)| match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some((idx, value)),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some((idx - 1, value)),
        })
        .collect();
}

/// 在 pos 处从 chat_history 移除条目后，修正所有按索引的缓存/映射（>pos 的 -1）。
/// 删除目标自身的渲染、流式和选择状态，防止后续更新指向相邻条目。
fn remove_chat_entry_and_shift_indices(state: &mut ChatState, pos: usize) {
    if pos >= state.chat_history.len() {
        return;
    }

    state.chat_history.remove(pos);
    state.virtual_list.remove_at(pos);
    state.virtual_list.mark_dirty_all();
    state.text_selection.clear();
    state.expanded_thinking_indices = state
        .expanded_thinking_indices
        .drain()
        .filter_map(|idx| match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some(idx),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(idx - 1),
        })
        .collect();

    if pos < state.last_item_heights.len() {
        state.last_item_heights.remove(pos);
    }

    shift_indexed_cache_after_removal(&mut state.rendered_cache, pos);
    shift_indexed_cache_after_removal(&mut state.last_rendered_stream_key, pos);
    shift_indexed_cache_after_removal(&mut state.streaming_height_floor, pos);

    for idx in state.message_start_indices.values_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            *idx = pos.min(state.chat_history.len());
        }
    }
    for idx in state.stream_targets.values_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            *idx = pos.min(state.chat_history.len());
        }
    }
    if let Some(idx) = state.pending_confirmation_entry_idx.as_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            state.pending_confirmation_entry_idx = None;
        }
    }
    if let Some(idx) = state.preview_last_entry_idx.as_mut() {
        if *idx > pos {
            *idx -= 1;
        } else if *idx == pos {
            state.preview_last_entry_idx = None;
        }
    }
    if let Some(idx) = state.chat_list_state.selected() {
        let remapped = match idx.cmp(&pos) {
            std::cmp::Ordering::Less => Some(idx),
            std::cmp::Ordering::Equal => {
                (!state.chat_history.is_empty()).then(|| pos.min(state.chat_history.len() - 1))
            }
            std::cmp::Ordering::Greater => Some(idx - 1),
        };
        state.chat_list_state.select(remapped);
    }
    for info in state.active_agent_tasks.values_mut() {
        if info.entry_idx > pos {
            info.entry_idx -= 1;
        }
    }

    state.total_rendered_lines = state.virtual_list.total_lines();
    state.scroll = state.scroll.min(state.total_rendered_lines);
}

fn handle_tool_result_message(
    state: &mut ChatState,
    message_id: u64,
    tool_call: StarToolCall,
    tool_result: ToolResult,
) {
    if state.is_awaiting_confirmation {
        if let Some(pending_id) = state.pending_tool_call_id.as_deref() {
            if pending_id == tool_call.id {
                state.is_awaiting_confirmation = false;
                state.pending_tool_call_id = None;
                state.pending_confirmation_entry_idx = None;
                state.confirmation_feedback_mode = false;
                state.pending_confirmation_feedback.clear();
            }
        }
    }
    if matches!(tool_call.function.name.as_str(), "TodoWrite") {
        state.task_panel.reload();
        state.task_panel.mark_modified();
        if !state.task_panel.is_visible && !state.task_panel.manually_hidden {
            let has_active = state.task_panel.task_manager.graph.nodes.values().any(|n| {
                matches!(
                    n.status,
                    crate::core::tasks::models::TaskStatus::Pending
                        | crate::core::tasks::models::TaskStatus::InProgress
                )
            });
            if has_active {
                state.task_panel.is_visible = true;
            }
        }
    }
    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);
    let elapsed_ms = state
        .tool_started_at
        .remove(&tool_call.id)
        .map(|t| t.elapsed().as_millis());
    refresh_current_tool_name(state);

    // ── Agent 工具异步启动检测 ──
    // 当 Agent 工具以 background=true 执行时，ToolResult.data 包含
    // { status: "async_launched", agent_id: "..." }
    // 此时创建 AgentTask 条目而非普通 ToolResult
    if (tool_call.function.name == "Agent" || tool_call.function.name == "agent")
        && tool_result.success
    {
        if let Some(data) = &tool_result.data {
            if let Some(status_str) = data.get("status").and_then(|v| v.as_str()) {
                if status_str == "async_launched" || status_str == "fork_launched" {
                    let agent_id = data
                        .get("agent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    // 类型标签由 AgentTool 写进 data，不再按 status 猜
                    let agent_type = data.get("agent_type").and_then(|v| v.as_str()).unwrap_or(
                        if status_str == "fork_launched" {
                            "Fork"
                        } else {
                            "Agent"
                        },
                    );
                    let name = data
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    // 描述优先取 data，回落到 tool_call 参数
                    let description = data
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| {
                            serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments)
                                .ok()?
                                .get("description")?
                                .as_str()
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_else(|| "Agent task".to_string());

                    handle_agent_task_update(
                        state,
                        AgentTaskUpdateArgs {
                            task_id: agent_id.to_string(),
                            agent_type: agent_type.to_string(),
                            description: description.clone(),
                            // 工具已交回控制权 → backgrounded（is_async +
                            // is_resolved），状态行显示任务描述而非工具进度
                            status: crate::types::AgentTaskStatus::Background,
                            tool_use_count: 0,
                            tokens: 0,
                            is_async: true,
                            is_resolved: true,
                            is_error: false,
                            last_tool_info: None,
                            name,
                            task_description: Some(description),
                            new_sub_entries: Vec::new(),
                        },
                    );

                    // 不再创建重复的 ToolCall 条目 — AgentTask 条目已包含完整信息
                    // 移除已有的 ToolCall streaming 条目
                    if let Some(tc_idx) = state.chat_history.iter().position(|e| {
                        e.entry_type == ChatEntryType::ToolCall
                            && e.tool_call
                                .as_ref()
                                .map(|tc| tc.id == tool_call.id)
                                .unwrap_or(false)
                    }) {
                        remove_chat_entry_and_shift_indices(state, tc_idx);
                    }
                    // 下一段主 agent 文本必须追加在 AgentTask 之后。若沿用删除后
                    // 的旧 target，它会指向刚创建的 AgentTask，导致最终回答混入进度卡片。
                    state
                        .stream_targets
                        .insert(message_id, state.chat_history.len());
                    return;
                }
            }
        }
    }

    let search_end_idx = state.chat_history.len();
    let mut found: Option<usize> = None;
    for i in (start_idx..search_end_idx).rev() {
        let e = &state.chat_history[i];
        if e.entry_type != ChatEntryType::ToolCall {
            continue;
        }
        if let Some(tc) = e.tool_call.as_ref() {
            if tc.id == tool_call.id {
                found = Some(i);
                break;
            }
        }
    }
    let elapsed = elapsed_ms.map(|ms| format_elapsed_for_tool(ms));
    let elapsed_for_status = elapsed.clone();
    let raw_out = if tool_result.success {
        tool_result.output.as_deref().unwrap_or("")
    } else {
        tool_result.error.as_deref().unwrap_or("")
    };
    let saved_path = if should_save_tool_output(raw_out) {
        save_tool_output(&tool_call, raw_out)
    } else {
        None
    };
    let mut content = if let Some(p) = saved_path.as_deref() {
        format_tool_result_with_saved_path(&tool_call, &tool_result, p)
    } else {
        format_tool_result(&tool_call, &tool_result)
    };
    content = inject_elapsed_into_tool_first_line(content, elapsed);
    // 保险清除：无论结果通过哪个分支到达（found 命中/未命中、是否被去重），
    // 只要 history 里存在匹配该 tool_call.id 的 ToolCall entry，就把它的
    // streaming 标记清掉。否则该工具行的 ● 圆点会因收不到结束事件而永远闪烁。
    for (idx, e) in state.chat_history.iter_mut().enumerate() {
        if e.entry_type == ChatEntryType::ToolCall
            && e.is_streaming == Some(true)
            && e.tool_call
                .as_ref()
                .map(|tc| tc.id == tool_call.id)
                .unwrap_or(false)
        {
            e.is_streaming = Some(false);
            state.rendered_cache.remove(&idx);
            state.virtual_list.mark_dirty(idx);
        }
    }
    // 去重：同一 tool_call 的 ToolResult 可能通过 stream_tx 和 event_tx 双路径到达，
    // 如果已存在相同 tool_call.id 的 ToolResult 条目则跳过。
    let already_has_result = state.chat_history.iter().rev().take(20).any(|e| {
        e.entry_type == ChatEntryType::ToolResult
            && e.tool_call
                .as_ref()
                .map(|tc| tc.id == tool_call.id)
                .unwrap_or(false)
    });
    if already_has_result {
        return;
    }
    if let Some(i) = found {
        let next_is_confirmation = if i + 1 < state.chat_history.len() {
            state.chat_history[i + 1].entry_type == ChatEntryType::ToolConfirmation
        } else {
            false
        };
        if next_is_confirmation {
            state.chat_history[i].is_streaming = Some(false);
            state.chat_history[i].tool_elapsed_ms = elapsed_ms;
            state.rendered_cache.remove(&i);
            state.virtual_list.mark_dirty(i);
            if !should_suppress_redundant_result_after_confirmation(&tool_call, &tool_result) {
                let mut entry =
                    ChatEntry::tool_result(content, tool_call.clone(), tool_result.clone())
                        .with_streaming(false);
                entry.tool_elapsed_ms = elapsed_ms;
                state.chat_history.push(entry);
            }
        } else {
            state.chat_history[i].is_streaming = Some(false);
            state.chat_history[i].tool_elapsed_ms = elapsed_ms;
            state.rendered_cache.remove(&i);
            state.virtual_list.mark_dirty(i);
            let mut entry = ChatEntry::tool_result(content, tool_call.clone(), tool_result.clone())
                .with_streaming(false);
            entry.tool_elapsed_ms = elapsed_ms;
            // 插入到对应 ToolCall 之后（而非追加到末尾）：
            // 并行工具调用时结果按完成顺序到达，若一律 push 到末尾，
            // 显示会变成 [工具A, 工具B, 结果A, 结果B]，结果脱离了各自的工具行
            let insert_pos = i + 1;
            state.chat_history.insert(insert_pos, entry);
            shift_index_caches_after_insert(state, insert_pos);
        }
    } else {
        let mut entry = ChatEntry::tool_result(content, tool_call.clone(), tool_result.clone())
            .with_streaming(false);
        entry.tool_elapsed_ms = elapsed_ms;
        state.chat_history.push(entry);
    }
    // 状态更新：Done 或 Error
    let tool_pretty = format_tool_name_for_status(&tool_call.function.name);
    let status = if tool_result.success {
        if let Some(ref e) = elapsed_for_status {
            format!("Done ({})", e)
        } else {
            format!("Done")
        }
    } else {
        // 区分"工具执行报错"与"用户拒绝/策略拒绝"：被拒的 ToolResult 的
        // error 文本含 denied/rejected（由 tool_executor / policy 生成）。
        // 单独显示为 ⛔ denied，而不是笼统的 Error，让用户一眼区分。
        let denied = tool_result
            .error
            .as_deref()
            .map(|e| {
                let lower = e.to_lowercase();
                lower.contains("denied") || lower.contains("rejected")
            })
            .unwrap_or(false);
        if denied {
            if let Some(ref e) = elapsed_for_status {
                format!("⛔ {} denied ({})", tool_pretty, e)
            } else {
                format!("⛔ {} denied", tool_pretty)
            }
        } else if let Some(ref e) = elapsed_for_status {
            format!("Error {} ({})", tool_pretty, e)
        } else {
            format!("Error {}", tool_pretty)
        }
    };
    emit_status_text(state, message_id, &status);
    append_transcript_event(
        state,
        "tool_result",
        Some(message_id),
        serde_json::json!({
            "tool_call_id": tool_call.id,
            "name": tool_call.function.name,
            "success": tool_result.success,
            "saved_path": saved_path,
        }),
    );
}

fn handle_content_message(state: &mut ChatState, message_id: u64, content: &str) {
    let content = sanitize_for_tui(content);
    state.last_token_time = Some(std::time::Instant::now());
    let verbose_debug_logging = crate::utils::logging::is_verbose_debug_logging_enabled();
    if verbose_debug_logging {
        crate::utils::logging::append_debug_log_line(&format!(
            "[DEBUG] StreamHandler: Processing Content message (len={}, message_id={})",
            content.len(),
            message_id
        ));
    }
    if let Some(&idx0) = state.stream_targets.get(&message_id) {
        let mut idx = idx0;
        if verbose_debug_logging {
            crate::utils::logging::append_debug_log_line(&format!(
                "[DEBUG] StreamHandler: target idx={}, history_len={}",
                idx,
                state.chat_history.len()
            ));
        }
        let mut should_create_new = false;
        if idx < state.chat_history.len() {
            let current_type = &state.chat_history[idx].entry_type;
            if verbose_debug_logging {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[DEBUG] StreamHandler: current_entry_type={:?}",
                    current_type
                ));
            }
            if *current_type == ChatEntryType::ToolCall
                || *current_type == ChatEntryType::ToolResult
                || *current_type == ChatEntryType::ToolConfirmation
            {
                if verbose_debug_logging {
                    crate::utils::logging::append_debug_log_line(
                        "[DEBUG] StreamHandler: Trigger new entry creation (Case 1: Tool entry detected)",
                    );
                }
                should_create_new = true;
            }
            if *current_type == ChatEntryType::Assistant {
                if idx < state.chat_history.len() - 1 {
                    if verbose_debug_logging {
                        crate::utils::logging::append_debug_log_line("[DEBUG] StreamHandler: Trigger new entry creation (Case 2: Assistant not last)");
                    }
                    should_create_new = true;
                }
                if state.chat_history[idx].is_streaming != Some(true) {
                    if verbose_debug_logging {
                        crate::utils::logging::append_debug_log_line("[DEBUG] StreamHandler: Trigger new entry creation (Case 3: Assistant stopped streaming)");
                    }
                    should_create_new = true;
                }
            }
        } else {
            if verbose_debug_logging {
                crate::utils::logging::append_debug_log_line(
                    "[DEBUG] StreamHandler: idx >= history_len (New Entry)",
                );
            }
        }
        if should_create_new {
            idx = state.chat_history.len();
            state.stream_targets.insert(message_id, idx);
            if verbose_debug_logging {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[DEBUG] StreamHandler: Updating target idx -> {}",
                    idx
                ));
            }
        }
        if idx == state.chat_history.len() {
            if verbose_debug_logging {
                crate::utils::logging::append_debug_log_line(
                    "[DEBUG] StreamHandler: Creating new Assistant Entry",
                );
            }
            state
                .chat_history
                .push(ChatEntry::assistant("").with_streaming(true));
        }
        if idx < state.chat_history.len() {
            state.chat_history[idx].content.push_str(&content);
            if verbose_debug_logging {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[DEBUG] StreamHandler: Appending content to idx={}, content_len={}",
                    idx,
                    state.chat_history[idx].content.len()
                ));
            }
            state.rendered_cache.remove(&idx);
            state.virtual_list.mark_dirty(idx);
        }
    }
}

/// Extract the last fenced code block from markdown content.
/// Returns the code block content (without fences) if found.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StarToolCallFunction;

    fn agent_tool_call() -> StarToolCall {
        StarToolCall {
            id: "agent-call".to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "Agent".to_string(),
                arguments: r#"{"description":"scan"}"#.to_string(),
            },
        }
    }

    fn async_agent_launch_result() -> ToolResult {
        ToolResult {
            success: true,
            output: Some(String::new()),
            error: None,
            data: Some(serde_json::json!({
                "status": "async_launched",
                "agent_id": "worker-1",
                "agent_type": "Explore",
                "description": "scan",
            })),
        }
    }

    #[test]
    fn async_agent_launch_remaps_indices_before_follow_up_updates() {
        let mut state = ChatState::new();
        state.chat_history.clear();
        state.chat_history.push(ChatEntry::assistant("before"));
        state
            .chat_history
            .push(ChatEntry::tool_call("Agent", agent_tool_call()).with_streaming(true));
        state.chat_history.push(
            ChatEntry::new(ChatEntryType::ToolConfirmation, String::new()).with_streaming(false),
        );
        state
            .chat_history
            .push(ChatEntry::assistant("after").with_streaming(true));
        state.stream_targets.insert(42, 3);
        state.message_start_indices.insert(42, 1);
        state.pending_confirmation_entry_idx = Some(2);
        state.preview_last_entry_idx = Some(3);
        state.chat_list_state.select(Some(3));
        state.expanded_thinking_indices.insert(3);
        state.last_item_heights = vec![5, 7, 11, 13];
        state.virtual_list.resize(4);
        for (idx, height) in [5, 7, 11, 13].into_iter().enumerate() {
            state.virtual_list.set_height(idx, height);
        }
        state.total_rendered_lines = state.virtual_list.total_lines();
        state.rendered_cache.insert(1, (7, Vec::new()));
        state.rendered_cache.insert(3, (13, Vec::new()));
        state.last_rendered_stream_key.insert(3, (1, 2));
        state.streaming_height_floor.insert(3, 13);

        handle_tool_result_message(
            &mut state,
            42,
            agent_tool_call(),
            async_agent_launch_result(),
        );

        assert_eq!(state.chat_history.len(), 4);
        assert_eq!(state.stream_targets.get(&42), Some(&4));
        assert_eq!(state.message_start_indices.get(&42), Some(&1));
        assert_eq!(state.pending_confirmation_entry_idx, Some(1));
        assert_eq!(state.preview_last_entry_idx, Some(2));
        assert_eq!(state.chat_list_state.selected(), Some(2));
        assert_eq!(
            state.expanded_thinking_indices,
            std::collections::HashSet::from([2])
        );
        assert_eq!(state.last_item_heights, vec![5, 11, 13]);
        assert_eq!(state.virtual_list.len(), 3);
        assert_eq!(state.total_rendered_lines, 29);
        assert!(!state.rendered_cache.contains_key(&1));
        assert!(state.rendered_cache.contains_key(&2));
        assert_eq!(state.last_rendered_stream_key.get(&2), Some(&(1, 2)));
        assert_eq!(state.streaming_height_floor.get(&2), Some(&13));
        assert_eq!(state.active_agent_tasks["worker-1"].entry_idx, 3);
        assert_eq!(state.chat_history[3].entry_type, ChatEntryType::AgentTask);

        handle_content_message(&mut state, 42, " updated");
        assert_eq!(state.chat_history[4].content, " updated");
        assert_eq!(state.chat_history[2].content, "after");

        handle_agent_task_update(
            &mut state,
            AgentTaskUpdateArgs {
                task_id: "worker-1".to_string(),
                agent_type: "Explore".to_string(),
                description: "scan".to_string(),
                status: crate::types::AgentTaskStatus::Running,
                tool_use_count: 1,
                tokens: 12,
                is_async: true,
                is_resolved: false,
                is_error: false,
                last_tool_info: Some("Reading config".to_string()),
                name: None,
                task_description: Some("scan".to_string()),
                new_sub_entries: Vec::new(),
            },
        );

        assert_eq!(state.active_agent_tasks["worker-1"].entry_idx, 3);
        assert_eq!(state.chat_history[3].agent_tool_use_count, Some(1));
        assert_eq!(
            state.chat_history[3].agent_last_tool_info.as_deref(),
            Some("Reading config")
        );
    }

    #[tokio::test]
    async fn reasoning_delta_refreshes_stream_liveness() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let message_id = 7;
        state.stream_targets.insert(message_id, 0);
        state.last_token_time =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(31));

        handle_stream_update(
            &mut state,
            StreamMessage::ReasoningDelta {
                message_id,
                content: "Checking the implementation".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();

        let elapsed = state.last_token_time.unwrap().elapsed();
        assert!(elapsed < std::time::Duration::from_secs(1));
        assert_eq!(
            state
                .chat_history
                .last()
                .unwrap()
                .reasoning_content
                .as_deref(),
            Some("Checking the implementation")
        );
    }

    #[tokio::test]
    async fn active_tool_output_and_results_preserve_parallel_lifecycle() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let message_id = 9;
        state.stream_targets.insert(message_id, 0);
        let first = StarToolCall {
            id: "call-a".to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "Read".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let second = StarToolCall {
            id: "call-b".to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "Bash".to_string(),
                arguments: "{}".to_string(),
            },
        };

        handle_stream_update(
            &mut state,
            StreamMessage::ToolCalls {
                message_id,
                tool_calls: vec![first.clone(), second.clone()],
            },
            &agent_tx,
        )
        .await
        .unwrap();
        state.last_token_time = Some(Instant::now() - std::time::Duration::from_secs(31));

        handle_stream_update(
            &mut state,
            StreamMessage::ToolOutput {
                message_id,
                tool_call_id: second.id.clone(),
                output: "still running".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert!(state.last_token_time.unwrap().elapsed() < std::time::Duration::from_secs(1));

        let success = ToolResult {
            success: true,
            output: None,
            error: None,
            data: None,
        };
        handle_tool_result_message(&mut state, message_id, first.clone(), success.clone());
        assert!(!state.tool_started_at.contains_key(&first.id));
        assert!(state.tool_started_at.contains_key(&second.id));
        assert_eq!(state.current_tool_name.as_deref(), Some("Bash"));
        assert!(state.chat_history.iter().any(|entry| {
            entry.tool_call.as_ref().map(|call| call.id.as_str()) == Some(second.id.as_str())
                && entry.is_streaming == Some(true)
        }));

        handle_tool_result_message(&mut state, message_id, second.clone(), success);
        assert!(state.tool_started_at.is_empty());
        assert_eq!(state.current_tool_name, None);
    }

    #[tokio::test]
    async fn global_search_results_apply_only_to_open_current_request() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        state.open_global_search();
        state.global_search_state.begin_request(7);

        handle_stream_update(
            &mut state,
            StreamMessage::GlobalSearchResults {
                request_id: 7,
                results: vec![crate::runtime::messages::GlobalSearchMatch {
                    file: "src/lib.rs".to_string(),
                    line_number: 3,
                    content: "needle".to_string(),
                    score: 0,
                }],
                truncated: false,
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.global_search_state.results.len(), 1);
        assert_eq!(state.global_search_state.results[0].file, "src/lib.rs");
        assert!(!state.global_search_state.is_searching);

        state.global_search_state.begin_request(8);
        handle_stream_update(
            &mut state,
            StreamMessage::GlobalSearchResults {
                request_id: 7,
                results: Vec::new(),
                truncated: true,
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.global_search_state.active_request_id, Some(8));
        assert_eq!(state.global_search_state.results.len(), 1);
        assert!(!state.global_search_state.truncated);

        state.pop_modal();
        handle_stream_update(
            &mut state,
            StreamMessage::GlobalSearchResults {
                request_id: 8,
                results: Vec::new(),
                truncated: true,
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.global_search_state.results.len(), 1);
        assert!(!state.global_search_state.truncated);
    }

    #[tokio::test]
    async fn confirmation_feedback_draft_resets_for_new_request_and_matching_result() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        state.confirmation_feedback_mode = true;
        state.pending_confirmation_feedback = "stale draft".to_string();
        let confirmation = crate::types::ToolConfirmation {
            tool_name: "Bash".to_string(),
            operation_type: crate::types::ConfirmationType::Generic,
            details: crate::types::ConfirmationDetails::Generic {
                title: "Confirm".to_string(),
                prompt: "Continue?".to_string(),
            },
            is_dangerous: false,
            outcome: None,
        };

        handle_stream_update(
            &mut state,
            StreamMessage::ToolConfirmationRequest {
                message_id: 1,
                tool_call_id: "call-1".to_string(),
                confirmation,
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert!(!state.confirmation_feedback_mode);
        assert!(state.pending_confirmation_feedback.is_empty());

        state.confirmation_feedback_mode = true;
        state.pending_confirmation_feedback = "result draft".to_string();
        handle_tool_result_message(
            &mut state,
            1,
            agent_tool_call(),
            ToolResult {
                success: true,
                output: None,
                error: None,
                data: None,
            },
        );
        assert!(state.confirmation_feedback_mode);
        assert_eq!(state.pending_confirmation_feedback, "result draft");

        handle_tool_result_message(
            &mut state,
            1,
            StarToolCall {
                id: "call-1".to_string(),
                ..agent_tool_call()
            },
            ToolResult {
                success: true,
                output: None,
                error: None,
                data: None,
            },
        );
        assert!(!state.confirmation_feedback_mode);
        assert!(state.pending_confirmation_feedback.is_empty());
    }

    #[tokio::test]
    async fn cache_warnings_are_transient_deduplicated_and_do_not_touch_history() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let history_len = state.chat_history.len();
        let costly_cache_write = crate::types::StarUsage {
            prompt_tokens: 900,
            completion_tokens: 10,
            total_tokens: 910,
            cache_read_tokens: 100,
            cache_creation_tokens: 20_000,
            cache_telemetry_reported: true,
        };

        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id: 41,
                tokens: 910,
                usage: Some(costly_cache_write.clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.toast_queue.len(), 1);
        assert!(matches!(state.toast_queue[0].kind, ToastKind::Warning));
        assert_eq!(state.chat_history.len(), history_len);

        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id: 41,
                tokens: 910,
                usage: Some(costly_cache_write),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.toast_queue.len(), 1);

        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id: 41,
                tokens: 1_010,
                usage: Some(crate::types::StarUsage {
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    total_tokens: 1_010,
                    cache_read_tokens: 900,
                    cache_creation_tokens: 30_000,
                    cache_telemetry_reported: true,
                }),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.toast_queue.len(), 1);
        assert_eq!(state.chat_history.len(), history_len);
    }

    #[tokio::test]
    async fn latest_provider_usage_replaces_prior_response_and_unknown_cache_clears_counters() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let message_id = 43;
        let first = crate::types::StarUsage {
            prompt_tokens: 900,
            completion_tokens: 10,
            total_tokens: 910,
            cache_read_tokens: 800,
            cache_creation_tokens: 50,
            cache_telemetry_reported: true,
        };
        let second = crate::types::StarUsage {
            prompt_tokens: 20,
            completion_tokens: 5,
            total_tokens: 25,
            cache_read_tokens: 999,
            cache_creation_tokens: 999,
            cache_telemetry_reported: false,
        };

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id,
                kind: StreamStartKind::UserTurn {
                    model: "gpt-4o".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: first.total_tokens,
                usage: Some(first.clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: second.total_tokens,
                usage: Some(second.clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.token_count, 25);
        let usage = state.token_usage.expect("latest usage");
        assert_eq!(usage.prompt_tokens, 20);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 25);
        assert!(!usage.cache_telemetry_reported);
        assert_eq!(state.cache_read_tokens, 0);
        assert_eq!(state.cache_creation_tokens, 0);
        let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
            + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
        assert_eq!(state.response_costs.get(&message_id), Some(&expected));
    }

    #[tokio::test]
    async fn operation_start_preserves_latest_provider_usage_and_cache_counters() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let usage = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 10,
            total_tokens: 110,
            cache_read_tokens: 40,
            cache_creation_tokens: 3,
            cache_telemetry_reported: true,
        };
        state.token_count = usage.total_tokens;
        state.token_usage = Some(usage.clone());
        state.cache_read_tokens = usage.cache_read_tokens as u64;
        state.cache_creation_tokens = usage.cache_creation_tokens as u64;

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id: 44,
                kind: StreamStartKind::Operation,
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.token_count, 110);
        let restored = state.token_usage.as_ref().expect("usage is preserved");
        assert_eq!(restored.total_tokens, usage.total_tokens);
        assert_eq!(restored.cache_read_tokens, usage.cache_read_tokens);
        assert_eq!(restored.cache_creation_tokens, usage.cache_creation_tokens);
        assert!(restored.cache_telemetry_reported);
        assert_eq!(state.cache_read_tokens, 40);
        assert_eq!(state.cache_creation_tokens, 3);
    }

    #[tokio::test]
    async fn new_request_clears_usage_while_compression_stats_preserve_it() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let usage = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 10,
            total_tokens: 110,
            cache_read_tokens: 40,
            cache_creation_tokens: 3,
            cache_telemetry_reported: true,
        };
        state.token_count = usage.total_tokens;
        state.token_usage = Some(usage.clone());
        state.cache_read_tokens = usage.cache_read_tokens as u64;
        state.cache_creation_tokens = usage.cache_creation_tokens as u64;

        handle_stream_update(
            &mut state,
            StreamMessage::StatsUpdate {
                au2_compressed: true,
                token_usage: Some(crate::types::StarUsage {
                    total_tokens: 9_999,
                    ..Default::default()
                }),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.token_usage.as_ref().unwrap().total_tokens, 110);
        assert_eq!(state.cache_read_tokens, 40);

        state.cache_warning_shown.insert((
            1,
            crate::agent::cache_warning::CacheWarningType::HighCreationCost,
        ));
        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id: 44,
                kind: StreamStartKind::UserTurn {
                    model: "test-model".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.token_count, 0);
        assert!(state.token_usage.is_none());
        assert_eq!(state.cache_read_tokens, 0);
        assert_eq!(state.cache_creation_tokens, 0);
        assert!(state.cache_warning_shown.is_empty());
    }

    #[tokio::test]
    async fn terminal_messages_settle_accumulated_cost_once_and_ignore_late_usage() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let message_id = 45;
        let first = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            ..Default::default()
        };
        let second = crate::types::StarUsage {
            prompt_tokens: 200,
            completion_tokens: 30,
            total_tokens: 230,
            ..Default::default()
        };
        let late = crate::types::StarUsage {
            prompt_tokens: 999,
            completion_tokens: 999,
            total_tokens: 1_998,
            ..Default::default()
        };

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id,
                kind: StreamStartKind::UserTurn {
                    model: "gpt-4o".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TextDelta {
                message_id,
                content: "answer".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        for usage in [&first, &second] {
            handle_stream_update(
                &mut state,
                StreamMessage::TokenCount {
                    message_id,
                    tokens: usage.total_tokens,
                    usage: Some((*usage).clone()),
                },
                &agent_tx,
            )
            .await
            .unwrap();
        }

        let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
            + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
        handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
            .await
            .unwrap();
        assert_eq!(state.total_cost, expected);
        assert_eq!(
            state.chat_history.last().and_then(|entry| entry.cost),
            Some(expected)
        );
        assert!(state.complete_task_message_ids.contains(&message_id));
        assert!(!state.response_costs.contains_key(&message_id));
        assert!(!state.response_models.contains_key(&message_id));

        handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
            .await
            .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: late.total_tokens,
                usage: Some(late),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        assert_eq!(state.total_cost, expected);
        assert_eq!(state.token_count, second.total_tokens);
    }

    #[tokio::test]
    async fn error_settles_accumulated_cost_once() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        let message_id = 46;
        let usage = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            ..Default::default()
        };
        let expected = crate::ui::utils::cost::compute_response_cost(&usage, "gpt-4o");

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id,
                kind: StreamStartKind::UserTurn {
                    model: "gpt-4o".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: usage.total_tokens,
                usage: Some(usage),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::Error {
                message_id,
                error: "invalid API key".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.total_cost, expected);
        assert_eq!(
            state.chat_history.last().and_then(|entry| entry.cost),
            Some(expected)
        );
        assert!(state.complete_task_message_ids.contains(&message_id));
        handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
            .await
            .unwrap();
        assert_eq!(state.total_cost, expected);
    }

    #[tokio::test]
    async fn auto_continue_retains_accumulated_cost_for_same_message_id() {
        let mut state = ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let message_id = 47;
        let first = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            ..Default::default()
        };
        let second = crate::types::StarUsage {
            prompt_tokens: 50,
            completion_tokens: 10,
            total_tokens: 60,
            ..Default::default()
        };
        state.auto_continue_enabled = true;
        state.auto_continue_remaining = 1;

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id,
                kind: StreamStartKind::UserTurn {
                    model: "gpt-4o".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TextDelta {
                message_id,
                content: "working".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::ToolCalls {
                message_id,
                tool_calls: vec![StarToolCall {
                    id: "call-1".to_string(),
                    call_type: "function".to_string(),
                    function: StarToolCallFunction {
                        name: "Read".to_string(),
                        arguments: "{}".to_string(),
                    },
                }],
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: first.total_tokens,
                usage: Some(first.clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
            .await
            .unwrap();
        let AgentRequest::SendMessage {
            message_id: continued_id,
            ..
        } = agent_rx.recv().await.expect("auto-continue request")
        else {
            panic!("expected auto-continue request");
        };
        assert_eq!(continued_id, message_id);
        assert_eq!(state.total_cost, 0.0);
        assert!(state.response_costs.contains_key(&message_id));

        handle_stream_update(
            &mut state,
            StreamMessage::Start {
                message_id,
                kind: StreamStartKind::UserTurn {
                    model: "different-model".to_string(),
                },
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TextDelta {
                message_id,
                content: " final".to_string(),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: second.total_tokens,
                usage: Some(second.clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();
        handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
            .await
            .unwrap();

        let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
            + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
        assert_eq!(state.total_cost, expected);
        assert!(!state.response_costs.contains_key(&message_id));
        assert!(!state.response_models.contains_key(&message_id));
    }

    #[tokio::test]
    async fn reported_zero_cache_telemetry_replaces_prior_cache_counters() {
        let mut state = ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        state.cache_read_tokens = 500;
        state.cache_creation_tokens = 250;

        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id: 42,
                tokens: 100,
                usage: Some(crate::types::StarUsage {
                    prompt_tokens: 100,
                    total_tokens: 100,
                    cache_telemetry_reported: true,
                    ..Default::default()
                }),
            },
            &agent_tx,
        )
        .await
        .unwrap();

        assert_eq!(state.cache_read_tokens, 0);
        assert_eq!(state.cache_creation_tokens, 0);
    }
}

fn extract_last_code_block(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut last_block_start: Option<usize> = None;
    let mut last_block_end: Option<usize> = None;
    let mut in_block = false;
    let mut block_start = 0;

    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            if in_block {
                last_block_start = Some(block_start);
                last_block_end = Some(i);
                in_block = false;
            } else {
                block_start = i + 1;
                in_block = true;
            }
        }
    }
    // Handle unclosed block at end of content
    if in_block {
        last_block_start = Some(block_start);
        last_block_end = Some(lines.len());
    }

    match (last_block_start, last_block_end) {
        (Some(start), Some(end)) if end > start => Some(lines[start..end].join("\n")),
        _ => None,
    }
}
