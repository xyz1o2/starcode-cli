//! ExitPlanModeTool — 退出计划模式并提交方案

use async_trait::async_trait;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::core::plan::{PlanManager, Plan};
use crate::core::tools::tools::{
    BaseDeclarativeTool, ConfirmationType, Kind, ToolCallConfirmationDetails, ToolError,
    ToolInvocation, ToolLocation, ToolResult,
};
use crate::types::ApprovalMode;
use super::enter_plan_mode::{AllowedPrompt, PlanModeState};

pub struct ExitPlanModeTool {
    pub plan_mode_state: Arc<Mutex<PlanModeState>>,
    pub approval_mode: Arc<Mutex<ApprovalMode>>,
}

impl ExitPlanModeTool {
    pub fn new(
        plan_mode_state: Arc<Mutex<PlanModeState>>,
        approval_mode: Arc<Mutex<ApprovalMode>>,
    ) -> Self {
        Self { plan_mode_state, approval_mode }
    }
}

pub struct ExitPlanModeInvocation {
    params: Value,
    plan_mode_state: Arc<Mutex<PlanModeState>>,
    approval_mode: Arc<Mutex<ApprovalMode>>,
}

#[async_trait]
impl ToolInvocation for ExitPlanModeInvocation {
    fn get_description(&self) -> String {
        "Exit Plan Mode".to_string()
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        Vec::new()
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ToolCallConfirmationDetails>, Box<dyn std::error::Error + Send + Sync>>> + Send + '_>>
    {
        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: ConfirmationType::Ask,
                title: "Exit Plan Mode".to_string(),
                prompt: "Exit Plan Mode: Submit your plan for user approval.".to_string(),
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
        let params = self.params.clone();

        Box::pin(async move {
            {
                let s = state.lock().unwrap();
                if !s.active {
                    return Ok(ToolResult {
                        output: "Not in Plan Mode".to_string(),
                        error: Some(ToolError {
                            error_type: "plan_mode".to_string(),
                            message: "Not in Plan Mode".to_string(),
                        }),
                        ..Default::default()
                    });
                }
            }

            let plan_content = params.get("plan").and_then(|p| p.as_str()).unwrap_or("").to_string();
            let title = params.get("title").and_then(|t| t.as_str()).unwrap_or("Execution Plan").to_string();

            let allowed_prompts: Vec<AllowedPrompt> = params
                .get("allowedPrompts")
                .and_then(|ap| serde_json::from_value(ap.clone()).ok())
                .unwrap_or_default();

            let plan_mgr = PlanManager::new();
            let mut plan = Plan::new(title.clone(), plan_content.clone());
            plan.tasks = PlanManager::parse_tasks_from_markdown(&plan_content);
            let plan_id = plan.id.clone();

            if let Err(e) = plan_mgr.save_plan(&plan) {
                log::warn!("Failed to persist plan: {}", e);
            }

            let previous_mode = { state.lock().unwrap().previous_mode.clone() };

            {
                let mut s = state.lock().unwrap();
                s.active = false;
                s.current_plan_id = Some(plan_id.clone());
                s.allowed_prompts = allowed_prompts.clone();
            }

            {
                let mut m = approval.lock().unwrap();
                *m = previous_mode.clone();
            }

            let msg = format!(
                "Plan submitted for approval.\nPlan ID: {}\nTitle: {}\nTasks: {}\n\n{}",
                plan_id, title, plan.tasks.len(), plan_content
            );

            Ok(ToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg),
                output: String::new(),
                error: None,
                data: Some(json!({
                    "plan_id": plan_id,
                    "title": title,
                    "task_count": plan.tasks.len(),
                    "allowed_prompts": allowed_prompts,
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for ExitPlanModeTool {
    fn name(&self) -> &str { "exit_plan_mode" }
    fn display_name(&self) -> &str { "Exit Plan Mode" }
    fn description(&self) -> &str {
        "Exit Plan Mode and submit your execution plan for user approval."
    }
    fn kind(&self) -> Kind { Kind::Think }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "plan": { "type": "string", "description": "The execution plan in Markdown" },
                "title": { "type": "string", "description": "A short title for the plan" },
                "allowedPrompts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string" },
                            "prompt": { "type": "string" }
                        },
                        "required": ["tool", "prompt"]
                    }
                }
            },
            "required": ["plan"]
        })
    }

    fn create_invocation(&self, params: Value) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(ExitPlanModeInvocation {
            params,
            plan_mode_state: self.plan_mode_state.clone(),
            approval_mode: self.approval_mode.clone(),
        }))
    }
}
