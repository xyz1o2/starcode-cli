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
    // 工具的 async_launched 结果可能晚于子代理终态到达 UI。终态是单向的：
    // 迟到的 Background / Running 快照不能把已经完成的任务重新挂回选择器。
    let incoming_terminal = matches!(
        args.status,
        crate::types::AgentTaskStatus::Completed
            | crate::types::AgentTaskStatus::Failed
            | crate::types::AgentTaskStatus::Rejected
            | crate::types::AgentTaskStatus::Killed
    );
    let first_terminal = incoming_terminal && prior.is_none_or(|info| info.finished_at.is_none());
    if prior.is_some_and(|info| info.finished_at.is_some()) && !incoming_terminal {
        return;
    }
    let started_at = prior
        .map(|i| i.started_at)
        .unwrap_or_else(std::time::Instant::now);
    // 首次进入**终态**时冻结耗时，之后不再随渲染增长。
    // `Background` 不算终态 —— 它只表示 Agent 工具交回了控制权，任务还在后台跑；
    // 早先按 `is_resolved` 判断会在启动那一刻就把 finished_at 钉住，
    // 于是后台代理的耗时永远显示 0s。
    let finished_at = prior
        .and_then(|i| i.finished_at)
        .or_else(|| incoming_terminal.then(std::time::Instant::now));
    let mut sub_entries = prior.map(|i| i.sub_entries.clone()).unwrap_or_default();
    merge_sub_entries(&mut sub_entries, &args.new_sub_entries);
    // 进度 chunk 未携带这些字段时沿用旧值，避免状态行闪回 "Initializing…"
    // 失败终态缺正文时只保留之前的失败原因，不能把成功的工具活动当作错误。
    let last_tool_info = args.last_tool_info.clone().or_else(|| {
        prior
            .filter(|info| {
                !matches!(args.status, crate::types::AgentTaskStatus::Failed)
                    || matches!(info.status, crate::types::AgentTaskStatus::Failed)
            })
            .and_then(|info| info.last_tool_info.clone())
    });
    let name = args
        .name
        .clone()
        .or_else(|| prior.and_then(|i| i.name.clone()));
    let task_description = args
        .task_description
        .clone()
        .or_else(|| prior.and_then(|i| i.task_description.clone()));

    let entry_idx = match prior.map(|i| i.entry_idx) {
        Some(idx)
            if state
                .chat_history
                .get(idx)
                .is_some_and(|entry| match entry.entry_type {
                    ChatEntryType::AgentTask => {
                        entry.agent_task_id.as_deref() == Some(args.task_id.as_str())
                    }
                    ChatEntryType::AgentGroup => entry
                        .agent_task_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&args.task_id)),
                    _ => false,
                }) =>
        {
            idx
        }
        _ => state
            .chat_history
            .iter()
            .position(|entry| match entry.entry_type {
                ChatEntryType::AgentTask => {
                    entry.agent_task_id.as_deref() == Some(args.task_id.as_str())
                }
                ChatEntryType::AgentGroup => entry
                    .agent_task_ids
                    .as_ref()
                    .is_some_and(|ids| ids.contains(&args.task_id)),
                _ => false,
            })
            .unwrap_or_else(|| attach_agent_task_entry(state, &args)),
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

    // 已在查看的代理结束时自动回主会话；终态记录留在聊天历史中，
    // 不让已结束的详情继续占着整个聊天区。
    if first_terminal && args.is_async {
        if state.viewing_agent_task_id.as_deref() == Some(args.task_id.as_str()) {
            state.exit_teammate_view();
            state.auto_follow = true;
        }
        // 行列表缩短后旧索引可能指向另一个正在运行的代理；焦点回到输入框，
        // 避免下一次 Enter 打开错误的任务。
        state.bg_agent_selection = None;
    }

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

/// 合并 teammate transcript 的增量，并按 tool_call.id 把结果贴回调用。
///
/// 工具事件按完成顺序到达，而调用按开始顺序记录；直接 append 会让并发结果
/// 堆在 transcript 尾部。这里只关联带非空 ID 的工具条目，不凭工具名或文本去重。
fn merge_sub_entries(
    existing: &mut Vec<crate::types::ChatEntry>,
    incoming: &[crate::types::ChatEntry],
) {
    use crate::types::{ChatEntryType, EntryStatus};

    for entry in incoming {
        let call_id = entry
            .tool_call
            .as_ref()
            .map(|call| call.id.as_str())
            .filter(|id| !id.is_empty());
        let Some(call_id) = call_id else {
            existing.push(entry.clone());
            continue;
        };

        match entry.entry_type {
            ChatEntryType::ToolCall => {
                if existing.iter().any(|prior| {
                    prior.entry_type == ChatEntryType::ToolCall
                        && prior
                            .tool_call
                            .as_ref()
                            .is_some_and(|call| call.id == call_id)
                }) {
                    continue;
                }

                // 结果可能因两个异步传输通道先于调用条目抵达；到达调用时，
                // 将它放到孤立结果之前，恢复一个可理解的调用组。
                let insert_pos = existing.iter().position(|prior| {
                    prior.entry_type == ChatEntryType::ToolResult
                        && prior
                            .tool_call
                            .as_ref()
                            .is_some_and(|call| call.id == call_id)
                });
                if let Some(insert_pos) = insert_pos {
                    let result_success = existing[insert_pos]
                        .tool_result
                        .as_ref()
                        .is_some_and(|result| result.success);
                    let mut call_entry = entry.clone();
                    call_entry.is_streaming = Some(false);
                    call_entry.status = Some(if result_success {
                        EntryStatus::Success
                    } else {
                        EntryStatus::SoftError
                    });
                    existing.insert(insert_pos, call_entry);
                } else {
                    existing.push(entry.clone());
                }
            }
            ChatEntryType::ToolResult => {
                if existing.iter().any(|prior| {
                    prior.entry_type == ChatEntryType::ToolResult
                        && prior
                            .tool_call
                            .as_ref()
                            .is_some_and(|call| call.id == call_id)
                }) {
                    continue;
                }

                let call_pos = existing.iter().position(|prior| {
                    prior.entry_type == ChatEntryType::ToolCall
                        && prior
                            .tool_call
                            .as_ref()
                            .is_some_and(|call| call.id == call_id)
                });
                if let Some(call_pos) = call_pos {
                    let success = entry
                        .tool_result
                        .as_ref()
                        .is_some_and(|result| result.success);
                    let tool_call = &mut existing[call_pos];
                    tool_call.is_streaming = Some(false);
                    tool_call.status = Some(if success {
                        EntryStatus::Success
                    } else {
                        EntryStatus::SoftError
                    });
                    existing.insert(call_pos + 1, entry.clone());
                } else {
                    // 保留尚未匹配的结果；调用晚到时由 ToolCall 分支插回结果之前。
                    existing.push(entry.clone());
                }
            }
            _ => existing.push(entry.clone()),
        }
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
