use crate::agent::agent_core::Agent;
use crate::agent::agent_llm::LlmResult;
use crate::agent::loop_engineering::{LoopState, RecoveryManager};
use crate::agent::messaging::AgentEvent;
use crate::agent::nudges;
use crate::agent::tool_routing::{
    build_tool_selection_system_message, resolved_tool_loop_repeat_threshold,
    select_tools_for_turn_for_client, ToolSelection,
};
use crate::agent::{hooks, tool_helpers};
use crate::types::{StarMessage, StarToolCall, ToolResult};
use std::collections::{HashMap, HashSet, VecDeque};

/// 主 agentic 循环的执行结果
type LoopResult = Result<Vec<StarMessage>, (Vec<StarMessage>, AgentEvent)>;

impl Agent {
    /// 执行主 agentic 循环。
    ///
    /// 事件通过 `event_tx` 实时发送，由调用方通过 `event_rx` 接收。
    pub(crate) async fn run_agentic_loop(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        all_tools: &[crate::types::StarTool],
        all_active_tools: &HashSet<String>,
        shortlist_profile: &str,
        history_len: usize,
        initial_tool_selection: &ToolSelection,
    ) -> LoopResult {
        let mut current_turn = 0i32;
        let max_turns = {
            let t = self.config.max_session_turns();
            (if t <= 0 { 200 } else { t }) as i32
        };
        let mut semantic_search_attempted = false;
        let mut navigator_skill_attempted = false;
        let mut analyzer_skill_attempted = false;
        let mut editor_skill_attempted = false;
        let mut project_map_attempted = false;
        let skip_verification =
            crate::agent::policies::automation::should_skip_verification(user_input);
        let mut verification_required = false;
        let tool_loop_repeat_threshold = resolved_tool_loop_repeat_threshold();
        let mut tool_signature_history: VecDeque<String> = VecDeque::new();
        let mut tool_loop_interventions = 0usize;
        let mut nudge_interventions = 0usize;
        let mut consecutive_read_only_turns = 0usize;
        let mut file_read_tracker: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        // 初始化恢复管理器，从环境变量读取后备模型/provider
        let mut fallback_providers: Vec<String> = Vec::new();
        if let Ok(fb) = std::env::var("STAR_FALLBACK_MODEL") {
            fallback_providers.push(fb);
        }
        if let Ok(fb) = std::env::var("STAR_FALLBACK_BASE_URL") {
            fallback_providers.push(fb);
        }
        let mut recovery_manager = RecoveryManager::new(fallback_providers);

        // 延迟初始化 ContextEngine（首次使用时才做 I/O）
        self.lazy_init().await;
        let mut loop_state = LoopState::new(max_turns as usize);

        // Handle prefetch logic
        self.handle_prefetch(
            user_input,
            messages,
            all_active_tools,
            history_len,
            &mut project_map_attempted,
            &mut semantic_search_attempted,
        )
        .await;

        // Main loop
        while current_turn < max_turns && loop_state.should_continue() {
            current_turn += 1;
            loop_state.next_turn();

            // Abort check
            if self
                .abort_flag
                .as_ref()
                .map(|f| f.load(std::sync::atomic::Ordering::SeqCst))
                .unwrap_or(false)
            {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AGENT] Loop exited: {} (turn={})",
                    StopReason::Aborted,
                    current_turn,
                ));
                return Err((messages.clone(), AgentEvent::Done));
            }

            // 消费异步 SubAgent 完成通知（注入为 user-role 消息）
            if let Some(ref queue) = self.async_notification_queue {
                let mut q = queue.lock().await;
                for notification in q.drain_for_next_turn() {
                    messages.push(notification.to_message());
                    // 实时推送到 UI，让用户立刻看到后台子 Agent 的完成情况
                    let status_str = match notification.status {
                        crate::agent::subagent::notification::NotificationStatus::Completed => {
                            "✓ Done"
                        }
                        crate::agent::subagent::notification::NotificationStatus::Failed => {
                            "✗ Failed"
                        }
                        crate::agent::subagent::notification::NotificationStatus::Killed => {
                            "⏹ Killed"
                        }
                    };
                    let note = format!(
                        "[SubAgent {}] {}  — {}",
                        notification.task_id, status_str, notification.summary
                    );
                    self.emit_direct_chunk(crate::types::StreamingChunk::assistant_note(
                        note.clone(),
                    ));

                    // 发送 AgentTaskUpdate 让 UI 更新对应的 AgentTask 条目
                    let agent_status = match notification.status {
                        crate::agent::subagent::notification::NotificationStatus::Completed => {
                            crate::types::AgentTaskStatus::Completed
                        }
                        crate::agent::subagent::notification::NotificationStatus::Failed
                        | crate::agent::subagent::notification::NotificationStatus::Killed => {
                            crate::types::AgentTaskStatus::Failed
                        }
                    };
                    // 类型标签跟着通知走，不再硬编码 "general-purpose"
                    let agent_type = notification
                        .agent_type
                        .clone()
                        .unwrap_or_else(|| "Agent".to_string());
                    self.emit_direct_chunk(crate::types::StreamingChunk::agent_task_update(
                        crate::types::AgentTaskUpdatePayload::new(
                            &notification.task_id,
                            agent_type,
                        )
                        .with_description(&notification.summary)
                        .with_status(agent_status)
                        .with_stats(
                            notification.usage.tool_uses as u32,
                            notification.usage.total_tokens as u32,
                        )
                        .with_async(true)
                        .with_last_tool_info(Some(status_str.to_string()))
                        .with_name(notification.name.clone())
                        .with_task_description(Some(notification.summary.clone()))
                        .with_sub_entries(notification.entries.clone()),
                    ));

                    crate::utils::logging::append_debug_log_line(&format!(
                        "[SubAgent] notification surfaced to UI: {}",
                        note
                    ));
                }
            }

            // Inject loop state context if there have been failures
            if loop_state.consecutive_failures > 0 {
                let context_summary = loop_state.generate_context_summary();
                crate::utils::logging::append_debug_log_line(&format!(
                    "[LOOP_CONTEXT] turn={}, failures={}: {}",
                    current_turn, loop_state.consecutive_failures, context_summary
                ));
            }

            // Compression Check
            if let Err(reason) = self
                .run_compression_check(user_input, messages, current_turn)
                .await
            {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[WARN] Context compression failed: {} — continuing anyway",
                    reason,
                ));
            }

            // Tool Selection and LLM Call
            crate::utils::logging::append_debug_log_line(&format!(
                "[AGENT] Turn {} starting execute_turn",
                current_turn
            ));
            let turn_result = self
                .execute_turn(
                    user_input,
                    messages,
                    all_tools,
                    all_active_tools,
                    shortlist_profile,
                    current_turn,
                    &mut semantic_search_attempted,
                    &mut navigator_skill_attempted,
                    &mut analyzer_skill_attempted,
                    &mut editor_skill_attempted,
                    &mut project_map_attempted,
                    skip_verification,
                    &mut verification_required,
                    tool_loop_repeat_threshold,
                    &mut tool_signature_history,
                    &mut tool_loop_interventions,
                    &mut nudge_interventions,
                    &mut consecutive_read_only_turns,
                    &mut file_read_tracker,
                    &mut recovery_manager,
                    &mut loop_state,
                    history_len,
                )
                .await;

            crate::utils::logging::append_debug_log_line(&format!(
                "[AGENT] Turn {} completed: {:?}",
                current_turn,
                match &turn_result {
                    TurnResult::Continue => "Continue",
                    TurnResult::Done => "Done",
                    TurnResult::Error(_) => "Error",
                }
            ));

            match turn_result {
                TurnResult::Continue => {
                    continue;
                }
                TurnResult::Done => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[AGENT] Loop exited: {} (turn={})",
                        StopReason::Completed,
                        current_turn,
                    ));
                    return Ok(messages.clone());
                }
                TurnResult::Error(event) => {
                    return Err((messages.clone(), event));
                }
            }
        }

        // Max turns reached or loop stopped
        let stop_reason = if !loop_state.should_continue() {
            StopReason::LoopStateStopped
        } else {
            StopReason::MaxTurns
        };
        crate::utils::logging::append_debug_log_line(&format!(
            "[AGENT] Loop exited: {} (turn={}, consecutive_failures={})",
            stop_reason, current_turn, loop_state.consecutive_failures,
        ));
        if !loop_state.should_continue() {
            Err((
                messages.clone(),
                AgentEvent::Error(format!("Loop stopped: {}", loop_state.format_status())),
            ))
        } else {
            Err((
                messages.clone(),
                AgentEvent::Error("Max turns reached".to_string()),
            ))
        }
    }

    // 处理预取逻辑
    pub(crate) async fn handle_prefetch(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        all_active_tools: &HashSet<String>,
        history_len: usize,
        project_map_attempted: &mut bool,
        semantic_search_attempted: &mut bool,
    ) {
        use crate::agent::tool_routing::{
            build_project_map_tool_call, build_semantic_search_tool_call,
            should_prefetch_project_map, should_prefetch_semantic_search,
        };

        if should_prefetch_project_map(user_input, all_active_tools, history_len) {
            *project_map_attempted = true;
            let project_map_tool_call = build_project_map_tool_call(user_input, 0);

            crate::utils::logging::append_debug_log_line(
                "[ACE_PREFETCH] first-turn overview request detected; prefetching project_map before the first model call",
            );

            messages.push(StarMessage::assistant_with_tool_calls(vec![
                project_map_tool_call.clone(),
            ]));

            match hooks::run_pre_tool_hooks(user_input, &project_map_tool_call).await {
                Ok(()) => {
                    let result = self.execute_single_tool(&project_map_tool_call).await;
                    hooks::run_post_tool_hooks(user_input, &project_map_tool_call, &result).await;
                    messages.push(StarMessage::tool(
                        project_map_tool_call.id.clone(),
                        result
                            .output
                            .clone()
                            .unwrap_or_else(|| result.error.clone().unwrap_or_default()),
                    ));
                }
                Err(reason) => {
                    let blocked_result = ToolResult {
                        success: false,
                        output: None,
                        error: Some(reason),
                        data: None,
                    };
                    messages.push(StarMessage::tool(
                        project_map_tool_call.id.clone(),
                        blocked_result.error.unwrap_or_default(),
                    ));
                }
            }
        } else if should_prefetch_semantic_search(user_input, all_active_tools, history_len) {
            *semantic_search_attempted = true;
            let semantic_tool_call = build_semantic_search_tool_call(user_input, 0);

            crate::utils::logging::append_debug_log_line(
                "[ACE_PREFETCH] first-turn conceptual query detected; prefetching semantic_search before the first model call",
            );

            messages.push(StarMessage::assistant_with_tool_calls(vec![
                semantic_tool_call.clone(),
            ]));

            match hooks::run_pre_tool_hooks(user_input, &semantic_tool_call).await {
                Ok(()) => {
                    let result = self.execute_single_tool(&semantic_tool_call).await;
                    hooks::run_post_tool_hooks(user_input, &semantic_tool_call, &result).await;
                    messages.push(StarMessage::tool(
                        semantic_tool_call.id.clone(),
                        result
                            .output
                            .clone()
                            .unwrap_or_else(|| result.error.clone().unwrap_or_default()),
                    ));
                }
                Err(reason) => {
                    let blocked_result = ToolResult {
                        success: false,
                        output: None,
                        error: Some(reason),
                        data: None,
                    };
                    messages.push(StarMessage::tool(
                        semantic_tool_call.id.clone(),
                        blocked_result.error.unwrap_or_default(),
                    ));
                }
            }
        }
    }

    // 执行单个工具（带进度报告）
    pub(crate) async fn execute_single_tool(&self, tool_call: &StarToolCall) -> ToolResult {
        let (mut progress_rx, tool_future) = tool_helpers::execute_single_tool_with_progress(
            self.tool_executor.clone(),
            tool_call.clone(),
            self.abort_token.clone(),
        );
        tokio::pin!(tool_future);

        loop {
            tokio::select! {
                Some(_progress) = progress_rx.recv() => {
                    // Progress events are handled by the caller
                }
                result = &mut tool_future => {
                    return result;
                }
            }
        }
    }

    /// 对工具结果执行预算限制
    ///
    /// 限制单个工具结果的大小，防止上下文爆炸
    /// 注意：Read等文件查看工具不会被截断
    fn apply_tool_result_budget(&self, messages: &mut Vec<StarMessage>) {
        use crate::agent::compact::tool_output_compact::ToolResultBudget;

        let budget = ToolResultBudget::new();
        let mut changed = false;

        // 首先收集工具名称映射（tool_call_id -> tool_name）
        let mut tool_name_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        tool_name_map.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        for msg in messages.iter_mut() {
            if msg.role == "tool" {
                if let Some(content) = &msg.content {
                    // 获取工具名称
                    let tool_name = msg
                        .tool_call_id
                        .as_ref()
                        .and_then(|id| tool_name_map.get(id))
                        .map(|s| s.as_str());

                    let original_len = content.len();
                    let enforced = budget.enforce(content, tool_name);
                    if enforced.len() < original_len {
                        msg.content = Some(enforced);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            crate::utils::logging::append_debug_log_line(
                "[COMPACT] Applied tool result budget to messages",
            );
        }
    }

    /// 释放前一轮次的工具结果原始数据
    ///
    /// 防止内存无限增长，只保留API需要的内容
    /// 注意：豁免 Read 等文件查看工具，避免 LLM 看不到完整内容导致反复重读
    fn release_stale_tool_results(&self, messages: &mut Vec<StarMessage>) {
        use crate::agent::compact::tool_output_compact::EXEMPT_TOOLS;

        let mut released_count = 0;

        // 首先收集工具名称映射（tool_call_id -> tool_name）
        let mut tool_name_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        tool_name_map.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        for msg in messages.iter_mut() {
            // 只处理工具消息（role="tool"）
            if msg.role == "tool" {
                // 检查工具是否豁免
                let is_exempt = msg
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| tool_name_map.get(id))
                    .map(|tool_name| EXEMPT_TOOLS.iter().any(|e| e == tool_name))
                    .unwrap_or(false);

                // 豁免工具不截断
                if is_exempt {
                    continue;
                }

                if let Some(content) = &msg.content {
                    // 如果工具结果包含大量数据，释放原始输出
                    if content.len() > 10_000 {
                        // 超过10K字符
                        // 保留摘要，释放原始数据
                        // 追加显式截断标记，避免 agent 误以为 200 字符摘要就是完整输出，
                        // 需要时可重新读取相关文件/重新搜索。
                        let trunc_marker = format!(
                            "; [TRUNCATED: 完整输出 {} 字符，仅保留开头 200 字符。如需完整内容请重新读取/搜索。]",
                            content.len()
                        );
                        let summary = if content.len() > 200 {
                            format!("{}...{}", &content[..200], trunc_marker)
                        } else {
                            content.clone()
                        };

                        msg.content = Some(summary);
                        released_count += 1;
                    }
                }
            }
        }

        if released_count > 0 {
            crate::utils::logging::append_debug_log_line(&format!(
                "[MEMORY] Released {} stale tool results to prevent memory growth",
                released_count
            ));
        }
    }

    // 运行压缩检查
    pub(crate) async fn run_compression_check(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        current_turn: i32,
    ) -> Result<(), String> {
        if let Err(reason) = hooks::run_stage_hooks(
            user_input,
            crate::core::hooks::store::ManagedHookEvent::PreCompact,
            "before_auto_compact",
            Some(serde_json::json!({
                "trigger": "auto",
                "turn": current_turn,
                "message_count": messages.len(),
            })),
        )
        .await
        {
            return Err(reason);
        }

        // 0. 释放前一轮次的工具结果原始数据（防止内存无限增长）
        self.release_stale_tool_results(messages);

        // 1. 先执行工具结果预算限制
        self.apply_tool_result_budget(messages);

        // 2. 执行预测性压缩（估算当前轮次增长是否会超过阈值）
        if let Some(predictive_result) = self.compact_manager.predictive_compact(messages) {
            if predictive_result.was_compacted {
                *messages = predictive_result.messages;
                messages.push(StarMessage::system(format!(
                    "Context was preemptively compressed using '{}' based on predicted growth. \
                     Original: {} tokens → Now: {} tokens. \
                     Continue the task based on the summarized context above.",
                    predictive_result.strategy_name,
                    predictive_result.original_token_count,
                    predictive_result.new_token_count,
                )));

                crate::utils::logging::append_debug_log_line(&format!(
                    "[COMPACT] Predictive compression: {} → {} tokens (strategy={})",
                    predictive_result.original_token_count,
                    predictive_result.new_token_count,
                    predictive_result.strategy_name,
                ));

                return Ok(());
            }
        }

        // 3. 使用新的 CompactManager 进行常规压缩
        let compact_result = self.compact_manager.compact(messages);

        // 如果 CompactManager 没有执行压缩，使用原有的 ContextCompressor 作为后备
        let compression_result = if compact_result.was_compacted {
            crate::agent::workflows::context_compression::CompressionResult {
                messages: compact_result.messages,
                was_compacted: true,
                original_token_count: compact_result.original_token_count,
                new_token_count: compact_result.new_token_count,
                threshold_tokens: self.compact_manager.config().max_tokens,
                decision: Box::leak(compact_result.strategy_name.into_boxed_str()),
            }
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(60),
                self.context_compressor
                    .compress_if_needed(messages.clone(), Some(&self.client)),
            )
            .await
            .unwrap_or_else(|_| {
                crate::utils::logging::append_debug_log_line(
                    "[WARN] Context compression timed out, skipping",
                );
                Ok(
                    crate::agent::workflows::context_compression::CompressionResult {
                        messages: messages.clone(),
                        was_compacted: false,
                        original_token_count: 0,
                        new_token_count: 0,
                        threshold_tokens: 0,
                        decision: "timeout_skip",
                    },
                )
            })
            .map_err(|e: Box<dyn std::error::Error + Send + Sync>| e.to_string())?
        };

        if compression_result.was_compacted {
            *messages = compression_result.messages;

            // Insert a natural language message about compression (no system marker prefix)
            messages.push(StarMessage::system(format!(
                "Context was compressed using '{}' to fit within the token budget. \
                 Previous messages were summarized. \
                 Original: {} tokens → Now: {} tokens. \
                 Continue the task based on the summarized context above.",
                compression_result.decision,
                compression_result.original_token_count,
                compression_result.new_token_count,
            )));

            crate::utils::logging::append_debug_log_line(&format!(
                "[COMPACT] Context compressed: {} → {} tokens (strategy={}, threshold={})",
                compression_result.original_token_count,
                compression_result.new_token_count,
                compression_result.decision,
                compression_result.threshold_tokens,
            ));

            // Emit ContextUpdated event
            if let Some(bus) = self.runtime_message_bus() {
                let _ = bus.publish(crate::core::confirmation_bus::types::Message::ContextUpdated(
                    crate::core::confirmation_bus::types::ContextUpdated {
                        message_type: crate::core::confirmation_bus::types::MessageBusType::ContextUpdated,
                        new_token_count: compression_result.new_token_count,
                        messages_count: messages.len(),
                    }
                )).await;
            }
        }

        Ok(())
    }

    // 执行单轮对话
    pub(crate) async fn execute_turn(
        &mut self,
        user_input: &str,
        messages: &mut Vec<StarMessage>,
        all_tools: &[crate::types::StarTool],
        all_active_tools: &HashSet<String>,
        shortlist_profile: &str,
        current_turn: i32,
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
        recovery_manager: &mut RecoveryManager,
        loop_state: &mut LoopState,
        history_len: usize,
    ) -> TurnResult {
        // Tool Selection
        if let Err(reason) = hooks::run_stage_hooks(
            user_input,
            crate::core::hooks::store::ManagedHookEvent::BeforeToolSelection,
            "before_tool_selection",
            Some(serde_json::json!({
                "turn": current_turn,
                "total_tools": all_tools.len(),
            })),
        )
        .await
        {
            return TurnResult::Error(AgentEvent::Error(reason));
        }

        let turn_tool_selection =
            select_tools_for_turn_for_client(&self.client, all_tools, user_input, current_turn);

        if !turn_tool_selection.tools.is_empty()
            && crate::core::config::models::is_deepseek_reasoner_model(
                self.client.get_current_model(),
            )
        {
            let reason = "DeepSeek `deepseek-reasoner` does not support function calling according to the official API docs. This agent workflow requires tools, so please switch to a tool-capable model such as `deepseek-chat`/`deepseek-v4-pro` with thinking mode enabled.".to_string();
            return TurnResult::Error(AgentEvent::Error(reason));
        }

        let turn_active_tools = turn_tool_selection.selected_names.clone();
        crate::utils::logging::append_debug_log_line(&format!(
            "[TOOL_ROUTER] turn={} profile={} shortlist={}/{} rationale={}",
            current_turn,
            shortlist_profile,
            turn_tool_selection.tools.len(),
            turn_tool_selection.total_tools,
            turn_tool_selection.rationale
        ));

        // Prepare request messages
        // Clone the messages and append tool selection system message.
        // The repair and normalize steps are applied to ensure consistency,
        // but we keep the original messages untouched for cache prefix stability.
        let mut request_messages = messages.clone();
        // build_tool_selection_system_message 目前返回空串；空 system 消息会白占一条
        // 消息槽并干扰前缀缓存，因此只在非空时追加。
        let tool_selection_note =
            build_tool_selection_system_message(&turn_tool_selection, current_turn);
        if !tool_selection_note.trim().is_empty() {
            request_messages.push(StarMessage::system(tool_selection_note));
        }
        // Only repair tool sequence if there are tool messages (avoid unnecessary mutation)
        if crate::agent::message_processing::tool_sequence_repair_needed(&request_messages) {
            crate::agent::message_processing::repair_tool_message_sequence(&mut request_messages);
        }
        crate::agent::message_processing::normalize_messages_for_llm(
            &mut request_messages,
            self.client.supports_thinking(),
        );

        // Before model hook
        if let Err(reason) = hooks::run_stage_hooks(
            user_input,
            crate::core::hooks::store::ManagedHookEvent::BeforeModel,
            "before_model",
            Some(serde_json::json!({
                "turn": current_turn,
                "selected_tools": turn_tool_selection.selected_names.iter().cloned().collect::<Vec<_>>(),
                "shortlist_size": turn_tool_selection.tools.len(),
            })),
        )
        .await
        {
            return TurnResult::Error(AgentEvent::Error(reason));
        }

        // Make LLM call
        crate::utils::logging::append_debug_log_line(&format!(
            "[AGENT] Turn {} calling LLM ({} messages, {} tools)",
            current_turn,
            request_messages.len(),
            turn_tool_selection.tools.len()
        ));
        let llm_result = self
            .call_llm(
                user_input,
                messages,
                &mut request_messages,
                &turn_tool_selection,
                current_turn,
                recovery_manager,
                loop_state,
            )
            .await;

        crate::utils::logging::append_debug_log_line(&format!(
            "[AGENT] Turn {} LLM call completed: {}",
            current_turn,
            match &llm_result {
                LlmResult::Success { tool_calls, .. } =>
                    format!("Success ({} tool_calls)", tool_calls.len()),
                LlmResult::Error(_) => "Error".to_string(),
                LlmResult::Retry => "Retry".to_string(),
            }
        ));

        match llm_result {
            LlmResult::Success {
                content,
                reasoning,
                tool_calls,
                usage,
                content_streamed,
                reasoning_streamed,
                was_truncated,
            } => {
                // Store streaming state so agent_run.rs can avoid duplicate emission
                self.last_content_streamed = content_streamed;
                self.last_reasoning_streamed = reasoning_streamed;

                // Record token usage for budget tracking
                if let Some(ref usage) = usage {
                    self.token_budget_tracker
                        .record_output_tokens(usage.completion_tokens as usize);
                }

                // Check token budget for auto-continuation
                let budget_decision = self.token_budget_tracker.should_continue();
                match &budget_decision {
                    crate::agent::token_budget::TokenBudgetDecision::Continue {
                        nudge_message,
                        continuation_count,
                        pct,
                        turn_tokens,
                        budget,
                    } => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[TOKEN_BUDGET] Continuation #{}: {}% ({} / {} tokens)",
                            continuation_count, pct, turn_tokens, budget
                        ));
                        // Inject continuation nudge
                        messages.push(StarMessage::user(nudge_message.clone()));
                        self.token_budget_tracker.increment_continuation();
                    }
                    crate::agent::token_budget::TokenBudgetDecision::Stop { reason } => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[TOKEN_BUDGET] Stop: {}",
                            reason
                        ));
                    }
                }

                // Truncation recovery: model hit max_tokens before finishing.
                // Inject a continuation nudge so the model picks up where it left off.
                if was_truncated {
                    crate::utils::logging::append_debug_log_line(
                        "[AGENT] response was truncated (finish_reason=length) — injecting continuation nudge",
                    );
                    messages.push(StarMessage::system(
                        nudges::NUDGE_TRUNCATED_STREAM.to_string(),
                    ));
                }

                // Process the response
                let result = self
                    .process_llm_response(
                        user_input,
                        messages,
                        content,
                        reasoning,
                        tool_calls,
                        usage,
                        content_streamed,
                        reasoning_streamed,
                        current_turn,
                        all_active_tools,
                        semantic_search_attempted,
                        navigator_skill_attempted,
                        analyzer_skill_attempted,
                        editor_skill_attempted,
                        project_map_attempted,
                        skip_verification,
                        verification_required,
                        tool_loop_repeat_threshold,
                        tool_signature_history,
                        tool_loop_interventions,
                        nudge_interventions,
                        consecutive_read_only_turns,
                        file_read_tracker,
                        &turn_active_tools,
                        loop_state,
                    )
                    .await;

                // If token budget says continue, override Done with Continue
                if matches!(
                    budget_decision,
                    crate::agent::token_budget::TokenBudgetDecision::Continue { .. }
                ) && matches!(result, TurnResult::Done)
                {
                    TurnResult::Continue
                } else {
                    result
                }
            }
            LlmResult::Error(event) => TurnResult::Error(event),
            LlmResult::Retry => TurnResult::Continue,
        }
    }
}

