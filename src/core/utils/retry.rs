use rand::Rng;
use std::time::Duration;
use tokio::time::sleep;

const RETRYABLE_NETWORK_CODES: &[&str] = &[
    "ECONNRESET",
    "ETIMEDOUT",
    "EPIPE",
    "ENOTFOUND",
    "EAI_AGAIN",
    "ECONNREFUSED",
];

const FETCH_FAILED_MESSAGE: &str = "fetch failed";

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
            max_attempts: 3,
            initial_delay_ms: 5000,
            max_delay_ms: 30000,
            retry_fetch_errors: false,
        }
    }
}

pub async fn retry_with_backoff<T, E, F, Fut>(mut f: F, options: RetryOptions) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut attempt = 0;
    let mut current_delay = options.initial_delay_ms;

    while attempt < options.max_attempts {
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

                let jitter =
                    current_delay as f64 * 0.3 * (rand::thread_rng().gen::<f64>() * 2.0 - 1.0);
                let delay_with_jitter = (current_delay as f64 + jitter).max(0.0) as u64;

                sleep(Duration::from_millis(delay_with_jitter)).await;
                current_delay = (current_delay * 2).min(options.max_delay_ms);
            }
        }
    }

    unreachable!()
}

pub fn is_retryable_error<E>(error: &E, retry_fetch_errors: bool) -> bool
where
    E: std::error::Error + ?Sized,
{
    let error_msg = error.to_string().to_lowercase();

    if RETRYABLE_NETWORK_CODES
        .iter()
        .any(|code| error_msg.contains(code.to_lowercase().as_str()))
    {
        return true;
    }

    if retry_fetch_errors && error_msg.contains(FETCH_FAILED_MESSAGE) {
        return true;
    }

    if error_msg.contains("429") || error_msg.contains("5") {
        return true;
    }

    false
}

pub async fn delay(ms: u64) {
    sleep(Duration::from_millis(ms)).await;
}
