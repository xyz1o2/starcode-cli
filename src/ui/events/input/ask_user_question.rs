//! AskUserQuestion 工具的交互输入（选项选择 + 自由文本回答）。
//!
//! 一次调用可以携带多个问题（对标 Claude Code 的 `questions` 数组），UI 逐题
//! 引导：每回答完一题把 `AskUserAnswer` 存进 `pending_question_answers` 并推进
//! `pending_question_index`，全部答完才发一次 `UserAnswers` —— 模型不会中途
//! 看到半套答案，用户也可以在任意一题按 Esc 整体放弃。

use super::*;

/// 把当前确认卡的当前问题提取成 owned 数据，避免与后续的状态变更借用冲突。
fn current_question_data(state: &ChatState) -> Option<(String, Vec<String>, bool)> {
    let entry = state
        .pending_confirmation_entry_idx
        .and_then(|idx| state.chat_history.get(idx))?;
    let conf = entry.confirmation.as_ref()?;
    match &conf.details {
        crate::types::ConfirmationDetails::AskUserQuestion { questions } => {
            let item = questions.get(state.pending_question_index)?;
            Some((
                item.question.clone(),
                item.options.iter().map(|o| o.label.clone()).collect(),
                item.multi_select,
            ))
        }
        _ => None,
    }
}

/// 更新确认卡的渲染结果缓存。
fn invalidate_cache(state: &mut ChatState) {
    if let Some(idx) = state.pending_confirmation_entry_idx {
        state.rendered_cache.remove(&idx);
    }
}

