use crate::agent::messaging::AsyncMessageQueue;
use crate::agent::StarAgent;
use crate::core::confirmation_bus::types::Message;
use crate::runtime::messages::{AgentRequest, StreamMessage};
use crate::utils::logging::append_debug_log_line;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Notify;

pub enum AgentRuntimeOutcome {
    Completed,
    WorkerClosed,
}

pub struct AgentRuntimeContext<'a> {
    pub tx: &'a mpsc::Sender<StreamMessage>,
    pub rx: &'a mut mpsc::Receiver<AgentRequest>,
    pub bus_rx: &'a mut Option<broadcast::Receiver<Message>>,
    pub project_root: Option<&'a Path>,
    pub steering_queue: &'a Arc<AsyncMessageQueue<(u64, String)>>,
    pub steering_signal: &'a Arc<Notify>,
    pub message_bus: &'a Arc<crate::core::confirmation_bus::MessageBus>,
}

pub async fn process_control_request(
    agent: &mut StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    request: AgentRequest,
) {
    if let Some(action) = crate::runtime::control_requests::handle_request(agent, tx, request).await
    {
        crate::runtime::checkpoints::handle_checkpoint_action(agent, tx, action).await;
    }
}

/// Process a user message through the agent runtime.
///
/// # Flow
/// 1. Send Start message to UI
/// 2. Run preflight hooks
/// 3. Check for blocking hook failures
/// 4. Run streaming session (LLM + tools)
/// 5. Run after-agent hooks
/// 6. Apply deferred runtime actions
///
/// # Error Handling
/// - Hook failures block the request
/// - Streaming errors are forwarded to UI
/// - Abort flag allows user cancellation
///
pub async fn process_message(
    agent: &mut StarAgent,
    context: AgentRuntimeContext<'_>,
    message_id: u64,
    message: &str,
) -> AgentRuntimeOutcome {
    append_debug_log_line(&format!(
        "[AgentRuntime] Processing message id={}, len={}",
        message_id,
        message.len()
    ));

    let _ = context.tx.send(StreamMessage::Start { message_id }).await;
    let has_preflight_hooks =
        crate::runtime::hooks::has_preflight_hooks(context.project_root).await;
    if has_preflight_hooks {
        let _ = context
            .tx
            .send(StreamMessage::AssistantNote {
                message_id,
                content: "Status: running preflight checks".to_string(),
            })
            .await;
    }

    let preflight_summary =
        crate::runtime::hooks::run_preflight_hooks(context.project_root, message).await;
    for note in preflight_summary.assistant_notes {
        let _ = context
            .tx
            .send(StreamMessage::AssistantNote {
                message_id,
                content: note,
            })
            .await;
    }

    if !preflight_summary.blocking_failures.is_empty() {
        let error_msg = format!(
            "Request blocked due to failing blocking hooks:\n- {}",
            preflight_summary.blocking_failures.join("\n- ")
        );
        append_debug_log_line(&format!("[AgentRuntime] {}", error_msg));
        let _ = context
            .tx
            .send(StreamMessage::Error {
                message_id,
                error: error_msg,
            })
            .await;
        crate::runtime::hooks::emit_notification_hook(
            "blocking hook failure",
            "hook_blocking_failure",
        )
        .await;
        let _ = context.tx.send(StreamMessage::Done { message_id }).await;
        return AgentRuntimeOutcome::Completed;
    }

    let abort_flag = agent.abort_handle();
    let _ = context
        .tx
        .send(StreamMessage::AssistantNote {
            message_id,
            content: "Status: processing request".to_string(),
        })
        .await;

    let current_model_snapshot = agent.model();
    let mut deferred = crate::runtime::session::DeferredRuntimeActions::default();

    append_debug_log_line("[AgentRuntime] Starting streaming session");
    if matches!(
        crate::runtime::streaming_session::run_streaming_session(
            agent,
            &mut deferred,
            crate::runtime::streaming_session::StreamingSessionContext {
                tx: context.tx,
                rx: context.rx,
                bus_rx: context.bus_rx,
                message_id,
                user_message: message,
                current_model_snapshot: &current_model_snapshot,
                project_root: context.project_root,
                abort_flag: &abort_flag,
                steering_queue: context.steering_queue,
                steering_signal: context.steering_signal,
                message_bus: context.message_bus,
            },
        )
        .await,
        crate::runtime::streaming_session::StreamingSessionResult::WorkerClosed
    ) {
        append_debug_log_line("[AgentRuntime] Streaming session returned WorkerClosed");
        return AgentRuntimeOutcome::WorkerClosed;
    }
    append_debug_log_line("[AgentRuntime] Streaming session completed");

    agent.reset_abort();

    for note in crate::runtime::hooks::run_after_agent_hooks(context.project_root, message).await {
        let _ = context
            .tx
            .send(StreamMessage::AssistantNote {
                message_id,
                content: note,
            })
            .await;
    }

    crate::runtime::session::apply_deferred_runtime_actions(
        agent,
        &mut deferred,
        context.tx,
        context.project_root,
    )
    .await;

    if let Some(action) = deferred.pending_checkpoint_action.take() {
        crate::runtime::checkpoints::handle_checkpoint_action(agent, context.tx, action).await;
    }

    append_debug_log_line(&format!(
        "[AgentRuntime] Message id={} processing complete",
        message_id
    ));
    AgentRuntimeOutcome::Completed
}
