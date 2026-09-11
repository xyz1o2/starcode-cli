use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;

/// 网络层可重试错误码 — 连接级别故障，与 HTTP 状态码无关
const RETRYABLE_NETWORK_CODES: &[&str] = &[
    "ECONNRESET",
    "ETIMEDOUT",
    "EPIPE",
    "ENOTFOUND",
    "EAI_AGAIN",
    "ECONNREFUSED",
    "ECONNABORTED",
    "EHOSTUNREACH",
];

/// HTTP 状态码：429 Rate Limit — 必须尊重 Retry-After
const STATUS_RATE_LIMIT: u16 = 429;
/// HTTP 状态码范围：5xx 服务端故障（可重试子集）
const STATUS_SERVER_ERROR_MIN: u16 = 500;
const STATUS_SERVER_ERROR_MAX: u16 = 599;

/// 5xx 中明确可重试的状态码（不包括 501 Not Implemented 等语义错误）
const RETRYABLE_SERVER_CODES: &[u16] = &[500, 502, 503, 504, 507, 529];

/// 默认退避参数
const DEFAULT_INITIAL_DELAY_MS: u64 = 1000;
const DEFAULT_MAX_DELAY_MS: u64 = 60_000;
const DEFAULT_MAX_ATTEMPTS: usize = 5;
/// 429 默认等待上限（无 Retry-After 时）
const RATE_LIMIT_DEFAULT_WAIT_MS: u64 = 30_000;
/// Jitter 范围系数 (±30%)
const JITTER_FACTOR: f64 = 0.3;

#[derive(Debug, Clone)]
pub struct RetryOptions {
    pub max_attempts: usize,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub retry_fetch_errors: bool,
}

impl Default for RetryOptions {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            initial_delay_ms: DEFAULT_INITIAL_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
            retry_fetch_errors: false,
        }
    }
}

/// 带状态码感知的重试选项 — 用于 HTTP 调用
#[derive(Debug, Clone)]
pub struct HttpRetryOptions {
    pub base: RetryOptions,
    /// 是否解析 Retry-After header（默认 true）
    pub respect_retry_after: bool,
}

impl Default for HttpRetryOptions {
    fn default() -> Self {
        Self {
            base: RetryOptions::default(),
            respect_retry_after: true,
        }
    }
}

/// 从 HTTP 错误消息中提取状态码（如 "429 Too Many Requests" → Some(429)）
fn extract_status_code(error_msg: &str) -> Option<u16> {
    // 匹配常见 HTTP 错误格式: "status 429", "code: 429", "429 ", "(429)"
    for word in error_msg.split(|c: char| !c.is_ascii_digit()) {
        if word.len() == 3 {
            if let Ok(code) = word.parse::<u16>() {
                if (400..600).contains(&code) {
                    return Some(code);
                }
            }
        }
    }
    None
}

/// 从错误消息中解析 Retry-After 秒数
/// 支持格式: "retry-after: 30", "Retry-After: 60", "retry after 45s"
fn parse_retry_after_seconds(error_msg: &str) -> Option<u64> {
    let lower = error_msg.to_lowercase();
    let patterns = ["retry-after:", "retry-after: ", "retry after "];
    for pattern in &patterns {
        if let Some(pos) = lower.find(pattern) {
            let rest = &lower[pos + pattern.len()..];
            // 提取数字部分
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(secs) = num_str.parse::<u64>() {
                if secs > 0 && secs < 3600 {
                    // 合理范围: 1s ~ 1h
                    return Some(secs);
                }
            }
        }
    }
    None
}

/// 根据状态码决定等待时间
fn delay_for_status(status_code: u16, error_msg: &str, respect_retry_after: bool) -> Option<u64> {
    match status_code {
        // 429: 优先用 Retry-After header
        STATUS_RATE_LIMIT => {
            if respect_retry_after {
                if let Some(secs) = parse_retry_after_seconds(error_msg) {
                    return Some(secs * 1000);
                }
            }
            Some(RATE_LIMIT_DEFAULT_WAIT_MS)
        }
        // 5xx 可重试: 指数退避
        code if RETRYABLE_SERVER_CODES.contains(&code) => None, // 由调用方做指数退避
        _ => None,
    }
}

/// 核心重试循环 — 指数退避 + jitter
pub async fn retry_with_backoff<T, E, F, Fut>(mut f: F, options: RetryOptions) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut attempt = 0;
    let mut current_delay = options.initial_delay_ms;

    loop {
        attempt += 1;
        match f().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if attempt >= options.max_attempts {
                    return Err(error);
                }
                if !is_retryable_error(&error, options.retry_fetch_errors) {
                    return Err(error);
                }

                let delay_ms = compute_delay_with_jitter(current_delay);
                sleep(Duration::from_millis(delay_ms)).await;
                current_delay = (current_delay * 2).min(options.max_delay_ms);
            }
        }
    }
}

