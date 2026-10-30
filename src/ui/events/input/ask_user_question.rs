//! AskUserQuestion 工具的交互输入（选项选择 + 自由文本回答）。
//!
use super::*;

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
    // Extract option info from the confirmation entry
    let (option_count, multi_select) = state
        .pending_confirmation_entry_idx
        .and_then(|idx| state.chat_history.get(idx))
        .and_then(|entry| entry.confirmation.as_ref())
        .map(|conf| match &conf.details {
            crate::types::ConfirmationDetails::AskUserQuestion {
                options,
                multi_select,
                ..
            } => (options.len(), *multi_select),
            _ => (0, false),
        })
        .unwrap_or((0, false));

    let invalidate_cache = |state: &mut ChatState| {
        if let Some(idx) = state.pending_confirmation_entry_idx {
            state.rendered_cache.remove(&idx);
        }
    };

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
                submit_ask_user_question_answer(
                    state,
                    agent_tx,
                    tool_call_id,
                    option_count,
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
        // "Other" is at index option_count (after all regular options)
        let other_idx = option_count;
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
                        submit_ask_user_question_answer(
                            state,
                            agent_tx,
                            tool_call_id,
                            option_count,
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
                } else if multi_select && !state.pending_question_selections.is_empty() {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                } else if !multi_select && option_count > 0 {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                }
                // If multi_select but nothing selected, ignore Enter
            }
            _ => {}
        }
    }

    Ok(())
}

/// Collect the user's answer from the AskUserQuestion UI state and submit it.
pub(super) async fn submit_ask_user_question_answer(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
    option_count: usize,
    multi_select: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect answer labels from the confirmation entry options
    let answers: Vec<String> = if multi_select {
        let mut indices = state.pending_question_selections.clone();
        indices.sort();
        indices
            .iter()
            .filter_map(|&idx| {
                state
                    .pending_confirmation_entry_idx
                    .and_then(|entry_idx| state.chat_history.get(entry_idx))
                    .and_then(|entry| entry.confirmation.as_ref())
                    .and_then(|conf| match &conf.details {
                        crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                            options.get(idx).map(|o| o.label.clone())
                        }
                        _ => None,
                    })
            })
            .collect()
    } else if option_count > 0 {
        let focused = state.pending_confirmation_choice;
        state
            .pending_confirmation_entry_idx
            .and_then(|entry_idx| state.chat_history.get(entry_idx))
            .and_then(|entry| entry.confirmation.as_ref())
            .and_then(|conf| match &conf.details {
                crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                    options.get(focused).map(|o| o.label.clone())
                }
                _ => None,
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };

    let text_input = if state.pending_other_input.is_empty() {
        None
    } else {
        Some(state.pending_other_input.clone())
    };

    // Update ChatEntry outcome text
    if let Some(idx) = state.pending_confirmation_entry_idx {
        if let Some(entry) = state.chat_history.get_mut(idx) {
            if let Some(conf) = entry.confirmation.as_mut() {
                let outcome_str = if answers.is_empty() && text_input.is_some() {
                    format!("User input: {}", text_input.as_ref().unwrap())
                } else if answers.is_empty() {
                    "No selection".to_string()
                } else if answers.len() == 1 {
                    if let Some(ref ti) = text_input {
                        format!("User selected: {} (with input: {})", answers[0], ti)
                    } else {
                        format!("User selected: {}", answers[0])
                    }
                } else {
                    format!(
                        "User selected {} options: {}",
                        answers.len(),
                        answers.join(", ")
                    )
                };
                conf.outcome = Some(outcome_str);
            }
        }
        state.rendered_cache.remove(&idx);
    }

    let _ = agent_tx
        .send(AgentRequest::ConfirmTool {
            tool_call_id: tool_call_id.to_string(),
            outcome: crate::types::ToolConfirmationOutcome::UserAnswer {
                answers,
                text_input,
            },
            feedback: None,
        })
        .await;

    state.clear_pending_confirmation();

    Ok(())
}
