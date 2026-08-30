use crate::agent::StarAgent;
use crate::runtime::messages::{AgentRequest, PendingCheckpointAction, StreamMessage};
use crate::utils::logging::append_debug_log_line;
use tokio::sync::mpsc;

pub async fn handle_request(
    agent: &mut StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    request: AgentRequest,
) -> Option<PendingCheckpointAction> {
    match request {
        AgentRequest::UpdateGitStatus(status) => {
            let _ = tx.send(StreamMessage::UpdateGitStatus(status)).await;
        }
        AgentRequest::Abort => {
            agent.abort();
        }
        AgentRequest::SetModel { model, provider_id } => {
            agent
                .set_model_with_provider(&model, provider_id.as_deref())
                .await;
            // Single load+save instead of two separate cycles
            if let Some(pid) = &provider_id {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_provider_and_model(pid, &model).await;
            } else {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_model(&model).await;
            }
        }
        AgentRequest::UpdateModel { model, provider_id } => {
            agent
                .set_model_with_provider(&model, provider_id.as_deref())
                .await;
            if let Some(pid) = &provider_id {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_provider_and_model(pid, &model).await;
            } else {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_model(&model).await;
            }
        }
        AgentRequest::ListCheckpoints { message_id } => {
            return Some(PendingCheckpointAction::List { message_id });
        }
        AgentRequest::RestoreCheckpoint { message_id, id } => {
            return Some(PendingCheckpointAction::Restore { message_id, id });
        }
        AgentRequest::ListModels => {
            append_debug_log_line("[DEBUG] Worker: Handling ListModels request");
            match agent.list_models().await {
                Ok(models) => {
                    append_debug_log_line(&format!(
                        "[DEBUG] Worker: ListModels success, count={}",
                        models.len()
                    ));
                    let _ = tx.send(StreamMessage::ModelsList(models)).await;
                }
                Err(e) => {
                    append_debug_log_line(&format!("[DEBUG] Worker: ListModels failed: {}", e));
                    let _ = tx.send(StreamMessage::ModelsError(e)).await;
                }
            }
        }
        AgentRequest::PluginToolsRefresh => {
            agent.refresh_plugin_tools().await;
        }
        AgentRequest::McpRefresh => {
            let res = agent.initialize_mcp().await;
            let ready = agent.is_mcp_ready();
            let _ = tx
                .send(StreamMessage::McpStatus {
                    ready,
                    error: res.err().map(|e| e.to_string()),
                })
                .await;
        }
        AgentRequest::McpListServers => {
            let servers = agent.mcp_list_servers().await;
            let _ = tx.send(StreamMessage::McpServers(servers)).await;
        }
        AgentRequest::McpListTools { server } => match agent.mcp_list_tools(&server).await {
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
        },
        AgentRequest::UpdateProviderConfig {
            provider_id,
            api_key,
            base_url,
            is_openai_compatible,
            model,
        } => {
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
                "[Worker] UpdateProviderConfig: provider={:?}, api_key={:?}, base_url={:?}, openai_compatible={:?}, model={:?}",
                provider_id, key_preview, base_url, is_openai_compatible, model
            ));
            agent.update_provider_config(provider_id, api_key, base_url, is_openai_compatible, model);
        }
        AgentRequest::MarkFilesAsRead(paths) => {
            for path in paths {
                agent.mark_file_as_read(&path).await;
            }
        }
        AgentRequest::SendMessage { .. } => {
            // Handled in worker main loop
        }
        AgentRequest::ToggleYoloMode => {
            let new_mode = agent.toggle_yolo_mode();
            let _ = tx
                .send(StreamMessage::ApprovalModeChanged { mode: new_mode })
                .await;
        }
        AgentRequest::SetApprovalMode(mode) => {
            agent.set_approval_mode(mode.clone());
            let _ = tx.send(StreamMessage::ApprovalModeChanged { mode }).await;
        }
        AgentRequest::Compress { message_id } => {
            let _ = tx.send(StreamMessage::Start { message_id }).await;
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
                }
            }
        }
        AgentRequest::ResetSession => {
            agent.clear_session_context();
        }
        AgentRequest::LoadConfiguredProviders => {
            let store = crate::core::config::provider_store::ProviderStore::new();
            if let Ok(config) = store.load().await {
                let ids = store.configured_provider_ids().await.unwrap_or_default();
                let _ = tx.send(StreamMessage::ConfiguredProviders(ids)).await;

                let startup_model = config
                    .active_provider_id
                    .as_deref()
                    .and_then(|provider_id| {
                        config
                            .providers
                            .get(provider_id)
                            .and_then(|provider| provider.selected_model.clone())
                    })
                    .or_else(|| config.active_model.clone());
                if let Some(startup_model_name) = startup_model.as_deref() {
                    let active_pid = config.active_provider_id.as_deref();
                    append_debug_log_line(&format!(
                        "[Worker] Restoring active model: {} (provider: {:?})",
                        startup_model_name, active_pid
                    ));
                    agent
                        .set_model_with_provider(startup_model_name, active_pid)
                        .await;
                }

                let current_model = startup_model.unwrap_or_default();
                let current_provider_id = config.active_provider_id.clone();
                let _ = tx
                    .send(StreamMessage::CurrentModelChanged {
                        model: current_model,
                        provider_id: current_provider_id,
                    })
                    .await;
            }
        }
        AgentRequest::ToolConfirmationResponse {
            tool_calls,
            message_id,
            approved,
            always_allow,
        } => {
            handle_tool_confirmation_response(
                agent,
                tx,
                tool_calls,
                message_id,
                approved,
                always_allow,
            )
            .await;
        }
        AgentRequest::ConfirmTool {
            tool_call_id,
            outcome,
        } => {
            handle_confirm_tool(agent, tx, tool_call_id, outcome).await;
        }
        AgentRequest::EmitStatus(status) => {
            let _ = tx
                .send(StreamMessage::StatusUpdate {
                    message_id: 0,
                    status,
                })
                .await;
        }
        AgentRequest::ResumeSession(_session_id) => {
            // Session resume is handled by the streaming request handler
        }
    }
    None
}

