use crate::agent::agent_core::Agent;
use crate::agent::messaging::AgentEvent;
use crate::types::StarMessage;
use futures::Stream;
use std::pin::Pin;

impl Agent {
    pub fn run_stream<'a>(
        &'a mut self,
        user_input: String,
    ) -> Pin<
        Box<
            dyn Stream<Item = Result<AgentEvent, Box<dyn std::error::Error + Send + Sync>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async_stream::try_stream! {
            let history_len = self
                .session_messages
                .iter()
                .filter(|m| m.role != "system")
                .count();

            // 3. Routing & Context (with optional LLM-based semantic upgrade for short inputs)
            yield AgentEvent::Trace {
                event: "agent_status".to_string(),
                payload: serde_json::json!({
                    "status": "analyzing",
                    "phase": "task_classification",
                    "message": "Analyzing task complexity...",
                }),
            };
            let request_complexity = crate::agent::router::Router::classify_with_semantic_upgrade(
                &self.client,
                &user_input,
                history_len,
            ).await;
            // Store complexity for dynamic thinking limits in call_llm
            self.task_complexity = request_complexity;
            let ctx = crate::core::routing::RoutingContext {
                history_length: history_len,
                request_complexity,
                user_override: None,
                default_model: self.client.model.clone(),
                fast_model: None,
                cheap_model: None,
            };

            // Complexity Hint Logic
            // Loaded from complexity-strategies.md (external dir overrides embedded).
            let complexity_hint = {
                let strategies = crate::core::prompts::loader::load_prompt("complexity-strategies.md");
                match ctx.request_complexity {
                    crate::core::routing::RequestComplexity::Complex => {
                        crate::utils::logging::append_debug_log_line("[ROUTER] Task complexity: Complex. Planning internally by default.");
                        extract_strategy_section(&strategies, "## COMPLEX").map(ToOwned::to_owned)
                    }
                    crate::core::routing::RequestComplexity::Medium => {
                        crate::utils::logging::append_debug_log_line("[ROUTER] Task complexity: Medium.");
                        extract_strategy_section(&strategies, "## MEDIUM").map(ToOwned::to_owned)
                    }
                    crate::core::routing::RequestComplexity::Simple => {
                        crate::utils::logging::append_debug_log_line("[ROUTER] Task complexity: Simple.");
                        extract_strategy_section(&strategies, "## SIMPLE").map(ToOwned::to_owned)
                    }
                }
            };

            // 从 strategies 文本中抽取指定小节（如 "## COMPLEX" 到下一个 "## "）
            fn extract_strategy_section<'a>(strategies: &'a str, section: &str) -> Option<&'a str> {
                let start = strategies.find(section)?;
                let rest = &strategies[start..];
                let end = rest[section.len()..]
                    .find("\n## ")
                    .map(|i| section.len() + i)
                    .unwrap_or(rest.len());
                Some(rest[..end].trim())
            }

            // 4. Get Tool Definitions (all tools + first-turn shortlist)
            let all_tools = self.tool_executor.get_tool_definitions();
            let mut all_active_tools = std::collections::HashSet::new();
            for tool in &all_tools {
                all_active_tools.insert(tool.function.name.clone());
            }
            let shortlist_profile = if self.client.is_kimi_code_provider() {
                "kimi_code_compact"
            } else {
                "default"
            };
            let initial_tool_selection =
                crate::agent::tool_routing::select_tools_for_turn_for_client(&self.client, &all_tools, &user_input, 1);
            yield AgentEvent::Trace {
                event: "routing_context_resolved".to_string(),
                payload: serde_json::json!({
                    "history_len": history_len,
                    "request_complexity": crate::agent::tool_routing::request_complexity_label(ctx.request_complexity),
                    "tool_count": all_tools.len(),
                    "initial_shortlist_size": initial_tool_selection.tools.len(),
                    "shortlist_profile": shortlist_profile,
                }),
            };

            // 5. Load Context + Auto-Plan in PARALLEL
            let cwd = crate::agent::hooks::cached_project_root()
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let load_dynamic_context = history_len > 0 || crate::agent::tool_routing::dynamic_context_first_turn_enabled();

            // Check if model supports thinking (using runtime detection if available)
            let is_thinking_model = self.client.supports_thinking();

            // Run context loading and auto-plan in PARALLEL using tokio::join!
            // Both tasks are independent and can run concurrently
            let context_future = async {
                if load_dynamic_context {
                    match self.context_engine.load_context_for_project(&cwd).await {
                        Ok(ctx) => {
                            if ctx.trim().is_empty() {
                                (None, "empty", "full", 0usize, None)
                            } else {
                                let chars = ctx.chars().count();
                                (Some(ctx), "loaded", "full", chars, None)
                            }
                        }
                        Err(e) => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[WARN] Failed to load dynamic context: {}",
                                e
                            ));
                            (None, "load_failed", "full", 0usize, Some(e.to_string()))
                        }
                    }
                } else {
                    crate::utils::logging::append_debug_log_line(
                        "[Context] Skipping heavy dynamic context matching on the first turn",
                    );
                    let ctx = self.context_engine.load_static_context_for_project();
                    if ctx.trim().is_empty() {
                        (None, "skipped_first_turn", "static_only", 0usize, None)
                    } else {
                        let chars = ctx.chars().count();
                        (Some(ctx), "skipped_first_turn", "static_only", chars, None)
                    }
                }
            };

            let plan_future = crate::agent::tool_routing::maybe_generate_auto_plan(
                &self.client,
                &user_input,
                &ctx.request_complexity,
                history_len,
                is_thinking_model,
            );

            // Execute both in parallel
            let (context_result, auto_plan_decision) = tokio::join!(context_future, plan_future);

            // Unpack context result
            let (
                dynamic_context,
                dynamic_context_status,
                dynamic_context_mode,
                dynamic_context_chars,
                dynamic_context_error,
            ) = context_result;

            yield AgentEvent::Trace {
                event: "dynamic_context_resolved".to_string(),
                payload: serde_json::json!({
                    "status": dynamic_context_status,
                    "mode": dynamic_context_mode,
                    "chars": dynamic_context_chars,
                    "error": dynamic_context_error,
                }),
            };

            // Process auto-plan result
            if let Some(plan) = &auto_plan_decision.plan {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AUTO_PLAN] Generated plan ({} chars)",
                    plan.chars().count()
                ));
            }
            yield AgentEvent::Trace {
                event: "auto_plan_decision".to_string(),
                payload: serde_json::json!({
                    "reason": auto_plan_decision.reason,
                    "generated": auto_plan_decision.plan.is_some(),
                    "history_len": auto_plan_decision.history_len,
                    "max_history": auto_plan_decision.max_history,
                    "max_chars": auto_plan_decision.max_chars,
                    "request_complexity": auto_plan_decision.request_complexity,
                    "plan_chars": auto_plan_decision.plan_chars,
                    "was_truncated": auto_plan_decision.was_truncated,
                }),
            };

            // 6. Build System Prompt (with Prompt Cache optimization)
            // Uses Anthropic-style cache_control: {"type": "ephemeral"} on static
            // system prompt parts, saving 30-50% token costs on repeated turns.
            // Gracefully degrades to plain system messages when cache is disabled.
            // Wrapped in spawn_blocking because prompt_builder makes synchronous git calls.
            let include_extended_bundle =
                crate::agent::prompt_builder::PromptBuilder::include_extended_bundle_for_history_len(history_len);
            let cwd_str = cwd.to_string_lossy().to_string();
            let selected_names = initial_tool_selection.selected_names.clone();
            let dynamic_ctx = dynamic_context;
            let complexity_hint_clone = complexity_hint;
            // Fix date string at session start to ensure deterministic serialization
            // for LLM prompt caching. The date should not change between turns.
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let cached_system_msgs = tokio::task::spawn_blocking(move || {
                crate::agent::prompt_builder::PromptBuilder::build_cached_system_messages(
                    crate::agent::prompt_builder::PromptMode::Agent,
                    &today,
                    std::env::consts::OS,
                    &cwd_str,
                    Some(&selected_names),
                    None,
                    dynamic_ctx,
                    complexity_hint_clone,
                    is_thinking_model,
                    include_extended_bundle,
                )
            }).await.unwrap_or_default();

            // 6. Execution Loop (Orchestrator Role)
            let mut messages = if self.session_messages.is_empty() {
                cached_system_msgs
            } else {
                let mut existing = self.session_messages.clone();
                // Remove all existing system messages at the beginning,
                // then prepend the fresh cached system messages.
                while existing.first().map(|m| m.role.as_str() == "system").unwrap_or(false) {
                    existing.remove(0);
                }
                let mut new_msgs = cached_system_msgs;
                new_msgs.append(&mut existing);
                new_msgs
            };
            if let Some(plan) = auto_plan_decision.plan {
                let template =
                    crate::core::prompts::loader::load_prompt("auto-plan-injection.md");
                messages.push(StarMessage::system(
                    crate::core::prompts::loader::render_template(
                        &template,
                        &[("plan", &plan)],
                    )
                ));
            }

            // Inject plan mode reminder if currently in Plan mode
            crate::agent::context::inject_plan_mode_reminder_if_needed(
                &self.approval_mode,
                &mut messages,
            );

            messages.push(StarMessage::user(user_input.clone()));

            // Create the event bridging channel and wire it on the agent.
            // Events are emitted through this channel via emit_event().
            let (event_tx, mut event_rx) =
                tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
            self.event_tx = Some(event_tx);

            // Run the main agentic loop in a background task
            let mut agent_for_loop = Agent::new(self.client.clone(), self.config.clone());
            agent_for_loop.event_tx = self.event_tx.clone();
            agent_for_loop.stream_tx = self.stream_tx.clone();
            agent_for_loop.approval_mode = self.approval_mode.clone();
            agent_for_loop.session_messages = self.session_messages.clone();
            agent_for_loop.abort_flag = self.abort_flag.clone();
            agent_for_loop.abort_token = self.abort_token.clone();
            agent_for_loop.task_complexity = self.task_complexity.clone();

            let (result_tx, result_rx) =
                tokio::sync::oneshot::channel::<(Result<Vec<StarMessage>, (Vec<StarMessage>, AgentEvent)>, Vec<StarMessage>)>();

            let user_input_clone = user_input.clone();
            let all_tools_clone = all_tools.clone();
            let all_active_tools_clone = all_active_tools.clone();
            let shortlist_profile_clone = shortlist_profile.to_string();
            let initial_tool_selection_clone = initial_tool_selection.clone();
            let mut messages_clone = messages.clone();

            tokio::spawn(async move {
                let result = agent_for_loop.run_agentic_loop(
                    &user_input_clone,
                    &mut messages_clone,
                    &all_tools_clone,
                    &all_active_tools_clone,
                    &shortlist_profile_clone,
                    history_len,
                    &initial_tool_selection_clone,
                ).await;
                // 将 messages_clone 中的非 system 消息同步到 session_messages
                // 这样下次对话时能保留完整的对话历史
                let non_system_messages: Vec<StarMessage> = messages_clone
                    .iter()
                    .filter(|m| m.role.as_str() != "system")
                    .cloned()
                    .collect();
                agent_for_loop.session_messages = non_system_messages;
                // 返回 result 和更新后的 session_messages
                let _ = result_tx.send((result, agent_for_loop.session_messages));
            });

            // Yield events in real-time as they arrive
            let mut result_rx = Some(result_rx);
            loop {
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        yield event;
                    }
                    result = async {
                        match &mut result_rx {
                            Some(rx) => rx.await.ok(),
                            None => None,
                        }
                    }, if result_rx.is_some() => {
                        // Loop completed, drain remaining events
                        while let Ok(event) = event_rx.try_recv() {
                            yield event;
                        }
                        // 更新 session_messages 以保留对话历史
                        if let Some((_, updated_session_messages)) = result {
                            self.session_messages = updated_session_messages;
                            // 持久化到磁盘，确保下次启动时可以恢复
                            self.persist_session_messages();
                        }
                        result_rx = None;
                        break;
                    }
                }
            }

            // Close the sender
            self.event_tx = None;

            // 通知 UI 处理完成
            yield AgentEvent::Done;
        })
    }

    pub async fn run(
        &mut self,
        user_input: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        crate::utils::logging::append_debug_log_line("[DEBUG] Agent::run: Starting");
        use futures::StreamExt;
        let mut stream = self.run_stream(user_input.to_string());
        crate::utils::logging::append_debug_log_line(
            "[DEBUG] Agent::run: run_stream created, polling events",
        );
        let mut final_response = String::new();

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => {
                    match event {
                        AgentEvent::Message(content) => {
                            final_response = content;
                        }
                        AgentEvent::Error(err) => {
                            return Err(err.into());
                        }
                        _ => {} // Ignore other events for legacy run
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(final_response)
    }
}
