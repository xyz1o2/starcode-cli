use crate::core::routing::RequestComplexity;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoPlanGateDecision {
    pub should_generate: bool,
    pub reason: &'static str,
    pub enabled: bool,
    pub max_history: usize,
}

fn auto_plan_enabled_from_env(raw: Option<String>) -> bool {
    raw.map(|value| {
        let value = value.trim().to_lowercase();
        value == "1" || value == "true" || value == "on"
    })
    .unwrap_or(false)
}

pub fn auto_plan_enabled() -> bool {
    static AUTO_PLAN_ENABLED: OnceLock<bool> = OnceLock::new();
    *AUTO_PLAN_ENABLED
        .get_or_init(|| auto_plan_enabled_from_env(std::env::var("STAR_AUTO_PLAN").ok()))
}

pub fn auto_plan_max_history() -> usize {
    static AUTO_PLAN_MAX_HISTORY: OnceLock<usize> = OnceLock::new();
    *AUTO_PLAN_MAX_HISTORY.get_or_init(|| {
        std::env::var("STAR_AUTO_PLAN_MAX_HISTORY")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(6)
    })
}

pub fn auto_plan_max_chars() -> usize {
    static AUTO_PLAN_MAX_CHARS: OnceLock<usize> = OnceLock::new();
    *AUTO_PLAN_MAX_CHARS.get_or_init(|| {
        std::env::var("STAR_AUTO_PLAN_MAX_CHARS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(6000)
    })
}

pub fn auto_plan_timeout_secs() -> u64 {
    static AUTO_PLAN_TIMEOUT_SECS: OnceLock<u64> = OnceLock::new();
    *AUTO_PLAN_TIMEOUT_SECS.get_or_init(|| {
        std::env::var("STAR_AUTO_PLAN_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(4)
            .clamp(1, 30)
    })
}

fn evaluate_auto_plan_gate_with_limits(
    enabled: bool,
    max_history: usize,
    request_complexity: &RequestComplexity,
    history_len: usize,
) -> AutoPlanGateDecision {
    if !enabled {
        return AutoPlanGateDecision {
            should_generate: false,
            reason: "disabled",
            enabled,
            max_history,
        };
    }
    if !matches!(request_complexity, RequestComplexity::Complex) {
        return AutoPlanGateDecision {
            should_generate: false,
            reason: "non_complex_request",
            enabled,
            max_history,
        };
    }
    if history_len > max_history {
        return AutoPlanGateDecision {
            should_generate: false,
            reason: "history_limit_exceeded",
            enabled,
            max_history,
        };
    }

    AutoPlanGateDecision {
        should_generate: true,
        reason: "eligible",
        enabled,
        max_history,
    }
}

pub fn evaluate_auto_plan_gate(
    request_complexity: &RequestComplexity,
    history_len: usize,
) -> AutoPlanGateDecision {
    evaluate_auto_plan_gate_with_limits(
        auto_plan_enabled(),
        auto_plan_max_history(),
        request_complexity,
        history_len,
    )
}

pub fn detect_skip_verification_pattern(input: &str) -> Option<&'static str> {
    let lower = input.to_lowercase();
    let patterns = [
        "skip tests",
        "skip test",
        "no tests",
        "dont run tests",
        "don't run tests",
        "do not run tests",
        "skip verification",
        "no verification",
    ];
    patterns.into_iter().find(|pattern| lower.contains(pattern))
}

pub fn should_skip_verification(input: &str) -> bool {
    detect_skip_verification_pattern(input).is_some()
}
 