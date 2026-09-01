use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;

#[derive(Clone)]
pub struct WaitTool;

impl WaitTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WaitParams {
    pub seconds: f64,
    #[serde(default)]
    pub reason: Option<String>,
}

pub struct WaitInvocation {
    params: WaitParams,
}

impl ToolInvocation for WaitInvocation {
    fn get_description(&self) -> String {
        let reason = self
            .params
            .reason
            .as_deref()
            .unwrap_or("waiting");
        format!("Wait {:.1}s ({})", self.params.seconds, reason)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let seconds = self.params.seconds.clamp(0.1, 300.0);
        let reason = self
            .params
            .reason
            .clone()
            .unwrap_or_else(|| "waiting".to_string());
        let signal = signal.cloned();

        Box::pin(async move {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)) => {
                    Ok(ToolResult {
                        llm_content: Some(format!("Waited {:.1}s ({})", seconds, reason)),
                        return_display: Some(format!("Waited {:.1}s", seconds)),
                        output: format!("Waited {:.1}s ({})", seconds, reason),
                        error: None,
                        data: None,
                    })
                }
                _ = async {
                    if let Some(s) = &signal {
                        s.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    Ok(ToolResult {
                        llm_content: Some("Wait cancelled by user".to_string()),
                        return_display: Some("Cancelled".to_string()),
                        output: "Wait cancelled".to_string(),
                        error: None,
                        data: None,
                    })
                }
            }
        })
    }
}

impl BaseDeclarativeTool for WaitTool {
    fn name(&self) -> &str {
        "wait"
    }

    fn display_name(&self) -> &str {
        "Wait"
    }

    fn description(&self) -> &str {
        "暂停执行指定秒数。可用于限速或等待外部进程。(Pause execution for a specified duration. Useful for rate-limiting or waiting for external processes.)"
    }

    fn kind(&self) -> Kind {
        Kind::Other
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "number",
                    "description": "Duration in seconds (0.1-300)"
                },
                "reason": {
                    "type": "string",
                    "description": "Why the pause is needed"
                }
            },
            "required": ["seconds"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WaitParams = serde_json::from_value(params)?;
        Ok(Box::new(WaitInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
