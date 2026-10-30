//! 后台代理任务的状态更新（AgentTaskUpdate 消息）。
//!
use super::*;

/// 处理 Agent 任务更新：创建或更新 chat_history 中的 AgentTask / AgentGroup 条目
///
/// 条目归属只在该 task 的**第一条**更新时决定，之后一直复用
/// `AgentTaskInfo::entry_idx`。否则每条进度更新都会重新走一遍归属判断，
/// 把同一个 agent 反复追加进 `agent_task_ids`。
pub(super) fn handle_agent_task_update(state: &mut ChatState, args: AgentTaskUpdateArgs) {
    use crate::types::ChatEntryType;

    let prior = state.active_agent_tasks.get(&args.task_id);
    let started_at = prior
        .map(|i| i.started_at)
        .unwrap_or_else(std::time::Instant::now);
    // 首次进入**终态**时冻结耗时，之后不再随渲染增长。
    // `Background` 不算终态 —— 它只表示 Agent 工具交回了控制权，任务还在后台跑；
    // 早先按 `is_resolved` 判断会在启动那一刻就把 finished_at 钉住，
    // 于是后台代理的耗时永远显示 0s。
    let is_terminal = matches!(
        args.status,
        crate::types::AgentTaskStatus::Completed
            | crate::types::AgentTaskStatus::Failed
            | crate::types::AgentTaskStatus::Rejected
    );
    let finished_at = prior
        .and_then(|i| i.finished_at)
        .or_else(|| is_terminal.then(std::time::Instant::now));
    let mut sub_entries = prior.map(|i| i.sub_entries.clone()).unwrap_or_default();
    sub_entries.extend(args.new_sub_entries.iter().cloned());
    // 进度 chunk 未携带这些字段时沿用旧值，避免状态行闪回 "Initializing…"
    let last_tool_info = args
        .last_tool_info
        .clone()
        .or_else(|| prior.and_then(|i| i.last_tool_info.clone()));
    let name = args
        .name
        .clone()
        .or_else(|| prior.and_then(|i| i.name.clone()));
    let task_description = args
        .task_description
        .clone()
        .or_else(|| prior.and_then(|i| i.task_description.clone()));

    let entry_idx = match prior.map(|i| i.entry_idx) {
        Some(idx) if idx < state.chat_history.len() => idx,
        _ => attach_agent_task_entry(state, &args),
    };

    // 独立 AgentTask 条目从 entry 自身取数渲染；AgentGroup 从
    // active_agent_tasks 取数，无需回写。
    let is_standalone = state
        .chat_history
        .get(entry_idx)
        .map(|e| e.entry_type == ChatEntryType::AgentTask)
        .unwrap_or(false);
    if is_standalone {
        if let Some(entry) = state.chat_history.get_mut(entry_idx) {
            entry.agent_description = Some(args.description.clone());
            entry.agent_status = Some(args.status.clone());
            entry.agent_tool_use_count = Some(args.tool_use_count);
            entry.agent_tokens = Some(args.tokens);
            entry.agent_is_resolved = Some(args.is_resolved);
            entry.agent_is_error = Some(args.is_error);
            entry.agent_is_async = Some(args.is_async);
            entry.agent_last_tool_info = last_tool_info.clone();
            entry.agent_name = name.clone();
            entry.agent_task_description = task_description.clone();
            if !sub_entries.is_empty() {
                entry.agent_sub_entries = Some(sub_entries.clone());
            }
        }
    }
    state.rendered_cache.remove(&entry_idx);
    state.virtual_list.mark_dirty(entry_idx);

    state.active_agent_tasks.insert(
        args.task_id.clone(),
        crate::ui::state::store::AgentTaskInfo {
            task_id: args.task_id,
            agent_type: args.agent_type,
            description: args.description,
            status: args.status,
            tool_use_count: args.tool_use_count,
            tokens: args.tokens,
            is_async: args.is_async,
            is_resolved: args.is_resolved,
            is_error: args.is_error,
            last_tool_info,
            name,
            task_description,
            started_at,
            finished_at,
            sub_entries,
            entry_idx,
        },
    );

    // 自动跟随到底部
    if state.auto_follow {
        state.scroll = state
            .virtual_list
            .total_lines()
            .saturating_sub(state.last_chat_height as usize);
    }
}

/// 决定一个**新** Agent 落在哪个 chat_history 条目上，返回该条目索引。
///
/// - 时间窗内已有 AgentGroup → 加入该组
/// - 时间窗内是独立 AgentTask → 原地升级为 AgentGroup，两个 agent 一起渲染
/// - 否则 → 新建独立 AgentTask 条目
pub(super) fn attach_agent_task_entry(state: &mut ChatState, args: &AgentTaskUpdateArgs) -> usize {
    use crate::types::{ChatEntry, ChatEntryType};

    let now = chrono::Utc::now();
    let recent = state.chat_history.iter().rposition(|e| {
        matches!(
            e.entry_type,
            ChatEntryType::AgentTask | ChatEntryType::AgentGroup
        ) && now.signed_duration_since(e.timestamp).num_milliseconds() < AGENT_GROUP_WINDOW_MS
    });

    let Some(idx) = recent else {
        let mut entry = ChatEntry::agent_task(&args.task_id, &args.agent_type);
        entry.agent_description = Some(args.description.clone());
        state.chat_history.push(entry);
        let idx = state.chat_history.len() - 1;
        state.virtual_list.mark_dirty(idx);
        return idx;
    };

    if state.chat_history[idx].entry_type == ChatEntryType::AgentGroup {
        if let Some(ids) = state.chat_history[idx].agent_task_ids.as_mut() {
            if !ids.contains(&args.task_id) {
                ids.push(args.task_id.clone());
            }
        }
    } else {
        // 独立 AgentTask 遇到并发的第二个 agent → 升级成组
        let mut ids: Vec<String> = state.chat_history[idx]
            .agent_task_id
            .clone()
            .into_iter()
            .collect();
        ids.push(args.task_id.clone());
        state.chat_history[idx] = ChatEntry::agent_group(ids.clone());
        // 组内既有成员的 entry_idx 仍指向同一索引，无需迁移，
        // 但要确保它们不会再被当作独立条目回写。
        for id in ids {
            if let Some(info) = state.active_agent_tasks.get_mut(&id) {
                info.entry_idx = idx;
            }
        }
    }

    state.rendered_cache.remove(&idx);
    state.virtual_list.mark_dirty(idx);
    idx
}

pub(super) fn settle_response_cost(
    state: &mut ChatState,
    message_id: u64,
    assistant_idx: Option<usize>,
) {
    let response_cost = state.response_costs.remove(&message_id).unwrap_or(0.0);
    state.response_models.remove(&message_id);
    if response_cost == 0.0 {
        return;
    }

    let start_idx = state
        .message_start_indices
        .get(&message_id)
        .copied()
        .unwrap_or(0);
    let entry_idx = assistant_idx.and_then(|idx| {
        if state
            .chat_history
            .get(idx)
            .is_some_and(|entry| entry.entry_type == ChatEntryType::Assistant)
        {
            return Some(idx);
        }

        let end_idx = idx.saturating_add(1).min(state.chat_history.len());
        state.chat_history[start_idx.min(end_idx)..end_idx]
            .iter()
            .rposition(|entry| entry.entry_type == ChatEntryType::Assistant)
            .map(|offset| start_idx.min(end_idx) + offset)
    });

    state.total_cost += response_cost;
    if let Some(idx) = entry_idx {
        let entry = &mut state.chat_history[idx];
        entry.cost = Some(entry.cost.unwrap_or(0.0) + response_cost);
    }
}
