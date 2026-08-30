/// Conversation summary prompts for context compression.
/// Loaded from centralized .md file for easy maintenance.

use crate::core::prompts::loader;

/// Summary prompt for context compression - loaded from file
/// (external dir overrides embedded, cached via loader).
pub fn summary_prompt() -> String {
    loader::load_prompt("conversation-summary.md")
}
