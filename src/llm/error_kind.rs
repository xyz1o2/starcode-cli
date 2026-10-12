//! LLM 错误分类 —— 一份口径，三处使用。
//!
//! 之前 401/402 的判断写在 `client.rs` 里（还是两份重复的），
//! `agent_llm::handle_llm_error` 自己又按子串猜一遍 `AgentError`，
//! UI 的 `error_overlay::classify_error` 再猜第三遍。结果是同一个 402
//! 在三处得到三种结论：client 给了友好提示、恢复层当成通用流式错误去
//! 压缩上下文重试、UI 归到 ProviderError 并显示"可重试"。
//!
//! 这里把分类收在一处：**能重试的**（限流 / 过载 / 5xx / 网络抖动）和
//! **重试也没用的**（认证 / 欠费 / 请求本身非法）必须分开 —— 对着一个
//! 余额耗尽的账号退避重试，只是把同一个错误重复三遍。
//!
//! 判定靠子串匹配：rig / reqwest / 各家 provider 的错误都是拼好的字符串，
//! 拿不到结构化的 status code。所以顺序有讲究 —— 先判精确的状态码和
//! 厂商文案，最后才落到宽泛的网络关键词上。

/// LLM 调用失败的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    /// 401 / 403 —— key 不对或没权限。重试无意义。
    Auth,
    /// 402 / 余额不足 / 配额耗尽 —— 要充钱或等配额重置。重试无意义。
    Quota,
    /// 429 —— 限流。退避后重试有效。
    RateLimit,
    /// 529 / overloaded —— 上游过载。退避后重试有效。
    Overloaded,
    /// 500 / 502 / 503 / 504 —— 上游故障。退避后重试可能有效。
    ServerError,
    /// 连接失败 / 超时 / DNS / 连接被重置 —— 本地或链路问题。重试有效。
    Network,
    /// 上下文超限 —— 靠压缩而不是重试解决。
    ContextWindow,
    /// 400 / 422 —— 请求本身非法（工具 schema、参数越界等）。重试无意义。
    BadRequest,
    /// 认不出来。当作可重试（一次），但不做上下文压缩。
    Unknown,
}

/// 错误文本里是否**作为一个独立的 token** 出现了某个 HTTP 状态码。
///
/// # 为什么要卡边界
///
/// 早先这里直接 `contains("401")` / `contains("403")`，而 provider 错误文本
/// 是自由文本，里面到处是数字：毫秒数、token 数、端口号、请求 ID、字节偏移。
/// 于是
///
/// ```text
/// "operation timed out after 40130ms"   // 含 "401" → 被判成 Auth
/// "prompt is too long: 250403 tokens"   // 含 "403" → 被判成 Auth
/// "error sending request for url (https://api.example.com:4013/v1)"
/// ```
///
/// 全部落进 `Auth`。而 `Auth` 是**终止类**（见 [`LlmErrorKind::is_terminal`]）：
/// agent 立刻停止重试，并把一条网络超时渲染成"The API key was rejected"。
/// 用户被指向 `/provider` 去换一个完全正确的 key —— 既没人能修，也没人知道
/// 真正该看的是超时。这比误报本身更糟。
///
/// 判定规则：出现位置的前后都不能是字母或数字。JSON 里的 `"code":"403"`
/// 两侧是引号，仍然算命中。
pub fn mentions_status_code(error: &str, code: u16) -> bool {
    let lower = error.to_lowercase();
    let needle = code.to_string();
    let bytes = lower.as_bytes();
    let mut search_from = 0usize;

    while let Some(rel) = lower[search_from..].find(&needle) {
        let start = search_from + rel;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        // 不是独立 token（"40130" 里的 "401"），从下一个字节继续找。
        search_from = start + 1;
        if search_from >= lower.len() {
            break;
        }
    }
    false
}