/// Handle keyboard input for the AskUserQuestion confirmation dialog.
/// Supports option navigation (Up/Down/j/k), direct selection (1-9),
/// multi-select toggle (Space), confirm (Enter), cancel (Esc),
/// and "Other" text input (Tab to switch, then type).
pub(super) async fn handle_ask_user_question_input(
    key: KeyEvent,
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((question_text, option_labels, multi_select)) = current_question_data(state) else {
        return Ok(());
    };
    let option_count = option_labels.len();
    let other_idx = option_count;

    if state.pending_question_other_focused {
        // "Other" text input mode
        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                        feedback: None,
                    })
                    .await;
                state.clear_pending_confirmation();
            }
            KeyCode::Tab => {
                // Switch back to option navigation
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                // Confirm with other input included
                submit_current_question(
                    state,
                    agent_tx,
                    tool_call_id,
                    &question_text,
                    &option_labels,
                    multi_select,
                )
                .await?;
            }
            KeyCode::Backspace => {
                state.pending_other_input.pop();
                invalidate_cache(state);
            }
            KeyCode::Char(ch) => {
                state.pending_other_input.push(ch);
                invalidate_cache(state);
            }
            _ => {}
        }
    } else {
        // Option navigation mode
        let total_items = option_count + 1; // options + Other

        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                        feedback: None,
                    })
                    .await;
                state.clear_pending_confirmation();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.pending_confirmation_choice > 0 {
                    state.pending_confirmation_choice -= 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = total_items - 1;
                }
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total_items > 0 && state.pending_confirmation_choice + 1 < total_items {
                    state.pending_confirmation_choice += 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = 0;
                }
                invalidate_cache(state);
            }
            KeyCode::Tab => {
                // Switch to "Other" input mode
                state.pending_question_other_focused = true;
                state.pending_confirmation_choice = other_idx;
                invalidate_cache(state);
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let digit = ch.to_digit(10).unwrap_or(0) as usize;
                if digit >= 1 && digit <= option_count {
                    let opt_idx = digit - 1;
                    if multi_select {
                        // Multi-select: just focus that option
                        state.pending_confirmation_choice = opt_idx;
                    } else {
                        // Single-select: select and confirm immediately
                        state.pending_confirmation_choice = opt_idx;
                        submit_current_question(
                            state,
                            agent_tx,
                            tool_call_id,
                            &question_text,
                            &option_labels,
                            multi_select,
                        )
                        .await?;
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Char(' ') if multi_select => {
                // Toggle current option in multi-select mode
                let focused = state.pending_confirmation_choice;
                if focused < option_count {
                    if let Some(pos) = state
                        .pending_question_selections
                        .iter()
                        .position(|&i| i == focused)
                    {
                        state.pending_question_selections.remove(pos);
                    } else {
                        state.pending_question_selections.push(focused);
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                if !multi_select && state.pending_confirmation_choice == other_idx {
                    // "Other" is focused — switch to text input mode instead of submitting
                    state.pending_question_other_focused = true;
                    invalidate_cache(state);
                } else if multi_select {
                    if !state.pending_question_selections.is_empty() {
                        submit_current_question(
                            state,
                            agent_tx,
                            tool_call_id,
                            &question_text,
                            &option_labels,
                            multi_select,
                        )
                        .await?;
                    }
                    // 多选没勾任何项：Enter 忽略，避免交上去一个空答案
                } else if state.pending_confirmation_choice < option_count {
                    submit_current_question(
                        state,
                        agent_tx,
                        tool_call_id,
                        &question_text,
                        &option_labels,
                        multi_select,
                    )
                    .await?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// 收集当前题的答案并存起来；这一题是最后一题时打包发 `UserAnswers`。
async fn submit_current_question(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
    question_text: &str,
    option_labels: &[String],
    multi_select: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let selected: Vec<String> = if multi_select {
        let mut indices = state.pending_question_selections.clone();
        indices.sort();
        indices
            .into_iter()
            .filter_map(|idx| option_labels.get(idx).cloned())
            .collect()
    } else {
        let focused = state.pending_confirmation_choice;
        option_labels
            .get(focused)
            .filter(|_| focused < option_labels.len())
            .cloned()
            .into_iter()
            .collect()
    };

    let text_input = if state.pending_other_input.trim().is_empty() {
        None
    } else {
        Some(state.pending_other_input.trim().to_string())
    };

    // 单选没选任何项且没有自由文本：交不上一份空答案。
    // （UI 入口保证了这条，这里只是防御。）
    if selected.is_empty() && text_input.is_none() {
        invalidate_cache(state);
        return Ok(());
    }

    let answer = crate::types::AskUserAnswer {
        question: question_text.to_string(),
        selected,
        text_input,
    };
    state.pending_question_answers.push(answer);

    // 推进到下一题或结束整个提问。
    let total_questions = state
        .pending_confirmation_entry_idx
        .and_then(|idx| state.chat_history.get(idx))
        .and_then(|entry| entry.confirmation.as_ref())
        .and_then(|conf| match &conf.details {
            crate::types::ConfirmationDetails::AskUserQuestion { questions } => {
                Some(questions.len())
            }
            _ => None,
        })
        .unwrap_or(0);

    state.pending_question_index += 1;
    if state.pending_question_index >= total_questions {
        let answers = state.pending_question_answers.clone();
        // 卡片上的结局摘要：每题一行 "question → 答案"。
        if let Some(idx) = state.pending_confirmation_entry_idx {
            if let Some(entry) = state.chat_history.get_mut(idx) {
                if let Some(conf) = entry.confirmation.as_mut() {
                    let summary = answers
                        .iter()
                        .map(|a| {
                            let chosen = if a.selected.is_empty() {
                                a.text_input
                                    .clone()
                                    .unwrap_or_else(|| "(empty)".to_string())
                            } else {
                                let mut v = a.selected.join(", ");
                                if let Some(ti) = &a.text_input {
                                    v.push_str(&format!(" (+ input: {})", ti));
                                }
                                v
                            };
                            format!("{} → {}", a.question, chosen)
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    conf.outcome = Some(if answers.len() == 1 {
                        summary
                    } else {
                        format!("Answered {} questions: {}", answers.len(), summary)
                    });
                }
            }
            state.rendered_cache.remove(&idx);
        }
        let _ = agent_tx
            .send(AgentRequest::ConfirmTool {
                tool_call_id: tool_call_id.to_string(),
                outcome: crate::types::ToolConfirmationOutcome::UserAnswers { answers },
                feedback: None,
            })
            .await;
        state.clear_pending_confirmation();
    } else {
        // 还有下一题：重置本题的交互状态，光标回到第一个选项。
        state.pending_question_selections.clear();
        state.pending_other_input.clear();
        state.pending_question_other_focused = false;
        state.pending_confirmation_choice = 0;
        invalidate_cache(state);
    }

    Ok(())
}
