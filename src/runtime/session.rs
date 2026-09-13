use crate::agent::messaging::AsyncMessageQueue;
use crate::agent::StarAgent;
use crate::core::confirmation_bus::types::{Message, MessageBusType, ToolConfirmationResponse};
use crate::runtime::messages::{
    AgentRequest, PendingCheckpointAction, RuntimeSettingsMutation, StreamMessage, StreamStartKind,
};
use crate::utils::logging::append_debug_log_line;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Notify;

/// 流式回合结束后才可接触 agent 原生上下文的操作。
///
/// 必须保持收到请求的顺序：例如 `AppendContext` 后的 save 应保存追加内容，
/// 而 save 后的 `AppendContext` 则不应提前混入那个快照。
enum DeferredContextAction {
    Append {
        content: String,
    },
    Reset,
    Save {
        id: String,
        history: Vec<crate::types::ChatEntry>,
        last_usage: Option<crate::types::StarUsage>,
        response: mpsc::Sender<Result<(), String>>,
    },
    Restore {
        messages: Vec<crate::types::StarMessage>,
        pending_local_context: Vec<String>,
        response: mpsc::Sender<Result<(), String>>,
    },
    /// 此项必须排在 save/restore 等上下文变更之后执行，确保快照拿到流结束同步的原生消息。
    Shutdown {
        response: mpsc::Sender<Result<(), String>>,
    },
}

