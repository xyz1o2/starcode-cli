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

pub(crate) fn build_codebase_search_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "CodebaseSearch",
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

/// 构造 skill 工具调用。
///
/// 用户问题必须放进 `args.objective`——SkillTool 只读 `skill` 和 `args`，
/// 其它字段（包括旧的 `task` 和 `turn`）被 serde 静默丢弃。若放在 `task` 里，
/// 子代理的 objective 会回退成技能名本身（如把 "navigator" 当成语义查询）。
fn build_skill_tool_call(skill: &str, user_input: &str, _turn: i32) -> StarToolCall {
    build_tool_call(
        "skill",
        &serde_json::json!({
            "skill": skill,
            "args": {"objective": user_input}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归 ACE-01：用户问题曾放在 `task` 字段里，而 SkillTool 只读
    /// `args.objective`，导致 navigator 子代理拿字符串 "navigator"
    /// 当成语义查询。真实问题必须出现在 objective 上。
    #[test]
    fn skill_call_carries_question_in_objective() {
        let call = build_navigator_skill_tool_call("where is auth handled", 3);
        assert_eq!(call.function.name, "skill");

        let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
        assert_eq!(
            args.get("args")
                .and_then(|a| a.get("objective"))
                .and_then(|o| o.as_str()),
            Some("where is auth handled"),
            "user question must land in args.objective"
        );
        // 旧的泄漏字段不能还在
        assert!(
            args.get("task").is_none(),
            "stale `task` field must be gone"
        );
        assert!(
            args.get("turn").is_none(),
            "stale `turn` field must be gone"
        );
    }

    #[test]
    fn analyzer_and_editor_skill_calls_also_carry_objective() {
        for call in [
            build_analyzer_skill_tool_call("explain the loop", 1),
            build_editor_skill_tool_call("fix the bug", 1),
        ] {
            let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
            let objective = args
                .get("args")
                .and_then(|a| a.get("objective"))
                .and_then(|o| o.as_str());
            assert!(objective.is_some_and(|o| !o.is_empty() && o != "analyzer" && o != "editor"));
        }
    }
}