impl LlmErrorKind {
    /// 按子串分类。调用方传原始错误文本即可，大小写不敏感。
    pub fn classify(error: &str) -> Self {
        let lower = error.to_lowercase();

        // ── 先判"重试也没用"的，这些必须优先于宽泛的网络关键词 ──
        // 欠费文案各家不同：OpenAI 是 insufficient_quota，DeepSeek 是
        // Insufficient Balance，硅基/阿里系常见 arrearage（欠费）。
        if mentions_status_code(&lower, 402)
            || lower.contains("payment required")
            || lower.contains("insufficient balance")
            || lower.contains("insufficient_quota")
            || lower.contains("insufficient quota")
            || lower.contains("exceeded your current quota")
            || lower.contains("arrearage")
            || lower.contains("billing")
            || lower.contains("credit balance")
            || lower.contains("out of credit")
        {
            return Self::Quota;
        }
        if mentions_status_code(&lower, 401)
            || mentions_status_code(&lower, 403)
            || lower.contains("unauthorized")
            || lower.contains("incorrect api key")
            || lower.contains("invalid api key")
            || lower.contains("invalid_api_key")
            || lower.contains("authentication")
            || lower.contains("permission denied")
        {
            return Self::Auth;
        }

        // ── 可退避重试的 ──
        if mentions_status_code(&lower, 429)
            || lower.contains("rate limit")
            || lower.contains("rate_limit")
        {
            return Self::RateLimit;
        }
        if lower.contains("too many requests") {
            return Self::RateLimit;
        }
        if mentions_status_code(&lower, 529)
            || lower.contains("overloaded")
            || lower.contains("capacity")
        {
            return Self::Overloaded;
        }

        // ── 上下文超限：在 5xx 之前判，"context_length_exceeded" 常伴 400 ──
        if lower.contains("context window exceeds")
            || lower.contains("context_length_exceeded")
            || lower.contains("context length")
            || lower.contains("maximum context")
            || lower.contains("too many tokens")
            || lower.contains("prompt is too long")
        {
            return Self::ContextWindow;
        }

        if mentions_status_code(&lower, 500)
            || mentions_status_code(&lower, 502)
            || mentions_status_code(&lower, 503)
            || mentions_status_code(&lower, 504)
            || lower.contains("internal server error")
            || lower.contains("bad gateway")
            || lower.contains("service unavailable")
            || lower.contains("gateway timeout")
        {
            return Self::ServerError;
        }

        // ── 网络层：放在状态码之后，避免吞掉带 "timeout" 字样的 504 ──
        if lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("connection closed")
            || lower.contains("connection aborted")
            || lower.contains("broken pipe")
            || lower.contains("dns")
            || lower.contains("failed to lookup")
            || lower.contains("unexpected eof")
            || lower.contains("tls")
            || lower.contains("certificate")
            || lower.contains("error sending request")
            || lower.contains("network")
        {
            return Self::Network;
        }

        if mentions_status_code(&lower, 400)
            || mentions_status_code(&lower, 422)
            || lower.contains("invalid_request")
            || lower.contains("bad request")
        {
            return Self::BadRequest;
        }

        Self::Unknown
    }

    /// 退避重试是否有意义。
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Overloaded | Self::ServerError | Self::Network | Self::Unknown
        )
    }

    /// 重试也不会变的错误 —— 应当立刻停下并告诉用户怎么修。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Auth | Self::Quota | Self::BadRequest)
    }

    /// 给 UI 和日志用的短标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Auth => "Auth Error",
            Self::Quota => "Quota / Billing",
            Self::RateLimit => "Rate Limit",
            Self::Overloaded => "Provider Overloaded",
            Self::ServerError => "Provider Error",
            Self::Network => "Network Error",
            Self::ContextWindow => "Context Overflow",
            Self::BadRequest => "Bad Request",
            Self::Unknown => "Error",
        }
    }

    /// 下一步该做什么。终止类错误必须有，否则用户只看到一串状态码。
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Self::Auth => Some(
                "The API key was rejected. Check it with `/provider` or set STAR_API_KEY, \
                 then retry.",
            ),
            Self::Quota => Some(
                "The account is out of balance or has exhausted its quota. Top up or wait for \
                 the quota to reset — retrying will fail the same way.",
            ),
            Self::RateLimit => Some(
                "Too many requests. The agent backs off automatically; slow down or switch \
                 model.",
            ),
            Self::Overloaded => Some(
                "The provider is overloaded. The agent retries with backoff; consider another \
                 model via `/model`.",
            ),
            Self::ServerError => {
                Some("The provider returned a server error. The agent retries with backoff.")
            }
            Self::Network => Some(
                "Could not reach the provider. Check connectivity/proxy \
                 (HTTPS_PROXY) and STAR_BASE_URL.",
            ),
            Self::ContextWindow => Some(
                "The request exceeded the context window. Run `/compact`, or lower \
                 STAR_CONTEXT_WINDOW.",
            ),
            Self::BadRequest => Some(
                "The provider rejected the request as malformed — usually an unsupported \
                 parameter or tool schema for this model.",
            ),
            Self::Unknown => None,
        }
    }
}

