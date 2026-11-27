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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn retry_wait_handles_control_requests_without_waiting_for_backoff() {
        crate::core::i18n::reset_for_tests();
        for scenario in ["abort", "shutdown", "closed", "flag", "ordinary"] {
            let (tx, mut events) = mpsc::channel(64);
            let (requests, mut rx) = mpsc::channel(4);
            let mut bus_rx = None;
            let abort_flag = Arc::new(AtomicBool::new(scenario == "flag"));
            let steering_queue = Arc::new(AsyncMessageQueue::new());
            let steering_signal = Arc::new(Notify::new());
            let message_bus = Arc::new(crate::core::confirmation_bus::MessageBus::new(
                crate::core::policy::PolicyEngine::new(Default::default()),
                false,
            ));
            let model_switch_cell = crate::agent::model_switch::new_cell();
            let mut deferred = DeferredRuntimeActions::default();
            match scenario {
                "abort" => requests.send(AgentRequest::Abort).await.unwrap(),
                "shutdown" => {
                    let (response, _rx) = mpsc::channel(1);
                    requests
                        .send(AgentRequest::Shutdown { response })
                        .await
                        .unwrap();
                }
                "ordinary" => requests
                    .send(AgentRequest::StopBackgroundAgent {
                        task_id: "missing".into(),
                    })
                    .await
                    .unwrap(),
                _ => {}
            }
            // 只有 closed 场景关闭输入，其他场景保持 sender 存活。
            let _requests = if scenario == "closed" {
                drop(requests);
                None
            } else {
                Some(requests)
            };
            let mut context = StreamingSessionContext {
                tx: &tx,
                rx: &mut rx,
                bus_rx: &mut bus_rx,
                message_id: 7,
                user_message: "test",
                images: &[],
                current_model_snapshot: "test",
                project_root: None,
                abort_flag: &abort_flag,
                steering_queue: &steering_queue,
                steering_signal: &steering_signal,
                message_bus: &message_bus,
                model_switch_cell: &model_switch_cell,
            };
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                wait_for_stream_retry(
                    &mut context,
                    &mut deferred,
                    None,
                    crate::llm::error_kind::LlmErrorKind::Network,
                    0,
                    if scenario == "ordinary" { 10 } else { 60_000 },
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("{scenario}: 控制请求必须在长退避结束前处理"));
            assert!(
                match scenario {
                    "closed" => matches!(result, Some(StreamingSessionResult::WorkerClosed)),
                    "shutdown" => matches!(result, Some(StreamingSessionResult::Shutdown)),
                    "ordinary" => result.is_none(),
                    _ => matches!(result, Some(StreamingSessionResult::Completed)),
                },
                "{scenario}"
            );
            if matches!(scenario, "abort" | "shutdown" | "flag") {
                assert!(abort_flag.load(Ordering::SeqCst));
            }
            let mut done_count = 0;
            let mut saw_ordinary_result = false;
            while let Ok(event) = events.try_recv() {
                match event {
                    StreamMessage::Done { message_id: 7 } => done_count += 1,
                    StreamMessage::BackgroundAgentStopResult { .. } => saw_ordinary_result = true,
                    _ => {}
                }
            }
            assert_eq!(
                done_count,
                usize::from(matches!(scenario, "abort" | "shutdown" | "flag")),
                "{scenario}"
            );
            assert_eq!(saw_ordinary_result, scenario == "ordinary");
        }
    }
}

pub enum StreamingSessionResult {
    Completed,
    WorkerClosed,
    Shutdown,
}