/// HTTP 感知重试 — 解析状态码 + Retry-After header
pub async fn retry_http_with_backoff<T, E, F, Fut>(
    mut f: F,
    options: HttpRetryOptions,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut attempt = 0;
    let mut current_delay = options.base.initial_delay_ms;

    loop {
        attempt += 1;
        match f().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                if attempt >= options.base.max_attempts {
                    return Err(error);
                }

                let error_msg = error.to_string().to_lowercase();
                let status = extract_status_code(&error_msg);

                // 检查是否可重试
                let retryable = match status {
                    Some(code) => is_retryable_status(code),
                    None => is_retryable_error(&error, options.base.retry_fetch_errors),
                };
                if !retryable {
                    return Err(error);
                }

                // 确定等待时间
                let delay_ms = if let Some(code) = status {
                    if let Some(fixed_ms) =
                        delay_for_status(code, &error_msg, options.respect_retry_after)
                    {
                        // 429 使用固定/Retry-After 等待
                        fixed_ms
                    } else {
                        // 5xx 使用指数退避
                        compute_delay_with_jitter(current_delay)
                    }
                } else {
                    compute_delay_with_jitter(current_delay)
                };

                sleep(Duration::from_millis(delay_ms)).await;
                current_delay = (current_delay * 2).min(options.base.max_delay_ms);
            }
        }
    }
}

/// 计算带 jitter 的延迟 (±JITTER_FACTOR)
fn compute_delay_with_jitter(base_delay_ms: u64) -> u64 {
    let jitter =
        base_delay_ms as f64 * JITTER_FACTOR * (rand::thread_rng().gen::<f64>() * 2.0 - 1.0);
    (base_delay_ms as f64 + jitter).max(0.0) as u64
}

/// HTTP 状态码是否可重试
pub fn is_retryable_status(status_code: u16) -> bool {
    status_code == STATUS_RATE_LIMIT || RETRYABLE_SERVER_CODES.contains(&status_code)
}

/// 错误是否可重试（基于错误消息）
pub fn is_retryable_error<E>(error: &E, retry_fetch_errors: bool) -> bool
where
    E: std::error::Error + ?Sized,
{
    let error_msg = error.to_string().to_lowercase();

    // 网络层错误
    if RETRYABLE_NETWORK_CODES
        .iter()
        .any(|code| error_msg.contains(code.to_lowercase().as_str()))
    {
        return true;
    }

    // fetch 失败
    if retry_fetch_errors && error_msg.contains("fetch failed") {
        return true;
    }

    // HTTP 状态码检查 — 精确匹配，避免 "contains 5" 误判
    if let Some(code) = extract_status_code(&error_msg) {
        return is_retryable_status(code);
    }

    // 兜底: 常见可重试关键词
    error_msg.contains("timeout")
        || error_msg.contains("timed out")
        || error_msg.contains("connection reset")
        || error_msg.contains("connection refused")
        || error_msg.contains("temporary failure")
}

pub async fn delay(ms: u64) {
    sleep(Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_status_code() {
        assert_eq!(extract_status_code("429 Too Many Requests"), Some(429));
        assert_eq!(extract_status_code("status 500"), Some(500));
        assert_eq!(extract_status_code("HTTP 502 Bad Gateway"), Some(502));
        assert_eq!(extract_status_code("error code: 503"), Some(503));
        assert_eq!(extract_status_code("connection timeout"), None);
        assert_eq!(extract_status_code("error 1234"), None); // 4 digits, out of range
    }

    #[test]
    fn test_parse_retry_after_seconds() {
        assert_eq!(parse_retry_after_seconds("retry-after: 30"), Some(30));
        assert_eq!(parse_retry_after_seconds("Retry-After: 60"), Some(60));
        assert_eq!(parse_retry_after_seconds("retry after 45s"), Some(45));
        assert_eq!(parse_retry_after_seconds("no header"), None);
        assert_eq!(parse_retry_after_seconds("retry-after: 99999"), None); // too large
    }

    #[test]
    fn test_is_retryable_status() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(501)); // Not Implemented
    }

    #[test]
    fn test_is_retryable_error_broad_five_fix() {
        // 旧 bug: "error 1235" 被 contains("5") 误判
        #[derive(Debug)]
        struct FakeError(String);
        impl std::fmt::Display for FakeError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl std::error::Error for FakeError {}

        // 包含 "5" 但不是 5xx 错误
        assert!(!is_retryable_error(&FakeError("error 1235".into()), false));
        // 真正的 5xx
        assert!(is_retryable_error(
            &FakeError("500 Internal Server Error".into()),
            false
        ));
        assert!(is_retryable_error(
            &FakeError("502 Bad Gateway".into()),
            false
        ));
        // 网络错误
        assert!(is_retryable_error(&FakeError("ECONNRESET".into()), false));
        // timeout
        assert!(is_retryable_error(
            &FakeError("connection timed out".into()),
            false
        ));
    }

    #[test]
    fn test_compute_delay_with_jitter() {
        // 多次调用应在合理范围内
        for _ in 0..100 {
            let d = compute_delay_with_jitter(1000);
            assert!(d >= 700 && d <= 1300, "jitter out of range: {d}");
        }
    }
}
