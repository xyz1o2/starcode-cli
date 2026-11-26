//! 后台轮询通过事件通道交回 UI，只有 UI 线程修改会话状态。
//! 普通循环任务在内存中合并积压，空闲时非阻塞派发；项目路径解析留在后台。
//! 轮询和通道错误通过状态栏或文件日志暴露，不把投递失败记成成功或完成。

use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use crate::ui::state::ChatState;
use std::collections::{HashMap, VecDeque};
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
    /// `/loop`：带所属存储路径及收集开始时间，防止切目录或暂停后旧事件重新入队。
    LoopTasksDue {
        tasks: Vec<crate::core::loops::LoopTask>,
        scope: PathBuf,
        collected_at: std::time::Instant,
    },
    /// `/goal-loop`：本轮到期、需要推进一轮的目标循环任务。带轮询时的 cwd ——
    /// 目标上下文（`.star/goals.json`）要用同一个目录读。
    GoalLoopTasksDue(Vec<crate::core::loops::GoalLoopTask>, PathBuf),
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
            let collected_at = std::time::Instant::now();
            // 普通循环跟随 /cd；同一次收集固定目录，避免 scope 与存储错配。
            let task_cwd = crate::core::utils::paths::current_dir_cached().clone();
            // scope 解析可能查文件系统，留在后台，不在 UI tick 中执行。
            let scope = crate::core::loops::loops_file_path(&task_cwd);
            let event =
                match crate::core::loops::tick_and_collect_due_tasks(&task_cwd, now_ts).await {
                    Ok(tasks) => BackgroundEvent::LoopTasksDue {
                        tasks,
                        scope,
                        collected_at,
                    },
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

            // `/goal-loop` 与 `/loop` 同一个节拍器：每 tick 单独取一次到期任务。
            // 收口（goal 完成 / 迭代用尽）已在 tick 内落盘，这里只发待触发的。
            let goal_event = match crate::core::loops::tick_and_collect_due_goal_loops(
                &loop_cwd, now_ts,
            )
            .await
            {
                Ok(due) => BackgroundEvent::GoalLoopTasksDue(due, loop_cwd.clone()),
                Err(err) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[GOAL-LOOP] poll failed: {}",
                        err
                    ));
                    // 复用 None 语义的错误事件没有单独类型；记日志即可，
                    // 下一 tick 自然重试。
                    continue;
                }
            };
            if loop_events.send(goal_event).await.is_err() {
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
        BackgroundEvent::LoopTasksDue {
            tasks,
            scope,
            collected_at,
        } => apply_loop_tasks(state, tasks, scope, collected_at),
        BackgroundEvent::GoalLoopTasksDue(tasks, cwd) => {
            apply_goal_loop_tasks(state, agent_tx, tasks, &cwd).await
        }
        BackgroundEvent::LoopFailed(err) => {
            state.current_status_line =
                Some(crate::core::i18n::t1("ui.loop.poll_failed").replace("{error}", &err));
            true
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

/// 普通 `/loop` 的 UI 待发队列；同一存储路径和任务名最多保留一个到期快照。
///
/// 与 `/schedule` 相同，只有空闲且 worker 通道有容量才提交历史与消息 id；不走
/// 用户输入入口，避免自动任务被解读成 steering、slash command 或 `@file`。
#[derive(Default)]
pub(crate) struct LoopTaskQueue {
    pending: VecDeque<PendingLoopTask>,
    /// None 表示已暂停/删除；Some 表示仅接收恢复/新增之后开始收集的事件。
    admission: HashMap<(PathBuf, String), Option<std::time::Instant>>,
    scope: Option<(PathBuf, PathBuf)>,
    scope_resolution: Option<(PathBuf, tokio::sync::oneshot::Receiver<PathBuf>)>,
}

struct PendingLoopTask {
    task: crate::core::loops::LoopTask,
    scope: PathBuf,
    collected_at: std::time::Instant,
}

/// 命令成功暂停或删除后，撤销积压并屏蔽已经在后台事件通道里的旧任务。
pub(crate) fn suppress_loop_task(state: &mut ChatState, cwd: &Path, name: &str) {
    let scope = crate::core::loops::loops_file_path(cwd);
    let queue = &mut state.pending_loop_tasks;
    queue
        .pending
        .retain(|pending| pending.scope != scope || pending.task.name != name);
    queue.admission.insert((scope, name.to_owned()), None);
}

/// 命令成功新增或恢复后，仅允许新的轮询结果，不能复活暂停前的在途事件。
pub(crate) fn allow_loop_task(state: &mut ChatState, cwd: &Path, name: &str) {
    let scope = crate::core::loops::loops_file_path(cwd);
    state
        .pending_loop_tasks
        .admission
        .insert((scope, name.to_owned()), Some(std::time::Instant::now()));
}

fn apply_loop_tasks(
    state: &mut ChatState,
    due_tasks: Vec<crate::core::loops::LoopTask>,
    scope: PathBuf,
    collected_at: std::time::Instant,
) -> bool {
    let queue = &mut state.pending_loop_tasks;
    for task in due_tasks {
        if !task.enabled || task.interval_minutes == 0 {
            continue;
        }
        if let Some(allowed_after) = queue.admission.get(&(scope.clone(), task.name.clone())) {
            if !allowed_after.is_some_and(|after| collected_at > after) {
                continue;
            }
        }
        let pending = PendingLoopTask {
            task,
            scope: scope.clone(),
            collected_at,
        };
        if let Some(existing) = queue.pending.iter_mut().find(|existing| {
            existing.scope == pending.scope && existing.task.name == pending.task.name
        }) {
            if pending.collected_at >= existing.collected_at {
                *existing = pending;
            }
        } else {
            queue.pending.push_back(pending);
        }
    }
    // 入队不代表已经发送，不写「已触发」历史或状态。
    false
}

/// 项目解析包含 exists/向上查找；切目录后交给阻塞池，UI 只消费内存结果。
fn resolved_loop_scope(queue: &mut LoopTaskQueue, cwd: &Path) -> Option<PathBuf> {
    if let Some((owner, scope)) = &queue.scope {
        if owner == cwd {
            return Some(scope.clone());
        }
    }
    if !queue
        .scope_resolution
        .as_ref()
        .is_some_and(|(owner, _)| owner == cwd)
    {
        let owner = cwd.to_path_buf();
        let path = owner.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(crate::core::loops::loops_file_path(&path));
        });
        queue.scope_resolution = Some((owner, rx));
    }
    let (_, rx) = queue.scope_resolution.as_mut()?;
    match rx.try_recv() {
        Ok(scope) => {
            queue.scope_resolution = None;
            queue.scope = Some((cwd.to_path_buf(), scope.clone()));
            Some(scope)
        }
        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
            queue.scope_resolution = None;
            None
        }
    }
}

