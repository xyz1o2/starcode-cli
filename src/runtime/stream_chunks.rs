use crate::core::i18n;
use crate::runtime::messages::StreamMessage;
use crate::types::StreamingChunkType;
use tokio::sync::mpsc;

pub async fn handle_stream_chunk(
    tx: &mpsc::Sender<StreamMessage>,
    message_id: u64,
    chunk: crate::types::StreamingChunk,
) -> bool {
    match chunk.chunk_type {
        StreamingChunkType::Content => {
            if let Some(content) = chunk.content {
                let _ = tx
                    .send(StreamMessage::Content {
                        message_id,
                        content,
                    })
                    .await;
            }
        }
        StreamingChunkType::TextDelta => {
            if let Some(content) = chunk.content {
                let _ = tx
                    .send(StreamMessage::TextDelta {
                        message_id,
                        content,
                    })
                    .await;
            }
        }
        StreamingChunkType::ReasoningDelta => {
            if let Some(content) = chunk.content {
                let _ = tx
                    .send(StreamMessage::ReasoningDelta {
                        message_id,
                        content,
                    })
                    .await;
            }
        }
        StreamingChunkType::Thinking => {
            if let Some(content) = chunk.content {
                let _ = tx
                    .send(StreamMessage::Thinking {
                        message_id,
                        content,
                    })
                    .await;
            }
        }
        StreamingChunkType::AssistantNote => {
            if let Some(content) = chunk.content {
                let _ = tx
                    .send(StreamMessage::AssistantNote {
                        message_id,
                        content,
                    })
                    .await;
            }
        }
        StreamingChunkType::Trace => {
            if let Some(trace_event) = chunk.trace_event {
                let _ = tx
                    .send(StreamMessage::Trace {
                        message_id,
                        event: trace_event.event,
                        payload: trace_event.payload,
                    })
                    .await;
            }
        }
        StreamingChunkType::TokenCount => {
            if let Some(tokens) = chunk.token_count {
                let _ = tx
                    .send(StreamMessage::TokenCount {
                        message_id,
                        tokens,
                        usage: chunk.token_usage.clone(),
                    })
                    .await;
            }
        }
        StreamingChunkType::ToolCalls => {
            if let Some(tool_calls) = chunk.tool_calls {
                let _ = tx
                    .send(StreamMessage::ToolCalls {
                        message_id,
                        tool_calls,
                    })
                    .await;
                // Yield to let the UI render tool call entries before results arrive
                tokio::task::yield_now().await;
            }
        }
        StreamingChunkType::ToolResult => {
            if let (Some(tool_call), Some(tool_result)) = (chunk.tool_call, chunk.tool_result) {
                if tool_call.function.name == "Todo"
                    || tool_call.function.name == "todo"
                    || tool_call.function.name == "complete_task"
                {
                    let _ = tx.send(StreamMessage::ReloadTasks).await;
                }
                let _ = tx
                    .send(StreamMessage::ToolResult {
                        message_id,
                        tool_call,
                        tool_result,
                    })
                    .await;
                // Yield between tool results so the UI can render each one separately
                tokio::task::yield_now().await;
            }
        }
        StreamingChunkType::ToolProgress => {
            if let Some(progress) = chunk.progress {
                if progress.status == crate::types::ToolProgressStatus::Running {
                    let _ = tx
                        .send(StreamMessage::ToolOutput {
                            message_id,
                            tool_call_id: progress
                                .tool_call_id
                                .unwrap_or(progress.tool_name.clone()),
                            output: progress.message,
                        })
                        .await;
                } else {
                    let prefix = match progress.status {
                        crate::types::ToolProgressStatus::Starting => {
                            i18n::t("ui.status.starting", "状态：", "Status: ")
                        }
                        crate::types::ToolProgressStatus::Running => {
                            i18n::t("ui.status.running", "状态：", "Status: ")
                        }
                        crate::types::ToolProgressStatus::Completed => {
                            i18n::t("ui.status.done", "完成：", "Done: ")
                        }
                        crate::types::ToolProgressStatus::Failed => {
                            i18n::t("ui.status.error", "异常：", "Error: ")
                        }
                    };

                    let content = format!("{}{} {}", prefix, progress.tool_name, progress.message);
                    let _ = tx
                        .send(StreamMessage::AssistantNote {
                            message_id,
                            content,
                        })
                        .await;
                }
            }
        }
        StreamingChunkType::ToolConfirmation => {
            if let Some(confirmation) = chunk.confirmation {
                let tool_call_id = chunk
                    .tool_call
                    .map(|tc| tc.id)
                    .unwrap_or_else(|| confirmation.tool_name.clone());
                let _ = tx
                    .send(StreamMessage::ToolConfirmationRequest {
                        message_id,
                        tool_call_id,
                        confirmation,
                    })
                    .await;
            }
        }
        StreamingChunkType::Done => {
            let _ = tx.send(StreamMessage::Done { message_id }).await;
            return true;
        }
        StreamingChunkType::AgentTaskUpdate => {
            if let (Some(task_id), Some(agent_type), Some(description), Some(status)) = (
                chunk.agent_task_id,
                chunk.agent_type,
                chunk.agent_description,
                chunk.agent_status,
            ) {
                let _ = tx
                    .send(StreamMessage::AgentTaskUpdate {
                        message_id,
                        task_id,
                        agent_type,
                        description,
                        status,
                        tool_use_count: chunk.agent_tool_use_count.unwrap_or(0),
                        tokens: chunk.agent_tokens.unwrap_or(0),
                        is_async: chunk.agent_is_async.unwrap_or(false),
                        is_resolved: chunk.agent_is_resolved.unwrap_or(false),
                        is_error: chunk.agent_is_error.unwrap_or(false),
                        last_tool_info: chunk.agent_last_tool_info,
                        new_sub_entries: chunk.agent_new_sub_entries.unwrap_or_default(),
                    })
                    .await;
            }
        }
    }
    false
}
