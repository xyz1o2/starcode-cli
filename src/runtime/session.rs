use crate::agent::messaging::AsyncMessageQueue;
use crate::agent::StarAgent;
use crate::core::confirmation_bus::types::{Message, MessageBusType, ToolConfirmationResponse};
use crate::runtime::messages::{AgentRequest, PendingCheckpointAction, StreamMessage};
use crate::utils::logging::append_debug_log_line;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Notify;

#[derive(Default)]
pub struct DeferredRuntimeActions {
    pub deferred_model: Option<(String, Option<String>)>,
    /// `Some(force)` = 流式期间来过 ListModels，回合结束后补上（force 语义见
    /// `AgentRequest::ListModels`）；多次请求里只要有一次 force 就按 force 算。
    pub pending_models_request: Option<bool>,
    pub pending_plugin_tools_refresh: bool,
    pub pending_mcp_refresh: bool,
    pub pending_mcp_list_servers: bool,
    pub pending_mcp_list_tools: Option<String>,
    pub pending_toggle_yolo: bool,
    pub pending_set_approval_mode: Option<crate::types::ApprovalMode>,
    pub pending_set_thinking_effort: Option<crate::types::ThinkingEffort>,
    pub pending_reset_session: bool,
    pub pending_tool_confirmation: Option<(Vec<crate::types::StarToolCall>, u64, bool, bool)>,
    pub pending_checkpoint_action: Option<PendingCheckpointAction>,
    pub pending_update_provider_config: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
        Option<String>,
    )>,
    pub pending_compress_request: Option<u64>,
    pub pending_generate_note: Option<(
        crate::runtime::messages::NoteKind,
        u64,
        Option<String>,
    )>,
    pub pending_mark_as_read: Vec<String>,
    /// 流式过程中收到的 `!command` 输出：等这一轮结束再追加进上下文，
    /// 免得在 agent 正读 session_messages 的时候插队改它。
    pub pending_context_appends: Vec<String>,
}

pub enum StreamingRequestOutcome {
    Continue,
    Break,
    Return,
}

pub struct StreamingRequestContext<'a> {
    pub tx: &'a mpsc::Sender<StreamMessage>,
    pub message_id: u64,
    pub user_message: &'a str,
    pub current_model_snapshot: &'a str,
    pub project_root: Option<&'a Path>,
    pub abort_flag: &'a std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub steering_queue: &'a Arc<AsyncMessageQueue<(u64, String)>>,
    pub steering_signal: &'a Arc<Notify>,
    pub message_bus: &'a Arc<crate::core::confirmation_bus::MessageBus>,
}

