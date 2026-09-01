use crate::core::routing::{RequestComplexity, RoutingContext};

// ── Complexity classification thresholds (length/history based — fast path) ──
const SIMPLE_MAX_CHARS: usize = 200;
const MEDIUM_MAX_CHARS: usize = 800;
const SIMPLE_MAX_HISTORY: usize = 4;
const COMPLEX_MIN_HISTORY: usize = 8;

pub struct Router;

impl Router {
    /// 快速同步分类：纯基于输入长度和对话历史。
    /// 不包含任何关键词/模式匹配——语义理解留给模型。
    pub fn classify(input: &str, history_length: usize) -> RequestComplexity {
        let char_count = input.chars().count();

        if history_length >= COMPLEX_MIN_HISTORY || char_count > MEDIUM_MAX_CHARS {
            return RequestComplexity::Complex;
        }

        if history_length < SIMPLE_MAX_HISTORY && char_count <= SIMPLE_MAX_CHARS {
            return RequestComplexity::Simple;
        }

        RequestComplexity::Medium
    }

    /// 模型驱动的语义升级：对短输入（长度判定为Simple），调用LLM评估实际工程复杂度。
    ///
    /// 仅在 `STAR_SEMANTIC_ROUTING=true` 且当前判定为Simple时触发。
    /// 超时或失败时回退到原判定，不阻塞快速路径。
    pub async fn classify_with_semantic_upgrade(
        client: &crate::llm::client::StarClient,
        input: &str,
        history_length: usize,
    ) -> RequestComplexity {
        let length_based = Self::classify(input, history_length);

        // 只对Simple做语义升级检查——Medium/Complex的长度判定已经足够
        if !matches!(length_based, RequestComplexity::Simple) {
            return length_based;
        }

        // 仅当环境变量启用时
        if !semantic_routing_enabled() {
            return length_based;
        }

        // 过短的输入不做语义检查（<10字符几乎不可能是大工程）
        if input.chars().count() < 10 {
            return length_based;
        }

        // 调用LLM做分类
        match tokio::time::timeout(
            std::time::Duration::from_secs(semantic_routing_timeout_secs()),
            Self::ask_llm_complexity(client, input),
        )
        .await
        {
            Ok(Ok(upgraded)) => upgraded,
            _ => {
                // 超时或失败，回退到长度判定
                crate::utils::logging::append_debug_log_line(
                    "[ROUTER] semantic classification failed/timed out, falling back to length-based",
                );
                length_based
            }
        }
    }

    /// 向LLM发送极简分类请求
    async fn ask_llm_complexity(
        client: &crate::llm::client::StarClient,
        input: &str,
    ) -> Result<RequestComplexity, Box<dyn std::error::Error + Send + Sync>> {
        let system = crate::types::StarMessage::system(
            "Classify this coding task. Reply with one word: Simple, Medium, or Complex."
                .to_string(),
        );
        let user =
            crate::types::StarMessage::user(format!("Task: {}\n\nComplexity (one word):", input));

        let resp = client.chat(vec![system, user], None, None, None).await?;
        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or("")
            .trim()
            .to_lowercase();

        let result = match text.as_str() {
            s if s.starts_with("complex") => RequestComplexity::Complex,
            s if s.starts_with("medium") => RequestComplexity::Medium,
            _ => RequestComplexity::Simple,
        };

        crate::utils::logging::append_debug_log_line(&format!(
            "[ROUTER] LLM semantic classification: input={}chars, result={:?}, raw={}",
            input.chars().count(),
            result,
            text,
        ));

        Ok(result)
    }

    pub fn build_context(
        user_input: &str,
        history_length: usize,
        user_override: Option<String>,
        default_model: String,
        fast_model: Option<String>,
        cheap_model: Option<String>,
    ) -> RoutingContext {
        let request_complexity = Self::classify(user_input, history_length);

        RoutingContext {
            history_length,
            request_complexity,
            user_override,
            default_model,
            fast_model,
            cheap_model,
        }
    }

    pub fn env_model(name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .and_then(|v| if v.trim().is_empty() { None } else { Some(v) })
    }
}

fn semantic_routing_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("STAR_SEMANTIC_ROUTING")
            .ok()
            .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on"))
            .unwrap_or(false)
    })
}

fn semantic_routing_timeout_secs() -> u64 {
    static TIMEOUT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        std::env::var("STAR_SEMANTIC_ROUTING_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .clamp(1, 5)
    })
}
