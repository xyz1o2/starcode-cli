use crate::core::i18n;
use crate::runtime::messages::AgentRequest;
use crate::types::{ChatEntry, ChatEntryType, ToolResult};
use crate::ui::services::at_processor;
use crate::ui::state::ChatState;
use crate::ui::utils::text::{is_status_text, sanitize_filename, strip_tool_running_prefix};
use crate::ui::utils::transcript::{append_transcript_event, build_user_transcript_payload};
use serde_json::json;
use std::time::Instant;
use tokio::sync::mpsc;

pub fn emit_status_text(state: &mut ChatState, message_id: u64, text: &str) {
    let note = text.trim();
    if note.is_empty() {
        return;
    }
    if !is_status_text(note) {
        return;
    }

    if state.current_status_line.as_deref() == Some(note) {
        return;
    }

    if message_id == 0 {
        state.current_status_line = Some(note.to_string());
        return;
    }

    state.current_status_line = Some(note.to_string());
}

pub fn recover_missing_tool_results(state: &mut ChatState, message_id: u64, reason: &str) {
    let Some(&assistant_idx) = state.stream_targets.get(&message_id) else {
        return;
    };
    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);

    let mut pending_ids: Vec<String> = Vec::new();
    let safe_end = assistant_idx.min(state.chat_history.len());
    for i in start_idx..safe_end {
        let e = &state.chat_history[i];
        if e.entry_type != ChatEntryType::ToolCall {
            continue;
        }
        if e.is_streaming != Some(true) {
            continue;
        }
        if let Some(tc) = e.tool_call.as_ref() {
            if state.tool_started_at.contains_key(&tc.id) {
                pending_ids.push(tc.id.clone());
            }
        }
    }

    if pending_ids.is_empty() {
        return;
    }

    let mut recovered_count: usize = 0;

    for tool_call_id in pending_ids {
        let mut found: Option<usize> = None;
        for i in (start_idx..safe_end).rev() {
            let e = &state.chat_history[i];
            if e.entry_type != ChatEntryType::ToolCall {
                continue;
            }
            if e.is_streaming != Some(true) {
                continue;
            }
            if let Some(tc) = e.tool_call.as_ref() {
                if tc.id == tool_call_id {
                    found = Some(i);
                    break;
                }
            }
        }

        // Clean up the tool timing entry even though we don't display the
        // elapsed time for interrupted tools.
        state.tool_started_at.remove(&tool_call_id);

        let Some(i) = found else {
            continue;
        };

        let brief = state.chat_history[i]
            .tool_call
            .as_ref()
            .map(|tc| strip_tool_running_prefix(&crate::ui::utils::format::format_tool_call(tc)))
            .unwrap_or_default();

        let (tool_name, tool_args) = state.chat_history[i]
            .tool_call
            .as_ref()
            .map(|tc| {
                let args = state
                    .tool_call_args_cache
                    .get(&tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| tc.function.arguments.clone());
                (tc.function.name.clone(), args)
            })
            .unwrap_or_else(|| {
                let args = state
                    .tool_call_args_cache
                    .get(&tool_call_id)
                    .cloned()
                    .unwrap_or_default();
                (String::new(), args)
            });

        // Silently mark missing tool as failed — no alarming warning message.
        // The tool call entry just transitions to a clean "interrupted" state.
        let content = format!("{} (interrupted)", brief);

        state.chat_history[i].entry_type = ChatEntryType::ToolResult;
        state.chat_history[i].content = content;
        state.chat_history[i].timestamp = chrono::Utc::now();
        state.chat_history[i].tool_result = Some(ToolResult {
            success: false,
            output: None,
            error: Some("stream ended before result".to_string()),
            data: None,
        });
        state.chat_history[i].is_streaming = Some(false);

        append_transcript_event(
            state,
            "tool_result_missing",
            Some(message_id),
            json!({
                "tool_call_id": tool_call_id,
                "name": tool_name,
                "arguments": tool_args,
                "reason": reason,
            }),
        );

        recovered_count += 1;
    }

    if recovered_count > 0 {
        // Log the recovery for diagnostics but don't show a distracting
        // warning card in the chat — the individual tool entries already
        // show "(interrupted)".
        append_transcript_event(
            state,
            "recovery_notice",
            Some(message_id),
            json!({
                "kind": "tool_result_missing",
                "count": recovered_count,
                "reason": reason,
            }),
        );
    }
}

