/// HTTP Hook执行器
/// 
/// 对标claude-code-main的src/utils/hooks/execHttpHook.ts
/// 通过HTTP POST执行Hook

use super::{HookDefinition, HookError, HookExecutor, HookResult, HookType};

/// HTTP Hook执行器
pub struct HttpHookExecutor {
    /// HTTP客户端
    client: reqwest::Client,
    /// 超时（毫秒）
    timeout_ms: u64,
    /// SSRF防护
    ssrf_protection: bool,
}

impl HttpHookExecutor {
    /// 创建新的HTTP Hook执行器
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout_ms: 600000, // 10 minutes
            ssrf_protection: true,
        }
    }

    /// 检查URL是否安全（SSRF防护）
    fn is_url_safe(&self, url: &str) -> bool {
        if !self.ssrf_protection {
            return true;
        }

        // 解析URL
        if let Ok(parsed) = url::Url::parse(url) {
            // 只允许HTTP和HTTPS
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return false;
            }

            // 检查是否是本地地址
            if let Some(host) = parsed.host_str() {
                if host == "localhost" || host == "127.0.0.1" || host == "::1" {
                    return false;
                }

                // 检查是否是私有IP
                if host.starts_with("10.") || host.starts_with("172.") || host.starts_with("192.168.") {
                    return false;
                }
            }

            true
        } else {
            false
        }
    }
}

#[async_trait::async_trait]
impl HookExecutor for HttpHookExecutor {
    async fn execute(
        &self,
        hook: &HookDefinition,
        input: &str,
        signal: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<HookResult, HookError> {
        let start_time = std::time::Instant::now();

        // 检查URL安全性
        if !self.is_url_safe(&hook.command) {
            return Err(HookError::PermissionError("URL blocked by SSRF protection".to_string()));
        }

        // 发送HTTP请求
        let response = self.client
            .post(&hook.command)
            .header("Content-Type", "application/json")
            .body(input.to_string())
            .send()
            .await
            .map_err(|e| HookError::NetworkError(e.to_string()))?;

        let status_code = response.status().as_u16();
        let body = response.text().await
            .map_err(|e| HookError::NetworkError(e.to_string()))?;

        let duration_ms = start_time.elapsed().as_millis() as u64;

        // 解析响应
        let success = status_code >= 200 && status_code < 300;
        let prevent_continuation = status_code == 403;

        Ok(HookResult {
            hook_id: hook.id.clone(),
            success,
            output: Some(body),
            error: if success { None } else { Some(format!("HTTP {}", status_code)) },
            exit_code: Some(status_code as i32),
            duration_ms,
            prevent_continuation,
            stop_reason: None,
        })
    }

    fn supports(&self, hook_type: &HookType) -> bool {
        *hook_type == HookType::Http
    }
}