/// 退避时仍消费 UI 请求，否则 Abort 只能排在 sleep 后面，无法及时取消重试。
async fn wait_for_stream_retry(
    context: &mut StreamingSessionContext<'_>,
    deferred: &mut DeferredRuntimeActions,
    subagent_tasks: Option<crate::agent::subagent::tasks::SharedSubagentTasks>,
    kind: crate::llm::error_kind::LlmErrorKind,
    attempt: u32,
    delay_ms: u64,
) -> Option<StreamingSessionResult> {
    let tx = context.tx;
    let message_id = context.message_id;
    let waiting = crate::llm::retry::wait_interruptible(
        std::time::Duration::from_millis(delay_ms),
        Some(context.abort_flag.as_ref()),
        |remaining| {
            let _ = tx.try_send(StreamMessage::RetryStatus {
                message_id,
                status: crate::llm::retry::waiting_status(kind, attempt, remaining),
            });
        },
    );
    tokio::pin!(waiting);
    loop {
        let outcome = tokio::select! {
            // 同时就绪时先处理取消，不能抢先发出下一次请求。
            biased;
            request = context.rx.recv() => {
                crate::runtime::session::handle_streaming_request(
                    deferred,
                    request,
                    StreamingRequestContext {
                        tx,
                        message_id,
                        user_message: context.user_message,
                        current_model_snapshot: context.current_model_snapshot,
                        project_root: context.project_root,
                        abort_flag: context.abort_flag,
                        steering_queue: context.steering_queue,
                        steering_signal: context.steering_signal,
                        message_bus: context.message_bus,
                        subagent_tasks: subagent_tasks.clone(),
                        model_switch_cell: context.model_switch_cell,
                    },
                ).await
            }
            completed = &mut waiting => {
                if completed {
                    return None;
                }
                let _ = tx.send(StreamMessage::Done { message_id }).await;
                return Some(StreamingSessionResult::Completed);
            }
        };
        let result = match outcome {
            StreamingRequestOutcome::Continue => continue,
            StreamingRequestOutcome::Return => return Some(StreamingSessionResult::WorkerClosed),
            // Abort 的 handler 已发 Done，不能重复发布终态。
            StreamingRequestOutcome::Abort => return Some(StreamingSessionResult::Completed),
            StreamingRequestOutcome::Shutdown => StreamingSessionResult::Shutdown,
            StreamingRequestOutcome::Break => StreamingSessionResult::Completed,
        };
        let _ = tx.send(StreamMessage::Done { message_id }).await;
        return Some(result);
    }
}

