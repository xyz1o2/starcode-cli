use crate::agent::tool_routing::{is_memory_tool_name, truncate_chars};
use crate::types::{StarToolCall, ToolResult};

fn validation_failure_summary(tool_name: &str, result: &ToolResult) -> Option<String> {
    let lower = tool_name.to_lowercase();
    if lower.contains("run_tests") {
        return run_tests_failure_summary(result);
    }
    if lower.contains("diagnostics") {
        return diagnostics_failure_summary(result);
    }
    if lower.contains("lint") || lower.contains("check") {
        return generic_failure_summary(result, "validation reported errors");
    }
    None
}

fn run_tests_failure_summary(result: &ToolResult) -> Option<String> {
    if let Some(data) = &result.data {
        if let Some(status) = data.get("status").and_then(|v| v.as_str()) {
            if status == "failed" {
                let summary = data
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tests failed");
                return Some(format!("run_tests failed: {}", summary));
            }
        }
    }
    if let Some(output) = &result.output {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(status) = json.get("status").and_then(|v| v.as_str()) {
                if status == "failed" {
                    let summary = json
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tests failed");
                    return Some(format!("run_tests failed: {}", summary));
                }
            }
        }
        let lower = output.to_lowercase();
        if lower.contains("failed") || lower.contains("fail") || lower.contains("error") {
            return Some("run_tests failed".to_string());
        }
    }
    None
}

fn diagnostics_failure_summary(result: &ToolResult) -> Option<String> {
    let output = result.output.as_deref().unwrap_or("").trim();
    if output.is_empty() {
        return None;
    }
    if output.contains("No diagnostics found") {
        return None;
    }
    if output.contains("[ERROR]") || output.contains(" ERROR ") {
        return Some("diagnostics reported errors".to_string());
    }
    None
}

fn generic_failure_summary(result: &ToolResult, fallback: &str) -> Option<String> {
    let output = result.output.as_deref().unwrap_or("").to_lowercase();
    if output.contains("error") || output.contains("fail") {
        return Some(fallback.to_string());
    }
    None
}

pub(crate) async fn maybe_write_reflection_memory(
    user_input: &str,
    tool_call: &StarToolCall,
    result: &ToolResult,
) {
    let enabled = std::env::var("STAR_AUTO_REFLECTION")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true);
    if !enabled {
        return;
    }

    if is_memory_tool_name(tool_call.function.name.as_str()) {
        return;
    }

    let failure_summary = if !result.success {
        Some(format!(
            "tool '{}' failed: {}",
            tool_call.function.name,
            result
                .error
                .clone()
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    } else {
        validation_failure_summary(&tool_call.function.name, result)
    };

    let Some(summary) = failure_summary else {
        return;
    };

    let task_excerpt = truncate_chars(user_input, 180);
    let reflection_text = format!(
        "Reflexion: task='{}'; {}. Consider adjusting inputs or approach.",
        task_excerpt, summary
    );

    let tool = crate::tools::memory::MemoryTool::new();
    let params = crate::tools::memory::MemoryParams {
        action: "save".to_string(),
        content: Some(reflection_text),
        query: None,
    };

    if let Err(err) = tool.execute_memory_op(&params).await {
        crate::utils::logging::append_debug_log_line(&format!(
            "[REFLEXION] Failed to write memory: {}",
            err
        ));
    }
}
