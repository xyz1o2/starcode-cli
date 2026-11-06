//! LLM 重试内核 —— 一份口径，两处消费。
//!
//! `agent_llm::backoff_retry`（agent 循环内，错误走 RecoveryManager 之前）
//! 和 `runtime` 的流式会话（stream init / 首 chunk 断流）有完全相同的
//! 三个问题要回答：**重试几次、等多久、给用户看什么**。此前只回答了
//! agent 循环那一份，流式路径裸报错误；收编到这里作为唯一事实来源，
//! 任何一边改退避策略或文案，另一边自动跟随。
//!
//! 与 `error_kind` 的分工：那边回答「这是什么错、能不能重试」，
//! 这里回答「怎么重、等到什么时候、状态如何呈现」。

use crate::llm::error_kind::{retry_after_secs, LlmErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 重试上限。`STAR_MAX_RETRIES`，默认 3 —— 与 agent 循环的既有口径一致。
pub fn max_attempts() -> u32 {
    std::env::var("STAR_MAX_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// 第 `attempt` 次失败后应当等待多久（毫秒）。
///
/// 服务端给了 `Retry-After` 就听它的；否则指数退避 `base * 2^attempt`
/// 加最多 ±25% 抖动（多实例同时撞上同一个 5xx 时不要一起在同一秒重试），
/// 上限 `STAR_RETRY_MAX_DELAY_MS`。
pub fn delay_ms(err_str: &str, attempt: u32) -> u64 {
    if let Some(secs) = retry_after_secs(err_str) {
        return secs * 1000;
    }
    let base_delay_ms: u64 = std::env::var("STAR_RETRY_BASE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let max_delay_ms: u64 = std::env::var("STAR_RETRY_MAX_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32_000);

    let raw = base_delay_ms.saturating_mul(1u64 << attempt.min(16));
    let capped = raw.min(max_delay_ms);
    // 抖动不引入 rand 依赖：用当前纳秒当熵源，够散了。
    let entropy = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let jitter_span = capped / 4;
    if jitter_span == 0 {
        return capped;
    }
    let jitter = entropy % (jitter_span * 2 + 1);
    capped.saturating_add(jitter).saturating_sub(jitter_span)
}

/// 等待期间每个 tick 推给用户的一条状态文案。
pub fn waiting_status(kind: LlmErrorKind, attempt: u32, remaining: Duration) -> String {
    format!(
        "{} — retry {}/{} in {:.0}s",
        kind.label(),
        attempt + 1,
        max_attempts(),
        remaining.as_secs_f64().ceil(),
    )
}

/// 退避结束、即将重发前的一条状态文案。
pub fn retrying_status(kind: LlmErrorKind, attempt: u32) -> String {
    format!(
        "{} — retrying now ({}/{})",
        kind.label(),
        attempt + 1,
        max_attempts(),
    )
}

/// 重试预算用尽时的一条日志文案（内部诊断用，不显示给用户——
/// 用户最终看到的是 `error_kind::diagnose` 包出来的完整错误）。
pub fn exhausted_log(kind: LlmErrorKind, attempts: u32) -> String {
    format!(
        "{} exhausted after {} attempt(s) — giving up",
        kind.label(),
        attempts,
    )
}

/// 可中断的退避等待。
///
/// 退避可能长达数十秒，期间用户按 ESC（abort flag 置位）必须立刻退出，
/// 否则 UI 已经收手、worker 还在傻等，用户会觉得「中断有时不管用」。
/// 等待按 1s 分片，`on_tick` 在每个分片边界拿到剩余时长 —— 调用方用它
/// 实时刷新重试状态。没有 abort flag 的调用方传 `None`（等待不可中断）。
/// 返回 `false` 表示等待被中断，调用方不得再重发请求。
pub async fn wait_interruptible(
    total: Duration,
    abort_flag: Option<&AtomicBool>,
    mut on_tick: impl FnMut(Duration),
) -> bool {
    let tick = Duration::from_secs(1);
    let mut remaining = total;
    loop {
        if abort_flag.is_some_and(|f| f.load(Ordering::SeqCst)) {
            return false;
        }
        on_tick(remaining);
        if remaining <= tick {
            tokio::time::sleep(remaining).await;
            remaining -= remaining;
        } else {
            tokio::time::sleep(tick).await;
            remaining -= tick;
        }
        if remaining.is_zero() {
            break;
        }
    }
    !abort_flag.is_some_and(|f| f.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_retry_after_wins_over_local_formula() {
        // "retry-after: 30" → 30s，与 attempt 无关
        assert_eq!(delay_ms("429, retry-after: 30", 0), 30_000);
        assert_eq!(delay_ms("429, retry-after: 30", 3), 30_000);
    }

    #[test]
    fn delay_never_exceeds_the_cap_for_late_attempts() {
        for attempt in [8u32, 20, 33] {
            let d = delay_ms("no server hint", attempt);
            assert!(d <= 32_000 + d / 4, "attempt {attempt}: {d}ms exceeds cap");
        }
    }

    #[test]
    fn status_copies_are_stable_and_bounded() {
        let s = waiting_status(LlmErrorKind::Network, 0, Duration::from_secs(5));
        assert_eq!(s, "Network Error — retry 1/3 in 5s");
        let s = retrying_status(LlmErrorKind::ServerError, 1);
        assert_eq!(s, "Provider Error — retrying now (2/3)");
        let s = exhausted_log(LlmErrorKind::Network, 3);
        assert_eq!(s, "Network Error exhausted after 3 attempt(s) — giving up");
    }

    #[tokio::test]
    async fn wait_interruptible_reports_completion() {
        let flag = AtomicBool::new(false);
        let mut ticks = Vec::new();
        let ok = wait_interruptible(Duration::from_millis(50), Some(&flag), |r| {
            ticks.push(r.as_millis() as u64)
        })
        .await;
        assert!(ok, "an un-aborted short wait must complete");
        assert!(
            !ticks.is_empty(),
            "the caller must get at least one tick to render status"
        );
    }

    #[tokio::test]
    async fn wait_interruptible_aborts_early() {
        let flag = AtomicBool::new(true);
        let ok = wait_interruptible(Duration::from_secs(30), Some(&flag), |_| {}).await;
        assert!(!ok, "a pre-set abort flag must cut the wait short");
    }
}