/// 把原始错误包成"标签 + 下一步 + 原文"。用户看到的就是这一段。
pub fn diagnose(error: &str) -> String {
    let kind = LlmErrorKind::classify(error);
    let mut out = format!("{}: ", kind.label());
    match kind.hint() {
        Some(hint) => out.push_str(hint),
        None => out.push_str("The model call failed."),
    }
    out.push_str("\n\nOriginal error: ");
    out.push_str(error);
    out
}

/// 从错误文本里抠出服务端建议的等待秒数。
///
/// 各家写法不一：`retry-after: 30`、`Retry-After: 30`、
/// `try again in 1.5s`、`please retry after 20 seconds`。抠不到就返回 None，
/// 由调用方用自己的指数退避。上限 120s —— 再长不如让用户自己决定。
pub fn retry_after_secs(error: &str) -> Option<u64> {
    let lower = error.to_lowercase();
    const MARKERS: [&str; 4] = ["retry-after", "retry after", "try again in", "retry in"];
    for marker in MARKERS {
        let Some(pos) = lower.find(marker) else {
            continue;
        };
        let tail = &lower[pos + marker.len()..];
        let digits: String = tail
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(secs) = digits.parse::<f64>() {
            if secs.is_finite() && secs > 0.0 {
                return Some((secs.ceil() as u64).min(120));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_beats_generic_network_words() {
        // 关键回归：这条以前会因为含 "connection" 被判成可重试的网络错误。
        let err = "HTTP 402 Payment Required: Insufficient Balance (connection to api ok)";
        assert_eq!(LlmErrorKind::classify(err), LlmErrorKind::Quota);
        assert!(!LlmErrorKind::classify(err).is_retryable());
        assert!(LlmErrorKind::classify(err).is_terminal());
    }

    #[test]
    fn openai_quota_wording_is_quota_not_rate_limit() {
        // OpenAI 用 429 承载配额耗尽，文案里才有真相；重试无用，必须判成 Quota。
        let err = "429: You exceeded your current quota, please check your plan and billing";
        assert_eq!(LlmErrorKind::classify(err), LlmErrorKind::Quota);
    }

    #[test]
    fn classifies_retryable_kinds() {
        for (err, want) in [
            ("rig error: 429 Too Many Requests", LlmErrorKind::RateLimit),
            ("Error 529: overloaded_error", LlmErrorKind::Overloaded),
            ("502 Bad Gateway", LlmErrorKind::ServerError),
            (
                "error sending request for url (https://api.example.com)",
                LlmErrorKind::Network,
            ),
            ("operation timed out", LlmErrorKind::Network),
        ] {
            assert_eq!(LlmErrorKind::classify(err), want, "for {err}");
            assert!(want.is_retryable(), "{want:?} should be retryable");
        }
    }

    #[test]
    fn auth_and_bad_request_are_terminal() {
        assert_eq!(
            LlmErrorKind::classify("401 Unauthorized"),
            LlmErrorKind::Auth
        );
        assert_eq!(
            LlmErrorKind::classify("400 invalid_request_error: unsupported tool"),
            LlmErrorKind::BadRequest
        );
        assert!(LlmErrorKind::classify("401 Unauthorized").is_terminal());
    }

    #[test]
    fn context_window_is_neither_retryable_nor_terminal() {
        let kind = LlmErrorKind::classify("400: context_length_exceeded");
        assert_eq!(kind, LlmErrorKind::ContextWindow);
        assert!(!kind.is_retryable());
        assert!(!kind.is_terminal());
    }

    #[test]
    fn parses_server_suggested_delay() {
        assert_eq!(retry_after_secs("429, retry-after: 30"), Some(30));
        assert_eq!(retry_after_secs("Please try again in 1.5s"), Some(2));
        assert_eq!(retry_after_secs("retry after 20 seconds"), Some(20));
        assert_eq!(retry_after_secs("no hint here"), None);
        // 上限保护：别让服务端一个离谱的值把 UI 挂住一小时。
        assert_eq!(retry_after_secs("retry-after: 99999"), Some(120));
    }

    // ── 状态码必须按独立 token 匹配 ──
    // 下面是 `contains("401")` / `contains("403")` 时代的真实回归：provider
    // 错误文本里到处是数字，裸子串匹配会把网络超时、token 数、端口号当成
    // 认证失败。而 Auth 是终止类 —— agent 会停止重试，并让用户去换一个
    // 根本没坏的 key。
    #[test]
    fn digits_inside_other_numbers_are_not_status_codes() {
        assert_eq!(
            LlmErrorKind::classify("operation timed out after 40130ms"),
            LlmErrorKind::Network,
            "a timeout must not be reported as an auth failure"
        );
        assert_eq!(
            LlmErrorKind::classify("operation timed out after 40300ms"),
            LlmErrorKind::Network
        );
        assert_eq!(
            LlmErrorKind::classify(
                "400 invalid_request_error: prompt is too long: 250403 tokens > 200000 maximum"
            ),
            LlmErrorKind::ContextWindow,
            "token counts must not be read as status codes"
        );
        assert_eq!(
            LlmErrorKind::classify(
                "error sending request for url (https://api.example.com:4013/v1)"
            ),
            LlmErrorKind::Network,
            "a port number must not be read as a status code"
        );
    }

    #[test]
    fn a_timeout_must_stay_retryable() {
        // 误报的实际代价在这里：Auth 是终止类，超时是可退避重试的。
        let kind = LlmErrorKind::classify("operation timed out after 40130ms");
        assert!(kind.is_retryable());
        assert!(!kind.is_terminal(), "a timeout must never be terminal");
    }

    #[test]
    fn standalone_status_codes_still_match() {
        // 边界收紧后，真正的 401/403/429/502 一个都不能漏判 —— 这是修复的红线。
        for (err, want) in [
            (
                "Provider error: rig openai stream error: ProviderResponseError: status 403 Forbidden",
                LlmErrorKind::Auth,
            ),
            ("401 Unauthorized", LlmErrorKind::Auth),
            ("HTTP/1.1 429 Too Many Requests", LlmErrorKind::RateLimit),
            ("502 Bad Gateway", LlmErrorKind::ServerError),
            (
                r#"{"error":{"code":"403","message":"forbidden"}}"#,
                LlmErrorKind::Auth,
            ),
            ("Error 529: overloaded_error", LlmErrorKind::Overloaded),
            ("402 Payment Required", LlmErrorKind::Quota),
        ] {
            assert_eq!(LlmErrorKind::classify(err), want, "for {err}");
        }
    }

    #[test]
    fn mentions_status_code_respects_token_boundaries() {
        assert!(mentions_status_code("status 403 Forbidden", 403));
        assert!(mentions_status_code("HTTP/1.1 401", 401));
        assert!(mentions_status_code("503", 503));
        // 非独立 token 一律不算
        assert!(!mentions_status_code("after 40130ms", 401));
        assert!(!mentions_status_code("250403 tokens", 403));
        assert!(!mentions_status_code("localhost:4013", 401));
        assert!(!mentions_status_code("no numbers here", 401));
    }

    #[test]
    fn diagnose_keeps_the_original_text() {
        let out = diagnose("HTTP 402 Insufficient Balance");
        assert!(out.starts_with("Quota / Billing:"));
        assert!(out.contains("Original error: HTTP 402 Insufficient Balance"));
    }
}
