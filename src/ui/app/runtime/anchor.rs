//! 退出会话时的白屏兜底：把这次会话的 id 锚到当前工作目录。

use super::*;

pub(super) fn exit_session_id(state: &ChatState, config: &crate::core::config::Config) -> String {
    // `/chat resume` 可以在同一个进程中多次切换 session，因此 UI 记录的实际已加载
    // filename 优先于启动 Config。启动时的 --resume 已使用 canonical filename 填入 Config。
    state.active_session_id.clone().unwrap_or_else(|| {
        if config.is_resume_session() {
            config.session_id().to_string()
        } else {
            format!("auto-{}", config.session_id())
        }
    })
}

/// 经 worker 保存完整快照后结束会话。worker 持有 canonical agent context，因此 UI
/// 只能提交 display transcript 和最新 provider usage，不能直接写 history-only JSON。
pub(super) async fn persist_session_on_exit(
    config: &Arc<crate::core::config::Config>,
    state: &ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Option<String> {
    // 事件循环异常退出时可能没收到切换回执。原生上下文不一定还是 UI 可见的
    // 那个，不能把两份不对应的状态写到同一个会话文件。
    if crate::ui::services::session_restore::awaiting_worker(state) {
        crate::utils::logging::append_debug_log_line(
            "[SESSION] Skipped exit snapshot: session restore was not acknowledged",
        );
        let _ = crate::commands::chat::request_worker_shutdown(agent_tx).await;
        return None;
    }

    // Exclude the transient welcome header entry from saved history.
    let history: Vec<_> = state
        .chat_history
        .iter()
        .filter(|entry| !entry.is_welcome)
        .cloned()
        .collect();

    if history.is_empty() {
        if let Err(error) = crate::commands::chat::request_worker_shutdown(agent_tx).await {
            crate::utils::logging::append_debug_log_line(&format!(
                "[SESSION] Failed to shut down an empty session: {}",
                error
            ));
        }
        return None;
    }

    let id = exit_session_id(state, config);
    match crate::commands::chat::request_session_save_then_shutdown(
        agent_tx,
        id.clone(),
        history,
        state.token_usage.clone(),
    )
    .await
    {
        Ok(()) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[SESSION] Saved complete session snapshot on exit: {}",
                id
            ));
            Some(id)
        }
        Err(error) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[SESSION] Failed to persist session on exit: {}",
                error
            ));
            None
        }
    }
}

/// 保存、Shutdown 回执与 worker 收尾共用一个退出预算，不限制工具审批等待。
///
/// 超时只取消本进程的 worker task，让调用方继续恢复终端；未收到完整回执时
/// 不返回保存成功的提示。正常路径仍由 worker 按 FIFO 保存后执行 SessionEnd。
pub(super) async fn finish_worker_on_exit(
    save: impl std::future::Future<Output = Option<String>>,
    mut worker: tokio::task::JoinHandle<()>,
    budget: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + budget;
    let saved_session_id = match tokio::time::timeout_at(deadline, save).await {
        Ok(saved) => saved,
        Err(_) => {
            crate::utils::logging::append_debug_log_line(
                "[SESSION] Exit budget exhausted; session save/shutdown was not acknowledged",
            );
            worker.abort();
            return None;
        }
    };
    match tokio::time::timeout_at(deadline, &mut worker).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::utils::logging::append_debug_log_line(&format!(
            "[Worker] Agent worker task failed during shutdown: {error}",
        )),
        Err(_) => {
            crate::utils::logging::append_debug_log_line(
                "[Worker] Exit budget exhausted while joining agent worker",
            );
            worker.abort();
        }
    }
    saved_session_id
}

/// 将启动阶段加载的持久化 display 状态装回新的 UI state。
///
/// 欢迎条目属于当前 TUI 进程，不能由旧快照替换；provider usage 则只恢复最近一次真实
/// 响应，缓存值还必须有显式遥测来源。
pub(super) fn restore_initial_session_state(
    state: &mut ChatState,
    initial_history: Vec<crate::types::ChatEntry>,
    initial_usage: Option<crate::types::StarUsage>,
    initial_session_id: Option<String>,
) {
    state.chat_history.extend(
        initial_history
            .into_iter()
            .filter(|entry| !entry.is_welcome),
    );
    state.active_session_id = initial_session_id;
    state.token_count = initial_usage
        .as_ref()
        .map(|usage| usage.total_tokens)
        .unwrap_or(0);
    state.cache_read_tokens = initial_usage
        .as_ref()
        .filter(|usage| usage.cache_telemetry_reported)
        .map(|usage| usage.cache_read_tokens as u64)
        .unwrap_or(0);
    state.cache_creation_tokens = initial_usage
        .as_ref()
        .filter(|usage| usage.cache_telemetry_reported)
        .map(|usage| usage.cache_creation_tokens as u64)
        .unwrap_or(0);
    state.token_usage = initial_usage;
    state.total_cost = state
        .chat_history
        .iter()
        .filter_map(|entry| entry.cost)
        .sum();
    state.response_costs.clear();
    state.response_models.clear();

    if state.chat_history.len() > 1 {
        state
            .chat_list_state
            .select(Some(state.chat_history.len() - 1));
    }
    state.clear_cache();
}
