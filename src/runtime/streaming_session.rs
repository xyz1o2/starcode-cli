use crate::agent::messaging::AsyncMessageQueue;
use crate::agent::StarAgent;
use crate::core::confirmation_bus::types::Message;
use crate::runtime::messages::{AgentRequest, StreamMessage};
use crate::runtime::session::{
    DeferredRuntimeActions, StreamingRequestContext, StreamingRequestOutcome,
};
use crate::utils::logging::append_debug_log_line;
use futures::stream::StreamExt;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Notify;

pub enum StreamingSessionResult {
    Completed,
    WorkerClosed,
}

pub struct StreamingSessionContext<'a> {
    pub tx: &'a mpsc::Sender<StreamMessage>,
    pub rx: &'a mut mpsc::Receiver<AgentRequest>,
    pub bus_rx: &'a mut Option<broadcast::Receiver<Message>>,
    pub message_id: u64,
    pub user_message: &'a str,
    pub current_model_snapshot: &'a str,
    pub project_root: Option<&'a Path>,
    pub abort_flag: &'a Arc<std::sync::atomic::AtomicBool>,
    pub steering_queue: &'a Arc<AsyncMessageQueue<(u64, String)>>,
    pub steering_signal: &'a Arc<Notify>,
    pub message_bus: &'a Arc<crate::core::confirmation_bus::MessageBus>,
}

/// Run a streaming session with the agent.
///
/// # Architecture
///
/// This function implements a `tokio::select!` loop that handles three concurrent event sources:
/// 1. **Confirmation bus messages** — tool confirmation requests from the agent
/// 2. **UI requests** — abort, model change, checkpoint restore, etc.
/// 3. **Stream chunks** — content, tool calls, tool results from the LLM
///
/// # Error Recovery
///
/// - Stream errors are sent as `StreamMessage::Error` to the UI
/// - The UI can abort via `AgentRequest::Abort` which sets the abort flag
/// - Tool events yield the task to allow the UI to render them separately
/// - Channel send errors are silently ignored (UI may have been closed)
///
pub async fn run_streaming_session(
    agent: &mut StarAgent,
    deferred: &mut DeferredRuntimeActions,
    context: StreamingSessionContext<'_>,
) -> StreamingSessionResult {
    append_debug_log_line(&format!(
        "[Worker] Starting agent stream: id={}, msg_len={}",
        context.message_id,
        context.user_message.len()
    ));

    // Set current_message_id on GlobalState so write tools (write_file / edit /
    // multi_edit) can associate file-history snapshots with this message round
    // via track_edit(message_id). The next run_streaming_session call will
    // overwrite this value, so no explicit clear is needed at function exit.
    if let Some(gs) = agent.runtime_global_state() {
        gs.set_current_message_id(Some(context.message_id)).await;
    }

    match agent
        .process_user_message_stream(context.user_message)
        .await
    {
        Ok(mut stream) => {
            let mut chunk_count: u64 = 0;
            'streaming: loop {
                tokio::select! {
                    Ok(msg) = async {
                        if let Some(rx) = &mut *context.bus_rx {
                            rx.recv().await
                        } else {
                            std::future::pending().await
                        }
                    } => {
                        crate::runtime::confirmation_bridge::relay_confirmation_bus_message(
                            context.tx,
                            context.message_id,
                            msg,
                        )
                        .await;
                    }
                    maybe_req = context.rx.recv() => {
                        match crate::runtime::session::handle_streaming_request(
                            deferred,
                            maybe_req,
                            StreamingRequestContext {
                                tx: context.tx,
                                message_id: context.message_id,
                                user_message: context.user_message,
                                current_model_snapshot: context.current_model_snapshot,
                                project_root: context.project_root,
                                abort_flag: context.abort_flag,
                                steering_queue: context.steering_queue,
                                steering_signal: context.steering_signal,
                                message_bus: context.message_bus,
                            },
                        ).await {
                            StreamingRequestOutcome::Continue => {}
                            StreamingRequestOutcome::Break => {
                                break 'streaming;
                            }
                            StreamingRequestOutcome::Return => {
                                return StreamingSessionResult::WorkerClosed;
                            }
                        }
                    }
                    chunk = stream.next() => {
                        match chunk {
                            Some(Ok(c)) => {
                                chunk_count += 1;
                                let is_tool = matches!(c.chunk_type,
                                    crate::types::StreamingChunkType::ToolCalls |
                                    crate::types::StreamingChunkType::ToolResult
                                );
                                let done = crate::runtime::stream_chunks::handle_stream_chunk(
                                    context.tx,
                                    context.message_id,
                                    c,
                                ).await;
                                if done {
                                    append_debug_log_line(&format!(
                                        "[Worker] Stream done after {} chunks",
                                        chunk_count
                                    ));
                                    break 'streaming;
                                }
                                // After tool events, yield the async task so the UI
                                // has a frame to render each tool call/result separately,
                                // preventing them from appearing all at once.
                                if is_tool {
                                    tokio::task::yield_now().await;
                                }
                            }
                            Some(Err(e)) => {
                                let error_msg = e.to_string();
                                append_debug_log_line(&format!(
                                    "[Worker] Stream error after {} chunks: {}",
                                    chunk_count, error_msg
                                ));
                                let _ = context.tx.send(StreamMessage::Error {
                                    message_id: context.message_id,
                                    error: error_msg,
                                }).await;
                                break 'streaming;
                            }
                            None => {
                                append_debug_log_line(&format!(
                                    "[Worker] Stream ended after {} chunks",
                                    chunk_count
                                ));
                                let _ = context.tx.send(StreamMessage::Done {
                                    message_id: context.message_id,
                                }).await;
                                break 'streaming;
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            let error_msg = e.to_string();
            append_debug_log_line(&format!("[Worker] Agent stream init failed: {}", error_msg));
            let _ = context
                .tx
                .send(StreamMessage::Error {
                    message_id: context.message_id,
                    error: error_msg,
                })
                .await;
        }
    }

    StreamingSessionResult::Completed
}