/// 单轮对话的结果
pub(crate) enum TurnResult {
    Continue,
    Done,
    Error(AgentEvent),
}

/// Agent 停止原因 — 对齐 Claude Code 的 11 种终止条件
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// AI 完成了任务（无 tool_use，且内容完整）
    Completed,
    /// 达到最大轮次限制
    MaxTurns,
    /// 用户中断（Ctrl+C）
    Aborted,
    /// 上下文窗口超限
    PromptTooLong,
    /// 连续失败次数超限
    ConsecutiveFailures,
    /// Nudge 次数超限（空响应/短文本循环）
    NudgeLimitReached,
    /// 工具循环检测
    ToolLoopDetected,
    /// 流式错误且恢复失败
    StreamingError,
    /// 循环状态指示停止
    LoopStateStopped,
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Completed => write!(f, "Task completed"),
            StopReason::MaxTurns => write!(f, "Max turns reached"),
            StopReason::Aborted => write!(f, "Aborted by user"),
            StopReason::PromptTooLong => write!(f, "Context window exceeded"),
            StopReason::ConsecutiveFailures => write!(f, "Too many consecutive failures"),
            StopReason::NudgeLimitReached => write!(f, "Nudge limit reached (no progress)"),
            StopReason::ToolLoopDetected => write!(f, "Tool loop detected"),
            StopReason::StreamingError => write!(f, "Streaming error"),
            StopReason::LoopStateStopped => write!(f, "Loop state stopped"),
        }
    }
}
