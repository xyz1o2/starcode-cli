use crate::commands::execution::{CommandContext, CommandResult};
use crate::runtime::messages::AgentRequest;
use crate::types::{ChatEntry, StarUsage};
use crate::utils::session_manager;
use chrono::Local;
use tokio::sync::mpsc;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        return list(ctx, &[]).await;
    }

    match args[0].as_str() {
        "save" => save(ctx, &args[1..]).await,
        "resume" | "load" => resume(ctx, &args[1..]).await,
        "list" | "ListDir" => list(ctx, &args[1..]).await,
        "delete" | "rm" => delete(ctx, &args[1..]).await,
        "share" => share(ctx, &args[1..]).await,
        _ => Err(format!("Unknown subcommand: {}", args[0])),
    }
}

/// 导出当前会话为可分享的文本文件（markdown 格式）
async fn share(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let label = if args.is_empty() {
        Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        args[0].clone()
    };

    let mut md = String::from("# Session Export\n\n");
    for entry in &ctx.state.chat_history {
        let role = match entry.entry_type {
            crate::types::ChatEntryType::User => "user",
            crate::types::ChatEntryType::Assistant => "assistant",
            crate::types::ChatEntryType::SystemMessage => "system",
            crate::types::ChatEntryType::ErrorMessage => "error",
            crate::types::ChatEntryType::CompactSummary => "summary",
            _ => continue,
        };
        md.push_str(&format!("**{}**:\n\n{}\n\n---\n\n", role, entry.content));
    }

    let dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".star")
        .join("exports");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create export dir: {}", e))?;
    }
    let file_name = format!("session_{}.md", label);
    let path = dir.join(&file_name);
    std::fs::write(&path, md).map_err(|e| e.to_string())?;

    ctx.state.current_status_line = Some(format!(
        "Session exported to {} ({} messages)",
        path.display(),
        ctx.state.chat_history.len()
    ));
    Ok(())
}

async fn save(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    let tag = if args.is_empty() {
        Local::now().format("%Y%m%d_%H%M%S").to_string()
    } else {
        args[0].clone()
    };

    request_session_save(
        ctx.agent_tx,
        tag.clone(),
        ctx.state
            .chat_history
            .iter()
            .filter(|entry| !entry.is_welcome)
            .cloned()
            .collect(),
        ctx.state.token_usage.clone(),
    )
    .await?;

    ctx.state.current_status_line = Some(format!("Session saved as '{}'", tag));
    Ok(())
}

async fn resume(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    if args.is_empty() {
        // 无参数：打开交互式会话选择菜单，而不是静默用最近会话替换当前对话
        let sessions = crate::utils::session_manager::list_session_summaries()
            .await
            .unwrap_or_default();
        if sessions.is_empty() {
            ctx.state.current_status_line = Some("No saved sessions yet.".to_string());
            return Ok(());
        }
        ctx.state.close_palette();
        ctx.state.show_session_menu = true;
        ctx.state.available_sessions = sessions;
        ctx.state.selected_session_index = 0;
        return Ok(());
    }

    let tag = &args[0];
    let session = session_manager::load_session(tag)
        .await
        .map_err(|e| e.to_string())?;
    let messages = session_manager::agent_messages_for_restore(&session);
    let pending_local_context = session.pending_local_context.clone().unwrap_or_default();
    request_session_restore(ctx.agent_tx, messages, pending_local_context).await?;
    restore_session_history(
        ctx.state,
        session.chat_history,
        session.last_usage,
        &format!("'{}'", tag),
    );
    ctx.state.active_session_id = Some(tag.clone());
    Ok(())
}

/// 将完整快照交给 worker；等待应答避免 UI 报成功但 session 文件并不存在。
pub(crate) async fn request_session_save(
    agent_tx: &mpsc::Sender<AgentRequest>,
    id: String,
    history: Vec<ChatEntry>,
    last_usage: Option<StarUsage>,
) -> Result<(), String> {
    let (response_tx, mut response_rx) = mpsc::channel(1);
    agent_tx
        .send(AgentRequest::SaveSession {
            id,
            history,
            last_usage,
            response: response_tx,
        })
        .await
        .map_err(|_| "Agent worker is unavailable; session was not saved.".to_string())?;
    wait_for_session_response(&mut response_rx, "save").await
}

