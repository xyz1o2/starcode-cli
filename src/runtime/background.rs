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

/// 后台轮询产出的事件。
///
/// 轮询本身（读 `.star/loops.json`、远程 inbox）只做 I/O 并把结果送回来，**不碰
/// UI 状态** —— 应用要改 `ChatState` 的部分由 UI 循环调用 [`apply_background_event`]
/// 完成。
#[derive(Debug)]
pub enum BackgroundEvent {
    /// `/loop`：本轮到期、且不在「最近刚跑完」窗口内的任务。
    LoopTasksDue(Vec<crate::core::loops::LoopTask>),
    /// `/loop`：轮询本身失败（磁盘/网络）。
    LoopFailed(String),
    /// `/remote`：远程控制 inbox 里这一轮取到的请求。
    RemoteRequests(crate::core::remote::RemoteDrainResult),
    /// `/remote`：轮询本身失败。
    RemoteFailed(String),
}

/// 启动 `/loop` 与 `/remote` 的周期轮询任务。
///
/// 此前这两个轮询直接 `await` 在 UI 事件循环里（各带 500ms 超时，每秒一次）：
/// 慢盘或网络盘上就是周期性吞掉按键输入 —— 循环停在 `tokio::time::timeout` 里时，
/// 键盘线程投递的事件要等它返回才被处理。现在 I/O 在独立任务里做，UI 循环只在
/// 收到 [`BackgroundEvent`] 时回写状态。
pub fn spawn_schedule_pollers(
    cwd: PathBuf,
    events: mpsc::Sender<BackgroundEvent>,
    interval: std::time::Duration,
) {
    let loop_events = events.clone();
    let loop_cwd = cwd.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // 第一次 tick 立刻返回：startup 时先兜一次，之后按节拍。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now_ts = chrono::Utc::now().timestamp();
            let event =
                match crate::core::loops::tick_and_collect_due_tasks(&loop_cwd, now_ts).await {
                    Ok(due) => BackgroundEvent::LoopTasksDue(due),
                    Err(err) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[LOOP] poll failed: {}",
                            err
                        ));
                        BackgroundEvent::LoopFailed(err)
                    }
                };
            if loop_events.send(event).await.is_err() {
                // UI 循环没了（退出中），跟着收工。
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let event = match crate::core::remote::drain_requests(&cwd).await {
                Ok(drained) => BackgroundEvent::RemoteRequests(drained),
                Err(err) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[REMOTE] poll failed: {}",
                        err
                    ));
                    BackgroundEvent::RemoteFailed(err)
                }
            };
            if events.send(event).await.is_err() {
                break;
            }
        }
    });
}

/// 把一个后台事件应用到 UI 状态。返回是否需要重绘。
///
/// 这是「轮询结果」与「UI 状态」之间唯一的接缝：I/O 在别的任务里做，状态的写只
/// 在这里发生，因此 `ChatState` 不需要跨任务共享。
pub async fn apply_background_event(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    event: BackgroundEvent,
) -> bool {
    match event {
        BackgroundEvent::LoopTasksDue(tasks) => apply_loop_tasks(state, agent_tx, tasks).await,
        BackgroundEvent::LoopFailed(err) => {
            state.current_status_line = Some(format!("/loop 调度失败: {}", err));
            false
        }
        BackgroundEvent::RemoteRequests(drained) => {
            apply_remote_requests(state, agent_tx, drained).await
        }
        BackgroundEvent::RemoteFailed(err) => {
            state.current_status_line = Some(format!("/remote 消费失败: {}", err));
            false
        }
    }
}

/// 触发到期的 `/loop` 任务：往历史里插一条 user 消息并送给 agent。
async fn apply_loop_tasks(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    due_tasks: Vec<crate::core::loops::LoopTask>,
) -> bool {
    let mut needs_redraw = false;
    let now_ts = chrono::Utc::now().timestamp();

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
    needs_redraw
}

/// 把远程控制请求转成消息送进 agent。
async fn apply_remote_requests(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    drained: crate::core::remote::RemoteDrainResult,
) -> bool {
    let mut needs_redraw = false;

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

    needs_redraw
}
