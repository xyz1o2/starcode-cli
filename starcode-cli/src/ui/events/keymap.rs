use crossterm::event::{KeyCode, KeyEvent};

use crate::ui::state::ChatState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    SelectPrev,
    SelectNext,
    AcceptSuggestion,
    AcceptCompletion,
}

pub fn map_key(state: &ChatState, key: &KeyEvent) -> Option<UiAction> {
    match key.code {
        KeyCode::Up => {
            if state.show_provider_menu
                || state.show_session_menu
                || state.show_mention_hints
                || state.show_command_hints
            {
                Some(UiAction::SelectPrev)
            } else {
                None
            }
        }
        KeyCode::Down => {
            if state.show_provider_menu
                || state.show_session_menu
                || state.show_mention_hints
                || state.show_command_hints
            {
                Some(UiAction::SelectNext)
            } else {
                None
            }
        }
        KeyCode::Enter => {
            if state.show_provider_menu
                || state.show_session_menu
                || state.show_mention_hints
                || state.show_command_hints
            {
                Some(UiAction::AcceptSuggestion)
            } else {
                None
            }
        }
        KeyCode::Tab | KeyCode::Char('\t') => {
            if state.show_mention_hints || state.show_command_hints {
                Some(UiAction::AcceptCompletion)
            } else {
                None
            }
        }
        _ => None,
    }
}
