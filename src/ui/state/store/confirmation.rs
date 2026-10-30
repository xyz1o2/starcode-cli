//! 待确认队列（工具审批排队）。
//!
use super::*;

impl ChatState {
    /// 把排队中的下一张确认卡设为当前等待回答的卡。
    ///
    /// 返回 `false` 表示队列已空。调用方通常是 [`Self::clear_pending_confirmation`]：
    /// 一张卡刚被回答/拒绝/取消，下一条并发请求要接上，否则它的门禁会一直等。
    pub fn activate_next_queued_confirmation(&mut self) -> bool {
        let Some(queued) = self.pending_confirmation_queue.pop_front() else {
            return false;
        };
        if queued.entry_idx >= self.chat_history.len() {
            // 卡片所属的回合已经被清掉了，继续找下一张。
            return self.activate_next_queued_confirmation();
        }

        // 排队时把 outcome 写成占位文案，现在要恢复成可回答的选项列表。
        if let Some(confirmation) = self.chat_history[queued.entry_idx].confirmation.as_mut() {
            confirmation.outcome = None;
        }
        self.chat_history[queued.entry_idx].is_streaming = Some(false);
        self.rendered_cache.remove(&queued.entry_idx);
        self.virtual_list.mark_dirty(queued.entry_idx);

        self.pending_confirmation_entry_idx = Some(queued.entry_idx);
        self.pending_message_id = Some(queued.message_id);
        self.pending_tool_call_id = Some(queued.tool_call_id);
        self.pending_confirmation_originating_id = queued.originating_tool_call_id;
        self.is_awaiting_confirmation = true;
        self.confirmation_feedback_mode = false;
        self.pending_confirmation_feedback.clear();
        self.pending_confirmation_choice = if queued.is_ask { 0 } else { 1 };
        self.pending_question_selections.clear();
        self.pending_other_input.clear();
        self.pending_question_other_focused = false;
        self.chat_list_state.select(Some(queued.entry_idx));
        true
    }

    /// 整个回合结束/中断：队列里的请求不再有意义，把卡片标成已取消。
    ///
    /// 注意顺序：先清队列再 [`Self::clear_pending_confirmation`]，否则清当前卡时
    /// 会把一张已经失效的请求重新变成等待态。
    pub fn discard_pending_confirmation_queue(&mut self, reason: &str) {
        let queued: Vec<QueuedConfirmation> = self.pending_confirmation_queue.drain(..).collect();
        for item in queued {
            if let Some(entry) = self.chat_history.get_mut(item.entry_idx) {
                if let Some(confirmation) = entry.confirmation.as_mut() {
                    if confirmation.outcome.is_none() {
                        confirmation.outcome = Some(reason.to_string());
                    }
                }
            }
            self.rendered_cache.remove(&item.entry_idx);
            self.virtual_list.mark_dirty(item.entry_idx);
        }
    }

    /// 把一张排队中的确认卡从队列里摘掉（对应工具已经有结果/已取消）。
    pub fn remove_queued_confirmation_by_originating_id(&mut self, originating_id: &str) {
        let mut removed = Vec::new();
        self.pending_confirmation_queue.retain(|item| {
            if item.originating_tool_call_id.as_deref() == Some(originating_id) {
                removed.push(item.entry_idx);
                false
            } else {
                true
            }
        });
        for idx in removed {
            if let Some(entry) = self.chat_history.get_mut(idx) {
                if let Some(confirmation) = entry.confirmation.as_mut() {
                    if confirmation.outcome.is_none() {
                        confirmation.outcome = Some(crate::core::i18n::t(
                            "ui.confirm.cancelled_turn",
                            "已取消 · 对应工具已有结果",
                            "Cancelled — the tool already finished",
                        ));
                    }
                }
            }
            self.rendered_cache.remove(&idx);
            self.virtual_list.mark_dirty(idx);
        }
    }

    /// 当前这张卡处理完了，接上下一张排队的卡。
    pub fn clear_pending_confirmation(&mut self) {
        self.is_awaiting_confirmation = false;
        self.pending_tool_call_id = None;
        self.pending_confirmation_originating_id = None;
        self.pending_confirmation_entry_idx = None;
        self.pending_confirmation_feedback.clear();
        self.confirmation_feedback_mode = false;
        self.pending_question_other_focused = false;
        self.pending_other_input.clear();
        self.pending_question_selections.clear();
        self.activate_next_queued_confirmation();
    }
}