pub async fn handle_streaming_request(
    deferred: &mut DeferredRuntimeActions,
    request: Option<AgentRequest>,
    context: StreamingRequestContext<'_>,
) -> StreamingRequestOutcome {
    match request {
        Some(AgentRequest::Abort) => {
            context.abort_flag.store(true, Ordering::SeqCst);
            crate::runtime::hooks::run_stop_hooks(
                context.project_root,
                context.user_message,
                "user_abort",
            )
            .await;
            let _ = context
                .tx
                .send(StreamMessage::AssistantNote {
                    message_id: context.message_id,
                    content: "Status: operation cancelled by user (ESC)".to_string(),
                })
                .await;
            crate::runtime::hooks::emit_notification_hook(
                "Status: operation cancelled by user (ESC)",
                "user_abort",
            )
            .await;
            let _ = context
                .tx
                .send(StreamMessage::Done {
                    message_id: context.message_id,
                })
                .await;
            StreamingRequestOutcome::Break
        }
        Some(AgentRequest::LoadConfiguredProviders) => {
            let store = crate::core::config::provider_store::ProviderStore::new();
            if let Ok(config) = store.load().await {
                let ids = store.configured_provider_ids().await.unwrap_or_default();
                let _ = context
                    .tx
                    .send(StreamMessage::ConfiguredProviders(ids))
                    .await;

                let current_provider_id = config.active_provider_id;
                let _ = context
                    .tx
                    .send(StreamMessage::CurrentModelChanged {
                        model: context.current_model_snapshot.to_string(),
                        provider_id: current_provider_id,
                    })
                    .await;
            }
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetModel { model, provider_id })
        | Some(AgentRequest::UpdateModel { model, provider_id }) => {
            deferred.deferred_model = Some((model, provider_id));
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::ListModels { force }) => {
            let pending = deferred.pending_models_request.unwrap_or(false);
            deferred.pending_models_request = Some(pending || force);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::PluginToolsRefresh) => {
            deferred.pending_plugin_tools_refresh = true;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::McpRefresh) => {
            deferred.pending_mcp_refresh = true;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::McpListServers) => {
            deferred.pending_mcp_list_servers = true;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::McpListTools { server }) => {
            deferred.pending_mcp_list_tools = Some(server);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::UpdateProviderConfig {
            provider_id,
            api_key,
            base_url,
            is_openai_compatible,
            model,
        }) => {
            let key_preview = api_key.as_ref().map(|k| {
                if k.len() > 8 {
                    format!("{}...{}", &k[..4], &k[k.len() - 4..])
                } else if k == "API_KEY_NOT_SET" {
                    "API_KEY_NOT_SET".to_string()
                } else {
                    "***".to_string()
                }
            });
            append_debug_log_line(&format!(
                "[Worker/Streaming] Deferred UpdateProviderConfig: api_key={:?}, model={:?}",
                key_preview, model
            ));
            deferred.pending_update_provider_config =
                Some((provider_id, api_key, base_url, is_openai_compatible, model));
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::UpdateGitStatus(status)) => {
            let _ = context
                .tx
                .send(StreamMessage::UpdateGitStatus(status))
                .await;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SendMessage {
            message_id,
            message,
        }) => {
            let _ = context.steering_queue.enqueue((message_id, message));
            context.steering_signal.notify_one();
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::MarkFilesAsRead(paths)) => {
            deferred.pending_mark_as_read.extend(paths);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::AppendContext { content }) => {
            deferred.pending_context_appends.push(content);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::ToggleYoloMode) => {
            deferred.pending_toggle_yolo = true;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::Compress { message_id }) => {
            deferred.pending_compress_request = Some(message_id);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::GenerateNote {
            kind,
            message_id,
            question,
        }) => {
            deferred.pending_generate_note = Some((kind, message_id, question));
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::ResetSession) => {
            deferred.pending_reset_session = true;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetApprovalMode(mode)) => {
            deferred.pending_set_approval_mode = Some(mode);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetThinkingEffort(effort)) => {
            // 本轮跑完再生效。中途打开思考会让接下来的一次请求带上
            // thinking 参数，而历史里已经有不含 thinking block 的
            // assistant 轮次 —— Anthropic 会因此报错。
            deferred.pending_set_thinking_effort = Some(effort);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::ListCheckpoints { message_id }) => {
            let _ = context
                .tx
                .send(StreamMessage::Done {
                    message_id: context.message_id,
                })
                .await;
            deferred.pending_checkpoint_action = Some(PendingCheckpointAction::List { message_id });
            StreamingRequestOutcome::Break
        }
        Some(AgentRequest::RestoreCheckpoint { message_id, id }) => {
            let _ = context
                .tx
                .send(StreamMessage::Done {
                    message_id: context.message_id,
                })
                .await;
            deferred.pending_checkpoint_action =
                Some(PendingCheckpointAction::Restore { message_id, id });
            StreamingRequestOutcome::Break
        }
        Some(AgentRequest::ToolConfirmationResponse {
            tool_calls,
            message_id,
            approved,
            always_allow,
        }) => {
            deferred.pending_tool_confirmation =
                Some((tool_calls, message_id, approved, always_allow));
            StreamingRequestOutcome::Break
        }
        Some(AgentRequest::ConfirmTool {
            tool_call_id,
            outcome,
        }) => {
            let confirmed = matches!(
                outcome,
                crate::types::ToolConfirmationOutcome::ProceedOnce
                    | crate::types::ToolConfirmationOutcome::ProceedAlways
                    | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave
                    | crate::types::ToolConfirmationOutcome::AllowSession
                    | crate::types::ToolConfirmationOutcome::UserAnswer { .. }
            );

            let msg = Message::ToolConfirmationResponse(ToolConfirmationResponse {
                message_type: MessageBusType::ToolConfirmationResponse,
                correlation_id: tool_call_id,
                confirmed,
                outcome: Some(outcome),
                requires_user_confirmation: None,
            });
            let _ = context.message_bus.publish(msg).await;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::EmitStatus(status)) => {
            let _ = context
                .tx
                .send(StreamMessage::StatusUpdate {
                    message_id: context.message_id,
                    status,
                })
                .await;
            StreamingRequestOutcome::Continue
        }
        None => StreamingRequestOutcome::Return,
        Some(AgentRequest::ResumeSession(_session_id)) => {
            // Session resume is handled by the control request handler
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::PluginOp { project_root, op }) => {
            // 插件市场后台操作不依赖 agent：即使正在流式回复中也直接执行
            crate::runtime::control_requests::spawn_plugin_op(&context.tx, project_root, op);
            StreamingRequestOutcome::Continue
        }
    }
}