/// 只在 worker 成功替换原生上下文后，调用方才能替换可见 transcript。
pub(crate) async fn request_session_restore(
    agent_tx: &mpsc::Sender<AgentRequest>,
    messages: Vec<crate::types::StarMessage>,
    pending_local_context: Vec<String>,
) -> Result<(), String> {
    let (response_tx, mut response_rx) = mpsc::channel(1);
    agent_tx
        .send(AgentRequest::RestoreSession {
            messages,
            pending_local_context,
            response: response_tx,
        })
        .await
        .map_err(|_| "Agent worker is unavailable; session was not restored.".to_string())?;
    wait_for_session_response(&mut response_rx, "restore").await
}

pub(crate) async fn request_session_save_then_shutdown(
    agent_tx: &mpsc::Sender<AgentRequest>,
    id: String,
    history: Vec<ChatEntry>,
    last_usage: Option<StarUsage>,
) -> Result<(), String> {
    let (save_response_tx, mut save_response_rx) = mpsc::channel(1);
    agent_tx
        .send(AgentRequest::SaveSession {
            id,
            history,
            last_usage,
            response: save_response_tx,
        })
        .await
        .map_err(|_| "Agent worker is unavailable; session was not saved.".to_string())?;

    let (shutdown_response_tx, mut shutdown_response_rx) = mpsc::channel(1);
    agent_tx
        .send(AgentRequest::Shutdown {
            response: shutdown_response_tx,
        })
        .await
        .map_err(|_| "Agent worker is unavailable; shutdown was not completed.".to_string())?;

    // 两个请求已经按 FIFO 入队。即使保存失败，也必须等 shutdown 回执，避免 UI
    // 在 SessionEnd hooks 尚未完成时提前离开终端。
    let save_result = wait_for_session_response(&mut save_response_rx, "save").await;
    let shutdown_result = wait_for_session_response(&mut shutdown_response_rx, "shutdown").await;
    match (save_result, shutdown_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(save_error), Ok(())) => Err(save_error),
        (Ok(()), Err(shutdown_error)) => Err(shutdown_error),
        (Err(save_error), Err(shutdown_error)) => Err(format!(
            "Session save failed: {}; shutdown failed: {}",
            save_error, shutdown_error
        )),
    }
}

/// 请求 worker 结束，并等待 SessionEnd hooks 和插件 shutdown 完成。
pub(crate) async fn request_worker_shutdown(
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), String> {
    let (response_tx, mut response_rx) = mpsc::channel(1);
    agent_tx
        .send(AgentRequest::Shutdown {
            response: response_tx,
        })
        .await
        .map_err(|_| "Agent worker is unavailable; shutdown was not completed.".to_string())?;
    wait_for_session_response(&mut response_rx, "shutdown").await
}

async fn wait_for_session_response(
    response_rx: &mut mpsc::Receiver<Result<(), String>>,
    operation: &str,
) -> Result<(), String> {
    match response_rx.recv().await {
        Some(result) => result,
        None => Err(format!(
            "Agent worker closed before session {} completed.",
            operation
        )),
    }
}

