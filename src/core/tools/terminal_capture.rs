use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct TerminalCaptureTool;

impl TerminalCaptureTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TerminalCaptureParams {
    pub lines: Option<u32>,
    pub panel_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TerminalCaptureOutput {
    pub content: String,
    pub line_count: u32,
}

pub struct TerminalCaptureInvocation {
    params: TerminalCaptureParams,
}

impl ToolInvocation for TerminalCaptureInvocation {
    fn get_description(&self) -> String {
        format!("Capture terminal output ({} lines)", 
            self.params.lines.unwrap_or(50))
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let lines = params.lines.unwrap_or(50);
            let panel_id = params.panel_id.unwrap_or_else(|| "current".to_string());

            // In a real implementation, this would:
            // 1. Capture the terminal output from the specified panel
            // 2. Return the captured content

            // For now, return a placeholder response
            let content = format!("Terminal output captured from panel '{}'\n", panel_id);
            let content = content + &"Example output line\n".repeat(lines as usize);

            Ok(ToolResult {
                llm_content: Some(format!("Captured {} lines from terminal panel '{}'", lines, panel_id)),
                return_display: Some(format!("Terminal captured: {} lines", lines)),
                output: serde_json::to_string(&TerminalCaptureOutput {
                    content: content.clone(),
                    line_count: lines,
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "lines": lines,
                    "panel_id": panel_id,
                    "content_length": content.len()
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for TerminalCaptureTool {
    fn name(&self) -> &str {
        "terminal_capture"
    }

    fn display_name(&self) -> &str {
        "TerminalCapture"
    }

    fn description(&self) -> &str {
        "捕获终端面板的输出内容。(Capture the output content of a terminal panel.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "lines": {
                    "type": "integer",
                    "description": "要捕获的行数，默认50 (Number of lines to capture, default: 50)"
                },
                "panel_id": {
                    "type": "string",
                    "description": "终端面板ID，默认为当前面板 (Terminal panel ID, defaults to current panel)"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TerminalCaptureParams = serde_json::from_value(params)?;
        Ok(Box::new(TerminalCaptureInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}