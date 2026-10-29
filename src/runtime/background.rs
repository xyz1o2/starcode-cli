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
/// - Failure reasons are carried all the way into the log
///   (`Status query failed (attempt N): <reason>`)
/// - Permanently-not-a-repo failures stop the loop entirely (之前以 60s 的
///   背退节奏永远空问下去，实测 new-api 这种解压目录就是 3 连无原因失败)
/// - Other git errors back off exponentially to avoid log spam
pub fn spawn_git_status_loop(agent_tx: mpsc::Sender<AgentRequest>, _initial_cwd: PathBuf) {
    tokio::spawn(async move {
        let mut consecutive_errors = 0u32;
        const MAX_BACKOFF_SECS: u64 = 60;

        loop {
            // Follow the live process root so `/cd` refreshes the status line
            // without restarting the loop or recreating the UI.
            let cwd = crate::core::utils::paths::current_dir_cached().clone();
            match crate::core::services::git_service::get_git_status(&cwd).await {
                Ok(Some(status)) => {
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
                // git 成功但输出没有分支头：解析层的「不是工作树」语义。
                // 这不是错误，重试也拿不到状态 —— 打一条说明后停止轮询。
                Ok(None) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[GIT] {}: 不含分支头的 status 输出，判定非工作树，轮询停止",
                        cwd.display()
                    ));
                    return;
                }
                Err(reason) => {
                    // 永久性的「这里不是仓库」：重试显然没用，直接收工。
                    if crate::core::services::git_service::is_permanently_not_a_repo(&reason) {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[GIT] {}: {reason} —— 非 git 仓库，状态轮询停止",
                            cwd.display()
                        ));
                        return;
                    }
                    consecutive_errors += 1;
                    // Log first few errors, then suppress to avoid log spam
                    if consecutive_errors <= 3 {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[GIT] Status query failed (attempt {}): {reason}",
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
    ///
    /// 带上 cwd —— 响应必须写回**轮询时用的那个**目录。轮询用 `ui_cwd`
    /// （随 `/cd` 变），若写回进程启动目录，`/cd` 之后外部 harness 就读不到
    /// outbox 了。
    RemoteRequests(crate::core::remote::RemoteDrainResult, std::path::PathBuf),
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

    let remote_cwd = cwd.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let event = match crate::core::remote::drain_requests(&remote_cwd).await {
                Ok(drained) => BackgroundEvent::RemoteRequests(drained, remote_cwd.clone()),
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
        BackgroundEvent::RemoteRequests(drained, cwd) => {
            apply_remote_requests(state, agent_tx, drained, &cwd).await
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

/// 把远程控制请求转成对应的 agent 动作。
///
/// # 协议
///
/// `crate::core::remote` 定义的四个动作：
///
/// - `send_message` —— 往会话里投一条消息（v1 就有的那个）
/// - `approve_tool` —— 批准 / 拒绝一次挂起的工具确认
/// - `cancel` —— 中止当前回合
/// - `ping` —— 探活，回一条 `pong`，不动会话
///
/// 每个动作都会往 `.star/remote/outbox.jsonl` 写一条响应，把 `request_id`
/// 原样带回 —— 外部 harness 靠它把响应和请求配对。
async fn apply_remote_requests(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    drained: crate::core::remote::RemoteDrainResult,
    cwd: &std::path::Path,
) -> bool {
    use crate::core::remote::{
        RemoteResponse, ACTION_APPROVE_TOOL, ACTION_CANCEL, ACTION_PING, ACTION_SEND_MESSAGE,
    };

    let mut needs_redraw = false;
    // cwd 由调用方给 —— **必须**和轮询时用的是同一个。轮询用 `ui_cwd`（随 `/cd`
    // 变），进程启动目录是另一回事：用后者写响应，`/cd` 之后外部 harness 就
    // 读不到 outbox 了。

    for req in drained.accepted {
        let source = if req.source.trim().is_empty() {
            "remote".to_string()
        } else {
            req.source.trim().to_string()
        };

        let response = match req.action.as_str() {
            ACTION_SEND_MESSAGE => {
                let message_id = state.next_message_id;
                state.next_message_id += 1;

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
                RemoteResponse::ok(
                    &req.protocol,
                    req.request_id.clone(),
                    ACTION_SEND_MESSAGE,
                    "已接收",
                )
            }
            ACTION_APPROVE_TOOL => {
                let approved = req.approved.unwrap_or(true);
                apply_remote_tool_decision(
                    state,
                    agent_tx,
                    &req.message,
                    approved,
                    &req.protocol,
                    req.request_id.clone(),
                )
                .await
            }
            ACTION_CANCEL => {
                let _ = agent_tx.send(AgentRequest::Abort).await;
                state.current_status_line = Some(format!("远程请求中止当前回合: {}", source));
                needs_redraw = true;
                RemoteResponse::ok(
                    &req.protocol,
                    req.request_id.clone(),
                    ACTION_CANCEL,
                    "已发送中止",
                )
            }
            ACTION_PING => {
                RemoteResponse::ok(&req.protocol, req.request_id.clone(), ACTION_PING, "pong")
            }
            // `drain_requests` 已经过滤过未知动作，走到这里说明协议层和执行层
            // 不同步了 —— 回一条 error 而不是静默丢弃，让外部 harness 看得见。
            other => RemoteResponse::error(
                &req.protocol,
                req.request_id.clone(),
                other,
                "执行层不认识这个动作",
            ),
        };

        let _ = crate::core::remote::append_response(cwd, &response).await;
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

/// 外部 harness 批准 / 拒绝一次挂起的工具确认。
///
/// # 为什么按 correlation id 匹配
///
/// 确认请求的 id 是 bus 的 `correlation_id`（`state.pending_tool_call_id` 和
/// `pending_confirmation_queue` 里每一项的 `tool_call_id` 都是它）。外部 harness
/// 看到的就是这个 id —— 它同时出现在 `StreamMessage::ToolConfirmationRequest`
/// 和 transcript 的事件里，所以两边对得上。
///
/// # 为什么走 `AgentRequest::ConfirmTool` 而不是直接发 bus 消息
///
/// `ConfirmTool` 是 UI 用户按键走的同一条路（`ui/events/input.rs:1469`），
/// worker 侧统一处理。直接往 bus 发消息会绕过 worker 的状态清理（确认卡不会
/// 从 UI 上收掉），留下一张永远等不到回答的卡。
async fn apply_remote_tool_decision(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
    approved: bool,
    protocol: &str,
    request_id: Option<String>,
) -> crate::core::remote::RemoteResponse {
    use crate::core::remote::RemoteResponse;
    use crate::types::ToolConfirmationOutcome;

    let tool_call_id = tool_call_id.trim();
    if tool_call_id.is_empty() {
        return RemoteResponse::error(
            protocol,
            request_id,
            crate::core::remote::ACTION_APPROVE_TOOL,
            "缺少 tool_call_id",
        );
    }

    // 确认请求是不是真的挂着 —— 不挂着的 id 批准了也没用。
    let known = state.pending_tool_call_id.as_deref() == Some(tool_call_id)
        || state
            .pending_confirmation_queue
            .iter()
            .any(|q| q.tool_call_id == tool_call_id);
    if !known {
        return RemoteResponse::error(
            protocol,
            request_id,
            crate::core::remote::ACTION_APPROVE_TOOL,
            format!("没有挂起的确认请求: {tool_call_id}"),
        );
    }

    let outcome = if approved {
        ToolConfirmationOutcome::ProceedOnce
    } else {
        ToolConfirmationOutcome::Cancel
    };
    let _ = agent_tx
        .send(AgentRequest::ConfirmTool {
            tool_call_id: tool_call_id.to_string(),
            outcome,
            feedback: None,
        })
        .await;

    state.current_status_line = Some(format!(
        "远程{}了工具确认: {}",
        if approved { "批准" } else { "拒绝" },
        tool_call_id
    ));
    RemoteResponse::ok(
        protocol,
        request_id,
        crate::core::remote::ACTION_APPROVE_TOOL,
        if approved { "已批准" } else { "已拒绝" },
    )
}
