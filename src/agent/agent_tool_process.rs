use crate::agent::agent_core::Agent;
use crate::agent::agent_loop::TurnResult;
use crate::agent::loop_engineering::{LoopState, LoopStrategy, StructuredError, ToolCallBudget};
use crate::agent::nudges;
use crate::agent::tool_routing::{
    build_analyzer_skill_tool_call, build_editor_skill_tool_call, build_json_fallback_prompt,
    build_navigator_skill_tool_call, build_project_map_tool_call, build_semantic_search_tool_call,
    build_tool_loop_signature, build_validation_tool_call, detect_tool_loop, has_action_intent,
    is_edit_tool_name, is_read_only_tool_name, is_validation_tool_name,
    json_fallback_extract_tool_call, resolved_read_only_turn_limit, select_best_auto_trigger,
    AutoTriggerKind,
};
use crate::agent::{hooks, reflection, tool_helpers};
use crate::core::i18n;
use crate::types::{StarMessage, StarToolCall, ToolResult};
use std::collections::{HashSet, VecDeque};

impl Agent {
    // 处理 LLM 响应（包括工具执行）
    pub(crate) async fn process_llm_response(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        content: String,
        reasoning: String,
        tool_calls: Vec<StarToolCall>,
        usage: Option<crate::types::StarUsage>,
        content_streamed: bool,
        reasoning_streamed: bool,
        current_turn: i32,
        all_active_tools: &HashSet<String>,
        semantic_search_attempted: &mut bool,
        navigator_skill_attempted: &mut bool,
        analyzer_skill_attempted: &mut bool,
        editor_skill_attempted: &mut bool,
        project_map_attempted: &mut bool,
        skip_verification: bool,
        verification_required: &mut bool,
        tool_loop_repeat_threshold: usize,
        tool_signature_history: &mut VecDeque<String>,
        tool_loop_interventions: &mut usize,
        nudge_interventions: &mut usize,
        consecutive_read_only_turns: &mut usize,
        file_read_tracker: &mut std::collections::HashMap<String, usize>,
        turn_active_tools: &HashSet<String>,
        loop_state: &mut LoopState,
    ) -> TurnResult {
        // Emit stats update
        if let Some(ref usage) = usage {
            crate::utils::logging::append_debug_log_line(&format!(
                "[TOKEN-API] Turn {} actual: prompt={}, completion={}, total={}",
                current_turn, usage.prompt_tokens, usage.completion_tokens, usage.total_tokens,
            ));
            // Surface usage to consumers (eval harness, UI, cost tracking).
            self.emit_event(crate::agent::messaging::AgentEvent::StatsUpdate {
                token_usage: Some(usage.clone()),
            });
        }

        // Check for tool loop
        if !tool_calls.is_empty() {
            let signature = build_tool_loop_signature(&tool_calls);
            tool_signature_history.push_back(signature);
            while tool_signature_history.len() > 12 {
                tool_signature_history.pop_front();
            }
            if let Some(loop_reason) =
                detect_tool_loop(tool_signature_history, tool_loop_repeat_threshold)
            {
                *tool_loop_interventions += 1;
                let guard_msg = format!(
                    "Tool loop guard: {}. The repeated tool call was not executed. Reuse the previous tool result already in the conversation, or switch to a different tool such as `Grep`, `Glob`, `Read`, or `ProjectMap`. Do not call the same tool with the same arguments again.",
                    loop_reason
                );
                messages.push(StarMessage::system(format!(
                    "[TOOL_LOOP_GUARD]\n{}\nContinue the task without repeating that tool call.",
                    guard_msg
                )));
                // 不清空历史：保留历史让同一模式的重复检测在下一轮更快触发，
                // 避免相同的工具调用模式需要重新累积 repeat_threshold 次才再次拦截。
                // 当 LLM 切换到不同工具时，detect_tool_loop 会自然地不再触发。

                let max_interventions = std::env::var("STAR_TOOL_LOOP_MAX_INTERVENTIONS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(5)
                    .clamp(2, 20);
                if *tool_loop_interventions >= max_interventions {
                    let msg = i18n::t(
                        "agent.tool_loop.guard.limit",
                        &format!("工具重复调用已被连续拦截 {} 次。请立即基于已有结果完成任务，不要再调用相同工具。直接用 Edit/Write 写出改动，或用 summary 总结已完成的工作。", max_interventions),
                        &format!("Tool loop guard triggered {} times. You MUST complete the task now based on existing results. Do NOT call the same tool again. Use edit/Write to make changes, or provide a summary of completed work.", max_interventions),
                    );
                    messages.push(StarMessage::system(msg));
                    // Reset counter so next intervention cycle starts fresh
                    *tool_loop_interventions = 0;
                    return TurnResult::Continue;
                }

                return TurnResult::Continue;
            }
        }

        // Build assistant message from collected content
        let mut assistant_message = StarMessage::assistant(content.clone());
        assistant_message.reasoning_content = if reasoning.is_empty() {
            None
        } else {
            Some(reasoning.clone())
        };
        if !tool_calls.is_empty() {
            assistant_message.tool_calls = Some(tool_calls.clone());
        }
        messages.push(assistant_message);

        // Check if we have tool calls to execute
        if !tool_calls.is_empty() {
            // Track which tools were attempted
            if tool_calls
                .iter()
                .any(|tc| tc.function.name == "SemanticSearch")
            {
                *semantic_search_attempted = true;
            }
            if tool_calls.iter().any(|tc| tc.function.name == "ProjectMap") {
                *project_map_attempted = true;
            }
            if tool_calls.iter().any(|tc| {
                tc.function.name == "skill" && tc.function.arguments.contains("\"navigator\"")
            }) {
                *navigator_skill_attempted = true;
            }
            if tool_calls.iter().any(|tc| {
                tc.function.name == "skill"
                    && (tc.function.arguments.contains("\"analyzer\"")
                        || tc.function.arguments.contains("\"task_type\":\"analyze\""))
            }) {
                *analyzer_skill_attempted = true;
            }

            // Emit the assistant's text / reasoning content BEFORE tool
            // execution so the UI renders it above the tool-call entries.
            // Skip if already streamed in real-time via Branch B (TextDelta/ReasoningDelta)
            // to avoid duplicating content in the UI.
            if !content.is_empty() && !content_streamed {
                self.emit_event(crate::agent::messaging::AgentEvent::Message(
                    content.clone(),
                ));
            }
            if !reasoning.is_empty() && !reasoning_streamed {
                self.emit_event(crate::agent::messaging::AgentEvent::ReasoningDelta(
                    reasoning.clone(),
                ));
            }

            // 不再合成 "Reading/Editing xxx" 之类的文字说明：
            // UI 的 ⏺ 工具行已完整显示工具名与参数，合成文字会与之重复（对标 Claude Code）

            // 模型产生了有效的工具调用 — 重置 nudge 计数器
            // 只有连续的空响应才会累积计数防止无限循环
            *nudge_interventions = 0;

            // Execute tool calls
            self.execute_tool_calls_in_loop(
                user_input,
                messages,
                &tool_calls,
                current_turn,
                skip_verification,
                verification_required,
                consecutive_read_only_turns,
                file_read_tracker,
                loop_state,
            )
            .await;

            // Continue to next round
            TurnResult::Continue
        } else {
            // No tool calls — detect empty/silent responses that indicate the model
            // refused to act (e.g. safety guardrails, context pollution). Inject a
            // recovery nudge and retry once instead of silently exiting the loop.
            let content_trimmed = content.trim();

            // Global nudge limit: prevent infinite loops from guards injecting nudges
            let max_nudge_interventions = std::env::var("STAR_NUDGE_MAX_INTERVENTIONS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(3)
                .clamp(1, 10);

            if content_trimmed.is_empty() && reasoning.is_empty() {
                // Truly empty response — no content AND no reasoning at all.
                // Use nudge_interventions counter as the sole termination
                // condition (not the fragile already_nudged message check).
                if *nudge_interventions >= max_nudge_interventions {
                    crate::utils::logging::append_debug_log_line(
                        "[AGENT] empty model response persisted after recovery nudge — giving up",
                    );
                    messages.pop();
                    return TurnResult::Done;
                }
                crate::utils::logging::append_debug_log_line(
                    "[AGENT] empty model response (no content, no reasoning, no tool calls) — injecting recovery nudge",
                );
                messages.pop();
                *nudge_interventions += 1;
                self.emit_event(crate::agent::messaging::AgentEvent::Trace {
                    event: "agent_status".to_string(),
                    payload: serde_json::json!({
                        "status": "retrying",
                        "phase": "empty_response_recovery",
                        "message": "Model returned empty response, retrying...",
                    }),
                });
                messages.push(StarMessage::system(
                    nudges::NUDGE_EMPTY_RESPONSE.to_string(),
                ));
                return TurnResult::Continue;
            }

            // Model produced reasoning but no content/tool_calls — this is a
            // "thinking-only" response (common with DeepSeek reasoning models).
            // Instead of treating it as empty, inject a targeted nudge asking
            // the model to produce a concrete response after its reasoning.
            if content_trimmed.is_empty() && !reasoning.is_empty() {
                if *nudge_interventions >= max_nudge_interventions {
                    crate::utils::logging::append_debug_log_line(
                        "[AGENT] thinking-only response persisted after recovery nudge — giving up",
                    );
                    messages.pop();
                    return TurnResult::Done;
                }
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AGENT] thinking-only response (reasoning={}chars, content=0) — injecting continue nudge",
                    reasoning.len(),
                ));
                messages.pop();
                *nudge_interventions += 1;
                self.emit_event(crate::agent::messaging::AgentEvent::Trace {
                    event: "agent_status".to_string(),
                    payload: serde_json::json!({
                        "status": "retrying",
                        "phase": "thinking_only_recovery",
                        "message": "Model produced reasoning but no response, retrying...",
                    }),
                });
                messages.push(StarMessage::system(nudges::NUDGE_THINKING_ONLY.to_string()));
                return TurnResult::Continue;
            }
            // No tool calls - check for auto-triggers
            let trigger_result = self
                .handle_auto_triggers(
                    user_input,
                    messages,
                    content_trimmed,
                    all_active_tools,
                    current_turn,
                    semantic_search_attempted,
                    navigator_skill_attempted,
                    analyzer_skill_attempted,
                    editor_skill_attempted,
                    project_map_attempted,
                    turn_active_tools,
                    skip_verification,
                    verification_required,
                    loop_state,
                )
                .await;

            // Auto-trigger executed a tool → model produced useful output,
            // reset the consecutive-empty counter. Only reset on Continue
            // (auto-trigger fired), NOT on Done (text-only — let the guards
            // accumulate nudge_interventions to eventually stop the loop).
            if matches!(trigger_result, TurnResult::Continue) {
                *nudge_interventions = 0;
            }

            // Conclusion guard: if the agent is about to finish (Done) but
            // the last assistant message has no meaningful content, inject a
            // request for a conclusion/summary so the user sees a result.
            if matches!(trigger_result, TurnResult::Done) && content_trimmed.len() < 20 {
                let already_asked_conclusion = messages
                    .last()
                    .map(|m| {
                        m.role == "system"
                            && m.content
                                .as_deref()
                                .unwrap_or("")
                                .starts_with("[CONCLUSION_REQUEST]")
                    })
                    .unwrap_or(false);
                if !already_asked_conclusion
                    && current_turn > 1
                    && *nudge_interventions < max_nudge_interventions
                {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[AGENT] turn {} finishing with minimal content ({} chars) — requesting conclusion",
                        current_turn, content_trimmed.len(),
                    ));
                    *nudge_interventions += 1;
                    messages.push(StarMessage::system(
                        nudges::NUDGE_CONCLUSION_REQUEST.to_string(),
                    ));
                    return TurnResult::Continue;
                } else {
                    // Conclusion guard passed (content is long enough, first turn,
                    // or nudge limit reached) — reset nudge counter before Done.
                    *nudge_interventions = 0;
                }
            }

            // Action nudge guard: if the model returned text but no tool calls
            // and the user's request looks like a real coding task, inject a nudge.
            // Heuristics:
            // - Long input (>50 chars) with action intent → likely a task
            // - Short input (<30 chars) → likely a question, don't nudge
            // - Medium input with action keywords → likely a task
            if matches!(trigger_result, TurnResult::Done) && user_input.chars().count() > 5 {
                let input_len = user_input.chars().count();
                let looks_like_task = if input_len > 50 {
                    // Long input: nudge if it has any action intent
                    has_action_intent(user_input)
                } else if input_len > 30 {
                    // Medium input: nudge only if strong action keywords present
                    has_action_intent(user_input)
                } else {
                    // Short input: likely a question, don't nudge
                    false
                };

                if looks_like_task {
                    // Check if the last message is already a nudge to avoid duplicates
                    let already_nudged = messages
                        .last()
                        .map(|m| {
                            m.role == "system"
                                && m.content
                                    .as_deref()
                                    .unwrap_or("")
                                    .starts_with("You responded with a text explanation")
                        })
                        .unwrap_or(false);
                    if already_nudged || *nudge_interventions >= max_nudge_interventions {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[AGENT] turn {} text-only nudge already present or limit reached — Done",
                            current_turn,
                        ));
                        return TurnResult::Done;
                    }
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[AGENT] turn {} produced text-only response with no tool calls — injecting action nudge",
                        current_turn,
                    ));
                    *nudge_interventions += 1;
                    messages.push(StarMessage::system(
                        nudges::NUDGE_ACTION_REQUIRED.to_string(),
                    ));
                    return TurnResult::Continue;
                }
            }

            // Model produced useful content without needing a nudge —
            // reset the consecutive-empty-response counter.
            *nudge_interventions = 0;

            trigger_result
        }
    }

    // 执行工具调用
    pub(crate) async fn execute_tool_calls_in_loop(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        tool_calls: &[StarToolCall],
        current_turn: i32,
        skip_verification: bool,
        verification_required: &mut bool,
        consecutive_read_only_turns: &mut usize,
        file_read_tracker: &mut std::collections::HashMap<String, usize>,
        loop_state: &mut LoopState,
    ) {
        // ── Budget enforcement ──
        // 在 hook 前接受整个模型批次，避免部分执行导致协议消息不完整。
        if let Err(exhausted) = accept_tool_calls(&mut loop_state.budget, tool_calls) {
            crate::utils::logging::append_debug_log_line(&format!(
                "[BUDGET] turn {} rejected tool calls: {:?}",
                current_turn, exhausted
            ));
            messages.push(StarMessage::system(
                exhausted.recovery_message(tool_calls.len()),
            ));
            return;
        }

        let mut runnable_tool_calls: Vec<StarToolCall> = Vec::new();
        for tool_call in tool_calls {
            match hooks::run_pre_tool_hooks(user_input, tool_call).await {
                Ok(()) => {
                    runnable_tool_calls.push(tool_call.clone());
                }
                Err(reason) => {
                    // ── Denial Tracking ──
                    // Detect same-tool consecutive denials (≥3 → auto-inject
                    // nudge). Mirrors Claude Code's denialTracking.ts.
                    if let Some(nudge_msg) = self
                        .denial_tracker
                        .record_denial(&tool_call.function.name, &reason)
                    {
                        let thresh = self.denial_tracker.threshold();
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[DENIAL_TRACKING] threshold ({thresh}) reached for {} — injecting nudge",
                            tool_call.function.name,
                        ));
                        messages.push(StarMessage::system(nudge_msg));
                    }

                    let blocked_result = ToolResult {
                        success: false,
                        output: None,
                        error: Some(reason),
                        data: None,
                    };
                    reflection::maybe_write_reflection_memory(
                        user_input,
                        tool_call,
                        &blocked_result,
                    )
                    .await;
                    messages.push(StarMessage::tool(
                        tool_call.id.clone(),
                        blocked_result.error.unwrap_or_default(),
                    ));
                }
            }
        }

        if !runnable_tool_calls.is_empty() {
            let mut successful_edit_happened = false;
            let mut successful_edit_tools = Vec::new();
            let mut validation_tool_happened = false;

            for segment in tool_execution_segments(&runnable_tool_calls) {
                match segment {
                    ToolExecutionSegment::Batch(indices) => {
                        let calls = indices
                            .iter()
                            .map(|&index| runnable_tool_calls[index].clone())
                            .collect::<Vec<_>>();
                        self.emit_tool_starts(&calls);
                        let results = self.tool_executor.execute_batch(calls, None, None).await;
                        for (index, result) in indices.into_iter().zip(results) {
                            self.consume_tool_result(
                                user_input,
                                messages,
                                &runnable_tool_calls[index],
                                result,
                                skip_verification,
                                verification_required,
                                loop_state,
                                &mut successful_edit_happened,
                                &mut successful_edit_tools,
                                &mut validation_tool_happened,
                            )
                            .await;
                        }
                    }
                    ToolExecutionSegment::Barrier(index) => {
                        let tool_call = &runnable_tool_calls[index];
                        self.emit_tool_starts(std::slice::from_ref(tool_call));
                        let result = self.execute_single_tool(tool_call).await;
                        self.consume_tool_result(
                            user_input,
                            messages,
                            tool_call,
                            result,
                            skip_verification,
                            verification_required,
                            loop_state,
                            &mut successful_edit_happened,
                            &mut successful_edit_tools,
                            &mut validation_tool_happened,
                        )
                        .await;
                    }
                }
            }

            if successful_edit_happened && !validation_tool_happened && !skip_verification {
                messages.push(StarMessage::system(
                    "Code edits were applied. Verification is required: run diagnostics or targeted tests before finalizing."
                        .to_string(),
                ));
            }

            // B1: Goal convergence nudge — injected ONCE per session, right
            // after the first successful edit. Keeps the agent from drifting
            // into unrelated code once the requested change is done.
            if successful_edit_happened {
                let already_nudged = messages.iter().any(|m| {
                    m.content
                        .as_deref()
                        .map(|c| c.contains("[GOAL_CONVERGE]"))
                        .unwrap_or(false)
                });
                if !already_nudged {
                    messages.push(StarMessage::system(
                        "[GOAL_CONVERGE] The user's request is the scope. If the requested change is complete and verified, stop here — do not fix unrelated issues, refactor adjacent code, or explore extra files. If verification has not run yet, verify the change now, then report done."
                            .to_string(),
                    ));
                }
            }

            // Track exploration depth and repeated file reads
            let had_action = runnable_tool_calls
                .iter()
                .any(|tc| !is_read_only_tool_name(&tc.function.name));
            if had_action {
                *consecutive_read_only_turns = 0;
                // B3: only reset counters for files that were actually edited.
                // Files read while exploring OTHER files keep their counts, so
                // the edit-one → explore-next pattern can't slip under the
                // REPEATED_READ radar.
                let edited_files: std::collections::HashSet<String> = runnable_tool_calls
                    .iter()
                    .filter(|tc| is_edit_tool_name(&tc.function.name))
                    .filter_map(|tc| extract_file_path_from_tool_call(tc))
                    .collect();
                if edited_files.is_empty() {
                    file_read_tracker.clear();
                } else {
                    file_read_tracker.retain(|f, _| !edited_files.contains(f));
                }
            } else {
                *consecutive_read_only_turns += 1;

                // Track which files are being read
                for tc in &runnable_tool_calls {
                    if is_read_only_tool_name(&tc.function.name) {
                        if let Some(file_path) = extract_file_path_from_tool_call(tc) {
                            let count = file_read_tracker.entry(file_path.clone()).or_insert(0);
                            *count += 1;

                            // If same file read 3+ times, inject strong nudge
                            if *count >= 3 {
                                messages.push(StarMessage::system(format!(
                                    "[REPEATED_READ] You have read '{}' {} times without making any changes. \
                                     You MUST now make the edit or provide your analysis. Do NOT read this file again.",
                                    file_path, count
                                )));
                            }
                        }
                    }
                }

                let read_only_limit = resolved_read_only_turn_limit();
                if *consecutive_read_only_turns >= read_only_limit {
                    let count = *consecutive_read_only_turns;
                    *consecutive_read_only_turns = 0;
                    // Send a natural language instruction instead of a system marker
                    messages.push(StarMessage::system(
                        i18n::t(
                            "agent.exploration.limit",
                            "你已经连续进行了 {count} 轮只读操作（读文件/搜索）却未修改任何代码。请立即根据已有信息执行代码修改。直接使用 Edit/Write 工具写出改动，不要再读更多文件。",
                            "You have performed {count} consecutive read-only operations (file reads/searches) without any code edits. Stop exploring and immediately make code changes based on the information you already have. Use Edit/Write tools directly, do not read more files.",
                        ).replace("{count}", &count.to_string())
                    ));
                }
            }
        }
    }

    /// 在执行段开始前按原始模型顺序发布工具开始事件。
    fn emit_tool_starts(&self, tool_calls: &[StarToolCall]) {
        for tool_call in tool_calls {
            self.emit_tool_started(tool_call);
            self.emit_event(crate::agent::messaging::AgentEvent::ToolStarted {
                tool_call: tool_call.clone(),
            });
        }
    }

    /// 消费一次工具执行结果，保证串行 barrier 与批处理的后处理完全一致。
    #[allow(clippy::too_many_arguments)]
    async fn consume_tool_result(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        tool_call: &StarToolCall,
        result: ToolResult,
        skip_verification: bool,
        verification_required: &mut bool,
        loop_state: &mut LoopState,
        successful_edit_happened: &mut bool,
        successful_edit_tools: &mut Vec<String>,
        validation_tool_happened: &mut bool,
    ) {
        self.emit_tool_finished(tool_call, &result);
        self.emit_event(crate::agent::messaging::AgentEvent::ToolFinished {
            tool_call: tool_call.clone(),
            result: result.clone(),
        });

        if is_edit_tool_name(&tool_call.function.name) && result.success {
            *successful_edit_happened = true;
            successful_edit_tools.push(tool_call.function.name.clone());
            self.verify_successful_edit(messages, tool_call);
        }
        if is_validation_tool_name(&tool_call.function.name) {
            *validation_tool_happened = true;
        }
        tool_helpers::update_verification_state(
            tool_call,
            &result,
            verification_required,
            skip_verification,
        );
        hooks::run_post_tool_hooks(user_input, tool_call, &result).await;
        reflection::maybe_write_reflection_memory(user_input, tool_call, &result).await;
        if let Some(instruction) =
            record_tool_outcome(loop_state, &tool_call.function.name, &result)
        {
            messages.push(StarMessage::system(instruction));
        }

        // Record file access in memory system
        if is_read_only_tool_name(&tool_call.function.name) && result.success {
            if let Some(file_path) = extract_file_path_from_tool_call(tool_call) {
                let _ = self
                    .memory_manager
                    .record_file_access(&file_path, None, None)
                    .await;
            }
        }

        let tool_msg_content = result
            .output
            .unwrap_or_else(|| result.error.unwrap_or_default());
        messages.push(StarMessage::tool(
            tool_call.id.clone(),
            tool_msg_content.clone(),
        ));

        if is_edit_tool_name(&tool_call.function.name)
            && tool_msg_content.contains(crate::core::tools::constants::EDIT_FILE_NOT_READ_MARKER)
        {
            self.handle_edit_not_read_error(messages, tool_call);
        }
    }

    /// 成功编辑后执行已有的语法验证，失败结果注入模型上下文。
    fn verify_successful_edit(&self, messages: &mut Vec<StarMessage>, tool_call: &StarToolCall) {
        let Some(file_path) = extract_file_path_from_tool_call(tool_call) else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&file_path) else {
            return;
        };
        let verify_result =
            crate::core::tools::verify_edit::verify_edit_syntax(&file_path, &content);
        if verify_result.syntax_ok {
            return;
        }

        let verify_msg =
            crate::core::tools::verify_edit::format_verification_result(&verify_result);
        crate::utils::logging::append_debug_log_line(&format!(
            "[VERIFY] Post-edit syntax check failed: {}",
            verify_msg
        ));
        messages.push(StarMessage::system(verify_msg));
    }

    /// 处理编辑未读错误
    fn handle_edit_not_read_error(
        &self,
        messages: &mut Vec<StarMessage>,
        tool_call: &StarToolCall,
    ) {
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
            let is_multi = args.get("edits").and_then(|e| e.as_array()).is_some();
            if is_multi {
                messages.push(StarMessage::system(format!(
                    "[REQUIRED ACTION] {} was blocked. The error above lists all unread files. \
                     Batch-call Read for EVERY file listed in the error in ONE response, then retry {}.",
                    tool_call.function.name, tool_call.function.name
                )));
            } else if let Some(fp) = args.get("file_path").and_then(|v| v.as_str()) {
                messages.push(StarMessage::system(format!(
                    "[REQUIRED ACTION] {} was blocked — '{}' has not been read. \
                     Call Read('{}') NOW, then retry. Do NOT retry without reading first.",
                    tool_call.function.name, fp, fp
                )));
            }
        }
    }

    /// 执行通用的自动触发器——构建工具调用、推送助手消息、执行工具、推送结果
    async fn run_auto_trigger_tool(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        tool_call: StarToolCall,
        verification_required: &mut bool,
        skip_verification: bool,
        loop_state: &mut LoopState,
    ) -> TurnResult {
        if let Err(exhausted) =
            accept_tool_calls(&mut loop_state.budget, std::slice::from_ref(&tool_call))
        {
            messages.push(StarMessage::system(exhausted.recovery_message(1)));
            return TurnResult::Continue;
        }
        messages.push(StarMessage::assistant_with_tool_calls(vec![
            tool_call.clone()
        ]));

        self.emit_event(crate::agent::messaging::AgentEvent::ToolStarted {
            tool_call: tool_call.clone(),
        });

        match hooks::run_pre_tool_hooks(user_input, &tool_call).await {
            Ok(()) => {
                let result = self.execute_single_tool(&tool_call).await;
                tool_helpers::update_verification_state(
                    &tool_call,
                    &result,
                    verification_required,
                    skip_verification,
                );
                hooks::run_post_tool_hooks(user_input, &tool_call, &result).await;
                reflection::maybe_write_reflection_memory(user_input, &tool_call, &result).await;
                messages.push(StarMessage::tool(
                    tool_call.id.clone(),
                    result
                        .output
                        .clone()
                        .unwrap_or_else(|| result.error.clone().unwrap_or_default()),
                ));
                self.emit_event(crate::agent::messaging::AgentEvent::ToolFinished {
                    tool_call: tool_call.clone(),
                    result: result.clone(),
                });
                TurnResult::Continue
            }
            Err(reason) => {
                // Denial tracking for auto-trigger tools too
                if let Some(nudge_msg) = self
                    .denial_tracker
                    .record_denial(&tool_call.function.name, &reason)
                {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[DENIAL_TRACKING] auto-trigger threshold ({}) reached for {} — injecting nudge",
                        self.denial_tracker.threshold(),
                        tool_call.function.name,
                    ));
                    messages.push(StarMessage::system(nudge_msg));
                }
                let blocked = ToolResult {
                    success: false,
                    output: None,
                    error: Some(reason),
                    data: None,
                };
                reflection::maybe_write_reflection_memory(user_input, &tool_call, &blocked).await;
                self.emit_event(crate::agent::messaging::AgentEvent::ToolFinished {
                    tool_call: tool_call.clone(),
                    result: blocked.clone(),
                });
                messages.push(StarMessage::tool(
                    tool_call.id.clone(),
                    blocked.error.unwrap_or_default(),
                ));
                TurnResult::Continue
            }
        }
    }

    /// 处理自动触发器（优先级驱动的选择，替代原串行 if-else 链）
    pub(crate) async fn handle_auto_triggers(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        current_content: &str,
        all_active_tools: &HashSet<String>,
        current_turn: i32,
        semantic_search_attempted: &mut bool,
        navigator_skill_attempted: &mut bool,
        analyzer_skill_attempted: &mut bool,
        editor_skill_attempted: &mut bool,
        project_map_attempted: &mut bool,
        turn_active_tools: &HashSet<String>,
        skip_verification: bool,
        verification_required: &mut bool,
        loop_state: &mut LoopState,
    ) -> TurnResult {
        // 使用优先级驱动选择替代原串行链
        let best = select_best_auto_trigger(
            *verification_required,
            skip_verification,
            user_input,
            current_content,
            all_active_tools,
            *semantic_search_attempted,
            *navigator_skill_attempted,
            *analyzer_skill_attempted,
            *editor_skill_attempted,
            *project_map_attempted,
        );

        let Some(selected) = best else {
            return TurnResult::Done;
        };

        crate::utils::logging::append_debug_log_line(&format!(
            "[ACE_AUTO] selected trigger {:?} (score={}, reason={}) turn={}",
            selected.kind, selected.score, selected.reason, current_turn,
        ));

        match selected.kind {
            AutoTriggerKind::Verification => {
                let tool_call = build_validation_tool_call(current_turn);
                crate::utils::logging::append_debug_log_line(&format!(
                    "[VERIFY_AUTO] auto-trigger diagnostics (turn={})",
                    current_turn
                ));
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
            AutoTriggerKind::SemanticSearch => {
                *semantic_search_attempted = true;
                let tool_call = build_semantic_search_tool_call(user_input, current_turn);
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
            AutoTriggerKind::NavigatorSkill => {
                *navigator_skill_attempted = true;
                let tool_call = build_navigator_skill_tool_call(user_input, current_turn);
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
            AutoTriggerKind::ProjectMap => {
                *project_map_attempted = true;
                let tool_call = build_project_map_tool_call(user_input, current_turn);
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
            AutoTriggerKind::AnalyzerSkill => {
                *analyzer_skill_attempted = true;
                let tool_call = build_analyzer_skill_tool_call(user_input, current_turn);
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
            AutoTriggerKind::JsonFallback => {
                let fallback_prompt =
                    build_json_fallback_prompt(current_content, turn_active_tools);
                let mut fallback_messages = messages.clone();
                fallback_messages.push(StarMessage::user(fallback_prompt));
                crate::agent::message_processing::repair_tool_message_sequence(
                    &mut fallback_messages,
                );
                crate::agent::message_processing::normalize_messages_for_llm(
                    &mut fallback_messages,
                    self.client.supports_thinking(),
                );

                match self.client.chat(fallback_messages, None, None, None).await {
                    Ok(response) => {
                        let response_text = response
                            .choices
                            .first()
                            .and_then(|c| c.message.content.as_deref())
                            .unwrap_or("");
                        if let Some(fb_tool) = json_fallback_extract_tool_call(response_text) {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[JSON_FALLBACK] extracted: {}",
                                fb_tool.function.name
                            ));
                            return self
                                .run_auto_trigger_tool(
                                    user_input,
                                    messages,
                                    fb_tool,
                                    verification_required,
                                    skip_verification,
                                    loop_state,
                                )
                                .await;
                        }
                    }
                    Err(e) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[JSON_FALLBACK] error: {}",
                            e
                        ));
                    }
                }
                // Fallback failed — inject nudge to use standard tool calling format
                crate::utils::logging::append_debug_log_line(
                    "[JSON_FALLBACK] extraction failed — injecting format nudge",
                );
                messages.push(StarMessage::system(
                    "Your response could not be parsed as a tool call. \
                     Please use the standard tool calling mechanism (not JSON text). \
                     Call the appropriate tools directly to continue the task."
                        .to_string(),
                ));
                TurnResult::Continue
            }
            AutoTriggerKind::EditorSkill => {
                *editor_skill_attempted = true;
                let tool_call = build_editor_skill_tool_call(user_input, current_turn);
                self.run_auto_trigger_tool(
                    user_input,
                    messages,
                    tool_call,
                    verification_required,
                    skip_verification,
                    loop_state,
                )
                .await
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ToolBudgetExhausted {
    Total {
        accepted: usize,
        pending: usize,
        limit: usize,
    },
    PerTool {
        tool_name: String,
        accepted: usize,
        pending: usize,
        limit: usize,
    },
}

impl ToolBudgetExhausted {
    fn recovery_message(&self, batch_size: usize) -> String {
        match self {
            Self::Total {
                accepted,
                pending,
                limit,
            } => format!(
                "Tool call budget exhausted this turn ({accepted}/{limit} calls already made, \
                 {pending} pending in this {batch_size}-call response). Summarize your progress \
                 so far and describe the remaining work."
            ),
            Self::PerTool {
                tool_name,
                accepted,
                pending,
                limit,
            } => format!(
                "Tool call limit for `{tool_name}` is exhausted this turn ({accepted}/{limit} \
                 already accepted, {pending} pending in this {batch_size}-call response). Do not \
                 execute a partial batch; use prior results or choose a different tool."
            ),
        }
    }
}

/// 原子地接受一次模型/自动触发提出的工具批次；拒绝时不改变现有计数。
fn accept_tool_calls(
    budget: &mut ToolCallBudget,
    tool_calls: &[StarToolCall],
) -> Result<(), ToolBudgetExhausted> {
    if budget.calls_this_turn + tool_calls.len() > budget.max_calls_per_turn {
        return Err(ToolBudgetExhausted::Total {
            accepted: budget.calls_this_turn,
            pending: tool_calls.len(),
            limit: budget.max_calls_per_turn,
        });
    }

    let mut proposed_by_tool = std::collections::HashMap::<&str, usize>::new();
    for tool_call in tool_calls {
        *proposed_by_tool
            .entry(tool_call.function.name.as_str())
            .or_default() += 1;
    }
    for (tool_name, pending) in proposed_by_tool {
        let accepted = budget
            .tool_calls
            .get(tool_name)
            .copied()
            .unwrap_or_default();
        if accepted + pending > budget.max_calls_per_tool {
            return Err(ToolBudgetExhausted::PerTool {
                tool_name: tool_name.to_string(),
                accepted,
                pending,
                limit: budget.max_calls_per_tool,
            });
        }
    }

    budget.calls_this_turn += tool_calls.len();
    for tool_call in tool_calls {
        *budget
            .tool_calls
            .entry(tool_call.function.name.clone())
            .or_default() += 1;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum ToolExecutionSegment {
    Batch(Vec<usize>),
    Barrier(usize),
}

/// 将必须独占的检索工具作为 barrier，保留相邻普通调用的批处理并发。
fn tool_execution_segments(tool_calls: &[StarToolCall]) -> Vec<ToolExecutionSegment> {
    let mut segments = Vec::new();
    let mut batch = Vec::new();

    for (index, tool_call) in tool_calls.iter().enumerate() {
        if matches!(
            tool_call.function.name.as_str(),
            "SemanticSearch" | "ProjectMap"
        ) {
            if !batch.is_empty() {
                segments.push(ToolExecutionSegment::Batch(std::mem::take(&mut batch)));
            }
            segments.push(ToolExecutionSegment::Barrier(index));
        } else {
            batch.push(index);
        }
    }
    if !batch.is_empty() {
        segments.push(ToolExecutionSegment::Batch(batch));
    }

    segments
}

/// Extract file path from tool call arguments
fn extract_file_path_from_tool_call(tool_call: &StarToolCall) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments).ok()?;

    // Try common parameter names
    for key in &["file_path", "filePath", "path", "filename", "file"] {
        if let Some(value) = args.get(*key).and_then(|v| v.as_str()) {
            return Some(value.to_string());
        }
    }

    // For multi_edit, extract from edits array
    if let Some(edits) = args.get("edits").and_then(|v| v.as_array()) {
        if let Some(first_edit) = edits.first() {
            if let Some(fp) = first_edit.get("file_path").and_then(|v| v.as_str()) {
                return Some(fp.to_string());
            }
        }
    }

    None
}

/// Record a tool execution outcome in the LoopState for failure tracking
/// and strategy adaptation. This was previously dead code — now wired up.
///
/// 返回一个可选的恢复指令：工具失败时，根据 `LoopState` 当前采纳的
/// `LoopStrategy` 生成明确的下一步动作，注入会话供 LLM 执行，使策略
/// 真正驱动行为（而非仅作展示/统计）。
fn record_tool_outcome(
    loop_state: &mut LoopState,
    tool_name: &str,
    result: &crate::types::ToolResult,
) -> Option<String> {
    if result.success {
        loop_state.record_success(tool_name, "tool_executed");
        return None;
    }

    let structured = StructuredError::from_tool_output(
        tool_name,
        result.output.as_deref().unwrap_or(""),
        result.error.as_deref().unwrap_or("unknown error"),
    );
    loop_state.record_failure(tool_name, "tool_executed", structured.clone());

    // 依据已采纳的策略生成可执行的恢复指令。
    let instruction = match loop_state.strategy {
        LoopStrategy::Normal => None,
        LoopStrategy::RetryWithDifferentArgs => Some(format!(
            "[RECOVERY] Tool `{}` failed. Retry with different arguments or a modified approach. Error: {}",
            tool_name, structured.error
        )),
        LoopStrategy::FallbackToSimplerTool => Some(format!(
            "[RECOVERY] Tool `{}` keeps failing. Switch to a simpler or alternative tool. Error: {}",
            tool_name, structured.error
        )),
        LoopStrategy::BreakAndReport => Some(format!(
            "[RECOVERY] Repeated failures with `{}`. STOP retrying this. Clearly report the blocker to the user and suggest concrete next steps. Error: {}",
            tool_name, structured.error
        )),
    };

    if instruction.is_some() {
        crate::utils::logging::append_debug_log_line(&format!(
            "[LOOP_CONTEXT] {:?} strategy -> recovery instruction injected",
            loop_state.strategy
        ));
    }

    instruction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StarToolCallFunction;

    fn tool_call(id: usize, name: &str) -> StarToolCall {
        StarToolCall {
            id: format!("call_{id}"),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn tool_budget_enforces_independent_per_tool_limits_and_turn_reset() {
        let mut budget = ToolCallBudget::new(50, 50.0);
        let reads = (0..15).map(|id| tool_call(id, "Read")).collect::<Vec<_>>();
        accept_tool_calls(&mut budget, &reads).expect("fifteenth Read is permitted");
        assert_eq!(budget.calls_this_turn, 15);
        assert_eq!(budget.tool_calls.get("Read"), Some(&15));

        let rejected = accept_tool_calls(&mut budget, &[tool_call(15, "Read")]);
        assert!(matches!(
            rejected,
            Err(ToolBudgetExhausted::PerTool {
                ref tool_name,
                accepted: 15,
                pending: 1,
                limit: 15,
            }) if tool_name == "Read"
        ));
        assert_eq!(budget.calls_this_turn, 15, "rejected batches stay atomic");
        assert_eq!(budget.tool_calls.get("Read"), Some(&15));

        accept_tool_calls(&mut budget, &[tool_call(16, "Grep")])
            .expect("different tools retain independent ceilings");
        assert_eq!(budget.tool_calls.get("Grep"), Some(&1));

        budget.reset_turn();
        assert_eq!(budget.calls_this_turn, 0);
        assert!(budget.tool_calls.is_empty());
        accept_tool_calls(&mut budget, &[tool_call(17, "Read")])
            .expect("the existing turn reset restores the tool quota");
    }

    #[test]
    fn tool_segments_preserve_order_around_long_running_barriers() {
        let calls = [
            tool_call(0, "Read"),
            tool_call(1, "Grep"),
            tool_call(2, "SemanticSearch"),
            tool_call(3, "Read"),
            tool_call(4, "ProjectMap"),
            tool_call(5, "Glob"),
        ];
        assert_eq!(
            tool_execution_segments(&calls),
            vec![
                ToolExecutionSegment::Batch(vec![0, 1]),
                ToolExecutionSegment::Barrier(2),
                ToolExecutionSegment::Batch(vec![3]),
                ToolExecutionSegment::Barrier(4),
                ToolExecutionSegment::Batch(vec![5]),
            ]
        );
    }

    #[test]
    fn tool_segments_batch_calls_without_or_beside_long_running_tools() {
        let regular = [tool_call(0, "Read"), tool_call(1, "Grep")];
        assert_eq!(
            tool_execution_segments(&regular),
            vec![ToolExecutionSegment::Batch(vec![0, 1])]
        );

        let edges = [
            tool_call(0, "SemanticSearch"),
            tool_call(1, "Read"),
            tool_call(2, "ProjectMap"),
        ];
        assert_eq!(
            tool_execution_segments(&edges),
            vec![
                ToolExecutionSegment::Barrier(0),
                ToolExecutionSegment::Batch(vec![1]),
                ToolExecutionSegment::Barrier(2),
            ]
        );
    }
}
