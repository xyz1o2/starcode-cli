//! EnterPlanModeTool — 进入计划模式

use async_trait::async_trait;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::core::tools::tools::{
    BaseDeclarativeTool, ConfirmationType, Kind, ToolCallConfirmationDetails, ToolError,
    ToolInvocation, ToolLocation, ToolResult,
};
use crate::types::{ApprovalMode, ToolConfirmationOutcome};

/// 计划模式状态管理
#[derive(Debug, Clone)]
pub struct PlanModeState {
    pub active: bool,
    pub previous_mode: ApprovalMode,
    pub current_plan_id: Option<String>,
    pub allowed_prompts: Vec<AllowedPrompt>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AllowedPrompt {
    pub tool: String,
    pub prompt: String,
}

impl Default for PlanModeState {
    fn default() -> Self {
        Self {
            active: false,
            previous_mode: ApprovalMode::Default,
            current_plan_id: None,
            allowed_prompts: Vec::new(),
        }
    }
}

pub struct EnterPlanModeTool {
    pub plan_mode_state: Arc<Mutex<PlanModeState>>,
    pub approval_mode: Arc<Mutex<ApprovalMode>>,
}

impl EnterPlanModeTool {
    pub fn new(
        plan_mode_state: Arc<Mutex<PlanModeState>>,
        approval_mode: Arc<Mutex<ApprovalMode>>,
    ) -> Self {
        Self {
            plan_mode_state,
            approval_mode,
        }
    }
}

pub struct EnterPlanModeInvocation {
    params: Value,
    plan_mode_state: Arc<Mutex<PlanModeState>>,
    approval_mode: Arc<Mutex<ApprovalMode>>,
}

#[async_trait]
impl ToolInvocation for EnterPlanModeInvocation {
    fn get_description(&self) -> String {
        "Enter Plan Mode".to_string()
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        Vec::new()
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: ConfirmationType::Ask,
                title: "Enter Plan Mode".to_string(),
                prompt: "Enter Plan Mode: AI will switch to read-only exploration.".to_string(),
                on_confirm: Arc::new(|_| {}),
            }))
        })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let state = self.plan_mode_state.clone();
        let approval = self.approval_mode.clone();
        let reason = self
            .params
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("Planning required")
            .to_string();

        Box::pin(async move {
            {
                let s = state.lock().unwrap();
                if s.active {
                    return Ok(ToolResult {
                        output: "Already in Plan Mode".to_string(),
                        error: Some(ToolError {
                            error_type: "plan_mode".to_string(),
                            message: "Already in Plan Mode".to_string(),
                        }),
                        ..Default::default()
                    });
                }
            }

            let current_mode = { approval.lock().unwrap().clone() };

            {
                let mut s = state.lock().unwrap();
                s.active = true;
                s.previous_mode = current_mode.clone();
                s.allowed_prompts.clear();
            }

            {
                let mut m = approval.lock().unwrap();
                *m = ApprovalMode::Plan;
            }

            let msg = format!(
                "Entered Plan Mode. Reason: {}\n\nYou are now in read-only exploration mode. \
                 When ready, call exit_plan_mode with your plan.",
                reason
            );

            Ok(ToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg),
                output: String::new(),
                error: None,
                data: Some(json!({ "mode": "plan", "reason": reason })),
            })
        })
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
        "Enter Plan Mode for read-only exploration before making changes."
    }
    fn kind(&self) -> Kind {
        Kind::Think
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "Why plan mode is needed" }
            },
            "required": ["reason"]
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(EnterPlanModeInvocation {
            params,
            plan_mode_state: self.plan_mode_state.clone(),
            approval_mode: self.approval_mode.clone(),
        }))
    }
}
