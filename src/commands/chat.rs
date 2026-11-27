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
        use crate::ui::services::session_list::{self, Target};
        session_list::start(
            ctx.state,
            Target::QuickMenu {
                from_palette: false,
            },
        );
        return Ok(());
    }

    crate::ui::services::session_restore::start(
        ctx.state,
        crate::ui::services::session_restore::Selection::Resume(args[0].clone()),
    )
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

/// Save the conversation before `/clear` removes it from the UI.
///
/// Claude Code keeps the previous conversation resumable; StarCode otherwise
/// only persists on exit, so clearing an unsaved session would lose it. The
/// label from `/clear <name>` becomes the session ID, while a bare `/clear`
/// reuses the active ID or creates a timestamped one.
pub(crate) async fn save_conversation_for_clear(
    state: &mut crate::ui::state::ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<Option<String>, String> {
    let history: Vec<ChatEntry> = state
        .chat_history
        .iter()
        .filter(|entry| !entry.is_welcome)
        .cloned()
        .collect();
    if history.is_empty() {
        return Ok(None);
    }

    let existing: std::collections::HashSet<String> = session_manager::list_sessions()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let active = state.active_session_id.clone();
    let requested = state
        .pending_clear_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let id = match requested {
        Some(name) => {
            let base = crate::commands::extended::sanitize_session_atom(name);
            if active.as_deref() == Some(base.as_str()) {
                base
            } else {
                crate::commands::extended::unique_session_id(&base, &existing, active.as_deref())
            }
        }
        None => match active {
            Some(id) => id,
            None => crate::commands::extended::unique_session_id(
                &format!("clear-{}", timestamp),
                &existing,
                None,
            ),
        },
    };

    let usage = state.token_usage.clone();
    request_session_save(agent_tx, id.clone(), history, usage).await?;
    Ok(Some(id))
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
pub(crate) fn restore_session_history(
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
    state.active_retry_status = None;
    state.cancelling_since = None;
    state.last_turn_failed = false;
    state.last_code_block_content = None;
    state.current_session_title = None;
    state.steered_user_messages.clear();
    state.processing_started_at = None;
    state.thinking_started_at = None;
    state.thinking_finished = None;
    state.active_message_id = None;
    state.stream_targets.clear();
    state.message_start_indices.clear();
    state.pending_tool_calls = None;
    state.pending_message_id = None;
    state.pending_confirmation = None;
    state.pending_confirmation_choice = 0;
    state.discard_pending_confirmation_queue(&crate::core::i18n::t1("ui.confirm.cancelled_clear"));
    state.clear_pending_confirmation();
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
        request_session_save, restore_session_history, save_conversation_for_clear, AgentRequest,
    };
    use crate::types::ChatEntry;
    use std::time::Instant;
    use tokio::sync::mpsc;

    #[test]
    fn restoring_history_resets_old_stream_and_tool_state() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        state.active_retry_status = Some("Retry 1/3".into());
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
        assert!(state.active_retry_status.is_none());
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
        let error = request_session_save(&agent_tx, "saved".into(), Vec::new(), None)
            .await
            .unwrap_err();
        assert_eq!(error, "Agent worker is unavailable; session was not saved.");
    }

    fn unique_label(prefix: &str) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{}-{}-{}", prefix, std::process::id(), nonce)
    }

    #[tokio::test]
    async fn clear_save_uses_requested_name_and_preserves_history_until_called() {
        let mut state = crate::ui::state::ChatState::new();
        state
            .chat_history
            .push(ChatEntry::user("previous question"));
        state
            .chat_history
            .push(ChatEntry::assistant("previous answer"));
        let label = unique_label("checkpoint");
        state.pending_clear_name = Some(label.clone());
        state.active_session_id = Some("active-session".to_string());
        state.token_usage = Some(crate::types::StarUsage {
            total_tokens: 42,
            ..Default::default()
        });

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            let request = agent_rx.recv().await.expect("save request");
            let AgentRequest::SaveSession {
                id,
                history,
                last_usage,
                response,
            } = request
            else {
                panic!("expected SaveSession");
            };
            assert!(id.starts_with("checkpoint-"));
            assert_ne!(id, "active-session");
            assert_eq!(history.len(), 2);
            assert_eq!(last_usage.map(|usage| usage.total_tokens), Some(42));
            response.send(Ok(())).await.unwrap();
        });

        let history_len = state.chat_history.len();
        let saved = save_conversation_for_clear(&mut state, &agent_tx)
            .await
            .unwrap();
        assert_eq!(saved.as_deref(), Some(label.as_str()));
        assert_eq!(
            state.chat_history.len(),
            history_len,
            "saving must not clear the UI"
        );
        assert_eq!(state.pending_clear_name.as_deref(), Some(label.as_str()));
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn clear_save_reuses_the_active_id_when_no_name_is_given() {
        let mut state = crate::ui::state::ChatState::new();
        state.chat_history.push(ChatEntry::user("active work"));
        state.active_session_id = Some("active-session".to_string());

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            let request = agent_rx.recv().await.expect("save request");
            let AgentRequest::SaveSession { id, response, .. } = request else {
                panic!("expected SaveSession");
            };
            assert_eq!(id, "active-session");
            response.send(Ok(())).await.unwrap();
        });

        let saved = save_conversation_for_clear(&mut state, &agent_tx)
            .await
            .unwrap();
        assert_eq!(saved.as_deref(), Some("active-session"));
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn clear_save_creates_a_timestamped_id_for_an_unnamed_session() {
        let mut state = crate::ui::state::ChatState::new();
        state.chat_history.push(ChatEntry::user("unsaved work"));

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            let request = agent_rx.recv().await.expect("save request");
            let AgentRequest::SaveSession { id, response, .. } = request else {
                panic!("expected SaveSession");
            };
            assert!(id.starts_with("clear-"), "got id: {id}");
            response.send(Ok(())).await.unwrap();
        });

        let saved = save_conversation_for_clear(&mut state, &agent_tx)
            .await
            .unwrap();
        assert!(saved.as_deref().unwrap().starts_with("clear-"));
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn clear_save_propagates_failure_and_keeps_the_conversation() {
        let mut state = crate::ui::state::ChatState::new();
        state
            .chat_history
            .push(ChatEntry::user("must not disappear"));

        let (agent_tx, mut agent_rx) = mpsc::channel(2);
        let worker = tokio::spawn(async move {
            let request = agent_rx.recv().await.expect("save request");
            let AgentRequest::SaveSession { response, .. } = request else {
                panic!("expected SaveSession");
            };
            response.send(Err("disk full".to_string())).await.unwrap();
        });

        let history_len = state.chat_history.len();
        let error = save_conversation_for_clear(&mut state, &agent_tx)
            .await
            .unwrap_err();
        assert!(error.contains("disk full"));
        assert_eq!(state.chat_history.len(), history_len);
        worker.await.unwrap();
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

/// `/resume` —— 开菜单挑一个会话恢复。
pub async fn resume_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    resume(ctx, &args).await
}

/// `/continue` —— 接着**最近**一次会话继续（对标 CC 的 `/continue` 与 `--continue`）。
///
/// 与 `/resume` 的分工：`/resume` 无参数时开菜单（要挑），`/continue` 无参数时直接
/// 接上最近的那个。此前 `/continue` 的描述写着"resume the current conversation"，
/// 实现却转手去开了菜单 —— 命令说明与行为不一致，用户按了等于没按。
pub async fn continue_cmd(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    crate::ui::services::session_restore::start(
        ctx.state,
        crate::ui::services::session_restore::Selection::Continue(args.into_iter().next()),
    )
}