/// 用磁盘会话替换可见历史，并清除依附旧历史的 UI 瞬态状态。
fn restore_session_history(
    state: &mut crate::ui::state::ChatState,
    restored_history: Vec<ChatEntry>,
    last_usage: Option<StarUsage>,
    resumed_label: &str,
) {
    let restored_len = restored_history.len();

    state.chat_history = restored_history;
    state.clear_cache();
    state.last_item_heights.clear();
    state.total_rendered_lines = 0;
    state.scroll = 0;
    state.auto_follow = true;
    state.is_streaming = false;
    state.is_processing = false;
    state.processing_started_at = None;
    state.thinking_started_at = None;
    state.active_message_id = None;
    state.stream_targets.clear();
    state.message_start_indices.clear();
    state.pending_tool_calls = None;
    state.pending_message_id = None;
    state.pending_confirmation = None;
    state.pending_confirmation_entry_idx = None;
    state.pending_confirmation_choice = 0;
    state.pending_confirmation_feedback.clear();
    state.is_awaiting_confirmation = false;
    state.pending_tool_call_id = None;
    state.current_tool_name = None;
    state.tool_started_at.clear();
    state.tool_call_args_cache.clear();
    state.tool_call_transcript_written.clear();
    state.token_count = last_usage
        .as_ref()
        .map(|usage| usage.total_tokens)
        .unwrap_or(0);
    state.token_usage = last_usage;
    state.cache_read_tokens = state
        .token_usage
        .as_ref()
        .filter(|usage| usage.cache_telemetry_reported)
        .map(|usage| usage.cache_read_tokens as u64)
        .unwrap_or(0);
    state.cache_creation_tokens = state
        .token_usage
        .as_ref()
        .filter(|usage| usage.cache_telemetry_reported)
        .map(|usage| usage.cache_creation_tokens as u64)
        .unwrap_or(0);
    state.total_cost = state
        .chat_history
        .iter()
        .filter_map(|entry| entry.cost)
        .sum();
    state.response_costs.clear();
    state.response_models.clear();
    state.complete_task_message_ids.clear();
    state.auto_continued_message_ids.clear();
    state.cache_warning_shown.clear();

    if restored_len == 0 {
        state.chat_list_state.select(None);
    } else {
        state.chat_list_state.select(Some(restored_len - 1));
    }

    state.current_status_line = Some(format!(
        "Resumed session {} ({} messages)",
        resumed_label, restored_len
    ));
}

#[cfg(test)]
mod tests {
    use super::{
        request_session_restore, request_session_save, restore_session_history, AgentRequest,
    };
    use crate::types::{ChatEntry, StarMessage};
    use std::time::Instant;
    use tokio::sync::mpsc;

    #[test]
    fn restoring_history_resets_old_stream_and_tool_state() {
        let mut state = crate::ui::state::ChatState::new();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(7);
        state.stream_targets.insert(7, 3);
        state.message_start_indices.insert(7, 1);
        state.current_tool_name = Some("Bash".to_string());
        state
            .tool_started_at
            .insert("call_1".to_string(), Instant::now());
        state.pending_confirmation = Some("confirm".to_string());
        state.pending_confirmation_entry_idx = Some(2);
        state.is_awaiting_confirmation = true;
        state.cache_read_tokens = 42;
        state.cache_creation_tokens = 24;

        restore_session_history(
            &mut state,
            vec![ChatEntry::user("restored"), ChatEntry::assistant("answer")],
            None,
            "'saved'",
        );
        state.active_session_id = Some("saved".to_string());

        assert_eq!(state.active_session_id.as_deref(), Some("saved"));
        assert_eq!(state.chat_history.len(), 2);
        assert!(!state.is_streaming);
        assert!(!state.is_processing);
        assert!(state.active_message_id.is_none());
        assert!(state.stream_targets.is_empty());
        assert!(state.message_start_indices.is_empty());
        assert!(state.current_tool_name.is_none());
        assert!(state.tool_started_at.is_empty());
        assert!(state.pending_confirmation.is_none());
        assert!(state.pending_confirmation_entry_idx.is_none());
        assert!(!state.is_awaiting_confirmation);
        assert_eq!(state.cache_read_tokens, 0);
        assert_eq!(state.cache_creation_tokens, 0);
        assert_eq!(state.chat_list_state.selected(), Some(1));
        assert_eq!(
            state.current_status_line.as_deref(),
            Some("Resumed session 'saved' (2 messages)")
        );
    }