pub fn save_tool_output(tool_call: &crate::types::StarToolCall, output: &str) -> Option<String> {
    let tool_name = sanitize_filename(&tool_call.function.name);
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!("tool_output_{}_{}.txt", tool_name, timestamp);

    let mut path = std::env::temp_dir();
    path.push("starcode");
    let _ = std::fs::create_dir_all(&path);
    path.push(&filename);

    if std::fs::write(&path, output).is_ok() {
        Some(path.to_string_lossy().to_string())
    } else {
        None
    }
}

/// 流式期间可安全立即执行的命令（纯 UI / 只读查询）。
/// 会替换会话（/resume /clear）、与 agent 交互（/compress /review）、
/// 或做破坏性操作（/session delete /restore）的命令不在此列，仍排队等待。
fn is_streaming_safe_command(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('/') else {
        return false;
    };
    let mut it = rest.split_whitespace();
    match (it.next(), it.next()) {
        // 无子命令限制的安全命令
        (
            Some(
                "tasks" | "todos" | "stats" | "cost" | "tokens" | "usage" | "status" | "about"
                | "version" | "help" | "tools" | "bashes" | "context" | "files" | "diff" | "export"
                | "doctor" | "bug" | "feedback" | "ide" | "theme" | "lang" | "vim" | "models"
                | "model" | "model-info" | "provider-info" | "token-count" | "workflows",
            ),
            _,
        ) => true,
        // 子命令受限的安全命令
        (Some("memory"), Some("show")) => true,
        (Some("session"), None | Some("list")) => true,
        _ => false,
    }
}

