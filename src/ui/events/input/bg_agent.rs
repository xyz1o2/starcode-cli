//! 后台代理选择器与各类模态框（主题 / 状态 / 日志 / 错误浮层）的按键处理。
//!
use super::*;

/// 后台代理选择器的按键处理（对标 background agent selector）。
///
/// 返回 `true` 表示按键已被选择器消费，调用方应直接结束本次事件处理。
///
/// 「光标停在哪一行」（`bg_agent_selection`）与「正在看谁的输出」（`viewing_agent_task_id`）
/// 是分开的两件事：`Enter` 只切换后者，焦点留在原行，用户可以连着 ↑/↓ 逐个翻看；
/// `Esc` 只收起选择器，不改变当前正在看的详情。
pub(super) fn handle_bg_agent_selector_key(state: &mut ChatState, key: KeyEvent) -> bool {
    // 行数每帧都可能变（后台代理还在陆续启动），先按当前快照收敛焦点
    let task_ids: Vec<String> = state
        .background_agent_rows()
        .iter()
        .map(|info| info.task_id.clone())
        .collect();
    if task_ids.is_empty() {
        state.bg_agent_selection = None;
        return false;
    }
    let last_idx = task_ids.len(); // 索引 0 是 main 行，代理从 1 开始
    let selected = state.bg_agent_selection.unwrap_or(0).min(last_idx);

    match key.code {
        KeyCode::Up => {
            if selected == 0 {
                // 已在 main 行再往上 → 焦点还给输入框
                state.bg_agent_selection = None;
            } else {
                state.bg_agent_selection = Some(selected - 1);
            }
            true
        }
        KeyCode::Down => {
            state.bg_agent_selection = Some((selected + 1).min(last_idx));
            true
        }
        KeyCode::Enter => {
            if selected == 0 {
                state.exit_teammate_view();
            } else {
                let task_id = task_ids[selected - 1].clone();
                if state.viewing_agent_task_id.as_deref() == Some(task_id.as_str()) {
                    // 再按一次 Enter 收回主会话，跟参照实现的 toggle 语义一致
                    state.exit_teammate_view();
                } else {
                    state.enter_teammate_view(&task_id);
                }
            }
            true
        }
        KeyCode::Esc => {
            state.bg_agent_selection = None;
            true
        }
        _ => false,
    }
}

/// 状态弹窗（Settings）：↑↓ 选设置项、Enter 执行、Esc 关闭。
pub(crate) async fn handle_status_modal_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Esc => {
            state.pop_modal();
        }
        KeyCode::Up => {
            let count = crate::ui::components::status_modal::settings_item_count();
            state.settings_selected_index = state
                .settings_selected_index
                .saturating_add(count)
                .saturating_sub(1)
                % count;
        }
        KeyCode::Down => {
            let count = crate::ui::components::status_modal::settings_item_count();
            state.settings_selected_index = (state.settings_selected_index + 1) % count;
        }
        KeyCode::Enter => {
            // Get the action for the selected setting and execute it
            if let Some(action) = crate::ui::components::status_modal::get_settings_action(state) {
                state.pop_modal();
                // Re-dispatch the action through execute_palette_action
                execute_palette_action(state, action, agent_tx).await?;
            }
        }
        _ => {}
    }
    Ok(true)
}

/// 日志/会话选择器：↑↓ 选择、输入过滤、Enter 恢复会话、Esc 关闭。
pub(crate) async fn handle_log_selector_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Enter => {
            if state.log_selector_state.is_loading {
                return Ok(true);
            }

            // 走统一命令入口，保留流式中的排队保护，并复用 /chat resume 的状态清理。
            if let Some(session) = state.log_selector_state.get_selected_session() {
                let command = format!("/chat resume {}", session.id);
                state.pop_modal();
                crate::ui::app::logic::enqueue_user_message(state, command, agent_tx).await?;
            }
        }
        KeyCode::Esc => {
            state.pop_modal();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.select_prev();
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.select_next();
            }
        }
        KeyCode::Char('/') => {
            if !state.log_selector_state.is_loading {
                // Focus search
                state.log_selector_state.search_query.clear();
            }
        }
        KeyCode::Backspace => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.search_query.pop();
                state.log_selector_state.selected_index = 0;
            }
        }
        KeyCode::Char(c) => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.search_query.push(c);
                state.log_selector_state.selected_index = 0;
            }
        }
        _ => {}
    }
    // 未匹配也消费：选择器是模态，键不该漏到下面的输入框。
    Ok(true)
}