pub struct StreamingSessionContext<'a> {
    pub tx: &'a mpsc::Sender<StreamMessage>,
    pub rx: &'a mut mpsc::Receiver<AgentRequest>,
    pub bus_rx: &'a mut Option<broadcast::Receiver<Message>>,
    pub message_id: u64,
    pub user_message: &'a str,
    pub images: &'a [crate::types::ImageAttachment],
    pub current_model_snapshot: &'a str,
    pub project_root: Option<&'a Path>,
    pub abort_flag: &'a Arc<std::sync::atomic::AtomicBool>,
    pub steering_queue: &'a Arc<AsyncMessageQueue<(u64, crate::types::UserInput)>>,
    pub steering_signal: &'a Arc<Notify>,
    pub message_bus: &'a Arc<crate::core::confirmation_bus::MessageBus>,
    /// 运行中切模型的交接槽（见 `agent::model_switch`）。
    pub model_switch_cell: &'a crate::agent::model_switch::ModelSwitchCell,
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
    mut context: StreamingSessionContext<'_>,
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

    let mut shutdown_requested = false;
    let mut abort_requested = false;
    // 流式失败的重试预算：init 失败与首 chunk 断流共用同一份
    // `StreamRetryPolicy`（参数、退避、文案全部来自 `llm::retry` 内核）。
    let mut retry_policy = crate::runtime::stream_retry::StreamRetryPolicy::default();
    'attempt: loop {
        // 先取得共享任务注册表，避免与流 future 对 agent 的可变借用重叠。
        let subagent_tasks = agent.runtime_subagent_tasks();
        match agent
            .process_user_input_stream(crate::types::UserInput {
                text: context.user_message.to_string(),
                images: context.images.to_vec(),
                intent: crate::types::InputIntent::Literal,
            })
            .await
        {
            Ok(mut stream) => {
                let mut chunk_count: u64 = 0;
                'streaming: loop {
                    tokio::select! {
                        Ok(msg) = async {
                            if shutdown_requested {
                                std::future::pending().await
                            } else if let Some(rx) = &mut *context.bus_rx {
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
                        maybe_req = context.rx.recv(), if !shutdown_requested && !abort_requested => {
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
                                    subagent_tasks: subagent_tasks.clone(),
                                    model_switch_cell: context.model_switch_cell,
                                },
                            ).await {
                                StreamingRequestOutcome::Continue => {}
                                StreamingRequestOutcome::Break => {
                                    break 'streaming;
                                }
                                StreamingRequestOutcome::Abort => {
                                    abort_requested = true;
                                    append_debug_log_line(
                                        "[Worker] Abort requested; draining cancelled agent stream",
                                    );
                                }
                                StreamingRequestOutcome::Return => {
                                    return StreamingSessionResult::WorkerClosed;
                                }
                                StreamingRequestOutcome::Shutdown => {
                                    shutdown_requested = true;
                                    append_debug_log_line(
                                        "[Worker] Shutdown requested; draining cancelled agent stream",
                                    );
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
                                    // 直接把未分类的原文抛给 UI 收场是旧缺陷：网络抖动
                                    // 本可以退避重来。交给 `StreamRetryPolicy` 决策 ——
                                    // 只有零 chunk（内容未落地）且可重试类别才重建流。
                                    match retry_policy.decide(&error_msg, chunk_count) {
                                        crate::runtime::stream_retry::StreamFailureDecision::Retry {
                                            kind,
                                            attempt,
                                            delay_ms,
                                            waiting_status,
                                            retrying_status,
                                        } => {
                                            let _ = context.tx.send(StreamMessage::RetryStatus {
                                                message_id: context.message_id,
                                                status: waiting_status.clone(),
                                            }).await;
                                            if let Some(result) = wait_for_stream_retry(
                                                &mut context, deferred, subagent_tasks.clone(), kind, attempt, delay_ms,
                                            ).await {
                                                return result;
                                            }
                                            append_debug_log_line(&format!(
                                                "[Worker] Stream retry: {}",
                                                retrying_status
                                            ));
                                            let _ = context.tx.send(StreamMessage::RetryStatus {
                                                message_id: context.message_id,
                                                status: retrying_status,
                                            }).await;
                                            continue 'attempt;
                                        }
                                        crate::runtime::stream_retry::StreamFailureDecision::GiveUp(diagnosis) => {
                                            let _ = context.tx.send(StreamMessage::Error {
                                                message_id: context.message_id,
                                                error: diagnosis,
                                            }).await;
                                            break 'attempt;
                                        }
                                    }
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
                break 'attempt;
            }
            Err(e) => {
                let error_msg = e.to_string();
                append_debug_log_line(&format!("[Worker] Agent stream init failed: {}", error_msg));
                match retry_policy.decide(&error_msg, 0) {
                    crate::runtime::stream_retry::StreamFailureDecision::Retry {
                        kind,
                        attempt,
                        delay_ms,
                        waiting_status,
                        retrying_status,
                    } => {
                        let _ = context
                            .tx
                            .send(StreamMessage::RetryStatus {
                                message_id: context.message_id,
                                status: waiting_status.clone(),
                            })
                            .await;
                        if let Some(result) = wait_for_stream_retry(
                            &mut context,
                            deferred,
                            subagent_tasks.clone(),
                            kind,
                            attempt,
                            delay_ms,
                        )
                        .await
                        {
                            return result;
                        }
                        append_debug_log_line(&format!(
                            "[Worker] Stream retry: {}",
                            retrying_status
                        ));
                        let _ = context
                            .tx
                            .send(StreamMessage::RetryStatus {
                                message_id: context.message_id,
                                status: retrying_status,
                            })
                            .await;
                        continue 'attempt;
                    }
                    crate::runtime::stream_retry::StreamFailureDecision::GiveUp(diagnosis) => {
                        let _ = context
                            .tx
                            .send(StreamMessage::Error {
                                message_id: context.message_id,
                                error: diagnosis,
                            })
                            .await;
                        break 'attempt;
                    }
                }
            }
        }
    }

    if shutdown_requested {
        StreamingSessionResult::Shutdown
    } else {
        StreamingSessionResult::Completed
    }
}
