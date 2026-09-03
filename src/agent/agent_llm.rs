use crate::agent::agent_core::Agent;
use crate::agent::loop_engineering::{
    AgentError, LoopState, RecoveryAction, RecoveryContext, RecoveryManager,
};
use crate::agent::messaging::AgentEvent;
use crate::agent::tool_routing::ToolSelection;
use crate::agent::{hooks, tool_helpers};
use crate::types::{StarMessage, StarToolCall};
use std::time::Instant;

/// LLM 调用的结果
pub(crate) enum LlmResult {
    Success {
        content: String,
        reasoning: String,
        tool_calls: Vec<StarToolCall>,
        usage: Option<crate::types::StarUsage>,
        /// Whether content was already streamed to the UI via TextDelta (Branch B).
        content_streamed: bool,
        /// Whether reasoning was already streamed to the UI via ReasoningDelta (Branch B).
        reasoning_streamed: bool,
        /// True when the model hit its output token limit (finish_reason="length").
        /// Truncated tool calls have already been removed.
        was_truncated: bool,
    },
    Error(AgentEvent),
    Retry,
}

impl Agent {
    // 调用 LLM 并处理流式响应
    pub(crate) async fn call_llm(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        request_messages: &mut Vec<StarMessage>,
        turn_tool_selection: &ToolSelection,
        current_turn: i32,
        recovery_manager: &mut RecoveryManager,
        loop_state: &mut LoopState,
    ) -> LlmResult {
        let verbose_debug_logging = crate::utils::logging::is_verbose_debug_logging_enabled();

        // Token 用量预估日志
        let estimated_tokens: usize = request_messages
            .iter()
            .map(|m| {
                let content_len = m.content.as_ref().map(|c| c.len()).unwrap_or(0);
                let reasoning_len = m.reasoning_content.as_ref().map(|r| r.len()).unwrap_or(0);
                let tool_call_len = m.tool_calls.as_ref().map(|tc| tc.len() * 80).unwrap_or(0);
                (content_len + reasoning_len) / 4 + tool_call_len
            })
            .sum();
        let tool_schema_json_len: usize = turn_tool_selection
            .tools
            .iter()
            .map(|t| {
                *self
                    .tool_schema_len_cache
                    .entry(t.function.name.clone())
                    .or_insert_with(|| serde_json::to_string(t).map(|s| s.len()).unwrap_or(0))
            })
            .sum();
        let tool_schema_tokens = tool_schema_json_len / 4;

        crate::utils::logging::append_debug_log_line(&format!(
            "[TOKEN] Turn {}: {} msgs, ~{} msg tokens + ~{} tool-schema tokens ({} tools, {} bytes JSON), ~{} total est.",
            current_turn,
            request_messages.len(),
            estimated_tokens,
            tool_schema_tokens,
            turn_tool_selection.tools.len(),
            tool_schema_json_len,
            estimated_tokens + tool_schema_tokens,
        ));

        // 预发送 Token 检查
        let context_window = self.context_compressor.context_window() as usize;
        let safety_threshold = (context_window as f64 * 0.80) as usize;
        if estimated_tokens + tool_schema_tokens > safety_threshold {
            crate::utils::logging::append_debug_log_line(&format!(
                "[WARN] Estimated tokens {} > {} (80% of {}), auto-compressing before send",
                estimated_tokens + tool_schema_tokens,
                safety_threshold,
                context_window,
            ));

            let compact_result = self.compact_manager.compact(request_messages);

            if compact_result.was_compacted {
                *request_messages = compact_result.messages;
            } else {
                // Clone messages before compression attempt so we can recover on failure
                let original_messages = request_messages.clone();
                match self
                    .context_compressor
                    .force_compress(request_messages.clone(), Some(&self.client))
                    .await
                {
                    Ok(compression_result) => {
                        *request_messages = compression_result.messages;
                    }
                    Err(e) => {
                        // Don't stop the agent for compression failures
                        // Restore original messages and continue
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[WARN] Pre-send compression failed: {} — using original messages",
                            e,
                        ));
                        *request_messages = original_messages;
                    }
                }
            }
        }

        // Make the actual LLM call — take ownership of request_messages
        // to avoid a full Vec clone (hundreds of KB on hot path)
        let msgs = std::mem::take(request_messages);

        // ── Token Budget Nudge ──
        // When the estimated token count exceeds 90% of the context window,
        // inject a system message telling the model to wrap up its work.
        // This prevents the agent from being abruptly cut off when the
        // context window is exhausted. Mirrors Claude Code's cost-tracker
        // nudge that fires at $5+ spend.
        let nudge_threshold = (context_window as f64 * 0.90) as usize;
        let total_est = estimated_tokens + tool_schema_tokens;
        let msgs = if total_est > nudge_threshold {
            let pct = ((total_est as f64 / context_window as f64) * 100.0) as u32;
            crate::utils::logging::append_debug_log_line(&format!(
                "[TOKEN_BUDGET] {pct}% of context window used (~{est}/{max} tokens) — injecting budget nudge",
                pct = pct,
                est = total_est,
                max = context_window,
            ));
            let mut nudged_msgs = msgs;
            let warning_template =
                crate::core::prompts::loader::load_prompt("token-budget-warning.md");
            nudged_msgs.push(StarMessage::system(
                crate::core::prompts::loader::render_template(
                    &warning_template,
                    &[
                        ("pct", &format!("{pct}")),
                        ("est", &format!("{total_est}")),
                        ("max", &format!("{context_window}")),
                    ],
                ),
            ));
            nudged_msgs
        } else {
            msgs
        };

        let mut stream = match self
            .client
            .chat_stream(msgs, Some(turn_tool_selection.tools.clone()), None, None)
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                return self
                    .handle_llm_error(
                        e.to_string(),
                        messages,
                        recovery_manager,
                        loop_state,
                        current_turn,
                        estimated_tokens,
                        context_window,
                    )
                    .await;
            }
        };

        let model_request_started_at = Instant::now();

        if verbose_debug_logging {
            crate::utils::logging::append_debug_log_line(
                "[DEBUG] Agent: chat_stream created, starting to read chunks",
            );
        }

        let mut current_content = String::new();
        let mut current_reasoning = String::new();
        let mut tool_calls: Vec<StarToolCall> = Vec::new();
        let mut content_streamed = false;
        let mut reasoning_streamed = false;
        let mut finish_reason: Option<String> = None;
        let mut last_usage: Option<crate::types::StarUsage> = None;

        // Thinking protection limits — prevent models from getting stuck in
        // infinite reasoning loops. Defaults scale by task complexity:
        //   Simple: 5000 tokens / 30s
        //   Medium: 15000 tokens / 60s
        //   Complex: 30000 tokens / 120s
        // Users can override via env vars.
        let complexity_default_tokens = match self.task_complexity {
            crate::core::routing::RequestComplexity::Simple => 5000,
            crate::core::routing::RequestComplexity::Medium => 15000,
            crate::core::routing::RequestComplexity::Complex => 30000,
        };
        let complexity_default_secs = match self.task_complexity {
            crate::core::routing::RequestComplexity::Simple => 30,
            crate::core::routing::RequestComplexity::Medium => 60,
            crate::core::routing::RequestComplexity::Complex => 120,
        };
        let max_thinking_tokens: usize = std::env::var("STAR_MAX_THINKING_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(complexity_default_tokens);
        // `thinking_started_at` tracks the LAST reasoning activity (not stream
        // start). Every incoming reasoning chunk refreshes it, so the
        // thinking limit acts as an IDLE timeout: a model may reason for as
        // long as it keeps producing thinking content. Only a pause longer
        // than max_thinking_duration (or exceeding max_thinking_tokens)
        // triggers the limit — this prevents thinking from being cut off
        // mid-reasoning on deep-thinker models (DeepSeek R1 etc.).
        let mut thinking_started_at = std::time::Instant::now();
        let max_thinking_duration = std::time::Duration::from_secs(
            std::env::var("STAR_MAX_THINKING_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(complexity_default_secs),
        );

        use futures::StreamExt;
        let mut chunk_count = 0;
        let mut first_model_chunk_seen = false;
        let mut thinking_limit_reached = false;
        let mut thinking_limit_reached_at: Option<std::time::Instant> = None;
        // Grace period after thinking limit: if the model STILL hasn't produced
        // text or tool calls within this window, abort the stream to avoid
        // hanging the UI on a runaway thinking loop. Default 3s, configurable.
        let thinking_abort_grace = std::time::Duration::from_secs(
            std::env::var("STAR_THINKING_ABORT_GRACE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        );
        // 流式停滞检测（类似 Claude Code 的 stall detection）：
        // - stall_threshold: 10s 无事件视为一次 stall
        // - idle_timeout: 30s 完全无响应则终止流
        let stall_threshold = std::time::Duration::from_secs(
            std::env::var("STAR_STREAM_STALL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        );
        let idle_timeout = std::time::Duration::from_secs(
            std::env::var("STAR_STREAM_IDLE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        );
        let mut last_event_at = Instant::now();
        let mut stall_count = 0u32;
        let mut total_stall_ms = 0u64;

        // Get abort token for user-initiated cancellation (Ctrl+C)
        let abort_token = self.abort_token.clone();

        loop {
            // Idle timeout: if no chunk arrives within idle_timeout,
            // abort the stream to prevent indefinite hanging.
            // Also check for user abort signal (Ctrl+C).
            // Also check for thinking timeout.
            let chunk_result = tokio::select! {
                result = stream.next() => result,
                _ = tokio::time::sleep(idle_timeout) => {
                    let elapsed = last_event_at.elapsed();
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[STALL] Stream idle timeout after {:.1}s (last event {:.1}s ago, {} stalls, {}ms total stall)",
                        idle_timeout.as_secs_f64(),
                        elapsed.as_secs_f64(),
                        stall_count,
                        total_stall_ms,
                    ));
                    break;
                }
                // Thinking timeout: independent of event stream
                _ = async {
                    if !thinking_limit_reached {
                        let remaining = max_thinking_duration.saturating_sub(thinking_started_at.elapsed());
                        if remaining.is_zero() {
                            return;
                        }
                        tokio::time::sleep(remaining).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }, if !thinking_limit_reached => {
                    thinking_limit_reached = true;
                    thinking_limit_reached_at = Some(std::time::Instant::now());
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[THINKING] timeout in select: {:.1}s elapsed (max {}s) — will abort if no content",
                        thinking_started_at.elapsed().as_secs_f64(),
                        max_thinking_duration.as_secs(),
                    ));
                    continue;
                }
                // Thinking abort grace: after thinking_limit_reached, wait
                // for the grace period then force-break the stream. This runs
                // independently of chunk arrivals, so it handles the case
                // where the model stops sending chunks entirely.
                _ = async {
                    if let Some(limit_at) = thinking_limit_reached_at {
                        let remaining = thinking_abort_grace.saturating_sub(limit_at.elapsed());
                        if remaining.is_zero() {
                            return;
                        }
                        tokio::time::sleep(remaining).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }, if thinking_limit_reached && current_content.trim().is_empty() && tool_calls.is_empty() => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[THINKING] abort grace timer fired — forcing stream break (no content/tool_calls after {:.1}s)",
                        thinking_limit_reached_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0),
                    ));
                    break;
                }
                _ = async {
                    if let Some(ref token) = abort_token {
                        token.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    crate::utils::logging::append_debug_log_line(
                        "[ABORT] User abort signal received during streaming — breaking stream",
                    );
                    break;
                }
            };
            let chunk_result = match chunk_result {
                Some(result) => result,
                None => break, // Stream ended normally
            };
            chunk_count += 1;
            if verbose_debug_logging && chunk_count <= 3 {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[DEBUG] Agent: Received chunk #{}",
                    chunk_count
                ));
            }

            match chunk_result {
                Ok(json) => {
                    // Handle trace events
                    if let Some(trace) = json.get("star_trace") {
                        if let Some(event_name) =
                            trace.get("event").and_then(|value| value.as_str())
                        {
                            // Trace events are handled by the caller
                            continue;
                        }
                    }

                    if !first_model_chunk_seen {
                        first_model_chunk_seen = true;
                    }

                    // Parse usage update
                    if let Some(usage) = json.get("usage").and_then(|v| v.as_object()) {
                        let prompt_tokens = usage
                            .get("prompt_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let completion_tokens = usage
                            .get("completion_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let reported = usage
                            .get("total_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let total_tokens = if reported > 0 {
                            reported
                        } else {
                            prompt_tokens + completion_tokens
                        };

                        // Log cache hit information (DeepSeek/Anthropic specific fields)
                        let cache_hit_tokens = usage
                            .get("prompt_cache_hit_tokens")
                            .or_else(|| usage.get("cache_read_input_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let cache_miss_tokens = usage
                            .get("prompt_cache_miss_tokens")
                            .or_else(|| usage.get("cache_creation_input_tokens"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;

                        if cache_hit_tokens > 0 || cache_miss_tokens > 0 {
                            let hit_rate = if cache_hit_tokens + cache_miss_tokens > 0 {
                                (cache_hit_tokens as f64
                                    / (cache_hit_tokens + cache_miss_tokens) as f64
                                    * 100.0) as u32
                            } else {
                                0
                            };
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[CACHE] Turn {}: hit={} miss={} rate={}%",
                                current_turn, cache_hit_tokens, cache_miss_tokens, hit_rate
                            ));
                        }

                        if total_tokens > 0 || prompt_tokens > 0 || completion_tokens > 0 {
                            last_usage = Some(crate::types::StarUsage {
                                prompt_tokens,
                                completion_tokens,
                                total_tokens,
                                cache_read_tokens: cache_hit_tokens,
                                cache_creation_tokens: cache_miss_tokens,
                            });
                        }
                    }

                    // Parse the JSON chunk
                    if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                        if let Some(choice) = choices.first() {
                            if let Some(delta) = choice.get("delta").and_then(|d| d.as_object()) {
                                // Text content — streamed in real-time via Branch B
                                // for typewriter effect, also buffered for session history.
                                // Only mark as streamed when stream_tx is active (interactive mode);
                                // in headless mode the Branch A emission is needed.
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                                {
                                    if !content.is_empty() {
                                        self.emit_direct_chunk(
                                            crate::types::StreamingChunk::text_delta(content),
                                        );
                                        content_streamed = self.stream_tx.is_some();
                                        current_content.push_str(content);
                                    }
                                }

                                // Reasoning content — streamed in real-time via Branch B
                                // for typewriter effect, also buffered for session history.
                                if let Some(raw_reasoning) =
                                    delta.get("reasoning_content").and_then(|r| r.as_str())
                                {
                                    if !raw_reasoning.is_empty() && !thinking_limit_reached {
                                        // Refresh the thinking idle timer: each incoming
                                        // reasoning chunk counts as activity, so the
                                        // limit only fires after a genuine pause.
                                        thinking_started_at = std::time::Instant::now();
                                        let reasoning =
                                            tool_helpers::sanitize_reasoning_content(raw_reasoning);
                                        if !reasoning.is_empty() {
                                            let thinking_tokens = current_reasoning.len() / 4;
                                            let thinking_elapsed = thinking_started_at.elapsed();

                                            if thinking_tokens >= max_thinking_tokens {
                                                thinking_limit_reached = true;
                                                thinking_limit_reached_at =
                                                    Some(std::time::Instant::now());
                                                crate::utils::logging::append_debug_log_line(&format!(
                                                    "[THINKING] limit reached: {} tokens (max {}) — suppressing further reasoning",
                                                    thinking_tokens, max_thinking_tokens,
                                                ));
                                            } else if thinking_elapsed >= max_thinking_duration {
                                                thinking_limit_reached = true;
                                                thinking_limit_reached_at =
                                                    Some(std::time::Instant::now());
                                                crate::utils::logging::append_debug_log_line(&format!(
                                                    "[THINKING] limit reached: {:.1}s elapsed (max {}s) — suppressing further reasoning",
                                                    thinking_elapsed.as_secs_f64(), max_thinking_duration.as_secs(),
                                                ));
                                            } else {
                                                self.emit_direct_chunk(
                                                    crate::types::StreamingChunk::reasoning_delta(
                                                        &reasoning,
                                                    ),
                                                );
                                                reasoning_streamed = self.stream_tx.is_some();
                                                current_reasoning.push_str(&reasoning);
                                            }
                                        }
                                    }
                                }

                                // Tool calls
                                if let Some(tc_array) =
                                    delta.get("tool_calls").and_then(|t| t.as_array())
                                {
                                    for tc in tc_array {
                                        let index =
                                            tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                                as usize;
                                        while tool_calls.len() <= index {
                                            tool_calls.push(StarToolCall {
                                                id: String::new(),
                                                call_type: "function".to_string(),
                                                function: crate::types::StarToolCallFunction {
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                },
                                            });
                                        }
                                        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                            if !id.is_empty() {
                                                tool_calls[index].id = id.to_string();
                                            }
                                        }
                                        if let Some(func) = tc.get("function") {
                                            if let Some(name) =
                                                func.get("name").and_then(|n| n.as_str())
                                            {
                                                if !name.is_empty() {
                                                    tool_calls[index].function.name =
                                                        name.to_string();
                                                }
                                            }
                                            if let Some(args) =
                                                func.get("arguments").and_then(|a| a.as_str())
                                            {
                                                tool_calls[index].function.arguments.push_str(args);
                                            }
                                        }
                                    }
                                }
                            }

                            // Handle finish reason
                            if let Some(finish) =
                                choice.get("finish_reason").and_then(|f| f.as_str())
                            {
                                if finish != "null" && !finish.is_empty() {
                                    finish_reason = Some(finish.to_string());
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if !current_content.is_empty() {
                        // 用 take 避免 clone：此处 return 后不再使用这些变量
                        let content = std::mem::take(&mut current_content);
                        let reasoning = std::mem::take(&mut current_reasoning);
                        let mut partial = StarMessage::assistant(content);
                        if !reasoning.is_empty() {
                            partial.reasoning_content = Some(reasoning);
                        }
                        messages.push(partial);
                    }

                    let err_str = e.to_string();
                    return self
                        .handle_llm_error(
                            err_str,
                            messages,
                            recovery_manager,
                            loop_state,
                            current_turn,
                            estimated_tokens,
                            context_window,
                        )
                        .await;
                }
            }

            // 流式停滞检测：每次收到事件后更新计时器
            {
                let since_last = last_event_at.elapsed();
                if since_last > stall_threshold {
                    stall_count += 1;
                    total_stall_ms += since_last.as_millis() as u64;
                }
                last_event_at = Instant::now();
            }

            // 检查 thinking 时间限制（即使没有新 reasoning 内容也要检查）
            // 这处理模型停发 reasoning 但不关流的情况
            if !thinking_limit_reached {
                let thinking_elapsed = thinking_started_at.elapsed();
                let thinking_tokens = current_reasoning.len() / 4;
                if thinking_tokens >= max_thinking_tokens {
                    thinking_limit_reached = true;
                    thinking_limit_reached_at = Some(std::time::Instant::now());
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[THINKING] limit reached: {} tokens (max {}) — suppressing further reasoning",
                        thinking_tokens, max_thinking_tokens,
                    ));
                } else if thinking_elapsed >= max_thinking_duration {
                    thinking_limit_reached = true;
                    thinking_limit_reached_at = Some(std::time::Instant::now());
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[THINKING] limit reached: {:.1}s elapsed (max {}s) — suppressing further reasoning",
                        thinking_elapsed.as_secs_f64(), max_thinking_duration.as_secs(),
                    ));
                }
            }

            // Abort runaway thinking: if the thinking limit was reached but the
            // model still hasn't produced any text or tool calls within the grace
            // period, drop the stream (cancelling the in-flight request) so the
            // recovery nudge in process_llm_response can retry the turn.
            if thinking_limit_reached && current_content.trim().is_empty() && tool_calls.is_empty()
            {
                if let Some(limit_at) = thinking_limit_reached_at {
                    if limit_at.elapsed() >= thinking_abort_grace {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[THINKING] aborting stream after {:.1}s grace (thinking limit reached, no content/tool_calls) — will trigger recovery",
                            limit_at.elapsed().as_secs_f64(),
                        ));
                        break;
                    }
                }
            }
        }

        // Detect thinking-loop: model exhausted its thinking budget without
        // producing meaningful text or tool calls. Log so the recovery nudge
        // in process_llm_response has context.
        if thinking_limit_reached && tool_calls.is_empty() && current_content.trim().len() < 20 {
            crate::utils::logging::append_debug_log_line(&format!(
                "[THINKING] thinking limit reached with no usable output (content={}chars, tool_calls=0, reasoning={}chars) — will trigger recovery",
                current_content.trim().len(),
                current_reasoning.len(),
            ));
        }

        // After model hook
        if let Err(reason) = hooks::run_stage_hooks(
            user_input,
            crate::core::hooks::store::ManagedHookEvent::AfterModel,
            "after_model",
            Some(serde_json::json!({
                "turn": current_turn,
                "finish_reason": finish_reason,
                "tool_calls_count": tool_calls.len(),
                "has_content": !current_content.is_empty(),
            })),
        )
        .await
        {
            return LlmResult::Error(AgentEvent::Error(reason));
        }

        // Filter out incomplete tool calls and assign missing IDs
        tool_calls.retain(|tc| !tc.function.name.is_empty());

        // Detect output-length truncation: the model hit max_tokens before
        // finishing.  Drop tool calls with empty arguments (they would
        // execute with "{}" and produce meaningless results) and tag the
        // response so process_llm_response can inject a continuation nudge.
        let was_truncated = finish_reason.as_deref() == Some("length");
        if was_truncated {
            crate::utils::logging::append_debug_log_line(&format!(
                "[AGENT] output truncated (finish_reason=length, {} tool_calls before cleanup)",
                tool_calls.len(),
            ));
            // Drop tool calls whose arguments are empty — they were cut off
            // mid-stream and would execute with meaningless "{}" args.
            let before = tool_calls.len();
            tool_calls.retain(|tc| !tc.function.arguments.is_empty());
            let dropped = before - tool_calls.len();
            if dropped > 0 {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AGENT] dropped {} truncated tool call(s) (empty arguments) — will retry",
                    dropped,
                ));
            }
        }

        for (i, tc) in tool_calls.iter_mut().enumerate() {
            if tc.id.is_empty() {
                tc.id = format!("call_auto_{}", i);
            }
            if tc.function.arguments.is_empty() {
                tc.function.arguments = "{}".to_string();
            }
        }

        // 成功响应：重置回退状态，使下一次失败从首选项重新开始
        self.model_fallback.reset();

        LlmResult::Success {
            content: current_content,
            reasoning: current_reasoning,
            tool_calls,
            usage: last_usage,
            content_streamed,
            reasoning_streamed,
            was_truncated,
        }
    }

    // 处理 LLM 错误
    pub(crate) async fn handle_llm_error(
        &mut self,
        err_str: String,
        messages: &mut Vec<StarMessage>,
        recovery_manager: &mut RecoveryManager,
        loop_state: &mut LoopState,
        current_turn: i32,
        estimated_tokens: usize,
        context_window: usize,
    ) -> LlmResult {
        let recovery_context = RecoveryContext {
            current_tokens: estimated_tokens,
            max_tokens: context_window,
            current_output_tokens: 0,
            max_output_tokens: 4096,
            turn_count: current_turn as u32,
            messages_since_last_compact: 0,
            last_tool_used: None,
            last_tool_error: None,
        };

        // 记录原始错误，便于诊断流式失败的根本原因
        let err_preview: String = err_str.chars().take(300).collect();
        crate::utils::logging::append_debug_log_line(&format!(
            "[LLM_ERROR] turn={} error={}",
            current_turn, err_preview,
        ));

        let agent_error = if err_str.contains("context window exceeds")
            || err_str.contains("context_length_exceeded")
        {
            AgentError::PromptTooLong
        } else if err_str.contains("max_output_tokens") {
            AgentError::MaxOutputTokens
        } else if err_str.contains("rate_limit") || err_str.contains("429") {
            AgentError::RateLimit
        } else {
            AgentError::StreamingError
        };

        match recovery_manager.handle_error(&agent_error, &recovery_context) {
            RecoveryAction::CompactAndRetry => {
                crate::utils::logging::append_debug_log_line(
                    "[RECOVERY] Attempting context compression and retry",
                );
                let compact_result = self.compact_manager.compact(messages);
                if compact_result.was_compacted {
                    *messages = compact_result.messages;
                }
                LlmResult::Retry
            }
            RecoveryAction::EscalateOutputTokens => {
                crate::utils::logging::append_debug_log_line(
                    "[RECOVERY] Escalating output tokens and retry",
                );
                // 实际提升 max_tokens：每次翻倍，上限 64K
                let current = self.client.default_max_tokens;
                let new_max = (current * 2).min(65536);
                if new_max > current {
                    self.client.default_max_tokens = new_max;
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[RECOVERY] Max output tokens: {} -> {}",
                        current, new_max
                    ));
                }
                // Claude Code 模式：注入 continue-where-you-left-off 提示
                messages.push(StarMessage::system(
                    "Continue where you left off. Your previous response was truncated. \
                     Pick up exactly where you stopped and complete the remaining work concisely."
                        .to_string(),
                ));
                LlmResult::Retry
            }
            RecoveryAction::InjectRecoveryMessage(msg) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Injecting recovery message: {}",
                    msg
                ));
                messages.push(StarMessage::system(msg));
                LlmResult::Retry
            }
            RecoveryAction::SwitchProviderAndRetry => {
                crate::utils::logging::append_debug_log_line(
                    "[RECOVERY] Switching provider and retry",
                );
                // 优先使用环境变量中配置的单个后备模型/端点（保持原有行为）
                let fallback_model = std::env::var("STAR_FALLBACK_MODEL").ok();
                let fallback_url = std::env::var("STAR_FALLBACK_BASE_URL").ok();
                if let Some(ref alt_model) = fallback_model {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[RECOVERY] Switching to STAR_FALLBACK_MODEL: {} -> {}",
                        self.client.model, alt_model
                    ));
                    self.client.set_model(alt_model);
                    self.emit_fallback_event(&err_str, alt_model, None);
                } else if let Some(ref url) = fallback_url {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[RECOVERY] Switching to STAR_FALLBACK_BASE_URL: {}",
                        url
                    ));
                    self.client.set_base_url(url);
                    self.emit_fallback_event(&err_str, &self.client.model, Some(url));
                } else if self.model_fallback.is_fallback_eligible_error(&err_str) {
                    // 列表式回退配置（STAR_FALLBACK_MODELS / STAR_FALLBACK_BASE_URLS）：
                    // 由 ModelFallbackManager 统一管理回退顺序与重试上限。
                    let original_model = self.client.model.clone();
                    match self.model_fallback.try_fallback(&original_model) {
                        crate::agent::model_fallback::FallbackDecision::Fallback {
                            model,
                            base_url,
                            reason,
                        } => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[MODEL_FALLBACK] Applying: {}",
                                reason
                            ));
                            if let Some(ref url) = base_url {
                                self.client.set_base_url(url);
                            } else {
                                self.client.set_model(&model);
                            }
                            self.emit_fallback_event(&err_str, &model, base_url.as_deref());
                        }
                        crate::agent::model_fallback::FallbackDecision::NoFallback { reason } => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[MODEL_FALLBACK] No fallback: {}",
                                reason
                            ));
                            self.fallback_cooldown().await;
                        }
                    }
                } else {
                    // 错误不可回退（如认证失败）：冷却等待后重试
                    self.fallback_cooldown().await;
                }
                LlmResult::Retry
            }
            RecoveryAction::StopWithError(error_msg) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Stopping with error: {}",
                    error_msg
                ));
                if err_str.contains("context window exceeds")
                    || err_str.contains("context_length_exceeded")
                {
                    crate::agent::model_catalog::halve_cached_context_window(
                        self.client.get_current_model(),
                    );
                    LlmResult::Error(AgentEvent::Error(format!(
                        "上下文窗口超限，已自动降低窗口大小。请重试（系统将使用更激进的压缩策略）。\nContext window exceeded, auto-reduced window size. Please retry.\nStream error: {}",
                        err_str
                    )))
                } else {
                    LlmResult::Error(AgentEvent::Error(format!("Stream error: {}", err_str)))
                }
            }
            RecoveryAction::Continue => {
                LlmResult::Error(AgentEvent::Error(format!("Stream error: {}", err_str)))
            }
            RecoveryAction::CircuitBreakerCooldown(duration) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Circuit breaker cooldown: {:?}",
                    duration
                ));
                // Send periodic heartbeats during cooldown so the UI's STALL
                // watchdog doesn't clear is_processing (30s threshold).
                // Also gives the user visible feedback that the agent is alive.
                let heartbeat_interval = std::time::Duration::from_secs(5);
                let mut remaining = duration;
                while remaining > std::time::Duration::ZERO {
                    let wait = remaining.min(heartbeat_interval);
                    self.emit_event(crate::agent::messaging::AgentEvent::Trace {
                        event: "agent_status".to_string(),
                        payload: serde_json::json!({
                            "status": "retrying",
                            "phase": "rate_limit_cooldown",
                            "message": format!(
                                "Rate limit hit — retrying in {}s",
                                remaining.as_secs(),
                            ),
                        }),
                    });
                    tokio::time::sleep(wait).await;
                    remaining = remaining.saturating_sub(wait);
                }
                LlmResult::Retry
            }
            RecoveryAction::FallbackToSimplerTool {
                original_tool,
                fallback_tool,
                reason,
            } => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Falling back from {} to {}: {}",
                    original_tool, fallback_tool, reason
                ));
                LlmResult::Retry
            }
            RecoveryAction::RetryWithDifferentArgs { suggestion } => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Retrying with different args: {}",
                    suggestion
                ));
                messages.push(StarMessage::system(format!(
                    "Previous attempt failed. {}",
                    suggestion
                )));
                LlmResult::Retry
            }
            RecoveryAction::SkipAndContinue { reason } => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RECOVERY] Skipping step: {}",
                    reason
                ));
                messages.push(StarMessage::system(format!(
                    "Skipping previous step: {}",
                    reason
                )));
                LlmResult::Retry
            }
        }
    }

    /// 回退生效时通过 Trace 事件通知 UI（对标 Claude Code 显示 fallback 提示）。
    /// 仅报告，不影响回退本身的成功与否。
    fn emit_fallback_event(&self, original_error: &str, model: &str, base_url: Option<&str>) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[MODEL_FALLBACK] emit_fallback_event model={} url={:?} err={}",
            model,
            base_url,
            original_error.chars().take(120).collect::<String>()
        ));
        self.emit_event(crate::agent::messaging::AgentEvent::Trace {
            event: "model_fallback".to_string(),
            payload: serde_json::json!({
                "model": model,
                "base_url": base_url,
                "error": original_error.chars().take(300).collect::<String>(),
            }),
        });
    }

    /// 无回退可用时的冷却等待（应对临时限流 429），保持原有行为
    async fn fallback_cooldown(&self) {
        let wait_secs = std::env::var("STAR_RATE_LIMIT_RETRY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10u64);
        crate::utils::logging::append_debug_log_line(&format!(
            "[RECOVERY] No fallback configured, waiting {}s before retry",
            wait_secs
        ));
        tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
    }
}
