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
use crate::runtime::messages::{AgentRequest, StreamMessage};
use crate::types::{ChatEntry, ChatEntryType, StarToolCall, ToolResult};
use crate::ui::app::logic::{
    emit_status_text, enqueue_user_message, recover_missing_tool_results, save_tool_output,
};
use crate::ui::state::store::ChatState;
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
            token_usage,
        } => {
            state.au2_compressed = au2_compressed;
            // Update cache stats from usage data (use latest values, not accumulate)
            if let Some(ref usage) = token_usage {
                if usage.cache_read_tokens > 0 || usage.cache_creation_tokens > 0 {
                    state.cache_read_tokens = usage.cache_read_tokens as u64;
                    state.cache_creation_tokens = usage.cache_creation_tokens as u64;
                }
            }
            state.token_usage = token_usage;
            append_transcript_event(
                state,
                "stats_update",
                state.active_message_id,
                serde_json::json!({
                    "au2_compressed": state.au2_compressed,
                    "token_usage": state.token_usage,
                }),
            );
        }
        StreamMessage::Start { message_id } => {
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
            message_id: _,
            tokens,
            usage,
        } => {
            state.token_count = tokens;
            if let Some(ref u) = usage {
                // Update cache token tracking
                if u.cache_read_tokens > 0 || u.cache_creation_tokens > 0 {
                    state.cache_read_tokens = u.cache_read_tokens as u64;
                    state.cache_creation_tokens = u.cache_creation_tokens as u64;
                }
                // Accumulate usage: keep max prompt_tokens, update completion_tokens
                // API returns cumulative usage per-turn, so we take the latest values
                match &mut state.token_usage {
                    Some(existing) => {
                        // prompt_tokens grows as context grows — keep the max
                        existing.prompt_tokens = existing.prompt_tokens.max(u.prompt_tokens);
                        // completion_tokens is per-turn — update if newer data available
                        if u.completion_tokens > 0 {
                            existing.completion_tokens = u.completion_tokens;
                        }
                        existing.total_tokens = existing.prompt_tokens + existing.completion_tokens;
                        existing.cache_read_tokens = u.cache_read_tokens;
                        existing.cache_creation_tokens = u.cache_creation_tokens;
                    }
                    None => {
                        state.token_usage = Some(u.clone());
                    }
                }
            } else if state.token_usage.is_none() {
                // No usage from API and no existing usage — estimate from token_count
                state.token_usage = Some(crate::types::StarUsage {
                    prompt_tokens: tokens,
                    completion_tokens: 0,
                    total_tokens: tokens,
                    ..Default::default()
                });
            }
        }
        StreamMessage::Done { message_id } => {
            handle_done_message(state, agent_tx, message_id).await?
        }
        StreamMessage::Error { message_id, error } => {
            recover_missing_tool_results(state, message_id, &error);

            // ── Cancel transition: preserve thinking block 1.5s after ESC/Ctrl+C ──
            let cancelling_graceful = state
                .cancelling_since
                .map(|t| t.elapsed() < std::time::Duration::from_millis(1500))
                .unwrap_or(false);

            // Don't clear Assistant's streaming state during cancel transition
            if !cancelling_graceful {
                if let Some(assistant_idx) = state.stream_targets.get(&message_id).copied() {
                    if assistant_idx < state.chat_history.len()
                        && state.chat_history[assistant_idx].entry_type == ChatEntryType::Assistant
                    {
                        finalize_entry_streaming(state, assistant_idx);
                    }
                }
            }

            state.is_processing = false;
            state.is_processing = false;
            state.current_tool_name = None;
            state.thinking_started_at = None;
            if !cancelling_graceful {
                state.is_streaming = false;
            }
            state.model_wait_started_at = None;
            state.processing_started_at = None;
            state.active_message_id = Some(message_id);
            // Clean up stream tracking maps (normally done in Done handler)
            state.stream_targets.remove(&message_id);
            state.message_start_indices.remove(&message_id);

            // Classify error and show overlay for retryable errors
            let error_type = crate::ui::components::error_overlay::classify_error(&error);
            if crate::ui::components::error_overlay::is_retryable(&error_type) {
                state.show_error_overlay = true;
                state.error_overlay_state =
                    crate::ui::components::error_overlay::ErrorOverlayState {
                        error_message: error.clone(),
                        error_type,
                        retry_count: 0,
                        max_retries: 10,
                        is_retrying: false,
                        selected_action: crate::ui::components::error_overlay::ErrorAction::Retry,
                    };
            }

            emit_status_text(
                state,
                message_id,
                &i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                    .replace("{error}", &error),
            );
            state.chat_history.push(
                ChatEntry::assistant(
                    i18n::t("ui.status.error", "Error: {error}", "Error: {error}")
                        .replace("{error}", &error),
                )
                .with_streaming(false),
            );
            append_transcript_event(
                state,
                "error",
                Some(message_id),
                serde_json::json!({
                    "error": error,
                }),
            );
            if !state.is_awaiting_confirmation {
                if let Some(next_input) = state.pending_user_messages.pop_front() {
                    let remaining = state.pending_user_messages.len();
                    if remaining > 0 {
                        state.current_status_line = Some(format!("\u{23f3} {} pending", remaining));
                    } else {
                        state.current_status_line = None;
                    }
                    enqueue_user_message(state, next_input, agent_tx).await?
                }
            }
        }
        StreamMessage::ModelsList(models) => {
            state.available_models = models.iter().map(|m| m.id.clone()).collect();
            state.available_models_info = models.clone(); // 保存完整的模型信息
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
            if state.show_palette
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
                );
            }
            // Fallback: models arrived, open palette in Model mode for selection
            else if !state.available_models.is_empty() {
                state.show_palette = true;
                state.palette_history.clear();
                state.palette_mode = crate::ui::state::palette::PaletteMode::Model;
                state.palette_items = crate::ui::components::palette::get_model_palette_items(
                    &state.available_models,
                    &state.current_model,
                    state.awaiting_models,
                    &state.model_provider_map,
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
                    "Build mode: confirm dangerous actions; manage tasks with Ctrl+B or /tasks",
                    "Build mode: confirm dangerous actions; manage tasks with Ctrl+B or /tasks",
                ),
                crate::types::ApprovalMode::Plan => i18n::t(
                    "ui.mode.plan.desc",
                    "Plan mode: read-only research; manage plans with /tasks or Ctrl+B",
                    "Plan mode: read-only research; manage plans with /tasks or Ctrl+B",
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
    // 首次进入 resolved 时冻结耗时，之后不再随渲染增长
    let finished_at = prior
        .and_then(|i| i.finished_at)
        .or_else(|| args.is_resolved.then(std::time::Instant::now));
    let mut sub_entries = prior.map(|i| i.sub_entries.clone()).unwrap_or_default();
    sub_entries.extend(args.new_sub_entries.iter().cloned());
    // 进度 chunk 未携带这些字段时沿用旧值，避免状态行闪回 "Initializing…"
    let last_tool_info = args
        .last_tool_info
        .clone()
        .or_else(|| prior.and_then(|i| i.last_tool_info.clone()));
    let name = args.name.clone().or_else(|| prior.and_then(|i| i.name.clone()));
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

async fn handle_done_message(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    message_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
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
        if idx < state.chat_history.len() {
            if state.chat_history[idx].entry_type == ChatEntryType::Assistant {
                if !cancelling_graceful {
                    finalize_entry_streaming(state, idx);
                }
            }
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
    // Auto-continue when tools were used and we haven't exceeded the limit.
    // Removed the `assistant_empty` requirement — even if the LLM gave partial
    // text, we should still continue if tools were involved (the task may be
    // incomplete). The nudge asks for a "final answer" so the LLM will wrap up.
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

        // 在聊天区域显示明确的继续提示，让用户知道 Agent 还在工作
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
        }
        state.rendered_cache.remove(&idx);
        state.last_rendered_stream_key.remove(&idx);
        state.virtual_list.mark_dirty(idx);
    }
    state.is_processing = false;
    state.current_tool_name = None;
    state.thinking_started_at = None;
    state.last_token_time = None;
    state.queued_messages_display.clear();
    if !cancelling_graceful {
        state.is_streaming = false;
        state.current_status_line = None;
        // 显示完成提示，让用户明确知道 Agent 已完成
        state.current_status_line = Some("✓ Done".to_string());
    }
    state.model_wait_started_at = None;
    state.processing_started_at = None;
    state.active_message_id = Some(message_id);
    state.complete_task_message_ids.insert(message_id);
    state.stream_targets.remove(&message_id);
    state.message_start_indices.remove(&message_id);

    // Compute and store per-response cost
    if let Some(idx) = assistant_idx {
        if idx < state.chat_history.len() {
            if let Some(ref usage) = state.token_usage {
                let cost =
                    crate::ui::utils::cost::compute_response_cost(usage, &state.current_model);
                state.chat_history[idx].cost = Some(cost);
                state.total_cost += cost;
            }
            // Extract the last code block for copy-on-key feature
            let content = &state.chat_history[idx].content;
            state.last_code_block_content = extract_last_code_block(content);
        }
    }

    append_transcript_event(
        state,
        "done",
        Some(message_id),
        serde_json::json!({
            "auto_continued": false,
        }),
    );
    if !state.is_awaiting_confirmation {
        if let Some(next_input) = state.pending_user_messages.pop_front() {
            let remaining = state.pending_user_messages.len();
            if remaining > 0 {
                state.current_status_line = Some(format!("\u{23f3} {} pending", remaining));
            } else {
                state.current_status_line = None;
            }
            enqueue_user_message(state, next_input, agent_tx).await?
        }
    }
    Ok(())
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
            }
        }
    }
    if matches!(
        tool_call.function.name.as_str(),
        "Todo" | "complete_task" | "TodoWrite"
    ) {
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
                    let agent_type = data
                        .get("agent_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or(if status_str == "fork_launched" {
                            "Fork"
                        } else {
                            "Agent"
                        });
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
                            serde_json::from_str::<serde_json::Value>(
                                &tool_call.function.arguments,
                            )
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
                        state.chat_history.remove(tc_idx);
                        state.virtual_list.mark_dirty_all();
                        state.rendered_cache.clear();
                    }
                    return;
                }
            }
        }
    }

    state.current_tool_name = None;
    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);
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
    let elapsed_ms = state
        .tool_started_at
        .remove(&tool_call.id)
        .map(|t| t.elapsed().as_millis());
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