pub async fn apply_deferred_runtime_actions(
    agent: &mut StarAgent,
    deferred: &mut DeferredRuntimeActions,
    tx: &mpsc::Sender<StreamMessage>,
    project_root: Option<&Path>,
) {
    if let Some((provider_id, api_key, base_url, is_openai_compatible, model)) =
        deferred.pending_update_provider_config.take()
    {
        agent.update_provider_config(provider_id, api_key, base_url, is_openai_compatible, model);
    }

    if deferred.pending_plugin_tools_refresh {
        deferred.pending_plugin_tools_refresh = false;
        agent.refresh_plugin_tools().await;
    }

    if deferred.pending_mcp_refresh {
        deferred.pending_mcp_refresh = false;
        let res = agent.initialize_mcp().await;
        let ready = agent.is_mcp_ready();
        let _ = tx
            .send(StreamMessage::McpStatus {
                ready,
                error: res.err().map(|e| e.to_string()),
            })
            .await;
    }

    if deferred.pending_mcp_list_servers {
        deferred.pending_mcp_list_servers = false;
        let servers = agent.mcp_list_servers().await;
        let _ = tx.send(StreamMessage::McpServers(servers)).await;
    }

    if let Some(server) = deferred.pending_mcp_list_tools.take() {
        match agent.mcp_list_tools(&server).await {
            Ok(tools) => {
                let _ = tx.send(StreamMessage::McpTools { server, tools }).await;
            }
            Err(e) => {
                let _ = tx
                    .send(StreamMessage::McpStatus {
                        ready: agent.is_mcp_ready(),
                        error: Some(e.to_string()),
                    })
                    .await;
            }
        }
    }

    if let Some((model, provider_id)) = deferred.deferred_model.take() {
        agent
            .set_model_with_provider(&model, provider_id.as_deref())
            .await;
    }

    if let Some(force) = deferred.pending_models_request.take() {
        match agent.list_models_cached(force).await {
            Ok(result) => {
                let _ = tx
                    .send(StreamMessage::ModelsList {
                        models: result.models,
                        cache_age_secs: result.cache_age_secs,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(StreamMessage::ModelsError(e)).await;
            }
        }
    }

    if !deferred.pending_mark_as_read.is_empty() {
        for path in deferred.pending_mark_as_read.drain(..) {
            agent.mark_file_as_read(&path).await;
        }
    }

    if !deferred.pending_context_appends.is_empty() {
        for content in deferred.pending_context_appends.drain(..) {
            agent.append_session_context(content);
        }
    }

    if let Some(message_id) = deferred.pending_compress_request.take() {
        let _ = tx.send(StreamMessage::Start { message_id }).await;
        let pre_compact_summary = crate::runtime::hooks::run_pre_compact_hooks(project_root).await;
        for note in pre_compact_summary.assistant_notes {
            let _ = tx
                .send(StreamMessage::AssistantNote {
                    message_id,
                    content: note,
                })
                .await;
        }

        if !pre_compact_summary.blocking_failures.is_empty() {
            let _ = tx
                .send(StreamMessage::Error {
                    message_id,
                    error: format!(
                        "Compression blocked due to failing PreCompact blocking hooks:\n- {}",
                        pre_compact_summary.blocking_failures.join("\n- ")
                    ),
                })
                .await;
            let _ = tx.send(StreamMessage::Done { message_id }).await;
        } else {
            match agent.compress_context().await {
                Ok(msg) => {
                    let _ = tx
                        .send(StreamMessage::Content {
                            message_id,
                            content: msg,
                        })
                        .await;
                    let _ = tx.send(StreamMessage::Done { message_id }).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(StreamMessage::Error {
                            message_id,
                            error: e.to_string(),
                        })
                        .await;
                    let _ = tx.send(StreamMessage::Done { message_id }).await;
                }
            }
        }
    }

    if let Some((kind, message_id, question)) = deferred.pending_generate_note.take() {
        let content = match agent.generate_note(kind, question).await {
            Ok(text) => text,
            Err(e) => format!("⚠️ {}", e),
        };
        let _ = tx
            .send(StreamMessage::NoteGenerated {
                message_id,
                kind,
                content,
            })
            .await;
    }

    if deferred.pending_toggle_yolo {
        deferred.pending_toggle_yolo = false;
        let new_mode = agent.toggle_yolo_mode();
        let _ = tx
            .send(StreamMessage::ApprovalModeChanged { mode: new_mode })
            .await;
    }

    if let Some(mode) = deferred.pending_set_approval_mode.take() {
        agent.set_approval_mode(mode.clone());
        let _ = tx.send(StreamMessage::ApprovalModeChanged { mode }).await;
    }

    if let Some(effort) = deferred.pending_set_thinking_effort.take() {
        crate::llm::thinking::set_session_effort(&effort);
        crate::utils::logging::append_agent_log_line(&format!(
            "[AgentRuntime] thinking effort -> {} (deferred)",
            effort.as_str()
        ));
    }

    if deferred.pending_reset_session {
        deferred.pending_reset_session = false;
        agent.clear_session_context();
    }

    if let Some((tool_calls, message_id, approved, always_allow)) =
        deferred.pending_tool_confirmation.take()
    {
        if approved || always_allow {
            let _ = tx.send(StreamMessage::Start { message_id }).await;

            for tool_call in tool_calls {
                let _ = tx
                    .send(StreamMessage::ToolCalls {
                        message_id,
                        tool_calls: vec![tool_call.clone()],
                    })
                    .await;

                match agent.execute_tool(&tool_call).await {
                    Ok(result) => {
                        agent.append_tool_result_message(&tool_call, &result);
                        let _ = tx
                            .send(StreamMessage::ToolResult {
                                message_id,
                                tool_call: tool_call.clone(),
                                tool_result: result,
                            })
                            .await;
                    }
                    Err(e) => {
                        let result = crate::types::ToolResult {
                            success: false,
                            output: None,
                            error: Some(e.to_string()),
                            data: None,
                        };
                        agent.append_tool_result_message(&tool_call, &result);
                        let _ = tx
                            .send(StreamMessage::ToolResult {
                                message_id,
                                tool_call: tool_call.clone(),
                                tool_result: result,
                            })
                            .await;
                    }
                }

                // After tool execution, check if mode was changed by on_confirm callback.
                // The on_confirm callback sets mode via tokio::spawn, so we wait briefly
                // and then broadcast the current state to UI.
                if tool_call.function.name == "enter_plan_mode"
                    || tool_call.function.name == "exit_plan_mode"
                {
                    // Give the async on_confirm callback time to execute
                    // Use multiple yields to ensure the spawned task completes
                    for _ in 0..10 {
                        tokio::task::yield_now().await;
                    }
                    // Small delay to ensure spawned task completes
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let current_mode = agent.get_approval_mode();
                    let _ = tx
                        .send(StreamMessage::ApprovalModeChanged { mode: current_mode })
                        .await;
                }
            }

            let _ = tx.send(StreamMessage::Done { message_id }).await;
        } else {
            let _ = tx
                .send(StreamMessage::AssistantNote {
                    message_id,
                    content: "Tool execution cancelled.".to_string(),
                })
                .await;
            let _ = tx.send(StreamMessage::Done { message_id }).await;
        }
    }
}