/// 每个 UI tick 重试积压；满通道和忙碌会话都只保留任务，不等待、不转向。
pub(crate) fn poll_loop_tasks(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> bool {
    if state.pending_loop_tasks.pending.is_empty() {
        return false;
    }
    let cwd = crate::core::utils::paths::current_dir_cached();
    let Some(scope) = resolved_loop_scope(&mut state.pending_loop_tasks, cwd) else {
        return false;
    };
    poll_loop_tasks_in_scope(state, agent_tx, &scope)
}

fn poll_loop_tasks_in_scope(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    scope: &Path,
) -> bool {
    state
        .pending_loop_tasks
        .pending
        .retain(|pending| pending.scope == scope);
    if state.pending_loop_tasks.pending.is_empty()
        || state.should_exit
        || state.pending_session_restore.is_some()
        || !state.pending_user_messages.is_empty()
        || state.approval_mode == crate::types::ApprovalMode::Plan
        || state.is_processing
        || state.is_streaming
        || state.cancelling_since.is_some()
        || state.is_awaiting_confirmation
        || state.pending_confirmation_entry_idx.is_some()
        || state
            .active_message_id
            .is_some_and(|id| !state.complete_task_message_ids.contains(&id))
        || state.network_offline
        || crate::core::offline::is_offline()
        || (state.current_model.trim().is_empty()
            && state
                .current_provider_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty()))
    {
        return false;
    }
    let permit = match agent_tx.try_reserve() {
        Ok(permit) => permit,
        Err(mpsc::error::TrySendError::Full(())) => return false,
        Err(mpsc::error::TrySendError::Closed(())) => {
            state.pending_loop_tasks.pending.clear();
            state.current_status_line = Some(crate::core::i18n::t1("ui.loop.worker_closed"));
            return true;
        }
    };
    let pending = state
        .pending_loop_tasks
        .pending
        .pop_front()
        .expect("pending loop task checked above");
    let task = pending.task;
    let message_id = state.next_message_id;
    let user_idx = state.chat_history.len();
    let message = format!(
        "[Scheduled Loop Task: {} | every {} minutes]\n{}",
        task.name, task.interval_minutes, task.prompt
    );
    permit.send(AgentRequest::SendMessage {
        message_id,
        message,
    });
    state.next_message_id += 1;
    state.active_message_id = Some(message_id);
    state.message_start_indices.insert(message_id, user_idx);
    state.chat_history.push(ChatEntry::user(format!(
        "[loop:{}] {}",
        task.name, task.prompt
    )));
    state.last_turn_failed = false;
    state.is_processing = true;
    state.processing_started_at = Some(std::time::Instant::now());
    state.processing_time_secs = 0;
    state.auto_follow = true;
    state.current_status_line =
        Some(crate::core::i18n::t1("ui.loop.dispatched").replace("{name}", &task.name));
    true
}

