use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// ScheduleWakeup — allows the AI to schedule a one-shot wakeup event.
/// Used by autonomous /loop mode for self-paced work intervals.
#[derive(Clone)]
pub struct ScheduleWakeupTool {
    /// Track last wakeup call time to prevent rapid loops
    last_call_time: Arc<AtomicI64>,
}

impl ScheduleWakeupTool {
    pub fn new() -> Self {
        Self {
            last_call_time: Arc::new(AtomicI64::new(0)),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ScheduleWakeupParams {
    /// Seconds from now to wake up (clamped 60..3600)
    pub delay_seconds: f64,
    /// Short reason for the delay
    pub reason: String,
    /// The prompt to fire on wakeup
    pub prompt: String,
}

pub struct ScheduleWakeupInvocation {
    params: ScheduleWakeupParams,
    last_call_time: Arc<AtomicI64>,
}

impl ToolInvocation for ScheduleWakeupInvocation {
    fn get_description(&self) -> String {
        format!(
            "Schedule wakeup in {:.0}s: {}",
            self.params.delay_seconds, self.params.reason
        )
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
        let delay = self.params.delay_seconds.clamp(60.0, 3600.0);
        let reason = self.params.reason.clone();
        let prompt = self.params.prompt.clone();
        let signal = signal.cloned();
        let last_call_time = self.last_call_time.clone();

        Box::pin(async move {
            // Cooldown check: prevent rapid successive calls (minimum 30 seconds between calls)
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let last_call = last_call_time.load(Ordering::Relaxed);
            let cooldown_remaining = 30 - (now - last_call);
            if cooldown_remaining > 0 {
                return Ok(ToolResult {
                    llm_content: Some(format!(
                        "Wakeup cooldown: please wait {} seconds before scheduling another wakeup.",
                        cooldown_remaining
                    )),
                    return_display: Some(format!("Cooldown: {}s remaining", cooldown_remaining)),
                    output: format!("Cooldown active. Wait {} seconds.", cooldown_remaining),
                    error: None,
                    data: None,
                });
            }
            last_call_time.store(now, Ordering::Relaxed);

            // If there's a cancellation signal, support early abort
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs_f64(delay)) => {
                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Waking up after {:.0}s. Reason: {}. Executing: {}",
                            delay, reason, prompt
                        )),
                        return_display: Some(format!("Woke up after {:.0}s", delay)),
                        output: format!(
                            "Wakeup after {:.0}s ({}). Resuming with: {}",
                            delay, reason, prompt
                        ),
                        error: None,
                        data: Some(serde_json::json!({
                            "wakeup": true,
                            "delay_seconds": delay,
                            "reason": reason,
                            "prompt": prompt,
                        })),
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
                        llm_content: Some("Wakeup cancelled by user".to_string()),
                        return_display: Some("Cancelled".to_string()),
                        output: "Wakeup cancelled".to_string(),
                        error: None,
                        data: None,
                    })
                }
            }
        })
    }
}

impl BaseDeclarativeTool for ScheduleWakeupTool {
    fn name(&self) -> &str {
        "schedule_wakeup"
    }

    fn display_name(&self) -> &str {
        "Schedule Wakeup"
    }

    fn description(&self) -> &str {
        "安排一次性唤醒事件。用于自主循环模式中自我调节工作节奏。(Schedule a one-shot wakeup event. Used by autonomous loop mode for self-paced work intervals. The runtime clamps delay to [60, 3600] seconds.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "number",
                    "description": "Seconds from now to wake up (60-3600). Pick cache-friendly values: under 270s keeps prompt cache warm, over 1200s amortizes the cache miss."
                },
                "reason": {
                    "type": "string",
                    "description": "One short sentence explaining the chosen delay. Shown to user."
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to enqueue at fire time."
                }
            },
            "required": ["delay_seconds", "reason", "prompt"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ScheduleWakeupParams = serde_json::from_value(params)?;
        Ok(Box::new(ScheduleWakeupInvocation {
            params,
            last_call_time: self.last_call_time.clone(),
        }))
    }
}
