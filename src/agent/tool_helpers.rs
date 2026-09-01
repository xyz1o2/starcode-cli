use crate::agent::tool_executor::ToolExecutor;
use crate::agent::tool_routing::{is_edit_tool_name, is_validation_tool_name};
use crate::types::{StarToolCall, ToolResult};
use std::sync::Arc;

pub(crate) fn execute_single_tool_with_progress<'a>(
    tool_executor: Arc<ToolExecutor>,
    tool_call: StarToolCall,
    abort_signal: Option<tokio_util::sync::CancellationToken>,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<String>,
    impl std::future::Future<Output = ToolResult> + Send + 'a,
) {
    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let update_output: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |msg| {
        let _ = progress_tx.send(msg);
    });

    let future = async move {
        let exec = tool_executor.execute_batch(vec![tool_call], Some(update_output), abort_signal);
        // 兜底超时：防止工具（尤其 SemanticSearch/ProjectMap/Grep 等长运行工具）
        // 内部挂起导致 emit_tool_finished 永不发出、UI 圆点永远闪烁。
        // 超时后返回带超时提示的 error result，调用方仍会 emit finished 停止闪烁。
        const TOOL_HARD_TIMEOUT_SECS: u64 = 600;
        match tokio::time::timeout(
            std::time::Duration::from_secs(TOOL_HARD_TIMEOUT_SECS),
            exec,
        )
        .await
        {
            Ok(results) => results
                .into_iter()
                .next()
                .unwrap_or(ToolResult {
                    success: false,
                    output: None,
                    error: Some("tool executor returned no result".to_string()),
                    data: None,
                }),
            Err(_) => ToolResult {
                success: false,
                output: None,
                error: Some(format!(
                    "Tool execution timed out after {}s",
                    TOOL_HARD_TIMEOUT_SECS
                )),
                data: None,
            },
        }
    };

    (progress_rx, future)
}

pub(crate) fn update_verification_state(
    tool_call: &StarToolCall,
    result: &ToolResult,
    verification_required: &mut bool,
    skip_verification: bool,
) {
    if is_edit_tool_name(&tool_call.function.name) && result.success && !skip_verification {
        *verification_required = true;
    }
    if is_validation_tool_name(&tool_call.function.name) {
        *verification_required = false;
    }
}

pub(crate) fn sanitize_reasoning_content(input: &str) -> String {
    let mut result = String::new();
    let mut skip_until_close = false;
    let mut close_tag = String::new();

    for line in input.lines() {
        let trimmed = line.trim();

        if skip_until_close {
            if trimmed == close_tag || trimmed.ends_with(&close_tag) {
                skip_until_close = false;
                close_tag.clear();
            }
            continue;
        }

        if trimmed.starts_with("<function=") || trimmed.starts_with("<tool_call>") {
            skip_until_close = true;
            close_tag = if trimmed.starts_with("<function=") {
                "</function>".to_string()
            } else {
                "</tool_call>".to_string()
            };
            continue;
        }

        if trimmed.starts_with("<parameter=") || trimmed.starts_with("</parameter>") {
            continue;
        }

        if trimmed == "<tool>" || trimmed == "</tool>" {
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    result.trim().to_string()
}