async fn handle_tool_confirmation_response(
    agent: &mut StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    tool_calls: Vec<crate::types::StarToolCall>,
    message_id: u64,
    approved: bool,
    always_allow: bool,
) {
    let verbose_logging = true;
    if verbose_logging {
        append_debug_log_line(&format!(
            "[DEBUG] Worker: received confirmation response - approved={}, always_allow={}, tool_calls={}",
            approved,
            always_allow,
            tool_calls.len()
        ));
    }

    if approved || always_allow {
        if verbose_logging {
            append_debug_log_line("[DEBUG] Worker: starting tool execution");
        }
        let _ = tx.send(StreamMessage::Start { message_id }).await;

        for (tool_idx, tool_call) in tool_calls.iter().enumerate() {
            let _ = tx
                .send(StreamMessage::ToolCalls {
                    message_id,
                    tool_calls: vec![tool_call.clone()],
                })
                .await;

            if verbose_logging {
                append_debug_log_line(&format!(
                    "[DEBUG] Worker: running tool {} (id: {})",
                    tool_call.function.name, tool_call.id
                ));
            }
            let _ = tx
                .send(StreamMessage::AssistantNote {
                    message_id,
                    content: format!(
                        "Running tool: {} ({} of {})",
                        tool_call.function.name,
                        tool_idx + 1,
                        tool_calls.len()
                    ),
                })
                .await;

            let default_timeout_secs = match tool_call.function.name.as_str() {
                "smart_edit" | "skill" => 240,
                "Bash" | "shell" => 120,
                _ => 180,
            };

            let timeout_secs = std::env::var("STAR_TOOL_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default_timeout_secs);

            let exec_fut = agent.execute_tool(tool_call);
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), exec_fut).await
            {
                Ok(Ok(result)) => {
                    if verbose_logging {
                        if result.success {
                            append_debug_log_line(&format!(
                                "[DEBUG] Worker: tool succeeded {} (id: {})",
                                tool_call.function.name, tool_call.id
                            ));
                        } else {
                            let err = result.error.as_deref().unwrap_or("unknown error");
                            append_debug_log_line(&format!(
                                "[DEBUG] Worker: tool failed {} (id: {}): {}",
                                tool_call.function.name, tool_call.id, err
                            ));
                        }
                    }
                    agent.append_tool_result_message(tool_call, &result);
                    let _ = tx
                        .send(StreamMessage::ToolResult {
                            message_id,
                            tool_call: tool_call.clone(),
                            tool_result: result,
                        })
                        .await;
                }
                Ok(Err(e)) => {
                    let result = crate::types::ToolResult {
                        success: false,
                        output: None,
                        error: Some(e.to_string()),
                        data: None,
                    };
                    agent.append_tool_result_message(tool_call, &result);
                    let _ = tx
                        .send(StreamMessage::ToolResult {
                            message_id,
                            tool_call: tool_call.clone(),
                            tool_result: result,
                        })
                        .await;
                }
                Err(_) => {
                    let error_msg = format!("Tool execution timed out ({}s)", timeout_secs);
                    if verbose_logging {
                        append_debug_log_line(&format!(
                            "[DEBUG] Worker: tool timeout {} (id: {})",
                            tool_call.function.name, tool_call.id
                        ));
                    }
                    let result = crate::types::ToolResult {
                        success: false,
                        output: None,
                        error: Some(error_msg),
                        data: Some(serde_json::json!({
                            "error_type": "timeout"
                        })),
                    };
                    agent.append_tool_result_message(tool_call, &result);
                    let _ = tx
                        .send(StreamMessage::ToolResult {
                            message_id,
                            tool_call: tool_call.clone(),
                            tool_result: result,
                        })
                        .await;
                }
            }
        }

        if verbose_logging {
            append_debug_log_line("[DEBUG] Worker: all tools finished, sending Done");
        }
        let _ = tx.send(StreamMessage::Done { message_id }).await;
    } else {
        if verbose_logging {
            append_debug_log_line("[DEBUG] Worker: user rejected execution (approved=false)");
        }
        let _ = tx
            .send(StreamMessage::AssistantNote {
                message_id,
                content: "Tool execution cancelled.".to_string(),
            })
            .await;
        let _ = tx.send(StreamMessage::Done { message_id }).await;
    }
}

async fn handle_confirm_tool(
    agent: &StarAgent,
    _tx: &mpsc::Sender<StreamMessage>,
    tool_call_id: String,
    outcome: crate::types::ToolConfirmationOutcome,
) {
    use crate::core::confirmation_bus::types::{Message, MessageBusType, ToolConfirmationResponse};

    let message_bus = agent.runtime_message_bus().unwrap_or_else(|| {
        use crate::core::policy::PolicyEngine;
        use crate::core::policy::PolicyEngineConfig;
        std::sync::Arc::new(crate::core::confirmation_bus::MessageBus::new(
            PolicyEngine::new(PolicyEngineConfig::default()),
            false,
        ))
    });
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

    let _ = message_bus.publish(msg).await;
}
