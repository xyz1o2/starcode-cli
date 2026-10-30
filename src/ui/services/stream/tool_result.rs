//! 工具结果消息与正文消息的处理。
//!
use super::*;

pub(super) fn handle_tool_result_message(
    state: &mut ChatState,
    message_id: u64,
    tool_call: StarToolCall,
    tool_result: ToolResult,
) {
    // 这条结果属于当前等待确认的那次调用吗？必须比**模型侧**的 id
    // （`pending_confirmation_originating_id`）。早先这里比的是
    // `pending_tool_call_id`，那是 bus 的 correlation id，永远不等于
    // `tool_call.id`：于是工具超时/失败之后卡片收不到任何收尾信号，
    // `is_awaiting_confirmation` 一直为真，后面每一次确认请求都被当成
    // "已有一张卡"而被顶掉。
    //
    // 排队中的请求也可能先等到结果（中断/取消）。先把它们摘掉，避免轮到它们
    // 时激活一张已经没有门禁在等的卡。
    state.remove_queued_confirmation_by_originating_id(&tool_call.id);
    if state.is_awaiting_confirmation {
        if let Some(originating) = state.pending_confirmation_originating_id.as_deref() {
            if originating == tool_call.id {
                state.clear_pending_confirmation();
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

pub(super) fn handle_content_message(state: &mut ChatState, message_id: u64, content: &str) {
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

pub(super) fn extract_last_code_block(content: &str) -> Option<String> {
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
