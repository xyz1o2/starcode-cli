use std::collections::HashSet;

use crate::core::prompts::{task_agent_usage, tool_list};

pub fn render(is_thinking_model: bool) -> String {
    render_for_tools(is_thinking_model, None)
}

pub fn render_for_tools(is_thinking_model: bool, active_tools: Option<&HashSet<String>>) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push("## Tools".to_string());
    parts.push(tool_list::render(is_thinking_model));
    if should_include_task_agent_usage(active_tools) {
        parts.push(task_agent_usage::render());
    }
    parts.join("\n\n")
}

fn should_include_task_agent_usage(active_tools: Option<&HashSet<String>>) -> bool {
    let Some(active_tools) = active_tools else {
        return true;
    };

    active_tools.iter().any(|tool| {
        matches!(
            tool.as_str(),
            "skill" | "Agent" | "delegate" | "task" | "TodoWrite"
        )
    })
}
