use crate::agent::planner::Planner;
use crate::llm::client::StarClient;

/// 自动计划决策
#[derive(Debug, Clone)]
pub(crate) struct AutoPlanDecision {
    pub(crate) plan: Option<String>,
    pub(crate) reason: &'static str,
    pub(crate) history_len: usize,
    pub(crate) max_history: usize,
    pub(crate) max_chars: usize,
    pub(crate) request_complexity: &'static str,
    pub(crate) plan_chars: usize,
    pub(crate) was_truncated: bool,
}

/// 可能生成自动计划
pub(crate) async fn maybe_generate_auto_plan(
    client: &StarClient,
    user_input: &str,
    request_complexity: &crate::core::routing::RequestComplexity,
    history_len: usize,
    is_thinking_model: bool,
) -> AutoPlanDecision {
    let gate = crate::agent::policies::automation::evaluate_auto_plan_gate(
        request_complexity,
        history_len,
    );
    let max_chars = crate::agent::policies::automation::auto_plan_max_chars();
    let timeout_secs = crate::agent::policies::automation::auto_plan_timeout_secs();
    let request_complexity = super::helpers::request_complexity_label(*request_complexity);

    if !gate.should_generate {
        return AutoPlanDecision {
            plan: None,
            reason: gate.reason,
            history_len,
            max_history: gate.max_history,
            max_chars,
            request_complexity,
            plan_chars: 0,
            was_truncated: false,
        };
    }

    let tool_catalog = crate::core::prompts::tool_catalog::render(is_thinking_model);
    let plan = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        Planner::make_plan(client, user_input, None, &tool_catalog),
    )
    .await
    {
        Ok(Ok(plan)) => plan,
        Ok(Err(_)) => {
            return AutoPlanDecision {
                plan: None,
                reason: "planner_failed",
                history_len,
                max_history: gate.max_history,
                max_chars,
                request_complexity,
                plan_chars: 0,
                was_truncated: false,
            };
        }
        Err(_) => {
            return AutoPlanDecision {
                plan: None,
                reason: "planner_timeout",
                history_len,
                max_history: gate.max_history,
                max_chars,
                request_complexity,
                plan_chars: 0,
                was_truncated: false,
            };
        }
    };
    let plan = plan.trim();
    if plan.is_empty() {
        return AutoPlanDecision {
            plan: None,
            reason: "empty_plan",
            history_len,
            max_history: gate.max_history,
            max_chars,
            request_complexity,
            plan_chars: 0,
            was_truncated: false,
        };
    }

    let plan_chars = plan.chars().count();
    let was_truncated = plan_chars > max_chars;
    let plan = if was_truncated {
        plan.chars().take(max_chars).collect::<String>()
    } else {
        plan.to_string()
    };

    AutoPlanDecision {
        plan: Some(plan),
        reason: "generated",
        history_len,
        max_history: gate.max_history,
        max_chars,
        request_complexity,
        plan_chars,
        was_truncated,
    }
}
