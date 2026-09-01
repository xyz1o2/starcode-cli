use crate::types::StarToolCall;

pub(crate) fn build_analyzer_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("analyzer", user_input, turn)
}

pub(crate) fn build_editor_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("editor", user_input, turn)
}

pub(crate) fn build_navigator_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("navigator", user_input, turn)
}

pub(crate) fn build_semantic_search_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "SemanticSearch",
        &serde_json::json!({"query": user_input, "turn": turn}),
    )
}

pub(crate) fn build_project_map_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "ProjectMap",
        &serde_json::json!({"query": user_input, "turn": turn}),
    )
}

pub(crate) fn build_validation_tool_call(turn: i32) -> StarToolCall {
    build_tool_call("get_diagnostics", &serde_json::json!({"turn": turn}))
}

pub(crate) fn build_json_fallback_prompt(
    content: &str,
    active_tools: &std::collections::HashSet<String>,
) -> String {
    let tools_str: Vec<&str> = active_tools.iter().map(|s| s.as_str()).collect();
    let tools_joined = tools_str.join(", ");
    format!(
        "Please respond with valid JSON. Active tools: {}\n\n{}",
        tools_joined, content
    )
}

pub(crate) fn json_fallback_extract_tool_call(response_text: &str) -> Option<StarToolCall> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(response_text) {
        if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
            let args = json
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            return Some(build_tool_call(name, &args));
        }
    }
    None
}

fn build_skill_tool_call(skill: &str, user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "skill",
        &serde_json::json!({
            "skill": skill,
            "task": user_input,
            "turn": turn
        }),
    )
}

fn build_tool_call(name: &str, args: &serde_json::Value) -> StarToolCall {
    StarToolCall {
        id: format!(
            "call_{}_{}",
            name,
            uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
        ),
        call_type: "function".to_string(),
        function: crate::types::StarToolCallFunction {
            name: name.to_string(),
            arguments: serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
        },
    }
}