pub async fn enqueue_user_message(
    state: &mut ChatState,
    user_input: String,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Ensure we jump back to latest messages when user submits input.
    state.auto_follow = true;

    // Trigger send animation
    state.send_animation_since = Some(Instant::now());

    // Clear saved draft
    state.clear_draft();

    // Save to command history (deduplicate consecutive entries)
    let trimmed = user_input.trim().to_string();
    if !trimmed.is_empty() {
        let is_duplicate = state
            .command_history
            .front()
            .map(|last| last == &trimmed)
            .unwrap_or(false);
        if !is_duplicate {
            state.command_history.push_front(trimmed.clone());
            if state.command_history.len() > 100 {
                state.command_history.pop_back();
            }
            crate::core::config::history_store::save_history(&state.command_history);
        }
    }
    // Reset history navigation state
    state.history_index = None;
    state.history_input_snapshot = None;

    if state.pending_confirmation_entry_idx.is_some() {
        state.pending_user_messages.push_back(user_input.clone());
        state
            .queued_messages_display
            .push_back((user_input, Instant::now()));
        let n = state.pending_user_messages.len();
        state.current_status_line = Some(format!("\u{23f3} {} pending", n));
        return Ok(());
    }

    // 流式期间的"安全命令"直接执行，不进等待队列：
    // 仅限纯 UI / 只读查询类命令；会改动会话状态或与 agent 交互的命令仍排队
    let is_safe_command_during_stream = is_streaming_safe_command(user_input.trim());

    if (state.is_processing || state.is_streaming) && !is_safe_command_during_stream {
        state.pending_user_messages.push_back(user_input.clone());
        state
            .queued_messages_display
            .push_back((user_input, Instant::now()));
        let n = state.pending_user_messages.len();
        state.current_status_line = Some(format!("\u{23f3} {} pending", n));
        return Ok(());
    }

    // Check for # shortcut (Memory)
    if user_input.trim().starts_with('#') {
        let content = user_input.trim()[1..].trim();
        if !content.is_empty() {
            let cmd_str = format!("/memory add {}", content);
            if let Some(parsed) = crate::commands::system::parse_command(&cmd_str) {
                let mut args_vec = Vec::new();
                if parsed.path.len() > 1 {
                    args_vec.extend_from_slice(&parsed.path[1..]);
                }
                if !parsed.args.is_empty() {
                    args_vec.extend(parsed.args.split_whitespace().map(|s| s.to_string()));
                }

                // Show the command in history as user message (preserving original input)
                state.chat_history.push(ChatEntry::user(user_input.clone()));

                let ctx = crate::commands::execution::CommandContext { state, agent_tx };

                match crate::commands::handle_command(&parsed.path[0], args_vec, ctx).await {
                    Ok(_) => return Ok(()),
                    Err(e) => {
                        state.chat_history.push(ChatEntry {
                            is_streaming: Some(false),
                            ..ChatEntry::assistant(format!("Command Error: {}", e))
                        });
                        return Ok(());
                    }
                }
            }
        }
    }

    // Check for slash command
    if user_input.trim().starts_with('/') {
        let (command_name, args_vec) =
            if let Some(parsed) = crate::commands::system::parse_command(&user_input) {
                let mut args_vec = Vec::new();
                if parsed.path.len() > 1 {
                    args_vec.extend_from_slice(&parsed.path[1..]);
                }
                if !parsed.args.is_empty() {
                    args_vec.extend(parsed.args.split_whitespace().map(|s| s.to_string()));
                }
                (parsed.path[0].clone(), args_vec)
            } else {
                let without_slash = user_input.trim()[1..].trim();
                let parts = without_slash
                    .split_whitespace()
                    .map(|part| part.to_string())
                    .collect::<Vec<_>>();
                if parts.is_empty() {
                    return Ok(());
                }
                (parts[0].clone(), parts[1..].to_vec())
            };

        // Show the command in history as user message
        state.chat_history.push(ChatEntry::user(user_input.clone()));

        let ctx = crate::commands::execution::CommandContext { state, agent_tx };

        match crate::commands::handle_command(&command_name, args_vec, ctx).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                state.chat_history.push(ChatEntry {
                    is_streaming: Some(false),
                    ..ChatEntry::assistant(format!("Command Error: {}", e))
                });
                return Ok(());
            }
        }
    }

    // Check for ! shortcut (Bash/Shell) - Direct Execution
    // Exclude markdown image syntax ![...] which starts with ! but is NOT a shell command
    if user_input.trim().starts_with('!') && !user_input.trim().starts_with("![") {
        if state.approval_mode == crate::types::ApprovalMode::Plan {
            state.chat_history.push(ChatEntry {
                 is_streaming: Some(false),
                 ..ChatEntry::assistant("⚠️ Plan Mode is active. Direct shell execution is disabled in this mode to prevent accidental changes. Please switch to Default mode (Shift+Tab) to execute commands.")
             });
            return Ok(());
        }

        let cmd = user_input.trim()[1..].trim();
        if !cmd.is_empty() {
            // Show the command in history as user message
            state.chat_history.push(ChatEntry::user(user_input.clone()));

            // Execute directly
            let cwd = crate::core::utils::paths::current_dir_cached();
            let config = crate::core::services::shell_execution_service::ShellExecutionConfig {
                terminal_width: None,
                terminal_height: None,
                pager: None,
                show_color: Some(true),
                default_fg: None,
                default_bg: None,
                sanitization_config:
                    crate::core::services::shell_execution_service::EnvironmentSanitizationConfig {
                        allowed_environment_variables: vec![],
                        blocked_environment_variables: vec![],
                        enable_environment_variable_redaction: false,
                    },
                disable_dynamic_line_trimming: None,
                scrollback: None,
            };

            let result_handle =
                crate::core::services::shell_execution_service::ShellExecutionService::execute(
                    cmd,
                    cwd.to_str().unwrap_or("."),
                    |_| {},
                    None,
                    config,
                )
                .await;

            match result_handle {
                Ok(handle) => {
                    // ShellExecutionService::execute returns a handle that runs in background.
                    // We await the result immediately for this synchronous-like CLI experience.
                    let result = handle.result.await;
                    match result {
                        Ok(res) => {
                            let content = if res.exit_code == Some(0) {
                                res.output
                            } else {
                                format!("Exit Code: {:?}\n\n{}", res.exit_code, res.output)
                            };

                            state.chat_history.push(ChatEntry {
                                is_streaming: Some(false),
                                ..ChatEntry::assistant(content)
                            });
                        }
                        Err(e) => {
                            state.chat_history.push(ChatEntry {
                                is_streaming: Some(false),
                                ..ChatEntry::assistant(format!("Execution Panic: {}", e))
                            });
                        }
                    }
                }
                Err(e) => {
                    state.chat_history.push(ChatEntry {
                        is_streaming: Some(false),
                        ..ChatEntry::assistant(format!("Execution Start Error: {}", e))
                    });
                }
            }
            return Ok(());
        }
    }

    if state.current_model.trim().is_empty()
        && state
            .current_provider_id
            .as_deref()
            .map(|provider_id| !provider_id.trim().is_empty())
            .unwrap_or(false)
    {
        state.show_palette = true;
        state.palette_history.clear();
        state.palette_mode = crate::ui::state::palette::PaletteMode::Model;
        if state.available_models.is_empty() {
            state.awaiting_models = true;
            let _ = agent_tx.send(AgentRequest::ListModels).await;
        }
        state.palette_items = crate::ui::components::palette::get_items(
            &crate::ui::state::palette::PaletteMode::Model,
            state,
        );
        state.selected_palette_index = 0;
        state.palette_filter.clear();
        state.chat_history.push(ChatEntry {
            is_streaming: Some(false),
            ..ChatEntry::assistant(
                &i18n::t(
                    "ui.provider.no_model_selected",
                    "当前 Provider 尚未选择模型。请先选择该 Provider 的模型后再发送消息。",
                    "No model selected for the current provider. Please select a model before sending a message.",
                ),
            )
        });
        state.current_status_line = Some(i18n::t(
            "ui.provider.no_model_selected.status",
            "请先选择当前 Provider 的模型",
            "Please select a model for the current provider",
        ));
        return Ok(());
    }

    let message_id = state.next_message_id;
    state.next_message_id += 1;
    state.active_message_id = Some(message_id);

    let auto_continue_max = std::env::var("STAR_AUTO_CONTINUE_MAX")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    state.auto_continue_remaining = auto_continue_max;
    state.auto_continued_message_ids.remove(&message_id);

    // Check for ! shortcut (Bash/Shell)
    let processing_input = user_input.clone();

    // 处理 @ 命令，读取文件内容
    let workspace_root = if at_processor::may_contain_at_command(&processing_input) {
        Some(crate::core::utils::paths::current_dir_cached().as_path())
    } else {
        None
    };
    let processed = at_processor::process_at_command(&processing_input, workspace_root);

    let user_entry_idx = state.chat_history.len();

    // 显示用户原始输入
    state.chat_history.push(ChatEntry::user(user_input.clone()));

    // 如果有文件内容，显示已读取的文件
    if !processed.file_contents.is_empty() {
        let fmt_size = |bytes: usize| -> String {
            if bytes >= 1024 {
                format!("{:.1} KB", bytes as f64 / 1024.0)
            } else {
                format!("{} B", bytes)
            }
        };

        let cwd = workspace_root.as_deref();
        let display_path = |raw: &str| -> String {
            let p = std::path::PathBuf::from(raw);
            if p.is_absolute() {
                if let Some(cwd) = cwd {
                    if let Ok(rel) = p.strip_prefix(cwd) {
                        let s = rel.to_string_lossy().to_string();
                        if !s.is_empty() {
                            return s;
                        }
                    }
                }
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    return name.to_string();
                }
            }
            raw.to_string()
        };

        let items: Vec<String> = processed
            .file_contents
            .iter()
            .map(|f| format!("{} ({})", display_path(&f.path), fmt_size(f.size)))
            .collect();

        let content = if items.len() == 1 {
            format!("状态：已读取 1 个文件: {}", items[0])
        } else {
            let mut s = format!("状态：已读取 {} 个文件:", items.len());
            for it in &items {
                s.push_str("\n  ");
                s.push_str(it);
            }
            s
        };

        emit_status_text(state, message_id, &content);

        // Notify agent that files have been read
        let paths: Vec<String> = processed
            .file_contents
            .iter()
            .map(|f| f.path.clone())
            .collect();
        let _ = agent_tx.send(AgentRequest::MarkFilesAsRead(paths)).await;
    }

    // 如果有错误，显示错误信息
    if !processed.errors.is_empty() {
        let errors_text = processed.errors.join("\n  - ");
        state.chat_history.push(ChatEntry::assistant(format!(
            "异常：文件读取错误:\n  - {}",
            errors_text
        )));
    }

    // 使用处理后的消息（包含文件内容）
    let final_message = at_processor::format_processed_message(&processed);

    append_transcript_event(
        state,
        "user",
        Some(message_id),
        build_user_transcript_payload(&user_input, &final_message, processed.file_contents.len()),
    );

    let start_idx = user_entry_idx;
    state.chat_history.push(ChatEntry {
        is_streaming: Some(true),
        ..ChatEntry::assistant("")
    });

    let response_idx = state.chat_history.len() - 1;
    state.stream_targets.insert(message_id, response_idx);
    state.message_start_indices.insert(message_id, start_idx);

    if !state.is_processing {
        state.is_processing = true;
        state.processing_started_at = Some(Instant::now());
        state.processing_time_secs = 0;
    }
    state.model_wait_started_at = None;
    state.cancelling_since = None;
    state.is_streaming = true;

    emit_status_text(state, message_id, "状态：分析中");

    crate::utils::logging::append_debug_log_line(&format!(
        "[UI] Enqueue user message: id={}, raw_chars={}, final_chars={}, attachments={}",
        message_id,
        user_input.chars().count(),
        final_message.chars().count(),
        processed.file_contents.len()
    ));

    // 发送处理后的消息（包含文件内容）给 Agent
    let _ = agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: final_message,
        })
        .await;
    Ok(())
}
