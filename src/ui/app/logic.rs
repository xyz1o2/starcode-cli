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
                | "model" | "model-info" | "provider-info" | "token-count" | "workflows"
                | "network" | "detach",
            ),
            _,
        ) => true,
        // 子命令受限的安全命令
        (Some("memory"), Some("show")) => true,
        (Some("session"), None | Some("list")) => true,
        _ => false,
    }
}

/// `!command` 输出进上下文前的截断：保留头尾，掐掉中间
///
/// 一次 `!cargo test` 可能刷出几万字符，整段塞进上下文会把预算吃干。
/// 报错信息通常在头部或尾部，中间是重复的进度行 —— 所以两头都留。
/// 上限用 STAR_LOCAL_COMMAND_OUTPUT_MAX_CHARS 调（默认 8000）。
fn truncate_local_command_output(output: &str) -> String {
    let max_chars = std::env::var("STAR_LOCAL_COMMAND_OUTPUT_MAX_CHARS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(8000);

    let chars: Vec<char> = output.chars().collect();
    if chars.len() <= max_chars {
        return output.to_string();
    }

    let head_len = max_chars * 2 / 3;
    let tail_len = max_chars - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    format!(
        "{head}\n\n... [{} chars omitted] ...\n\n{tail}",
        chars.len() - max_chars
    )
}

/// 运行中输入的内容是不是一条「需求」。
///
/// `/command`、`#memory`、`!shell` 都有各自的处理路径（开面板 / 改会话 / 直接跑
/// 命令），不能当需求插进对话。`![` 是 markdown 图片语法，与 `!shell` 无关，
/// 按需求处理。
fn is_plain_requirement(input: &str) -> bool {
    let trimmed = input.trim_start();
    if trimmed.starts_with('/') || trimmed.starts_with('#') {
        return false;
    }
    !(trimmed.starts_with('!') && !trimmed.starts_with("!["))
}

/// 把运行中输入的需求插进**当前这一轮**。
///
/// 之前的行为是排队进 `pending_user_messages`，等本轮跑完才发出去 —— 用户在
/// agent 还在跑时提的补充要求，模型要等到整轮结束才看得见；这期间它可能已经
/// 沿着错误的方向跑完了，用户还得再等一整轮才看得到自己的补充被采纳。
///
/// 现在这条需求立刻进入对话：runtime 侧把回合中的 `SendMessage` 收进 steering
/// 队列，agent 侧在每轮开始时消费它、作为 user 消息插进对话。
///
/// 聊天区里的顺序是「助手已输出的前半 / 用户这条 / 助手还没写完的后半」。
/// 前半在这里被收尾，后半另起一条继续流 —— 切点必须落在 markdown 的安全边界
/// 上，否则前半的代码块没闭合，渲染时会把用户这条一起当成代码。
async fn steer_user_message(
    state: &mut ChatState,
    user_input: String,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    // `@file` 引用与普通发送走同一套处理：插进对话的必须是展开后的正文。
    let workspace_root = if at_processor::may_contain_at_command(&user_input) {
        Some(crate::core::utils::paths::current_dir_cached().as_path())
    } else {
        None
    };
    let processed =
        at_processor::process_at_command(&user_input, workspace_root, &state.extra_working_dirs);
    let final_message = at_processor::format_processed_message(&processed);

    // 助手正在输出的那段正文要在这里切开：前半留在用户消息之前，还没写完的
    // 后半挪到用户消息之后接着流。
    //
    // 切点必须落在 markdown 的安全边界上，否则前半的代码块没闭合，渲染时会把
    // 用户那条消息一起当成代码 —— 消息就"插到 markdown 里面"了。
    //
    // `stream_targets` **可能是悬空的**：工具调用之后它指向 `chat_history.len()`
    // （等下一段内容再建一条助手条目，见 stream.rs 的 ToolCalls 分支）。那种
    // 时候没有可切的助手条目，硬切就越界了。
    let mut resumed_entry: Option<(u64, String)> = None;
    if let Some(turn_id) = state.active_message_id {
        if let Some(&idx) = state.stream_targets.get(&turn_id) {
            if idx < state.chat_history.len()
                && state.chat_history[idx].entry_type == ChatEntryType::Assistant
            {
                let (head, tail) = crate::utils::markdown_parser::split_markdown_at_safe_boundary(
                    &state.chat_history[idx].content,
                );
                state.chat_history[idx].content = head;
                crate::ui::services::stream::finalize_entry_streaming(state, idx);
                resumed_entry = Some((turn_id, tail));
            }
        }
    }

    // 用户输入照常显示在聊天区。
    state.chat_history.push(ChatEntry::user(user_input.clone()));

    // 助手那条要么被切走（后半另起一条继续流），要么本来就不存在/是悬空的 ——
    // 两种情况都得把流的落点挪到用户消息**之后**，否则下一段助手内容会被追加
    // 到用户这条上（`handle_content_message` 只认助手/工具条目）。
    if let Some(turn_id) = state.active_message_id {
        match resumed_entry {
            Some((tid, tail)) if !tail.is_empty() => {
                let tail_idx = state.chat_history.len();
                state
                    .chat_history
                    .push(ChatEntry::assistant(tail).with_streaming(true));
                state.stream_targets.insert(tid, tail_idx);
                state.rendered_cache.remove(&tail_idx);
                state.virtual_list.mark_dirty(tail_idx);
            }
            _ => {
                // 指向末尾：下一段内容会在这里另起一条助手条目。
                state
                    .stream_targets
                    .insert(turn_id, state.chat_history.len());
            }
        }
    }

    // 这条属于**当前这一轮**，但要用自己的 message_id：万一当前这轮在 agent
    // 消费它之前就结束了，runtime 会把它当成新的一条用户消息起新一轮 ——
    // 沿用当前回合的 id 会让那一轮的 Done 被当成重复消息吞掉。
    let message_id = state.next_message_id;
    state.next_message_id += 1;

    append_transcript_event(
        state,
        "user",
        Some(message_id),
        build_user_transcript_payload(&user_input, &final_message, processed.file_contents.len()),
    );

    crate::utils::logging::append_debug_log_line(&format!(
        "[UI] Steering user message into the running turn: id={}, raw_chars={}, final_chars={}",
        message_id,
        user_input.chars().count(),
        final_message.chars().count(),
    ));

    // 发出：runtime 在回合进行中会把它收进 steering 队列，agent 在下一轮开始时消费。
    let _ = agent_tx
        .send(AgentRequest::SendMessage {
            message_id,
            message: final_message,
        })
        .await;

    state.current_status_line = Some(crate::core::i18n::t(
        "ui.status.steered",
        "已插入当前回合",
        "Added to the running turn",
    ));

    Ok(())
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

    // 等待工具确认 / 本轮还在跑 —— 输入进队列，由输入框上方的排队面板
    // （queued_input，对标 PromptInputQueuedCommands）原样显示出来，
    // 所以这里不再往状态栏塞 "⏳ N pending" 这种看不出内容的提示。
    //
    // 需求是个例外：它插进当前这一轮（模型下一次调用就能看到），不用等本轮
    // 跑完 —— 工具在等审批时用户补充的说明，正是要赶在下一轮之前送达的。
    if state.pending_confirmation_entry_idx.is_some() {
        if is_plain_requirement(&user_input) {
            return steer_user_message(state, user_input, agent_tx).await;
        }
        state.pending_user_messages.push_back(user_input);
        return Ok(());
    }

    // 流式期间的"安全命令"直接执行，不进等待队列：
    // 仅限纯 UI / 只读查询类命令；会改动会话状态或与 agent 交互的命令仍排队
    let is_safe_command_during_stream = is_streaming_safe_command(user_input.trim());

    if (state.is_processing || state.is_streaming) && !is_safe_command_during_stream {
        // 需求插进当前这一轮（模型下一次调用就能看到），不用等本轮跑完。
        // 命令类（/ # !）仍排队 —— 它们各有各的处理路径，不是需求。
        if is_plain_requirement(&user_input) {
            return steer_user_message(state, user_input, agent_tx).await;
        }
        state.pending_user_messages.push_back(user_input);
        return Ok(());
    }

    // Check for # shortcut (Memory)
    if user_input.trim().starts_with('#') {
        let content = user_input.trim()[1..].trim();
        if !content.is_empty() {
            if let Some(parsed) = crate::commands::system::parse_command("/memory add") {
                // Keep the tagged text opaque: `#foo/bar` is memory content,
                // not a command path or a quoted argument.
                let mut args_vec = crate::commands::system::parsed_command_args(&parsed);
                args_vec.push(content.to_string());

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
                let args_vec = crate::commands::system::parsed_command_args(&parsed);
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
    //
    // `!cmd`  → 本地执行，并把输出追加进模型上下文（对标 Claude Code）
    // `!!cmd` → 本地执行，输出只给人看，不进上下文
    if user_input.trim().starts_with('!') && !user_input.trim().starts_with("![") {
        if state.approval_mode == crate::types::ApprovalMode::Plan {
            state.chat_history.push(ChatEntry {
                 is_streaming: Some(false),
                 ..ChatEntry::assistant("⚠️ Plan Mode is active. Direct shell execution is disabled in this mode to prevent accidental changes. Please switch to Default mode (Shift+Tab) to execute commands.")
             });
            return Ok(());
        }

        let trimmed = user_input.trim();
        let local_only = trimmed.starts_with("!!");
        let cmd = trimmed[if local_only { 2 } else { 1 }..].trim();
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

                            // 先喂给模型再回显：`!!` 才跳过这一步。
                            // 包成 <local-command-stdout> 是为了让模型明确知道
                            // 这段是用户自己在终端里跑出来的，不是它调工具的结果。
                            if !local_only {
                                let payload = format!(
                                    "<local-command-stdout cmd=\"{}\">\n{}\n</local-command-stdout>",
                                    cmd.replace('"', "'"),
                                    truncate_local_command_output(&content)
                                );
                                let _ = agent_tx
                                    .send(AgentRequest::AppendContext { content: payload })
                                    .await;
                            }

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

    // 离线模式：拒绝发送真正的消息（/、#、! 前缀在上方已分流，能走到这里的是普通消息）。
    // 调用方在 enqueue 之前就清空了输入框（input.rs 的 reset_main_textarea），所以这里
    // 把文本放回去，用户 /network off 之后可以直接重发，不用重新打一遍。
    if state.network_offline || crate::core::offline::is_offline() {
        state.push_toast(
            "Offline mode is ON — message not sent. Use /network off to go back online.",
            crate::ui::state::store::ToastKind::Warning,
        );
        state.textarea.insert_str(&user_input);
        return Ok(());
    }

    if state.current_model.trim().is_empty()
        && state
            .current_provider_id
            .as_deref()
            .map(|provider_id| !provider_id.trim().is_empty())
            .unwrap_or(false)
    {
        state.palette_history.clear();
        let was_already_open = state.is_palette_open()
            && state.palette_mode == crate::ui::state::palette::PaletteMode::Model;
        state.open_palette(crate::ui::state::palette::PaletteMode::Model);
        if state.available_models.is_empty() {
            state.awaiting_models = true;
            state.awaiting_models_since = Some(std::time::Instant::now());
            let _ = agent_tx
                .send(AgentRequest::ListModels { force: false })
                .await;
        }
        // 已经在模型面板上、而且上一条就是这条提示时，**不要再推一条一模一样的**。
        //
        // 没有这个去重就会形成死循环：Esc 重发上一条 → 这里挡住并开面板 → 用户再按
        // Esc → 再重发 → 再推一条 …… 屏幕上一排相同的 "No model selected"，而且
        // 一次真正的问题（为什么模型是空的）被完全淹没。真实事故里这个循环转了 6 次。
        if !was_already_open {
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
        }
        state.current_status_line = Some(i18n::t(
            "ui.provider.no_model_selected.status",
            "请先选择当前 Provider 的模型",
            "Please select a model for the current provider",
        ));
        // 清掉失败标志：这条消息不可能成功，`last_turn_failed` 再留着只会让 Esc
        // 继续无意义地重发（每次都撞回这里）。
        state.last_turn_failed = false;
        return Ok(());
    }

    // /break-cache 的消费点：清掉进程内提示词缓存，让这一条消息的系统提示重新读盘，
    // 然后解除标记（只作用于下一条消息，对标 Claude Code 的一次性 cache break）。
    if state.break_cache_next {
        state.break_cache_next = false;
        let dropped = crate::core::prompts::loader::invalidate_cache();
        crate::utils::logging::append_debug_log_line(&format!(
            "[CACHE-BREAK] Dropped {} cached prompt file(s) before sending",
            dropped
        ));
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
    let processed = at_processor::process_at_command(
        &processing_input,
        workspace_root,
        &state.extra_working_dirs,
    );

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
            format!(
                "{} 1 {}: {}",
                crate::core::i18n::t("status.read_files", "状态：已读取", "Read"),
                crate::core::i18n::t("status.file", "个文件", "file"),
                items[0]
            )
        } else {
            let mut s = format!(
                "{} {} {}:",
                crate::core::i18n::t("status.read_files", "状态：已读取", "Read"),
                items.len(),
                crate::core::i18n::t("status.files", "个文件", "files")
            );
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

    // 新回合真的发出去了，之前那次失败不再是"当前状态"。
    state.last_turn_failed = false;

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

#[cfg(test)]
mod no_model_guard_tests {
    /// 回归："没选模型"的保护不能形成死循环。
    ///
    /// 真实事故：apinex 余额耗尽报错 → 用户按 Esc 重发 → `current_model` 是空的
    /// → `enqueue_user_message` 的保护代为打开模型面板并推一条 "No model
    /// selected…" → 再按 Esc → 再重发 → 再推一条 …… 屏幕上 6 行一模一样的报错，
    /// 而真正的问题（模型为什么是空的）被完全淹没。
    ///
    /// 面板已经开着、且上一条就是这条提示时，必须安静下来：不开第二次、不推第二条。
    #[tokio::test]
    async fn the_no_model_guard_does_not_repeat_itself() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_provider_id = Some("apinex".to_string());
        state.current_model = String::new();
        state.last_turn_failed = true;
        let (tx, _rx) = tokio::sync::mpsc::channel(16);

        crate::ui::app::logic::enqueue_user_message(&mut state, "hi".to_string(), &tx)
            .await
            .unwrap();
        assert_eq!(
            state
                .chat_history
                .iter()
                .filter(|e| e.content.contains("No model selected"))
                .count(),
            1,
            "第一次应当推一条提示"
        );
        assert!(state.is_palette_open(), "第一次应当打开模型面板");

        // 再来一次（模拟用户再按 Esc 重发）
        crate::ui::app::logic::enqueue_user_message(&mut state, "hi".to_string(), &tx)
            .await
            .unwrap();
        assert_eq!(
            state
                .chat_history
                .iter()
                .filter(|e| e.content.contains("No model selected"))
                .count(),
            1,
            "提示不能重复推 —— 这正是那 6 行重复报错的成因"
        );
        assert!(
            !state.last_turn_failed,
            "这条消息不可能成功，别再让 Esc 重发它"
        );
    }

    /// 选了模型时不受影响 —— 保护只针对"必然失败"这一种情况。
    #[tokio::test]
    async fn the_no_model_guard_does_not_fire_when_a_model_is_selected() {
        let mut state = crate::ui::state::ChatState::new();
        state.current_provider_id = Some("apinex".to_string());
        state.current_model = "free/deepseek-v4.1-flash".to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        crate::ui::app::logic::enqueue_user_message(&mut state, "hi".to_string(), &tx)
            .await
            .unwrap();

        assert!(!state.is_palette_open(), "有模型时不该被劫持去开模型面板");
        assert!(!state
            .chat_history
            .iter()
            .any(|e| e.content.contains("No model selected")));
        assert!(rx.try_recv().is_ok(), "消息应当真的被发出去");
    }
}

#[cfg(test)]
mod steering_tests {
    use crate::types::ChatEntryType;
    use crate::ui::state::ChatState;

    /// 运行中插入的需求会把助手正在写的那段输出切成两半：前半在用户消息之前，
    /// 后半在之后。**代码块不能被劈开** —— 前半的 ``` 没闭合时，渲染会把后面
    /// 整段（含用户那条消息）当成代码，消息就"插到 markdown 里面"了。
    #[tokio::test]
    async fn steering_splits_an_open_code_block_cleanly() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(7);

        // 助手正在写一个代码块，围栏还没闭合
        let streaming = "说明文字\n\n```rust\nfn main() {\n";
        let assistant_idx = state.chat_history.len();
        state
            .chat_history
            .push(crate::types::ChatEntry::assistant(streaming.to_string()).with_streaming(true));
        state.stream_targets.insert(7, assistant_idx);

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        super::steer_user_message(&mut state, "顺便补个测试".to_string(), &tx)
            .await
            .expect("steering should succeed");

        // 顺序：助手前半 / 用户消息 / 助手后半
        assert_eq!(
            state.chat_history.len(),
            assistant_idx + 3,
            "{:?}",
            state.chat_history
        );
        assert_eq!(
            state.chat_history[assistant_idx].entry_type,
            ChatEntryType::Assistant
        );
        assert_eq!(state.chat_history[assistant_idx].content, "说明文字\n\n");
        assert_eq!(
            state.chat_history[assistant_idx + 1].entry_type,
            ChatEntryType::User
        );
        assert_eq!(
            state.chat_history[assistant_idx + 2].entry_type,
            ChatEntryType::Assistant
        );

        // 代码块整体在后半，前半不留下没闭合的围栏
        let tail = &state.chat_history[assistant_idx + 2].content;
        assert!(
            tail.starts_with("```rust") && tail.contains("fn main() {"),
            "代码块应完整留在后半: {:?}",
            tail
        );
        assert!(
            !state.chat_history[assistant_idx].content.contains("```"),
            "前半不能留下没闭合的围栏: {:?}",
            state.chat_history[assistant_idx].content
        );

        // 后续输出继续落到后半那条
        assert_eq!(
            state.stream_targets.get(&7),
            Some(&(assistant_idx + 2)),
            "后续输出要接着后半那条"
        );

        // 需求确实发出去了
        assert!(rx.try_recv().is_ok(), "需求应当真的被发出去");
    }

    /// 正文不在代码块里时，按段落边界切，两半都是完整段落。
    #[tokio::test]
    async fn steering_splits_at_a_paragraph_boundary() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(3);

        let assistant_idx = state.chat_history.len();
        state.chat_history.push(
            crate::types::ChatEntry::assistant("第一段。\n\n第二段。\n\n第三段".to_string())
                .with_streaming(true),
        );
        state.stream_targets.insert(3, assistant_idx);

        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        super::steer_user_message(&mut state, "补充".to_string(), &tx)
            .await
            .expect("steering should succeed");

        assert_eq!(state.chat_history.len(), assistant_idx + 3);
        assert_eq!(
            state.chat_history[assistant_idx].content,
            "第一段。\n\n第二段。\n\n"
        );
        assert_eq!(state.chat_history[assistant_idx + 2].content, "第三段");
    }

    /// 端到端：画出来的画面里，用户那条消息**不在代码块里面**。
    ///
    /// 这是用户看到的那个现象：围栏没闭合时，渲染会把后面整段（含用户消息）
    /// 都当成代码，消息就"插到 markdown 里面"了。
    #[tokio::test]
    async fn the_steered_message_is_not_rendered_inside_the_code_block() {
        let mut state = ChatState::test_fixture();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(7);

        let assistant_idx = state.chat_history.len();
        state.chat_history.push(
            crate::types::ChatEntry::assistant("说明文字\n\n```rust\nfn main() {\n".to_string())
                .with_streaming(true),
        );
        state.stream_targets.insert(7, assistant_idx);
        state.rendered_cache.clear();

        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        super::steer_user_message(&mut state, "顺便补个测试".to_string(), &tx)
            .await
            .expect("steering should succeed");
        state.rendered_cache.clear();

        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        let texts: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        let user_line = texts
            .iter()
            .position(|t| t.contains("顺便补个测试"))
            .expect("用户消息要出现在画面上");
        let code_top = texts
            .iter()
            .position(|t| t.contains('┌'))
            .expect("代码块要有上边框");
        let code_bottom = texts
            .iter()
            .position(|t| t.contains('└'))
            .expect("代码块要有下边框");

        assert!(
            user_line < code_top,
            "用户消息不该落在代码块里（它应该在代码块之前）: {:?}",
            texts
        );
        assert!(code_top < code_bottom, "代码块要完整: {:?}", texts);
    }

    /// 回归：工具调用之后 `stream_targets` 是**悬空**的（指向
    /// `chat_history.len()`，等下一段内容再建一条助手条目）。
    /// 那个窗口里插入需求会越界 panic（index out of bounds）。
    #[tokio::test]
    async fn steering_while_the_stream_target_dangles_does_not_panic() {
        let mut state = ChatState::test_fixture();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(11);

        // 助手那条已经收尾，工具调用条目也进去了；流的落点悬空在末尾
        state.chat_history.push(
            crate::types::ChatEntry::assistant("我要先看看这个文件".to_string())
                .with_streaming(false),
        );
        state.chat_history.push(crate::types::ChatEntry::tool_call(
            "Read",
            crate::types::StarToolCall {
                id: "call-1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                },
            },
        ));
        state.stream_targets.insert(11, state.chat_history.len());
        assert_eq!(
            state.stream_targets.get(&11),
            Some(&state.chat_history.len()),
            "前提：落点是悬空的"
        );

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        super::steer_user_message(&mut state, "顺便补个测试".to_string(), &tx)
            .await
            .expect("steering should not panic");

        // 需求照常显示，也照常发出去
        assert!(state
            .chat_history
            .iter()
            .any(|e| e.content == "顺便补个测试"));
        assert!(rx.try_recv().is_ok());

        // 流的落点必须挪到用户消息之后 —— 否则下一段助手内容会被追加到用户那条上
        let user_idx = state
            .chat_history
            .iter()
            .position(|e| e.content == "顺便补个测试")
            .expect("用户消息在");
        assert!(
            state.stream_targets.get(&11).copied().unwrap_or(0) > user_idx,
            "落点必须在用户消息之后: {:?}",
            state.stream_targets
        );
    }
}

#[cfg(test)]
mod local_command_tests {
    use super::truncate_local_command_output;

    #[test]
    fn short_output_passes_through_untouched() {
        assert_eq!(truncate_local_command_output("ok\n"), "ok\n");
    }

    #[test]
    fn long_output_keeps_head_and_tail() {
        // 不动环境变量（单测并行，set_var 会漏给别的测试）：默认上限 8000，
        // 直接喂一段更长的。
        let out = truncate_local_command_output(&format!(
            "{}{}{}",
            "H".repeat(100),
            "M".repeat(20_000),
            "T".repeat(100)
        ));
        assert!(out.starts_with("HHHH"));
        assert!(out.ends_with("TTTT"));
        assert!(out.contains("chars omitted"));
    }

    #[test]
    fn multibyte_output_does_not_split_a_char() {
        // 4 个汉字 × 3000 = 12000 字符，超过默认 8000 上限
        let out = truncate_local_command_output(&"编译错误".repeat(3000));
        assert!(out.starts_with('编'));
        assert!(out.contains("chars omitted"));
        // 按字符切而不是按字节切：切点不会落在汉字中间产生替换字符
        assert!(!out.contains('\u{FFFD}'));
    }
}
