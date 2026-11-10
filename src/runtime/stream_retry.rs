//! 流式会话失败的重试决策。
//!
//! 流式路径此前对失败只有一个动作：把原始错误字符串抛给 UI 收场。
//! 网络抖动 —— 恰恰是最应该自动恢复的失败类型 —— 表现为"转圈之后突然
//! 报错"，而 agent 循环那边的同型错误却有完整的退避重试。收口到这里：
//! 流式 init 与流中断共用一份决策（要不要重试、等多久、用户看到什么），
//! 参数、退避公式与状态文案全部来自 `llm::retry` 内核的唯一口径。
//!
//! 为什么已经发出过内容的流不能整条重试：TextDelta 已经铺进 UI 的
//! ChatEntry，重发会把同一段正文再铺一遍 —— 表现为文字重复而不是恢复。
//! 只重试"零 chunk"的失败（init 失败与首 chunk 断流），这与 Claude Code
//! 的"首请求失败可安全重来"行为一致。

use crate::llm::error_kind::{diagnose, LlmErrorKind};
use crate::llm::retry;
use crate::utils::logging::append_debug_log_line;
use std::collections::HashMap;

/// 一次流式失败的处理结论。
pub(crate) enum StreamFailureDecision {
    /// 退避后整条流重建重试。`waiting_status` / `retrying_status` 是按
    /// 内核文案生成的两条状态 —— 等待前后各发一次，与 agent 循环一致。
    Retry {
        delay_ms: u64,
        waiting_status: String,
        retrying_status: String,
    },
    /// 立刻收尾：诊断全文（标签 + 下一步 + `Original error:` 原文）交 UI。
    GiveUp(String),
}

/// 流式会话的重试预算。按错误类别分桶计数，一次网络抖动不该消耗
/// 5xx 的重试预算 —— 与 `agent_llm::backoff_retry` 同一口径。
#[derive(Default)]
pub(crate) struct StreamRetryPolicy {
    counts: HashMap<String, u32>,
}

impl StreamRetryPolicy {
    pub(crate) fn decide(&mut self, err_str: &str, chunks_received: u64) -> StreamFailureDecision {
        let kind = LlmErrorKind::classify(err_str);

        // 终止类（认证 / 欠费 / 请求非法）与内容已落地的流：重试不可能
        // 得到不同结果，给完整诊断立刻收场。
        if kind.is_terminal() || chunks_received > 0 {
            return StreamFailureDecision::GiveUp(diagnose(err_str));
        }

        // Unknown 与 agent 循环同口径：认不出的错误可退避重试（对标
        // Claude Code `shouldRetry` 的粗判），不做上下文压缩。
        let retry_eligible = matches!(
            kind,
            LlmErrorKind::Network
                | LlmErrorKind::ServerError
                | LlmErrorKind::Overloaded
                | LlmErrorKind::RateLimit
                | LlmErrorKind::Unknown
        );
        let max_attempts = retry::max_attempts();
        if !retry_eligible || max_attempts == 0 {
            return StreamFailureDecision::GiveUp(diagnose(err_str));
        }

        let key = format!("llm_backoff:{:?}", kind);
        let attempt = self.counts.get(&key).copied().unwrap_or(0);
        if attempt >= max_attempts {
            append_debug_log_line(&format!(
                "[STREAM_RETRY] {}",
                retry::exhausted_log(kind, max_attempts),
            ));
            return StreamFailureDecision::GiveUp(diagnose(err_str));
        }
        self.counts.insert(key, attempt + 1);

        let delay = retry::delay_ms(err_str, attempt);
        append_debug_log_line(&format!(
            "[STREAM_RETRY] {} attempt {}/{} — waiting {}ms",
            kind.label(),
            attempt + 1,
            max_attempts,
            delay,
        ));
        StreamFailureDecision::Retry {
            delay_ms: delay,
            waiting_status: retry::waiting_status(
                kind,
                attempt,
                std::time::Duration::from_millis(delay),
            ),
            retrying_status: retry::retrying_status(kind, attempt),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retries_until_give_up(policy: &mut StreamRetryPolicy, err: &str) -> usize {
        let mut n = 0;
        loop {
            match policy.decide(err, 0) {
                StreamFailureDecision::Retry { .. } => n += 1,
                StreamFailureDecision::GiveUp(_) => return n,
            }
        }
    }

    #[test]
    fn zero_chunk_network_failure_retries_then_gives_up() {
        assert!(matches!(
            StreamRetryPolicy::default().decide("connection refused", 0),
            StreamFailureDecision::Retry { .. }
        ));
        let mut policy = StreamRetryPolicy::default();
        assert_eq!(
            retries_until_give_up(&mut policy, "connection refused"),
            retry::max_attempts() as usize,
            "重试次数必须与内核上限一致"
        );
        assert!(matches!(
            policy.decide("connection refused", 0),
            StreamFailureDecision::GiveUp(text) if text.contains("connection refused")
        ));
    }

    #[test]
    fn stream_with_delivered_content_must_not_retry() {
        // 正文已经铺进 UI 的流重发即重复，必须直接诊断收场。
        assert!(matches!(
            StreamRetryPolicy::default().decide("connection reset mid-stream", 1),
            StreamFailureDecision::GiveUp(_)
        ));
    }

    #[test]
    fn terminal_kinds_never_retry() {
        assert!(matches!(
            StreamRetryPolicy::default().decide("401 Unauthorized", 0),
            StreamFailureDecision::GiveUp(_)
        ));
        assert!(matches!(
            StreamRetryPolicy::default().decide("402 Payment Required: Insufficient Balance", 0),
            StreamFailureDecision::GiveUp(_)
        ));
    }

    #[test]
    fn retry_budgets_are_per_kind() {
        let mut policy = StreamRetryPolicy::default();
        for _ in 0..retry::max_attempts() {
            assert!(matches!(
                policy.decide("connection refused", 0),
                StreamFailureDecision::Retry { .. }
            ));
        }
        // 5xx 用自己的预算，而不是被网络错误耗尽。
        assert!(matches!(
            policy.decide("502 Bad Gateway", 0),
            StreamFailureDecision::Retry { .. }
        ));
    }

    #[test]
    fn retry_status_copy_carries_the_label_and_counter() {
        let mut policy = StreamRetryPolicy::default();
        match policy.decide("operation timed out", 0) {
            StreamFailureDecision::Retry {
                waiting_status,
                retrying_status,
                ..
            } => {
                // 延迟带 ±25% 抖动，剩余秒数上取整会落在 1-2s 之间，
                // 只锁文案结构不锁具体秒数。
                assert!(waiting_status.starts_with("Network Error — retry 1/3 in "));
                assert_eq!(retrying_status, "Network Error — retrying now (1/3)");
            }
            _ => panic!("network failure should retry"),
        }
    }
}
