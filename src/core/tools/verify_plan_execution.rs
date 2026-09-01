use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct VerifyPlanExecutionTool;

impl VerifyPlanExecutionTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct VerifyPlanParams {
    pub plan_summary: String,
    pub verification_notes: Option<String>,
    pub all_steps_completed: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct VerifyPlanOutput {
    pub verified: bool,
    pub summary: String,
}

pub struct VerifyPlanInvocation {
    params: VerifyPlanParams,
}

impl ToolInvocation for VerifyPlanInvocation {
    fn get_description(&self) -> String {
        format!(
            "Verify plan execution: {}",
            if self.params.all_steps_completed {
                "all steps completed"
            } else {
                "incomplete"
            }
        )
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
            let verified = params.all_steps_completed;
            let summary = params.plan_summary.clone();

            let llm_content = if verified {
                format!("Plan verified: {}", summary)
            } else {
                format!("Plan verification failed: {}", summary)
            };

            let return_display = if verified {
                "Plan verified: all steps completed".to_string()
            } else {
                "Plan verification failed: incomplete".to_string()
            };

            Ok(ToolResult {
                llm_content: Some(llm_content),
                return_display: Some(return_display),
                output: serde_json::to_string(&VerifyPlanOutput {
                    verified,
                    summary,
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "verified": verified,
                    "summary": params.plan_summary,
                    "notes": params.verification_notes
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for VerifyPlanExecutionTool {
    fn name(&self) -> &str {
        "verify_plan_execution"
    }

    fn display_name(&self) -> &str {
        "VerifyPlan"
    }

    fn description(&self) -> &str {
        "验证计划是否已正确执行，在退出计划模式前确认所有步骤已完成。(Verify that a plan was executed correctly before exiting plan mode.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan_summary": {
                    "type": "string",
                    "description": "已执行计划的摘要 (A summary of the plan that was executed.)"
                },
                "verification_notes": {
                    "type": "string",
                    "description": "验证过程中的注释和发现的问题 (Notes on what was verified and any issues found during verification.)"
                },
                "all_steps_completed": {
                    "type": "boolean",
                    "description": "所有计划步骤是否已成功完成 (Whether all planned steps were completed successfully.)"
                }
            },
            "required": ["plan_summary", "all_steps_completed"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: VerifyPlanParams = serde_json::from_value(params)?;
        Ok(Box::new(VerifyPlanInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}