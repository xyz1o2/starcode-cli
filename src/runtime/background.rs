use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use crate::ui::state::ChatState;
use std::path::Path;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Spawn background git status polling loop.
///
/// Architecture notes:
/// - Runs as a tokio task (not a blocking thread)
/// - Polls every 5 seconds
/// - Sends git status updates to UI via agent_tx
/// - Gracefully handles git errors (e.g. not a git repo)
/// - Uses exponential backoff on repeated failures to avoid log spam
pub fn spawn_git_status_loop(agent_tx: mpsc::Sender<AgentRequest>, cwd: PathBuf) {
    tokio::spawn(async move {
        let mut consecutive_errors = 0u32;
        const MAX_BACKOFF_SECS: u64 = 60;

        loop {
            match crate::core::services::git_service::get_git_status(&cwd).await {
                Some(status) => {
                    consecutive_errors = 0; // Reset on success
                    let mut status_str = status.branch;
                    let mut flags = Vec::new();
                    if status.has_staged {
                        flags.push("+");
                    }
                    if status.has_modified {
                        flags.push("*");
                    }
                    if status.has_untracked {
                        flags.push("?");
                    }
                    if !flags.is_empty() {
                        status_str = format!("{} [{}]", status_str, flags.join(""));
                    }
                    let _ = agent_tx
                        .send(AgentRequest::UpdateGitStatus(status_str))
                        .await;
                }
                None => {
                    consecutive_errors += 1;
                    // Log first few errors, then suppress to avoid log spam
                    if consecutive_errors <= 3 {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[GIT] Status query failed (attempt {})",
                            consecutive_errors
                        ));
                    }
                }
            }

            // Adaptive polling interval: normal = 5s, on errors = exponential backoff
            let sleep_secs = if consecutive_errors == 0 {
                5
            } else {
                (5u64 * (1 << (consecutive_errors - 1).min(4))).min(MAX_BACKOFF_SECS)
            };
            tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
        }
    });
}

/// Poll scheduled loop tasks.
pub async fn poll_loop_tasks(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    cwd: &Path,
) -> Result<bool, String> {
    let mut needs_redraw = false;
    let now_ts = chrono::Utc::now().timestamp();

    match crate::core::loops::tick_and_collect_due_tasks(cwd, now_ts).await {
        Ok(due_tasks) => {
            for task in due_tasks {
                // Skip if task was recently completed (within 50% of interval)
                if crate::core::loops::is_task_recently_completed(&task, now_ts) {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[LOOP] Skipping recently completed task: {}",
                        task.name
                    ));
                    continue;
                }

                let message_id = state.next_message_id;
                state.next_message_id += 1;

                state.chat_history.push(ChatEntry::user(format!(
                    "[loop:{}] {}",
                    task.name, task.prompt
                )));

                let message = format!(
                    "[Scheduled Loop Task: {} | every {} minutes]\n{}",
                    task.name, task.interval_minutes, task.prompt
                );
                let _ = agent_tx
                    .send(AgentRequest::SendMessage {
                        message_id,
                        message,
                    })
                    .await;

                state.current_status_line = Some(format!("已触发 /loop 任务: {}", task.name));
                needs_redraw = true;
            }
            Ok(needs_redraw)
        }
        Err(err) => {
            state.current_status_line = Some(format!("/loop 调度失败: {}", err));
            Ok(false)
        }
    }
}

/// Poll remote control inbox.
pub async fn poll_remote_requests(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    cwd: &Path,
) -> Result<bool, String> {
    let mut needs_redraw = false;

    match crate::core::remote::drain_requests(cwd).await {
        Ok(drained) => {
            for req in drained.accepted {
                let message_id = state.next_message_id;
                state.next_message_id += 1;

                let source = if req.source.trim().is_empty() {
                    "remote".to_string()
                } else {
                    req.source.trim().to_string()
                };

                state.chat_history.push(ChatEntry::user(format!(
                    "[remote:{}] {}",
                    source, req.message
                )));

                let message = format!("[Remote Control: {}]\n{}", source, req.message);
                let _ = agent_tx
                    .send(AgentRequest::SendMessage {
                        message_id,
                        message,
                    })
                    .await;

                state.current_status_line = Some(format!("已接收远程请求: {}", source));
                needs_redraw = true;
            }

            if !drained.rejected.is_empty() {
                state.current_status_line = Some(format!(
                    "/remote 丢弃了 {} 条非法请求",
                    drained.rejected.len()
                ));
                needs_redraw = true;
            }

            Ok(needs_redraw)
        }
        Err(err) => {
            state.current_status_line = Some(format!("/remote 消费失败: {}", err));
            Ok(false)
        }
    }
}