#[derive(Default)]
pub struct DeferredRuntimeActions {
    /// 运行时设置必须保留 UI 到达顺序：在流式回合结束后逐项确认，绝不能把
    /// 模型和 thinking 的最新值各自覆盖成两条无序的副作用。
    pending_runtime_settings: VecDeque<RuntimeSettingsMutation>,
    /// `Some(force)` = 流式期间来过 ListModels，回合结束后补上（force 语义见
    /// `AgentRequest::ListModels`）；多次请求里只要有一次 force 就按 force 算。
    pub pending_models_request: Option<bool>,
    pub pending_plugin_tools_refresh: bool,
    pub pending_mcp_refresh: bool,
    pub pending_mcp_list_servers: bool,
    pub pending_mcp_list_tools: Option<String>,
    pub pending_toggle_yolo: bool,
    pub pending_set_approval_mode: Option<crate::types::ApprovalMode>,
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
    pub pending_generate_note: Option<(crate::runtime::messages::NoteKind, u64, Option<String>)>,
    pub pending_mark_as_read: Vec<String>,
    /// 流式过程中收到的会话上下文变更。必须按接收顺序串行执行，避免
    /// `AppendContext`、save、restore 之间的边界互相穿插。
    pending_context_actions: VecDeque<DeferredContextAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingRequestOutcome {
    Continue,
    Break,
    /// 已请求中断；继续 drain 当前 stream，等待 agent 同步最终原生上下文。
    Abort,
    Return,
    /// 已请求终止；继续 drain 当前 stream，等待 agent 同步最终原生上下文。
    Shutdown,
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
            StreamingRequestOutcome::Abort
        }
        Some(AgentRequest::Shutdown { response }) => {
            // 不直接丢掉 outer stream：它负责接收 agent loop 的最终 session_messages。
            // 保持 drain，等取消后的 Done 抵达再让 worker 统一收尾。
            context.abort_flag.store(true, Ordering::SeqCst);
            deferred
                .pending_context_actions
                .push_back(DeferredContextAction::Shutdown { response });
            StreamingRequestOutcome::Shutdown
        }
        Some(AgentRequest::LoadConfiguredProviders) => {
            // provider 列表仅供 UI 菜单显示；不得在流式回合内从磁盘重置已冻结的
            // runtime model/provider，也不发送会冒充 active 状态的旧消息。
            let store = crate::core::config::provider_store::ProviderStore::new();
            let ids = store.configured_provider_ids().await.unwrap_or_default();
            let _ = context
                .tx
                .send(StreamMessage::ConfiguredProviders(ids))
                .await;
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetModel { model, provider_id })
        | Some(AgentRequest::UpdateModel { model, provider_id }) => {
            deferred
                .pending_runtime_settings
                .push_back(RuntimeSettingsMutation::legacy_model(model, provider_id));
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::UpdateRuntimeSettings(mutation)) => {
            deferred.pending_runtime_settings.push_back(mutation);
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
            deferred
                .pending_context_actions
                .push_back(DeferredContextAction::Append { content });
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
            deferred
                .pending_context_actions
                .push_back(DeferredContextAction::Reset);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetApprovalMode(mode)) => {
            deferred.pending_set_approval_mode = Some(mode);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::ReloadPermissions) => {
            // 不推迟：用户刚在 `/permissions allow` 里放行的规则，同一回合的下一个
            // 工具调用就该认。重载走 PolicyEngine 的写锁，和检查路径互不阻塞。
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let count = context.message_bus.reload_permission_rules(&cwd).await;
            append_debug_log_line(&format!(
                "[Worker/Streaming] permission rules reloaded ({} settings rules)",
                count
            ));
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::SetThinkingEffort(effort)) => {
            // 本轮跑完再生效。中途打开思考会让接下来的一次请求带上
            // thinking 参数，而历史里已经有不含 thinking block 的
            // assistant 轮次 —— Anthropic 会因此报错。
            deferred
                .pending_runtime_settings
                .push_back(RuntimeSettingsMutation::legacy_thinking_effort(effort));
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
            feedback,
        }) => {
            let msg = Message::ToolConfirmationResponse(
                ToolConfirmationResponse::from_user_decision(tool_call_id, outcome, feedback),
            );
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
        Some(AgentRequest::SaveSession {
            id,
            history,
            last_usage,
            response,
        }) => {
            deferred
                .pending_context_actions
                .push_back(DeferredContextAction::Save {
                    id,
                    history,
                    last_usage,
                    response,
                });
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::RestoreSession {
            messages,
            pending_local_context,
            response,
        }) => {
            deferred
                .pending_context_actions
                .push_back(DeferredContextAction::Restore {
                    messages,
                    pending_local_context,
                    response,
                });
            StreamingRequestOutcome::Continue
        }
        None => {
            context.abort_flag.store(true, Ordering::SeqCst);
            StreamingRequestOutcome::Return
        }
        Some(AgentRequest::PluginOp { project_root, op }) => {
            // 插件市场后台操作不依赖 agent：即使正在流式回复中也直接执行
            crate::runtime::control_requests::spawn_plugin_op(&context.tx, project_root, op);
            StreamingRequestOutcome::Continue
        }
        Some(AgentRequest::RunGlobalSearch {
            request_id,
            query,
            cwd,
            cancellation,
        }) => {
            // 全局搜索与主 agent 回合无关，必须立刻后台执行，不能等流式结束。
            crate::runtime::control_requests::spawn_global_search(
                &context.tx,
                request_id,
                query,
                cwd,
                cancellation,
            );
            StreamingRequestOutcome::Continue
        }
    }
}