/// 重发最近一条用户消息。
///
/// 只在**当前回合确实失败**时返回 `Some`：一个正常收尾的会话里按 Esc 不该凭空
/// 多出一条消息。`last_turn_failed` 由 `handle_error_message` 置位，由
/// `handle_done_message` 和真正发起新回合时清掉，所以它同时也是"当前输入框是空的
/// 且没有别的理由不发消息"这一约束的判据。
pub(crate) async fn retry_last_user_message(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !state.last_turn_failed {
        return Ok(false);
    }
    let Some(last_user) = state
        .chat_history
        .iter()
        .rev()
        .find(|e| e.entry_type == ChatEntryType::User)
    else {
        // 失败回合之前没有用户消息（例如恢复出来的半截会话）：没有可重发的内容，
        // 但要把状态清掉，否则 Esc 会永远卡在这个分支里。
        state.last_turn_failed = false;
        return Ok(false);
    };
    let msg = last_user.content.clone();
    crate::ui::app::logic::enqueue_user_message(state, msg, agent_tx).await?;
    Ok(true)
}

/// 错误覆盖层：Enter 执行选中项、r 重试上一条、s 换 provider、c/Esc 关闭、↑↓ 切换选项。
pub(crate) async fn handle_error_overlay_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Enter 执行当前高亮的选项。之前界面画着 "Enter 确认" 却没有 Enter 分支，
    // `_ => {}` + `Ok(true)` 把按键吞掉 —— 高亮的那一项按下去什么都不会发生。
    if key.code == KeyCode::Enter {
        match state.error_overlay_state.selected_action {
            crate::ui::components::error_overlay::ErrorAction::Retry => {
                state.pop_modal();
                retry_last_user_message(state, agent_tx).await?;
            }
            crate::ui::components::error_overlay::ErrorAction::SwitchProvider => {
                state.pop_modal();
                state.open_palette(PaletteMode::Provider);
            }
            crate::ui::components::error_overlay::ErrorAction::Cancel => {
                state.pop_modal();
            }
        }
        return Ok(true);
    }
    match key.code {
        KeyCode::Char('r') | KeyCode::Char('R') => {
            state.pop_modal();
            retry_last_user_message(state, agent_tx).await?;
        }
        KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => {
            state.pop_modal();
            // 关掉浮层不等于放弃这次重试机会：Esc 之后在输入框按 Esc 仍然能重发。
            // 只有用户发了新消息或跑完一轮，这个标志才会被清掉。
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            // Switch provider
            state.pop_modal();
            state.open_palette(PaletteMode::Provider);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.error_overlay_state.select_prev();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.error_overlay_state.select_next();
        }
        _ => {}
    }
    Ok(true)
}

/// 主题选择器：↑↓ 导航（实时预览）、Enter 应用、Esc 取消。
///
/// 从 `handle_overlay_input` 抽出来的独立处理器：模态的按键处理应当住在
/// `modal_input::handle_modal_key` 那张表里，而不是散落在阶梯的中段。
pub(crate) fn handle_theme_picker_key(state: &mut ChatState, key: KeyEvent) -> bool {
    let themes = crate::ui::components::highlight::theme_picker::available_themes();
    let count = themes.len();
    let prev_index = state.selected_theme_index;
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected_theme_index = if state.selected_theme_index == 0 {
                count.saturating_sub(1)
            } else {
                state.selected_theme_index - 1
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected_theme_index = (state.selected_theme_index + 1) % count.max(1);
        }
        KeyCode::Enter => {
            let name = themes
                .get(state.selected_theme_index)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            if state.theme_manager.set_theme(&name) {
                state.current_status_line = Some(format!("Theme: {}", name));
            }
            state.pop_modal();
            return true;
        }
        KeyCode::Esc => {
            // 取消：恢复进入 picker 前的主题
            if let Some(crate::ui::state::modal::Modal::ThemePicker { prev_theme }) =
                state.pop_modal()
            {
                if let Some(prev) = prev_theme {
                    state.theme_manager.set_theme(&prev);
                }
            }
            return true;
        }
        _ => {}
    }
    // 实时预览：光标移动即切换主题
    if prev_index != state.selected_theme_index {
        if let Some(t) = themes.get(state.selected_theme_index) {
            state.theme_manager.set_theme(&t.name);
        }
    }
    true
}
