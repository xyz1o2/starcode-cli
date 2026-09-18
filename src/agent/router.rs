use crate::core::routing::{RequestComplexity, RoutingContext};

// ── Complexity classification thresholds (length/history based — fast path) ──
const SIMPLE_MAX_CHARS: usize = 200;
const MEDIUM_MAX_CHARS: usize = 800;
const SIMPLE_MAX_HISTORY: usize = 4;
const COMPLEX_MIN_HISTORY: usize = 8;

pub struct Router;

/// 跨文件/全局作用域信号：描述虽短，但影响面横跨多个文件。
const CROSS_FILE_PHRASES: &[&str] = &[
    "all callers",
    "every caller",
    "all references",
    "all usages",
    "all occurrences",
    "find all",
    "replace all",
    "update all",
    "across the codebase",
    "across files",
    "cross-file",
    "multiple files",
    "several files",
    "entire project",
    "whole project",
    "entire codebase",
    "whole codebase",
    "entire repo",
    "re-export",
    "reexport",
    "rename",
    "refactor",
    "migrate",
    "migration",
    "all tests",
    "all failing",
    "every test",
    "all files",
    "所有调用",
    "所有引用",
    "所有用法",
    "所有出现",
    "全部替换",
    "全部更新",
    "跨文件",
    "多个文件",
    "整个项目",
    "整个代码库",
    "整个仓库",
    "重命名",
    "改名",
    "重构",
    "迁移",
    "所有测试",
    "所有失败的测试",
    "导出",
];

/// 多步骤信号：任务由多个有依赖关系的阶段组成。
const MULTI_STEP_PHRASES: &[&str] = &[
    "multi-step",
    "multistep",
    "multiple steps",
    "step by step",
    "step 1",
    "phase 1",
    "milestone",
    "in stages",
    "多步骤",
    "分步骤",
    "分阶段",
    "第一步",
];

fn has_cross_file_signals(lower: &str) -> bool {
    CROSS_FILE_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

fn has_multi_step_signals(lower: &str) -> bool {
    MULTI_STEP_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

impl Router {
    /// 快速同步分类：基于输入长度、对话历史，以及跨文件/多步骤的内容信号。
    ///
    /// 纯长度启发会把"跨文件重命名"、"修掉所有失败的测试"这类短描述判成
    /// Simple——而只有 Complex 才给 thinking 预算并放行 auto-plan，这类任务
    /// 因此长期拿不到资源。内容信号专门补这个缺口；真正的语义理解仍留给模型。
    pub fn classify(input: &str, history_length: usize) -> RequestComplexity {
        let char_count = input.chars().count();

        if history_length >= COMPLEX_MIN_HISTORY || char_count > MEDIUM_MAX_CHARS {
            return RequestComplexity::Complex;
        }

        // 短描述也可能是跨文件/多阶段重活：内容信号直接提为 Complex
        let lower = input.to_lowercase();
        if has_cross_file_signals(&lower) || has_multi_step_signals(&lower) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_simple_input_stays_simple() {
        assert_eq!(
            Router::classify("read this file", 0),
            RequestComplexity::Simple
        );
    }

    #[test]
    fn long_input_is_complex() {
        let long = "x".repeat(900);
        assert_eq!(Router::classify(&long, 0), RequestComplexity::Complex);
    }

    /// 这两条是本改动的核心：描述很短，但都是跨文件重活。纯长度启发会把它们
    /// 判成 Simple，而只有 Complex 才给 thinking 预算并放行 auto-plan。
    #[test]
    fn short_cross_file_prompt_promotes_to_complex() {
        assert_eq!(
            Router::classify("Rename foo to bar across the codebase", 0),
            RequestComplexity::Complex
        );
        assert_eq!(
            Router::classify("Fix all failing tests in this project", 0),
            RequestComplexity::Complex
        );
    }

    #[test]
    fn short_multi_step_prompt_promotes_to_complex() {
        assert_eq!(
            Router::classify("Do this step by step", 0),
            RequestComplexity::Complex
        );
    }

    #[test]
    fn chinese_cross_file_prompt_promotes_to_complex() {
        assert_eq!(
            Router::classify("把 foo 重命名为 bar，更新所有引用", 0),
            RequestComplexity::Complex
        );
    }

    /// 信号匹配必须是整词级精确，不能把日常短句也提级。
    #[test]
    fn unrelated_short_input_is_not_promoted() {
        assert_eq!(
            Router::classify("explain the function in this file", 0),
            RequestComplexity::Simple
        );
    }
}