pub async fn apply_deferred_context_actions(
    agent: &mut StarAgent,
    deferred: &mut DeferredRuntimeActions,
) -> Option<mpsc::Sender<Result<(), String>>> {
    // 在 agent 已完成本回合并同步原生消息后，按请求抵达的原始顺序处理。
    // 这样 `!command` 输出、重置、restore、save 都不会越过彼此的上下文边界。
    while let Some(action) = deferred.pending_context_actions.pop_front() {
        match action {
            DeferredContextAction::Append { content } => agent.append_session_context(content),
            DeferredContextAction::Reset => agent.clear_session_context(),
            DeferredContextAction::Save {
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
            DeferredContextAction::Restore {
                messages,
                pending_local_context,
                response,
            } => {
                agent.replace_session_context(messages, pending_local_context);
                let _ = response.send(Ok(())).await;
            }
            DeferredContextAction::Shutdown { response } => return Some(response),
        }
    }

    None
}

/// 按 UI 到达顺序安装本轮结束后才可生效的设置，并为每项发送终态确认。
#[derive(Default)]
struct RuntimeSettingFields {
    model: bool,
    context_window: bool,
    thinking_effort: bool,
}

impl RuntimeSettingFields {
    fn from_mutation(mutation: &RuntimeSettingsMutation) -> Self {
        Self {
            model: mutation.model.is_some(),
            context_window: mutation.context_window.is_some(),
            thinking_effort: mutation.thinking_effort.is_some(),
        }
    }

    fn is_empty(&self) -> bool {
        !self.model && !self.context_window && !self.thinking_effort
    }

    fn cover(&mut self, mutation: &RuntimeSettingsMutation) {
        self.model |= mutation.model.is_some();
        self.context_window |= mutation.context_window.is_some();
        self.thinking_effort |= mutation.thinking_effort.is_some();
    }

    fn covers(&self, fields: &Self) -> bool {
        (!fields.model || self.model)
            && (!fields.context_window || self.context_window)
            && (!fields.thinking_effort || self.thinking_effort)
    }
}

/// 只有 revisioned 请求才参与 supersession：旧协议讯息既不能成为候选项，也不该
/// 覆盖新 UI 意图。持久化请求还要求后续的 persistent 更新覆盖每一个字段，避免
/// 丢掉用户最后一次需要写盘的意图。
fn is_fully_superseded<'a>(
    candidate: &RuntimeSettingsMutation,
    later_mutations: impl Iterator<Item = &'a RuntimeSettingsMutation>,
) -> bool {
    if candidate.ui_revision == RuntimeSettingsMutation::LEGACY_UI_REVISION {
        return false;
    }
    let candidate_fields = RuntimeSettingFields::from_mutation(candidate);
    if candidate_fields.is_empty() {
        return false;
    }

    let mut all_later_fields = RuntimeSettingFields::default();
    let mut persistent_later_fields = RuntimeSettingFields::default();
    for later in later_mutations {
        if later.ui_revision == RuntimeSettingsMutation::LEGACY_UI_REVISION {
            continue;
        }
        all_later_fields.cover(later);
        if later.persistence
            == crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent
        {
            persistent_later_fields.cover(later);
        }
    }

    all_later_fields.covers(&candidate_fields)
        && (candidate.persistence
            != crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent
            || persistent_later_fields.covers(&candidate_fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context_policy::ContextWindowSelection;
    use crate::runtime::messages::{
        RuntimeModelSelection, RuntimeSettingsMutation, RuntimeSettingsPersistencePolicy,
    };
    use crate::types::ThinkingEffort;
    use std::sync::atomic::AtomicBool;

    fn mutation(
        ui_revision: u64,
        persistence: RuntimeSettingsPersistencePolicy,
        model: bool,
        context_window: bool,
        thinking_effort: bool,
    ) -> RuntimeSettingsMutation {
        RuntimeSettingsMutation {
            ui_revision,
            persistence,
            model: model.then(|| RuntimeModelSelection {
                model: format!("model-{ui_revision}"),
                provider_id: Some("test-provider".to_string()),
            }),
            context_window: context_window
                .then_some(ContextWindowSelection::Fixed(200_000 + ui_revision as u32)),
            thinking_effort: thinking_effort.then_some(ThinkingEffort::High),
        }
    }

    #[test]
    fn runtime_setting_supersession_requires_full_field_and_persistence_coverage() {
        use RuntimeSettingsPersistencePolicy::{Persistent, SessionOnly};

        let cases = [
            (
                mutation(1, Persistent, true, false, false),
                vec![mutation(2, Persistent, true, false, false)],
                true,
                "persistent same field",
            ),
            (
                mutation(1, SessionOnly, true, false, false),
                vec![mutation(2, Persistent, true, false, false)],
                true,
                "session-only replaced persistently",
            ),
            (
                mutation(1, Persistent, true, false, false),
                vec![mutation(2, SessionOnly, true, false, false)],
                false,
                "persistent intent cannot be replaced session-only",
            ),
            (
                mutation(1, Persistent, true, false, false),
                vec![mutation(2, Persistent, false, false, true)],
                false,
                "disjoint fields",
            ),
            (
                mutation(1, Persistent, true, true, false),
                vec![mutation(2, Persistent, true, false, false)],
                false,
                "partial coverage",
            ),
            (
                mutation(1, Persistent, true, true, false),
                vec![
                    mutation(2, Persistent, true, false, false),
                    mutation(3, Persistent, false, true, false),
                ],
                true,
                "collective persistent coverage",
            ),
            (
                mutation(1, Persistent, true, true, false),
                vec![
                    mutation(2, Persistent, true, false, false),
                    mutation(3, SessionOnly, false, true, false),
                ],
                false,
                "collective runtime coverage without persistence coverage",
            ),
        ];

        for (candidate, later, expected, label) in cases {
            assert_eq!(
                is_fully_superseded(&candidate, later.iter()),
                expected,
                "{label}"
            );
        }
    }

    #[test]
    fn legacy_and_empty_runtime_mutations_are_never_superseded() {
        use RuntimeSettingsPersistencePolicy::Persistent;

        let legacy = mutation(
            RuntimeSettingsMutation::LEGACY_UI_REVISION,
            Persistent,
            true,
            false,
            false,
        );
        assert!(!is_fully_superseded(
            &legacy,
            [mutation(1, Persistent, true, false, false)].iter()
        ));

        let candidate = mutation(1, Persistent, true, false, false);
        assert!(!is_fully_superseded(
            &candidate,
            [mutation(
                RuntimeSettingsMutation::LEGACY_UI_REVISION,
                Persistent,
                true,
                false,
                false,
            )]
            .iter()
        ));

        let empty = mutation(1, Persistent, false, false, false);
        assert!(!is_fully_superseded(
            &empty,
            [mutation(2, Persistent, true, false, false)].iter()
        ));
    }

    #[tokio::test]
    async fn deferred_runtime_settings_acknowledge_supersession_before_applying_survivor() {
        use RuntimeSettingsPersistencePolicy::SessionOnly;

        let mut agent = crate::agent::StarAgent::new_for_runtime_settings_test().await;
        let mut deferred = DeferredRuntimeActions::default();
        deferred
            .pending_runtime_settings
            .push_back(mutation(1, SessionOnly, false, true, false));
        deferred
            .pending_runtime_settings
            .push_back(mutation(2, SessionOnly, false, true, false));
        let (tx, mut rx) = mpsc::channel(3);

        apply_deferred_runtime_settings(&mut agent, &mut deferred, &tx).await;

        let first = rx.recv().await.expect("superseded acknowledgement");
        let second = rx.recv().await.expect("surviving acknowledgement");
        let StreamMessage::RuntimeSettingsAcknowledged(first) = first else {
            panic!("expected first runtime settings acknowledgement");
        };
        let StreamMessage::RuntimeSettingsAcknowledged(second) = second else {
            panic!("expected second runtime settings acknowledgement");
        };

        assert_eq!(
            (
                first.ui_revision,
                first.outcome,
                first.snapshot.runtime_revision
            ),
            (
                1,
                crate::core::context_policy::RuntimeSettingOutcome::Superseded,
                0
            )
        );
        assert_eq!(
            (
                second.ui_revision,
                second.outcome,
                second.snapshot.runtime_revision
            ),
            (
                2,
                crate::core::context_policy::RuntimeSettingOutcome::Applied,
                1
            )
        );
        assert_eq!(
            second.snapshot.active.context_policy.selection,
            ContextWindowSelection::Fixed(200_002)
        );
        assert!(deferred.pending_runtime_settings.is_empty());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn deferred_disjoint_runtime_settings_apply_in_fifo_order() {
        use RuntimeSettingsPersistencePolicy::SessionOnly;

        let mut agent = crate::agent::StarAgent::new_for_runtime_settings_test().await;
        let mut deferred = DeferredRuntimeActions::default();
        deferred
            .pending_runtime_settings
            .push_back(mutation(1, SessionOnly, false, true, false));
        deferred
            .pending_runtime_settings
            .push_back(mutation(2, SessionOnly, false, false, true));
        let (tx, mut rx) = mpsc::channel(3);

        apply_deferred_runtime_settings(&mut agent, &mut deferred, &tx).await;

        let first = rx.recv().await.expect("first runtime acknowledgement");
        let second = rx.recv().await.expect("second runtime acknowledgement");
        let StreamMessage::RuntimeSettingsAcknowledged(first) = first else {
            panic!("expected first runtime settings acknowledgement");
        };
        let StreamMessage::RuntimeSettingsAcknowledged(second) = second else {
            panic!("expected second runtime settings acknowledgement");
        };

        assert_eq!(
            (
                first.ui_revision,
                first.outcome,
                first.snapshot.runtime_revision
            ),
            (
                1,
                crate::core::context_policy::RuntimeSettingOutcome::Applied,
                1
            )
        );
        assert_eq!(
            first.snapshot.active.context_policy.selection,
            ContextWindowSelection::Fixed(200_001)
        );
        assert_eq!(
            (
                second.ui_revision,
                second.outcome,
                second.snapshot.runtime_revision
            ),
            (
                2,
                crate::core::context_policy::RuntimeSettingOutcome::Applied,
                2
            )
        );
        assert_eq!(
            second.snapshot.active.context_policy.selection,
            ContextWindowSelection::Fixed(200_001)
        );
        assert_eq!(second.snapshot.active.thinking_effort, ThinkingEffort::High);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn configured_provider_loading_emits_metadata_only() {
        let mut deferred = DeferredRuntimeActions::default();
        let (tx, mut rx) = mpsc::channel(1);
        let steering_queue = Arc::new(AsyncMessageQueue::new());
        let steering_signal = Arc::new(Notify::new());
        let message_bus = Arc::new(crate::core::confirmation_bus::MessageBus::new(
            crate::core::policy::PolicyEngine::new(Default::default()),
            false,
        ));
        let abort_flag = Arc::new(AtomicBool::new(false));

        let outcome = handle_streaming_request(
            &mut deferred,
            Some(AgentRequest::LoadConfiguredProviders),
            StreamingRequestContext {
                tx: &tx,
                message_id: 7,
                user_message: "test",
                current_model_snapshot: "active-model",
                project_root: None,
                abort_flag: &abort_flag,
                steering_queue: &steering_queue,
                steering_signal: &steering_signal,
                message_bus: &message_bus,
            },
        )
        .await;

        assert_eq!(outcome, StreamingRequestOutcome::Continue);
        assert!(matches!(
            rx.recv().await,
            Some(StreamMessage::ConfiguredProviders(_))
        ));
        assert!(deferred.pending_runtime_settings.is_empty());
    }
}

pub async fn apply_deferred_runtime_settings(
    agent: &mut StarAgent,
    deferred: &mut DeferredRuntimeActions,
    tx: &mpsc::Sender<StreamMessage>,
) {
    while let Some(mutation) = deferred.pending_runtime_settings.pop_front() {
        if is_fully_superseded(&mutation, deferred.pending_runtime_settings.iter()) {
            crate::runtime::control_requests::acknowledge_superseded_runtime_settings(
                agent,
                tx,
                mutation.ui_revision,
            )
            .await;
        } else {
            crate::runtime::control_requests::apply_runtime_settings_and_persist(
                agent, tx, mutation,
            )
            .await;
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

    apply_deferred_runtime_settings(agent, deferred, tx).await;

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

    if let Some(response) = apply_deferred_context_actions(agent, deferred).await {
        // 正常路径不应看到 Shutdown：agent_runtime 会在流 drain 后先截获它，
        // 再由 worker 在 SessionEnd 后确认。保留防御性错误，避免调用方无止境等待。
        let _ = response
            .send(Err("Shutdown bypassed the worker lifecycle.".to_string()))
            .await;
        return;
    }

    if let Some(message_id) = deferred.pending_compress_request.take() {
        let _ = tx
            .send(StreamMessage::Start {
                message_id,
                kind: StreamStartKind::Operation,
            })
            .await;
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

    if let Some((tool_calls, message_id, approved, always_allow)) =
        deferred.pending_tool_confirmation.take()
    {
        if approved || always_allow {
            let _ = tx
                .send(StreamMessage::Start {
                    message_id,
                    kind: StreamStartKind::Operation,
                })
                .await;

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
