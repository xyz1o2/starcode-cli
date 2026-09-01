use crate::agent::StarAgent;
use crate::runtime::messages::{PendingCheckpointAction, StreamMessage};
use tokio::sync::mpsc;

pub async fn handle_checkpoint_action(
    agent: &mut StarAgent,
    tx: &mpsc::Sender<StreamMessage>,
    action: PendingCheckpointAction,
) {
    match action {
        PendingCheckpointAction::List { message_id } => {
            let _ = tx.send(StreamMessage::Start { message_id }).await;
            match agent.list_checkpoints().await {
                Ok(ids) => {
                    let content = if ids.is_empty() {
                        "(no checkpoints)".to_string()
                    } else {
                        format!("checkpoints:\n{}", ids.join("\n"))
                    };
                    let _ = tx
                        .send(StreamMessage::Content {
                            message_id,
                            content,
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
        PendingCheckpointAction::Restore { message_id, id } => {
            let _ = tx.send(StreamMessage::Start { message_id }).await;
            match agent.restore_checkpoint(&id).await {
                Ok((hist, summary)) => {
                    let _ = tx
                        .send(StreamMessage::RestoreCheckpointApplied {
                            message_id,
                            checkpoint_id: id,
                            summary: summary.clone(),
                            chat_history: hist,
                        })
                        .await;
                    let _ = tx
                        .send(StreamMessage::AssistantNote {
                            message_id,
                            content: format!("Status: checkpoint restored\n{}", summary),
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
    }
}
