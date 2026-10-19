use crate::agent::StarAgent;
use crate::runtime::messages::{
    AgentRequest, PendingCheckpointAction, RuntimeSettingsMutation, StreamMessage, StreamStartKind,
};
use crate::utils::logging::append_debug_log_line;
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
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
        AgentRequest::Shutdown { response } => {
            // worker 在空闲状态截获此请求，经过唯一的 SessionEnd epilogue 后才确认。
            let _ = response
                .send(Err(
                    "Shutdown must be handled by the agent worker.".to_string()
                ))
                .await;
        }
        AgentRequest::SetModel { model, provider_id }
        | AgentRequest::UpdateModel { model, provider_id } => {
            apply_runtime_settings_and_persist(
                agent,
                tx,
                RuntimeSettingsMutation::legacy_model(model, provider_id),
            )
            .await;
        }
        AgentRequest::UpdateRuntimeSettings(mutation) => {
            apply_runtime_settings_and_persist(agent, tx, mutation).await;
        }
        AgentRequest::ListCheckpoints { message_id } => {
            return Some(PendingCheckpointAction::List { message_id });
        }
        AgentRequest::RestoreCheckpoint { message_id, id } => {
            return Some(PendingCheckpointAction::Restore { message_id, id });
        }
        AgentRequest::ListModels { force } => {
            append_debug_log_line(&format!(
                "[DEBUG] Worker: Handling ListModels request (force={})",
                force
            ));
            // `/models` 可能触发网络请求（最长 8 秒超时），如果在主循环中 await，
            // 这段时间内用户发送的 UpdateRuntimeSettings / UpdateProviderConfig 等
            // 控制请求都会被积压在 channel 里，导致“切换模型卡住”。因此把列表拉取
            // 放到后台任务，主循环立即继续处理后续请求。
            let client = agent.client.clone();
            let tx = tx.clone();
            // 这个 spawn 的结果**必须**回到 UI —— `awaiting_models` 只在收到
            // `ModelsList` / `ModelsError` 时清掉。所以下面必须保证任何退出路径
            // 都发得出消息：panic 被 catch 住并补一条 `ModelsError`。
            //
            // 以前裸 `tokio::spawn` + `match`：future 一旦 panic，JoinHandle 没人
            // await，**一条日志、一条消息都没有**，UI 那边 `awaiting_models` 就永远
            // 停在 true，模型面板的转圈停不下来。真实事故：日志里出现
            // `[ListModels] Client state: …` 却再也没有后续，而进程本身活得很好。
            tokio::spawn(async move {
                // 整条链路的**硬上限**。
                //
                // `active_fetch_timeout()` 只罩住了 `/models` 这一次网络请求。围绕它的
                // 还有 `ProviderStore::load()`（读 user-settings.json + 从 cwd 往上找
                // 项目级覆盖）、上下文窗口缓存灌盘、模型列表落盘 —— 这些**都没有超时**。
                // 在 WSL 的 `/mnt/h` 上，任何一次文件 I/O 卡住，整条链路就永远不返回：
                // 真实事故里日志停在 `[ListModels] Client state: …` 之后 70 多秒毫无
                // 动静，而进程本身活得好好的（`base_url=https://api.apinex.bond`，8 秒
                // 的网络超时早该生效了，卡的是别处）。
                //
                // 所以把"拉一次模型列表"整体圈进一个 deadline：超时也要**带着已经拿到的
                // 结果回去**，而不是让 UI 干等。
                let reply = AssertUnwindSafe(async {
                    match tokio::time::timeout(
                        crate::agent::model_list::whole_list_timeout(),
                        crate::agent::model_list::list_models_with_mode(&client, force),
                    )
                    .await
                    {
                        Ok(inner) => inner,
                        Err(_) => {
                            append_debug_log_line(&format!(
                                "[DEBUG] Worker: ListModels exceeded the whole-request \
                                 deadline ({}s); returning whatever was gathered",
                                crate::agent::model_list::whole_list_timeout().as_secs()
                            ));
                            // 超时不是"没有结果"：把磁盘缓存里的那份交回去，面板照样
                            // 有东西看。拿不到才让调用方去显示错误。
                            Ok(crate::agent::model_list::stale_snapshot_for(&client))
                        }
                    }
                })
                .catch_unwind()
                .await;
                match reply {
                    Ok(Ok(result)) => {
                        append_debug_log_line(&format!(
                            "[DEBUG] Worker: ListModels success, count={}, cache_age={:?}",
                            result.models.len(),
                            result.cache_age_secs
                        ));
                        // 列表是刚从 store 扇出来的，所以顺手把 provider 名单也
                        // 同步一次，两者必须看到同一份配置。
                        //
                        // 不发的话，UI 手里的 `configured_providers` 会停在启动那一刻：
                        // 外部工具（cc switch 之类）直接改配置新增的 provider，它的模型
                        // 会出现在列表里、归属也写得清清楚楚，但 `ChatState::
                        // provider_is_live` 却判它是已死的 —— 于是那个本来能用的模型
                        // 反而不被采信，退到当前 provider 上。这就是"刚加完 provider
                        // 就切不过去"的来源。
                        let store = crate::core::config::provider_store::ProviderStore::new();
                        let ids = store.configured_provider_ids().await.unwrap_or_default();
                        let _ = tx.send(StreamMessage::ConfiguredProviders(ids)).await;
                        let _ = tx
                            .send(StreamMessage::ModelsList {
                                models: result.models,
                                cache_age_secs: result.cache_age_secs,
                            })
                            .await;
                    }
                    Ok(Err(e)) => {
                        append_debug_log_line(&format!("[DEBUG] Worker: ListModels failed: {}", e));
                        let _ = tx.send(StreamMessage::ModelsError(e)).await;
                    }
                    Err(_) => {
                        append_debug_log_line(
                            "[DEBUG] Worker: ListModels panicked; replying with ModelsError so \
                             the UI stops waiting",
                        );
                        let _ = tx
                            .send(StreamMessage::ModelsError(
                                "Model list request failed unexpectedly.".to_string(),
                            ))
                            .await;
                    }
                }
            });
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
            agent.update_provider_config(
                provider_id,
                api_key,
                base_url,
                is_openai_compatible,
                model,
            );
        }
        AgentRequest::MarkFilesAsRead(paths) => {
            for path in paths {
                agent.mark_file_as_read(&path).await;
            }
        }
        AgentRequest::AppendContext { content } => {
            agent.append_session_context(content);
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
        AgentRequest::ReloadPermissions => {
            let count = reload_permission_rules(agent).await;
            crate::utils::logging::append_agent_log_line(&format!(
                "[AgentRuntime] permission rules reloaded ({} settings rules)",
                count
            ));
        }
        AgentRequest::SetThinkingEffort(effort) => {
            apply_runtime_settings_and_persist(
                agent,
                tx,
                RuntimeSettingsMutation::legacy_thinking_effort(effort),
            )
            .await;
        }
        AgentRequest::Compress { message_id } => {
            let _ = tx
                .send(StreamMessage::Start {
                    message_id,
                    kind: StreamStartKind::Operation,
                })
                .await;
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
        AgentRequest::LensRebuild { message_id, enrich } => {
            let _ = tx
                .send(StreamMessage::Start {
                    message_id,
                    kind: StreamStartKind::Operation,
                })
                .await;
            let storage = crate::core::config::storage::Storage::new(
                std::env::current_dir().unwrap_or_default(),
            );
            let profile = crate::core::context::lens_service::scope_profile();
            let outcome = if enrich {
                let client = agent.get_client();
                crate::core::context::lens_service::rebuild_enriched(&storage, profile, &client)
                    .await
                    .map(|(r, n)| {
                        format!(
                            "# StarLens build (enriched)\n\n- indexed: {}\n- enriched (F/R/S): {}\n- observed: {}\n- excluded: {}",
                            r.stats.indexed, n, r.stats.observed, r.stats.excluded
                        )
                    })
            } else {
                crate::core::context::lens_service::rebuild(&storage, profile).map(|r| {
                    format!(
                        "# StarLens build\n\n- indexed: {}\n- observed: {}\n- excluded: {}",
                        r.stats.indexed, r.stats.observed, r.stats.excluded
                    )
                })
            };
            match outcome {
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
        AgentRequest::LoadConfiguredProviders => {
            // 这里只加载 provider 菜单元数据。启动时的 model/provider 已在构造 StarAgent
            // 前按 CLI/env/settings 优先级解析；绝不能在初始 runtime acknowledgement 后再
            // 由磁盘 provider store 覆盖运行中 client。
            let store = crate::core::config::provider_store::ProviderStore::new();
            let ids = store.configured_provider_ids().await.unwrap_or_default();
            let _ = tx.send(StreamMessage::ConfiguredProviders(ids)).await;
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
            feedback,
        } => {
            handle_confirm_tool(agent, tx, tool_call_id, outcome, feedback).await;
        }
        AgentRequest::EmitStatus(status) => {
            let _ = tx
                .send(StreamMessage::StatusUpdate {
                    message_id: 0,
                    status,
                })
                .await;
        }
        AgentRequest::SaveSession {
            id,
            history,
            last_usage,
            response,
        } => {
            let (messages, pending_local_context) = agent.session_context_snapshot();
            let result = crate::utils::session_manager::save_session_snapshot(
                &id,
                &history,
                Some(messages),
                Some(pending_local_context),
                last_usage,
            )
            .await
            .map_err(|error| error.to_string());
            let _ = response.send(result).await;
        }
        AgentRequest::RestoreSession {
            messages,
            pending_local_context,
            response,
        } => {
            agent.replace_session_context(messages, pending_local_context);
            let _ = response.send(Ok(())).await;
        }
        AgentRequest::GenerateNote {
            kind,
            message_id,
            question,
        } => {
            // 非流式控制路径：与 ListModels 等一致，直接在 worker 上生成，
            // 结果经 NoteGenerated 回 UI（session.rs 的 deferred 路径同样直等）。
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
        AgentRequest::PluginOp { project_root, op } => {
            spawn_plugin_op(tx, project_root, op);
        }
        AgentRequest::RunGlobalSearch {
            request_id,
            query,
            cwd,
            cancellation,
        } => {
            spawn_global_search(tx, request_id, query, cwd, cancellation);
        }
    }
    None
}

/// 发送已被后续排队更新完全覆盖的确认。此路径刻意不调用 agent 的应用入口，
/// 也绝不能启动 settings 文件持久化。
pub(crate) async fn acknowledge_superseded_runtime_settings(
    agent: &StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    ui_revision: u64,
) {
    let _ = tx
        .send(StreamMessage::RuntimeSettingsAcknowledged(
            agent.superseded_runtime_settings_acknowledgement(ui_revision),
        ))
        .await;
}

/// 通过唯一的 session-owned coordinator 应用设置。worker acknowledgement 先精确
/// 表示 runtime 安装状态；用户设置文件持久化随后经独立事件回 UI，绝不能混为一谈。
pub(crate) async fn apply_runtime_settings_and_persist(
    agent: &mut StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    mutation: RuntimeSettingsMutation,
) {
    let persistence_mutation = mutation.clone();
    let acknowledgement = agent.apply_runtime_settings(mutation).await;
    let persistence_revision = acknowledgement.ui_revision;
    let runtime_applied = matches!(
        acknowledgement.outcome,
        crate::core::context_policy::RuntimeSettingOutcome::Applied
            | crate::core::context_policy::RuntimeSettingOutcome::Degraded
    );

    let _ = tx
        .send(StreamMessage::RuntimeSettingsAcknowledged(acknowledgement))
        .await;

    if !runtime_applied
        || persistence_revision == RuntimeSettingsMutation::LEGACY_UI_REVISION
        || persistence_mutation.persistence
            == crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly
    {
        return;
    }

    let tx = tx.clone();
    tokio::spawn(async move {
        let result = async {
            let manager = crate::core::config::settings_manager::SettingsManager::new()
                .map_err(|error| error.to_string())?;
            let mut settings = manager
                .load_user_settings()
                .await
                .map_err(|error| error.to_string())?;
            if let Some(context_window) = persistence_mutation.context_window {
                settings.context_window = context_window.as_fixed();
            }
            if let Some(thinking_effort) = persistence_mutation.thinking_effort {
                settings.thinking_effort = Some(thinking_effort.as_str().to_string());
            }
            manager
                .save_user_settings(&settings)
                .await
                .map_err(|error| error.to_string())?;

            if let Some(model) = persistence_mutation.model {
                let store = crate::core::config::provider_store::ProviderStore::new();
                if let Some(provider_id) = model.provider_id.as_deref() {
                    store
                        .set_active_provider_and_model(provider_id, &model.model)
                        .await
                        .map_err(|error| error.to_string())?;
                } else {
                    store
                        .set_active_model(&model.model)
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
            Ok::<(), String>(())
        }
        .await;

        let _ = tx
            .send(StreamMessage::RuntimeSettingsPersistenceCompleted {
                ui_revision: persistence_revision,
                error: result.err(),
            })
            .await;
    });
}

/// 在后台任务中执行插件市场操作（git clone 等可能耗时数秒），
/// 完成后把结果经 [`StreamMessage::PluginOpResult`] 推回 UI。
/// 控制请求路径与流式会话路径共用此入口。
pub fn spawn_plugin_op(
    tx: &mpsc::Sender<StreamMessage>,
    project_root: std::path::PathBuf,
    op: crate::runtime::messages::PluginOp,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let message = run_plugin_op(&project_root, op).await;
        let _ = tx.send(StreamMessage::PluginOpResult { message }).await;
    });
}

/// 在后台执行全局搜索。取消的请求不会发送结果，UI 端的请求 ID 仍是最终竞态保护。
pub fn spawn_global_search(
    tx: &mpsc::Sender<StreamMessage>,
    request_id: u64,
    query: String,
    cwd: std::path::PathBuf,
    cancellation: tokio_util::sync::CancellationToken,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        if cancellation.is_cancelled() {
            return;
        }
        let result =
            crate::ui::components::highlight::search::execute_search(&query, &cwd, &cancellation)
                .await;
        if cancellation.is_cancelled() {
            return;
        }
        if let Some((results, truncated)) = result {
            let _ = tx
                .send(StreamMessage::GlobalSearchResults {
                    request_id,
                    results,
                    truncated,
                })
                .await;
        }
    });
}

/// 执行插件市场后台操作，返回要显示给用户的消息（None = 静默成功）。
async fn run_plugin_op(
    project_root: &std::path::Path,
    op: crate::runtime::messages::PluginOp,
) -> Option<String> {
    use crate::core::plugins::marketplace as mp;
    use crate::runtime::messages::PluginOp;
    match op {
        PluginOp::EnsureDefaultMarketplace => {
            match mp::ensure_default_marketplace(project_root).await {
                Ok(Some(name)) => Some(format!("Auto-registered default marketplace '{}'", name)),
                Ok(None) => Some("Default marketplace auto-registration skipped".to_string()),
                Err(e) => Some(format!("Default marketplace: {}", e)),
            }
        }
        PluginOp::AddMarketplace { source } => {
            match mp::add_marketplace(project_root, &source).await {
                Ok(m) => Some(format!("Added marketplace '{}'", m.name)),
                Err(e) => Some(format!("Error: {}", e)),
            }
        }
        PluginOp::RemoveMarketplace { name } => {
            match mp::remove_marketplace(project_root, &name).await {
                Ok(true) => Some(format!("Removed marketplace {}", name)),
                Ok(false) => Some(format!("Marketplace not found: {}", name)),
                Err(e) => Some(format!("Error: {}", e)),
            }
        }
        PluginOp::InstallPlugin { plugin, scope } => {
            match mp::install_marketplace_plugin(project_root, &plugin, &scope).await {
                Ok(_) => Some(format!("Installed plugin {} ({})", plugin.name, scope)),
                Err(e) => Some(format!("Error: {}", e)),
            }
        }
        PluginOp::UpdateMarketplace { name } => {
            match mp::update_marketplace(project_root, &name).await {
                Ok(msg) => Some(msg),
                Err(e) => Some(format!("Error: {}", e)),
            }
        }
    }
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
        let _ = tx
            .send(StreamMessage::Start {
                message_id,
                kind: StreamStartKind::Operation,
            })
            .await;

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

            let timeout_secs =
                crate::core::self_tuning::u64_value("tools.confirmed_call_timeout_secs")
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

/// 让运行时的 PolicyEngine 重新读盘。规则活在 MessageBus 里，
/// UI 侧改完 `.star/settings.local.json` 只能靠这条路生效。
async fn reload_permission_rules(agent: &StarAgent) -> usize {
    let Some(bus) = agent.runtime_message_bus() else {
        return 0;
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    bus.reload_permission_rules(&cwd).await
}

async fn handle_confirm_tool(
    agent: &StarAgent,
    _tx: &mpsc::Sender<StreamMessage>,
    tool_call_id: String,
    outcome: crate::types::ToolConfirmationOutcome,
    feedback: Option<String>,
) {
    use crate::core::confirmation_bus::types::{Message, ToolConfirmationResponse};

    let message_bus = agent.runtime_message_bus().unwrap_or_else(|| {
        use crate::core::policy::{PolicyEngine, PolicyEngineConfig};
        // 同 worker.rs 的兜底：规则和审批模式都要带上，否则 runtime 缺席时
        // 用户选的 yolo/plan 会退化成逐次询问。
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let approval_mode = agent.policy_approval_mode();
        std::sync::Arc::new(crate::core::confirmation_bus::MessageBus::new(
            PolicyEngine::with_project_rules(
                PolicyEngineConfig {
                    approval_mode: Some(approval_mode),
                    ..Default::default()
                },
                &cwd,
            ),
            false,
        ))
    });
    let msg = Message::ToolConfirmationResponse(ToolConfirmationResponse::from_user_decision(
        tool_call_id,
        outcome,
        feedback,
    ));

    let _ = message_bus.publish(msg).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn superseded_settings_acknowledgement_emits_no_persistence_event() {
        let agent = StarAgent::new_for_runtime_settings_test().await;
        let (tx, mut rx) = mpsc::channel(2);

        acknowledge_superseded_runtime_settings(&agent, &tx, 17).await;

        let message = rx.recv().await.expect("superseded acknowledgement");
        match message {
            StreamMessage::RuntimeSettingsAcknowledged(acknowledgement) => {
                assert_eq!(acknowledgement.ui_revision, 17);
                assert_eq!(
                    acknowledgement.outcome,
                    crate::core::context_policy::RuntimeSettingOutcome::Superseded
                );
            }
            other => panic!("expected runtime settings acknowledgement, got {other:?}"),
        }
        assert!(tokio::time::timeout(Duration::from_millis(25), rx.recv())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn cancelled_global_search_emits_no_results() {
        let (tx, mut rx) = mpsc::channel(1);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        spawn_global_search(
            &tx,
            7,
            "needle".to_string(),
            std::path::PathBuf::from("."),
            cancellation,
        );

        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err());
    }
}
