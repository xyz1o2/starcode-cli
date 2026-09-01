use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolInvocation, ToolLocation,
    ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterPlanModeParams {
    pub reason: String,
}

pub struct EnterPlanModeTool {
    message_bus: Arc<MessageBus>,
}

impl EnterPlanModeTool {
    pub fn new(_config: Arc<Config>, message_bus: Arc<MessageBus>) -> Self {
        Self { message_bus }
    }
}

impl BaseDeclarativeTool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn display_name(&self) -> &str {
        "Enter Plan Mode"
    }

    fn description(&self) -> &str {
        "Transitions the agent into Plan Mode for explicit user-approved planning. Do not use this by default for ordinary implementation work; plan internally and proceed unless the user asks for plan mode, the requirements are materially ambiguous, or the operation is high risk."
    }

    fn kind(&self) -> Kind {
        Kind::Other
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "The reason for entering plan mode (e.g., 'Designing authentication flow')."
                }
            },
            "required": ["reason"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: EnterPlanModeParams = serde_json::from_value(params)?;
        Ok(Box::new(EnterPlanModeInvocation {
            params,
            message_bus: self.message_bus.clone(),
        }))
    }
}

pub struct EnterPlanModeInvocation {
    params: EnterPlanModeParams,
    message_bus: Arc<MessageBus>,
}

impl ToolInvocation for EnterPlanModeInvocation {
    fn get_description(&self) -> String {
        format!("Enter Plan Mode: {}", self.params.reason)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let reason = self.params.reason.clone();
        let message_bus = self.message_bus.clone();
        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                title: "Enter Plan Mode?".to_string(),
                prompt: format!(
                    "The agent wants to enter Plan Mode for the following reason:\n\n{}",
                    reason
                ),
                on_confirm: std::sync::Arc::new(move |outcome| {
                    let confirmed = matches!(
                        outcome,
                        crate::types::ToolConfirmationOutcome::ProceedOnce
                            | crate::types::ToolConfirmationOutcome::ProceedAlways
                            | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave
                            | crate::types::ToolConfirmationOutcome::AllowSession
                    );

                    if confirmed {
                        let bus = message_bus.clone();
                        tokio::spawn(async move {
                            bus.set_approval_mode(crate::core::policy::types::ApprovalMode::Plan)
                                .await;
                        });
                    }
                }),
            }))
        })
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
        Box::pin(async move {
            Ok(ToolResult {
                llm_content: Some("Entered Plan Mode. You are now in a restricted mode where write operations are disabled. Use 'exit_plan_mode' when you are ready to implement.".to_string()),
                return_display: Some("Entered Plan Mode.".to_string()),
                output: "Entered Plan Mode successfully.".to_string(),
                error: None,
                data: None,
            })
        })
    }
}
