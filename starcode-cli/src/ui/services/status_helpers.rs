use crate::ui::utils::text::format_elapsed_for_tool;
use super::*;
use std::time::Instant;

use crate::core::i18n;
use crate::runtime::messages::StreamMessage;
use crate::types::{ChatEntry, ChatEntryType};
use crate::ui::state::store::ChatState;

pub(super) fn should_suppress_redundant_result_after_confirmation(
    tool_call: &crate::types::StarToolCall,
    tool_result: &crate::types::ToolResult,
) -> bool {
    tool_result.success
        && matches!(
            tool_call.function.name.as_str(),
            "enter_plan_mode" | "exit_plan_mode"
        )
}

 
pub(super) fn truncate_status_detail(detail: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    for (idx, ch) in detail.chars().enumerate() {
        if idx >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

pub(super) fn format_tool_name_for_status(name: &str) -> String {
    match name {
        "SemanticSearch" => "semantic search".to_string(),
        "ProjectMap" => "project map".to_string(),
        "Write" => "write file".to_string(),
        "Read" => "read file".to_string(),
        "multi_edit" => "multi edit".to_string(),
        "enter_plan_mode" => "enter plan mode".to_string(),
        "exit_plan_mode" => "exit plan mode".to_string(),
        _ => name.replace(['_', '-'], " "),
    }
}

pub(super) fn format_running_tool_label(state: &ChatState, tool_call_id: &str, tool_name: &str) -> String {
    let pretty_name = format_tool_name_for_status(tool_name);
    match state
        .tool_started_at
        .get(tool_call_id)
        .map(|started_at| started_at.elapsed().as_millis())
    {
        Some(ms) if ms >= 1_000 => format!("{} ({})", pretty_name, format_elapsed_for_tool(ms)),
        _ => pretty_name,
    }
}