/// 触发到期的 `/goal-loop` 任务：把目标上下文包成一轮轮次的提示词交给 agent。
async fn apply_goal_loop_tasks(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    due_tasks: Vec<crate::core::loops::GoalLoopTask>,
    cwd: &Path,
) -> bool {
    let mut needs_redraw = false;

    for task in due_tasks {
        let message_id = state.next_message_id;
        state.next_message_id += 1;

        // 目标上下文用 tick 时的快照：goal 标题/进度可能在上轮之后被改过。
        let mut message = format!(
            "[Goal Loop: {} | iteration {}/{} | every {} minutes]\n",
            task.name, task.iterations_run, task.max_iterations, task.interval_minutes
        );
        match crate::core::loops::goal_prompt_for(cwd, &task.goal_id).await {
            Ok(Some(prompt)) => message.push_str(&prompt),
            Ok(None) => {
                message.push_str(&format!("## Current Goal: {}\n", task.goal_title));
            }
            Err(_) => {
                message.push_str(&format!("## Current Goal: {}\n", task.goal_title));
            }
        }
        message.push_str(
            "\nAdvance this goal one round: make the next concrete step, verify it, then \\n             stop cleanly. Do not start unrelated work.\n",
        );
        message.push_str(&format!(
            "Once the goal is fully achieved and verified, tell the user to run \\n             `/goal done {}` — the loop stops on its own next tick.",
            crate::commands::parity::short_id(&task.goal_id)
        ));

        state.chat_history.push(ChatEntry::user(format!(
            "[goal-loop:{}] (iteration {}/{}) {}",
            task.name, task.iterations_run, task.max_iterations, task.goal_title
        )));

        let _ = agent_tx
            .send(AgentRequest::SendMessage {
                message_id,
                message,
            })
            .await;

        state.current_status_line = Some(format!(
            "已触发 goal-loop: {} (第 {}/{} 轮)",
            task.name, task.iterations_run, task.max_iterations
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::loops::LoopTask;
    use std::time::{Duration, Instant};

    fn task(name: &str, prompt: &str) -> LoopTask {
        LoopTask {
            name: name.into(),
            prompt: prompt.into(),
            interval_minutes: 5,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            last_run_at: Some(1),
            next_run_at: 301,
            completion_count: 0,
            last_completed_at: None,
        }
    }

    fn scope() -> PathBuf {
        PathBuf::from("/virtual/project/.star/loops.json")
    }

    fn queue(state: &mut ChatState, tasks: Vec<LoopTask>) {
        apply_loop_tasks(state, tasks, scope(), Instant::now());
    }

    #[tokio::test]
    async fn loop_dispatch_commits_history_and_id_only_after_send() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        let message_id = state.next_message_id;
        let history_len = state.chat_history.len();
        let command_history = state.command_history.clone();
        queue(&mut state, vec![task("check", "/help @not-an-input-file")]);

        assert_eq!(state.chat_history.len(), history_len);
        assert_eq!(state.next_message_id, message_id);
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentRequest::SendMessage { message_id: id, message })
                if id == message_id && message == "[Scheduled Loop Task: check | every 5 minutes]\n/help @not-an-input-file"
        ));
        assert_eq!(state.next_message_id, message_id + 1);
        assert_eq!(state.chat_history.len(), history_len + 1);
        assert_eq!(
            state.chat_history[history_len].content,
            "[loop:check] /help @not-an-input-file"
        );
        assert_eq!(state.command_history, command_history);
        assert_eq!(state.active_message_id, Some(message_id));
        assert_eq!(
            state.message_start_indices.get(&message_id),
            Some(&history_len)
        );
        assert!(state.is_processing);
        assert!(state.processing_started_at.is_some());
        assert!(state.pending_loop_tasks.pending.is_empty());
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn full_worker_channel_retains_loop_without_blocking_or_success_history() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(AgentRequest::Abort).unwrap();
        let message_id = state.next_message_id;
        let history_len = state.chat_history.len();
        state.current_status_line = Some("unchanged".into());
        tokio::time::timeout(
            Duration::from_millis(100),
            apply_background_event(
                &mut state,
                &tx,
                BackgroundEvent::LoopTasksDue {
                    tasks: vec![task("check", "verify")],
                    scope: scope(),
                    collected_at: Instant::now(),
                },
            ),
        )
        .await
        .expect("applying a due event must not wait for worker channel capacity");
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert_eq!(state.next_message_id, message_id);
        assert_eq!(state.chat_history.len(), history_len);
        assert_eq!(state.current_status_line.as_deref(), Some("unchanged"));
        assert!(!state.is_processing);
        assert_eq!(state.pending_loop_tasks.pending.len(), 1);
        assert!(matches!(rx.try_recv(), Ok(AgentRequest::Abort)));
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentRequest::SendMessage { .. })
        ));
    }

    #[tokio::test]
    async fn closed_worker_channel_does_not_consume_loop_id_or_history() {
        let mut state = ChatState::test_fixture();
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let message_id = state.next_message_id;
        let history_len = state.chat_history.len();
        queue(&mut state, vec![task("check", "verify")]);
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert_eq!(state.next_message_id, message_id);
        assert_eq!(state.chat_history.len(), history_len);
        assert!(!state.is_processing);
        assert!(state.active_message_id.is_none());
        assert!(state.pending_loop_tasks.pending.is_empty());
        assert_eq!(
            state.current_status_line,
            Some(crate::core::i18n::t1("ui.loop.worker_closed"))
        );
    }

    #[tokio::test]
    async fn running_turn_coalesces_due_tasks_without_steering() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(2);
        state.is_processing = true;
        state.active_message_id = Some(999);
        queue(
            &mut state,
            vec![task("check", "old"), task("other", "second")],
        );
        queue(&mut state, vec![task("check", "latest")]);
        assert_eq!(state.pending_loop_tasks.pending.len(), 2);
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(rx.try_recv().is_err());
        assert!(state.steered_user_messages.is_empty());

        // watchdog 只放掉 UI spinner，不是 worker 真实结束的证明。
        state.is_processing = false;
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.complete_task_message_ids.insert(999);
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        let first_id = match rx.try_recv().unwrap() {
            AgentRequest::SendMessage {
                message_id,
                message,
            } => {
                assert!(message.ends_with("\nlatest"));
                message_id
            }
            _ => panic!("expected loop message"),
        };
        assert_eq!(state.pending_loop_tasks.pending.len(), 1);
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(rx.try_recv().is_err());
        state.is_processing = false;
        state.complete_task_message_ids.insert(first_id);
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(
            matches!(rx.try_recv(), Ok(AgentRequest::SendMessage { message, .. }) if message.ends_with("\nsecond"))
        );
    }

    #[tokio::test]
    async fn loop_waits_for_user_queue_plan_confirmation_and_offline_gates() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        queue(&mut state, vec![task("check", "verify")]);
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::command_eligible("/help".into()));
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.pending_user_messages.clear();
        state.approval_mode = crate::types::ApprovalMode::Plan;
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.approval_mode = crate::types::ApprovalMode::Default;
        state.is_awaiting_confirmation = true;
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.is_awaiting_confirmation = false;
        state.is_streaming = true;
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.is_streaming = false;
        state.cancelling_since = Some(Instant::now());
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.cancelling_since = None;
        state.network_offline = true;
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        state.network_offline = false;
        state.current_provider_id = Some("configured-provider".into());
        state.current_model.clear();
        assert!(!poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
        assert!(rx.try_recv().is_err());
        state.current_model = "selected-model".into();
        assert!(poll_loop_tasks_in_scope(&mut state, &tx, &scope()));
    }

    #[tokio::test]
    async fn loop_discards_tasks_from_another_project_before_dispatch() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        let message_id = state.next_message_id;
        queue(&mut state, vec![task("old-project", "must not run")]);
        assert!(!poll_loop_tasks_in_scope(
            &mut state,
            &tx,
            Path::new("/other/.star/loops.json")
        ));
        assert!(state.pending_loop_tasks.pending.is_empty());
        assert_eq!(state.next_message_id, message_id);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn paused_or_removed_loop_rejects_inflight_events_even_after_resume() {
        let root = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        let task_scope = crate::core::loops::loops_file_path(root.path());
        let old_collection = Instant::now();
        apply_loop_tasks(
            &mut state,
            vec![task("check", "old")],
            task_scope.clone(),
            old_collection,
        );
        suppress_loop_task(&mut state, root.path(), "check");
        assert!(state.pending_loop_tasks.pending.is_empty());
        apply_loop_tasks(
            &mut state,
            vec![task("check", "late event")],
            task_scope.clone(),
            old_collection,
        );
        assert!(state.pending_loop_tasks.pending.is_empty());
        allow_loop_task(&mut state, root.path(), "check");
        apply_loop_tasks(
            &mut state,
            vec![task("check", "still old")],
            task_scope.clone(),
            old_collection,
        );
        assert!(state.pending_loop_tasks.pending.is_empty());
        apply_loop_tasks(
            &mut state,
            vec![task("check", "new")],
            task_scope,
            Instant::now(),
        );
        assert_eq!(state.pending_loop_tasks.pending.len(), 1);
        assert_eq!(state.pending_loop_tasks.pending[0].task.prompt, "new");
    }

    #[tokio::test]
    async fn subdirectory_scope_matches_project_and_resolves_without_changing_cwd() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("src");
        std::fs::create_dir(root.path().join(".git")).unwrap();
        std::fs::create_dir(&child).unwrap();
        let task_scope = crate::core::loops::loops_file_path(root.path());
        let mut queue = LoopTaskQueue::default();
        let resolved = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(scope) = resolved_loop_scope(&mut queue, &child) {
                    break scope;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(resolved, task_scope);
        assert_eq!(
            resolved_loop_scope(&mut queue, &child),
            Some(task_scope.clone())
        );

        let mut state = ChatState::test_fixture();
        apply_loop_tasks(
            &mut state,
            vec![task("check", "verify")],
            task_scope.clone(),
            Instant::now(),
        );
        suppress_loop_task(&mut state, &child, "check");
        assert!(state.pending_loop_tasks.pending.is_empty());
        assert!(state
            .pending_loop_tasks
            .admission
            .contains_key(&(task_scope, "check".into())));
    }

    #[tokio::test]
    async fn loop_poll_failure_requests_redraw() {
        let mut state = ChatState::test_fixture();
        let (tx, _rx) = mpsc::channel(1);
        assert!(
            apply_background_event(
                &mut state,
                &tx,
                BackgroundEvent::LoopFailed("unreadable".into())
            )
            .await
        );
        assert_eq!(
            state.current_status_line,
            Some(crate::core::i18n::t1("ui.loop.poll_failed").replace("{error}", "unreadable"))
        );
    }
}
