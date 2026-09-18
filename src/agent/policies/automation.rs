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
    // 空串视同未设置（与 is_log_enabled / Router::env_model 的口径一致）：
    // `export STAR_AUTO_PLAN=$UNDEFINED` 这类误写不该静默关掉计划。
    let value = raw.and_then(|v| {
        let v = v.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_lowercase())
        }
    });

    match value {
        // 只有明确的假值才关：与 is_log_enabled 同口径，未识别值不静默关掉
        // 一个默认开启的能力（否则 STAR_AUTO_PLAN=yes_typo 会悄悄禁用计划）。
        Some(v) => !(v == "0" || v == "false" || v == "off" || v == "no" || v == "disabled"),
        // 默认开启：跨文件/多步骤任务依赖前置计划锚定步骤，关掉时这类任务
        // 一上来就缺规划，长程推理随轮次推进快速失焦。需要时可用 STAR_AUTO_PLAN=0 关闭。
        None => true,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个闸门必须同时开才会生成计划：enabled（默认开）× Complex。
    /// 之前这两道闸对跨文件任务是同时关的——router 把短描述判成 Simple，
    /// 而 STAR_AUTO_PLAN 默认 off——所以这类任务永远拿不到前置计划。
    #[test]
    fn complex_request_within_history_is_eligible() {
        let decision = evaluate_auto_plan_gate_with_limits(true, 6, &RequestComplexity::Complex, 2);
        assert!(decision.should_generate);
        assert_eq!(decision.reason, "eligible");
    }

    #[test]
    fn non_complex_request_never_plans_even_when_enabled() {
        let decision = evaluate_auto_plan_gate_with_limits(true, 6, &RequestComplexity::Simple, 2);
        assert!(!decision.should_generate);
        assert_eq!(decision.reason, "non_complex_request");
    }

    #[test]
    fn enabled_flag_short_circuits_before_complexity_check() {
        let decision =
            evaluate_auto_plan_gate_with_limits(false, 6, &RequestComplexity::Complex, 2);
        assert!(!decision.should_generate);
        assert_eq!(decision.reason, "disabled");
    }

    #[test]
    fn long_history_blocks_planning() {
        let decision =
            evaluate_auto_plan_gate_with_limits(true, 6, &RequestComplexity::Complex, 12);
        assert!(!decision.should_generate);
        assert_eq!(decision.reason, "history_limit_exceeded");
    }

    /// 默认开启是本改动的核心：未设 STAR_AUTO_PLAN 时不能退回关闭。
    #[test]
    fn auto_plan_defaults_to_enabled_when_unset() {
        assert!(auto_plan_enabled_from_env(None));
        assert!(auto_plan_enabled_from_env(Some(String::new())));
        assert!(auto_plan_enabled_from_env(Some("garbage".to_string())));
    }

    #[test]
    fn auto_plan_can_be_disabled_explicitly() {
        assert!(!auto_plan_enabled_from_env(Some("0".to_string())));
        assert!(!auto_plan_enabled_from_env(Some("false".to_string())));
        assert!(!auto_plan_enabled_from_env(Some("off".to_string())));
    }
}

pub fn should_skip_verification(input: &str) -> bool {
    detect_skip_verification_pattern(input).is_some()
}
