//! 工具调用的审批（确认 / 拒绝 / 反馈）。
//!
use super::*;

/// 工具审批（AskUserQuestion / 权限确认）与权限说明、权限调试面板。
///
/// 只处理 [`KeyContext::ToolConfirmation`]：这个上下文本身就意味着「模态栈空、
/// 没有行内确认」，因此这里不必再重复判断，也就不存在「漏写一个守卫就穿透」
/// 的可能（那正是 `has_overlay_active()` 时代的毛病）。
///
/// 主题选择器、状态弹窗、任务面板都已在自己的分发表项里，不再经过本函数。
pub(super) async fn handle_tool_approval(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if state.is_awaiting_confirmation {
        if let Some(id) = state.pending_tool_call_id.clone() {
            let is_ask = state
                .pending_confirmation_entry_idx
                .and_then(|idx| state.chat_history.get(idx))
                .and_then(|entry| entry.confirmation.as_ref())
                .map(|conf| {
                    matches!(
                        conf.operation_type,
                        crate::types::ConfirmationType::AskUserQuestion
                    )
                })
                .unwrap_or(false);

            if is_ask {
                handle_ask_user_question_input(key, state, agent_tx, &id).await?;
            } else if state.confirmation_feedback_mode {
                // Feedback input mode: Tab back to options, Enter submits with feedback
                match key.code {
                    KeyCode::Tab => {
                        state.confirmation_feedback_mode = false;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Esc => {
                        state.confirmation_feedback_mode = false;
                        state.pending_confirmation_feedback.clear();
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Backspace => {
                        state.pending_confirmation_feedback.pop();
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Enter => {
                        // Submit with current choice + feedback
                        let outcome = match state.pending_confirmation_choice {
                            1 => crate::types::ToolConfirmationOutcome::ProceedOnce,
                            2 => crate::types::ToolConfirmationOutcome::AllowSession,
                            3 => crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave,
                            _ => crate::types::ToolConfirmationOutcome::Cancel,
                        };
                        let feedback = if state.pending_confirmation_feedback.is_empty() {
                            None
                        } else {
                            Some(state.pending_confirmation_feedback.clone())
                        };
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    let outcome_str = match state.pending_confirmation_choice {
                                        1 => "Allowed (once)",
                                        2 => "Allowed (session)",
                                        3 => "Allowed (always)",
                                        _ => "Denied",
                                    };
                                    if let Some(ref fb) = feedback {
                                        conf.outcome =
                                            Some(format!("{} with feedback: {}", outcome_str, fb));
                                    } else {
                                        conf.outcome = Some(outcome_str.to_string());
                                    }
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome,
                                feedback,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    KeyCode::Char(ch) => {
                        state.pending_confirmation_feedback.push(ch);
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    // Tab: 切换到反馈输入模式（对标 Claude Code "Tab to provide feedback"）
                    KeyCode::Tab => {
                        state.confirmation_feedback_mode = true;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    // Shift+Tab: 快速批准（对标 Claude Code quick approve）
                    KeyCode::BackTab => {
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    conf.outcome = Some("Quick approved (Shift+Tab)".to_string());
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome: crate::types::ToolConfirmationOutcome::ProceedOnce,
                                feedback: None,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    // Ctrl+E: 切换权限解释区 / Ctrl+D: 切换 debug 详情（Claude Code 风格）
                    KeyCode::Char('e') | KeyCode::Char('E')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        state.show_permission_explanation = !state.show_permission_explanation;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    // 权限调试面板。绑 Ctrl+Shift+D 而不是 Ctrl+D —— Ctrl+D 现在唯一表示
                    // 「退出」（见本文件上方的 Ctrl+D 分支），同一个键挂三种语义必然打架。
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        state.show_permission_debug = !state.show_permission_debug;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('1' | 'y' | 'Y') => {
                        state.pending_confirmation_choice = 1;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('2' | 's' | 'S') => {
                        state.pending_confirmation_choice = 2;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('3' | 'a' | 'A') => {
                        state.pending_confirmation_choice = 3;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('4' | 'd' | 'D' | 'n' | 'N') => {
                        state.pending_confirmation_choice = 4;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Esc => {
                        state.pending_confirmation_choice = 4;
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome: crate::types::ToolConfirmationOutcome::Cancel,
                                feedback: None,
                            })
                            .await;
                        // 先取索引再清状态：`clear_pending_confirmation` 会把
                        // `pending_confirmation_entry_idx` 置空，卡片上的 "Denied"
                        // 就没机会作废重绘了。
                        let rendered_idx = state.pending_confirmation_entry_idx;
                        state.clear_pending_confirmation();
                        if let Some(idx) = rendered_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Up => {
                        if state.pending_confirmation_choice > 1 {
                            state.pending_confirmation_choice -= 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Down => {
                        if state.pending_confirmation_choice < 4 {
                            state.pending_confirmation_choice += 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Enter => {
                        let outcome = match state.pending_confirmation_choice {
                            1 => crate::types::ToolConfirmationOutcome::ProceedOnce,
                            2 => crate::types::ToolConfirmationOutcome::AllowSession,
                            3 => crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave,
                            _ => crate::types::ToolConfirmationOutcome::Cancel,
                        };
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    let outcome_str = match state.pending_confirmation_choice {
                                        1 => "Allowed (once)",
                                        2 => "Allowed (session)",
                                        3 => "Allowed (always)",
                                        _ => "Denied",
                                    };
                                    conf.outcome = Some(outcome_str.to_string());
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        // Collect feedback if user typed any
                        let feedback = if state.pending_confirmation_feedback.is_empty() {
                            None
                        } else {
                            Some(state.pending_confirmation_feedback.clone())
                        };
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome,
                                feedback,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    _ => {}
                }
            }
        } else {
            // 没有可回答的 id 却还挂着等待态：这是状态已经错乱的表现，
            // 只翻一个标志位会把 `pending_*` 留在半清不楚的状态上。
            state.clear_pending_confirmation();
        }
        return Ok(true);
    }

    Ok(false)
}

/// 任务面板（常驻）：编辑、上下移动、状态切换、层级调整。
///
/// 由 [`handle_key_event`] 的上下文分发表在 `KeyContext::TaskPanel` 时调用。
/// 面板只吃 Ctrl/Alt 组合键与 ↑/↓/Enter，普通字符落到输入框 —— 这条规则现在
/// 由 `KeyContext` 表达，不再需要各处手写 `&& !modal_open` 守卫。
pub(crate) fn handle_task_panel_key(state: &mut ChatState, key: KeyEvent) -> bool {
    if state.task_panel.is_editing() {
        state.task_panel.handle_edit_input(key);
        return true;
    }

    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        state
            .task_panel
            .enter_edit_mode(crate::ui::components::task_panel::EditMode::Title);
        return true;
    }

    // 与权限调试面板一致：Ctrl+D 归「退出」，任务描述编辑改用 Ctrl+Shift+D。
    if key.code == KeyCode::Char('d')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        state
            .task_panel
            .enter_edit_mode(crate::ui::components::task_panel::EditMode::Description);
        return true;
    }

    if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('v') {
        state.task_panel.cycle_view_mode();
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("View: {}", state.task_panel.view_mode),
        );
        return true;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('j') => {
                state.task_panel.next();
                return true;
            }
            KeyCode::Char('k') => {
                state.task_panel.previous();
                return true;
            }
            KeyCode::Char(' ') => {
                state.task_panel.toggle_status();
                crate::ui::app::logic::emit_status_text(state, 0, "Task status toggled");
                return true;
            }
            KeyCode::Char('s') => {
                state.task_panel.skip_task();
                crate::ui::app::logic::emit_status_text(state, 0, "Task skipped");
                return true;
            }
            KeyCode::Char('n') => {
                state.task_panel.add_new_task();
                crate::ui::app::logic::emit_status_text(state, 0, "New task added");
                return true;
            }
            KeyCode::Delete => {
                state.task_panel.delete_selected();
                crate::ui::app::logic::emit_status_text(state, 0, "Task deleted");
                return true;
            }
            KeyCode::Up => {
                state.task_panel.move_up();
                return true;
            }
            KeyCode::Down => {
                state.task_panel.move_down();
                return true;
            }
            _ => {}
        }
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Right => {
                state.task_panel.indent();
                return true;
            }
            KeyCode::Left => {
                state.task_panel.outdent();
                return true;
            }
            _ => {}
        }
    }

    false
}

/// 通用输入对话框（API Key / Base URL / 重命名）与 provider 多字段表单。
///
/// 由 `modal_input::handle_modal_key` 在栈顶是 [`Modal::InputModal`] 时调用。
pub(crate) async fn handle_input_modal(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !state.is_input_modal_open() {
        return Ok(false);
    }

    // "新增 provider" 表单：字段切换 / 类型切换 / 操作行 由这里接管。Type 字段
    // （下标 0）和操作行都是选项不是文本，普通字符键必须吃掉，否则会灌进
    // 共享的 textarea 里。
    let is_provider_form = matches!(
        state.input_context,
        Some(crate::ui::state::palette::InputContext::ProviderForm)
    );
    let form_on_type_field = is_provider_form && state.provider_form.active_field == 0;
    let form_on_actions = is_provider_form && state.provider_form.on_actions;

    match key.code {
        KeyCode::Tab | KeyCode::Down if is_provider_form => {
            switch_provider_form_field(state, true);
            return Ok(true);
        }
        KeyCode::BackTab | KeyCode::Up if is_provider_form => {
            switch_provider_form_field(state, false);
            return Ok(true);
        }
        KeyCode::Left if form_on_type_field => {
            state.provider_form.cycle_type(false);
            return Ok(true);
        }
        KeyCode::Right if form_on_type_field => {
            state.provider_form.cycle_type(true);
            return Ok(true);
        }
        // Type 是选项字段，回车不当提交用：确认当前类型并跳到 Name。不然焦点
        // 落在 Type 上时按回车只会去提交一个空表单，报 "Base URL is required."
        KeyCode::Enter if form_on_type_field => {
            switch_provider_form_field(state, true);
            return Ok(true);
        }
        // 操作行：←/→ 在 Back / Save 之间选
        KeyCode::Left | KeyCode::Right if form_on_actions => {
            state.provider_form.action_index = 1 - state.provider_form.action_index;
            return Ok(true);
        }
        // 操作行回车：选中项是 Save 就提交，是 Back 就取消回上一层
        KeyCode::Enter if form_on_actions => {
            if state.provider_form.action_index == 1 {
                state.input_context = Some(crate::ui::state::palette::InputContext::ProviderForm);
                return submit_provider_form(state, agent_tx).await;
            }
            cancel_provider_form(state);
            return Ok(true);
        }
        // Type 字段和操作行不接收自由文本，按键一律吞掉，避免污染共享 textarea
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
            if form_on_type_field || form_on_actions =>
        {
            return Ok(true);
        }
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Ok(mut clipboard) = Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    state.modal_textarea.insert_str(&text);
                }
            }
        }
        KeyCode::Esc => {
            // 底层还有模态（如 Plugins 弹窗）时 cancel 只关输入框，回到该模态
            cancel_provider_form(state);
            return Ok(true);
        }
        KeyCode::Enter => {
            state.input_modal_value = collect_modal_input(&state.modal_textarea);

            if let Some(ctx) = state.input_context.take() {
                match ctx {
                    crate::ui::state::palette::InputContext::ContextWindow => {
                        let input = state.input_modal_value.trim().to_string();
                        match crate::core::context_policy::ContextWindowSelection::parse(&input) {
                            Ok(selection) => {
                                state.exit_input_modal();
                                if request_runtime_settings(
                                    state,
                                    None,
                                    Some(selection),
                                    None,
                                    agent_tx,
                                )
                                .await
                                {
                                    state.current_status_line = Some(format!(
                                        "Requesting context window {}…",
                                        selection.display()
                                    ));
                                }
                            }
                            Err(error) => {
                                state.current_status_line =
                                    Some(format!("Invalid context window size: {}.", error));
                            }
                        }
                    }
                    crate::ui::state::palette::InputContext::ProviderForm => {
                        // 提交校验失败时 submit 不关模态只设 error。context 被
                        // take 走又没还回来的话，show_input_modal 还开着、
                        // 渲染却退化成单字段的通用输入框（还带着别的弹窗残留的
                        // title）。先还回去，失败时表单还在原地。
                        state.input_context =
                            Some(crate::ui::state::palette::InputContext::ProviderForm);
                        return submit_provider_form(state, agent_tx).await;
                    }
                    crate::ui::state::palette::InputContext::MarketplaceSource => {
                        let source = state.input_modal_value.trim().to_string();
                        state.exit_input_modal();

                        if source.is_empty() {
                            state.input_context = None;
                            return Ok(true);
                        }

                        // add_marketplace 需要克隆仓库，可能耗时数秒：后台执行，
                        // 完成后经 StreamMessage::PluginOpResult 回填消息并刷新
                        let cwd = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        state.plugin_op_pending = true;
                        state.plugin_message = Some(format!("Adding marketplace {}...", source));
                        let _ = agent_tx
                            .send(AgentRequest::PluginOp {
                                project_root: cwd,
                                op: crate::runtime::messages::PluginOp::AddMarketplace { source },
                            })
                            .await;
                        state.plugin_index = 0;
                        state.input_context = None;
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddWorkingDir => {
                        let path = state.input_modal_value.clone();
                        state.exit_input_modal();
                        state.input_context = None;

                        if path.trim().is_empty() {
                            return Ok(true);
                        }

                        match crate::commands::extended::add_working_dir_to_state(state, &path) {
                            Ok(msg) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(msg).with_streaming(false),
                                );
                            }
                            Err(e) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(format!("❌ {}", e))
                                        .with_streaming(false),
                                );
                            }
                        }
                        return Ok(true);
                    }
                }
            } else {
                state.exit_input_modal();
            }
        }
        _ => {
            state.modal_textarea.input(key);
        }
    }
    Ok(true)
}