    #[test]
    fn restoring_history_restores_only_reported_cache_telemetry_and_persisted_cost() {
        let mut state = crate::ui::state::ChatState::new();
        state.response_costs.insert(9, 4.0);
        state.response_models.insert(9, "stale-model".to_string());
        let usage = crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            cache_read_tokens: 70,
            cache_creation_tokens: 10,
            cache_telemetry_reported: true,
        };
        let mut first_answer = ChatEntry::assistant("first answer");
        first_answer.cost = Some(0.125);
        let mut second_answer = ChatEntry::assistant("second answer");
        second_answer.cost = Some(1.875);

        restore_session_history(
            &mut state,
            vec![ChatEntry::user("restored"), first_answer, second_answer],
            Some(usage),
            "'saved'",
        );

        assert_eq!(state.token_count, 120);
        let restored = state.token_usage.as_ref().expect("usage is restored");
        assert_eq!(restored.prompt_tokens, 100);
        assert_eq!(restored.cache_read_tokens, 70);
        assert_eq!(state.cache_read_tokens, 70);
        assert_eq!(state.cache_creation_tokens, 10);
        assert_eq!(state.total_cost, 2.0);
        assert!(state.response_costs.is_empty());
        assert!(state.response_models.is_empty());

        restore_session_history(
            &mut state,
            Vec::new(),
            Some(crate::types::StarUsage {
                total_tokens: 50,
                cache_read_tokens: 999,
                cache_creation_tokens: 999,
                cache_telemetry_reported: false,
                ..Default::default()
            }),
            "'legacy'",
        );
        assert_eq!(state.token_count, 50);
        assert_eq!(state.cache_read_tokens, 0);
        assert_eq!(state.cache_creation_tokens, 0);
        assert_eq!(state.total_cost, 0.0);
        assert_eq!(state.chat_list_state.selected(), None);
    }

    #[tokio::test]
    async fn session_worker_helpers_require_an_acknowledgement() {
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let save_task = tokio::spawn({
            let agent_tx = agent_tx.clone();
            async move { request_session_save(&agent_tx, "saved".to_string(), Vec::new(), None).await }
        });
        let request = agent_rx.recv().await.expect("save request");
        let AgentRequest::SaveSession { response, .. } = request else {
            panic!("expected SaveSession");
        };
        response.send(Ok(())).await.unwrap();
        assert!(save_task.await.unwrap().is_ok());

        drop(agent_rx);
        let error = request_session_restore(&agent_tx, Vec::new(), Vec::new())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "Agent worker is unavailable; session was not restored."
        );
    }

    #[tokio::test]
    async fn restore_request_carries_pending_local_context() {
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let restore_task = tokio::spawn({
            let agent_tx = agent_tx.clone();
            async move {
                request_session_restore(
                    &agent_tx,
                    vec![StarMessage::user("restored context")],
                    vec!["local command output".to_string()],
                )
                .await
            }
        });

        let request = agent_rx.recv().await.expect("restore request");
        let AgentRequest::RestoreSession {
            messages,
            pending_local_context,
            response,
        } = request
        else {
            panic!("expected RestoreSession");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("restored context"));
        assert_eq!(pending_local_context, vec!["local command output"]);
        response.send(Ok(())).await.unwrap();
        assert!(restore_task.await.unwrap().is_ok());
    }
}

async fn list(ctx: CommandContext<'_>, _args: &[String]) -> CommandResult {
    let summaries = session_manager::list_session_summaries()
        .await
        .map_err(|e| e.to_string())?;

    let content = if summaries.is_empty() {
        "No saved sessions found.".to_string()
    } else {
        let mut lines = format!("Saved sessions ({}):\n", summaries.len());
        for s in &summaries {
            lines.push_str(&format!("• {}\n  {} · {}\n", s.id, s.title, s.subtitle));
        }
        lines
    };

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}

async fn delete(ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    if args.is_empty() {
        return Err("Usage: /chat delete <tag>".to_string());
    }
    let tag = &args[0];

    session_manager::delete_session(tag)
        .await
        .map_err(|e| e.to_string())?;

    ctx.state.current_status_line = Some(format!("Deleted session '{}'", tag));
    Ok(())
}

pub async fn resume_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    resume(ctx, &args).await
}
