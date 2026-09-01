use crate::agent::messaging::AsyncMessageQueue;
use crate::agent::StarAgent;
use crate::runtime::messages::{AgentRequest, StreamMessage};
use crate::utils::logging::append_debug_log_line;
use tokio::sync::mpsc;

/// Agent worker — processes requests from the UI and sends stream messages back.
///
/// Architecture:
/// - Runs in a dedicated tokio task (not a thread)
/// - Receives AgentRequest from UI via `rx`
/// - Sends StreamMessage back to UI via `tx`
/// - Uses steering_queue for interrupt/steering messages
/// - Subscribes to message_bus for tool confirmation requests
///
/// Error handling strategy:
/// - Channel errors (tx closed) → exit gracefully (UI was closed)
/// - Agent processing errors → send StreamMessage::Error back to UI
/// - Panics are caught by tokio's task panic handler
pub async fn agent_worker(
    mut agent: StarAgent,
    mut rx: mpsc::Receiver<AgentRequest>,
    tx: mpsc::Sender<StreamMessage>,
) {
    let steering_queue = std::sync::Arc::new(AsyncMessageQueue::new());
    agent.set_steering_queue(steering_queue.clone());

    let steering_signal = std::sync::Arc::new(tokio::sync::Notify::new());
    agent.set_steering_signal(steering_signal.clone());

    let worker_cwd = std::env::current_dir().ok();

    let message_bus = agent.runtime_message_bus().unwrap_or_else(|| {
        use crate::core::policy::PolicyEngine;
        use crate::core::policy::PolicyEngineConfig;
        std::sync::Arc::new(crate::core::confirmation_bus::MessageBus::new(
            PolicyEngine::new(PolicyEngineConfig::default()),
            false,
        ))
    });
    let mut bus_rx = Some(message_bus.subscribe());

    crate::runtime::hooks::run_session_start(worker_cwd.as_deref()).await;

    append_debug_log_line("[Worker] Started, entering main loop");

    loop {
        // Priority: check steering queue first (for interrupts/steering)
        let next_message = if let Some(msg) = steering_queue.try_next() {
            append_debug_log_line("[Worker] Got message from steering_queue");
            Some(msg)
        } else {
            // Wait for UI requests
            append_debug_log_line("[Worker] Waiting for message on rx...");
            match rx.recv().await {
                Some(AgentRequest::SendMessage {
                    message_id,
                    message,
                }) => {
                    append_debug_log_line(&format!(
                        "[Worker] Received SendMessage: id={}, msg_len={}",
                        message_id,
                        message.len()
                    ));
                    Some((message_id, message))
                }
                Some(other) => {
                    append_debug_log_line(&format!(
                        "[Worker] Received control request: {:?}",
                        std::mem::discriminant(&other)
                    ));
                    crate::runtime::agent_runtime::process_control_request(&mut agent, &tx, other)
                        .await;
                    continue;
                }
                None => {
                    crate::runtime::hooks::run_session_end(
                        worker_cwd.as_deref(),
                        "worker_channel_closed",
                    )
                    .await;
                    append_debug_log_line("[Worker] rx closed, exiting");
                    return;
                }
            }
        };

        let Some((message_id, message)) = next_message else {
            continue;
        };

        append_debug_log_line(&format!(
            "[Worker] Processing message id={}",
            message_id
        ));

        // Check if tx is still open before processing
        if tx.is_closed() {
            append_debug_log_line("[Worker] tx closed before processing, exiting");
            return;
        }

        if matches!(
            crate::runtime::agent_runtime::process_message(
                &mut agent,
                crate::runtime::agent_runtime::AgentRuntimeContext {
                    tx: &tx,
                    rx: &mut rx,
                    bus_rx: &mut bus_rx,
                    project_root: worker_cwd.as_deref(),
                    steering_queue: &steering_queue,
                    steering_signal: &steering_signal,
                    message_bus: &message_bus,
                },
                message_id,
                &message,
            )
            .await,
            crate::runtime::agent_runtime::AgentRuntimeOutcome::WorkerClosed
        ) {
            append_debug_log_line("[Worker] process_message returned WorkerClosed");
            return;
        }

        append_debug_log_line(&format!(
            "[Worker] Message id={} processing complete",
            message_id
        ));
    }
}
