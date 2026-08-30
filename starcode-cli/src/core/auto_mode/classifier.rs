//! Auto Mode 分类器
//!
//! 对标 Claude Code 的 yoloClassifier：
//! - 两阶段分类流水线（fast + thinking）
//! - 基于完整对话上下文的判断
//! - 三种裁决：allow / deny / ask

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};

use super::prompts;
use super::dangerous_patterns;

/// 分类器决策
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassifierDecision {
    Allow,
    Deny,
    Ask,
}

impl std::fmt::Display for ClassifierDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassifierDecision::Allow => write!(f, "allow"),
            ClassifierDecision::Deny => write!(f, "deny"),
            ClassifierDecision::Ask => write!(f, "ask"),
        }
    }
}

/// 分类阶段
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassifierStage {
    Fast,
    Thinking,
}

impl std::fmt::Display for ClassifierStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassifierStage::Fast => write!(f, "fast"),
            ClassifierStage::Thinking => write!(f, "thinking"),
        }
    }
}

/// 分类器结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierResult {
    pub decision: ClassifierDecision,
    pub reason: String,
    pub stage: ClassifierStage,
    pub thinking: Option<String>,
    pub model: String,
    pub latency_ms: f64,
    pub fallback: bool,
    pub unavailable: bool,
    pub transcript_too_long: bool,
}

impl ClassifierResult {
    pub fn allow(reason: impl Into<String>, stage: ClassifierStage) -> Self {
        Self {
            decision: ClassifierDecision::Allow,
            reason: reason.into(),
            stage,
            thinking: None,
            model: "local".to_string(),
            latency_ms: 0.0,
            fallback: false,
            unavailable: false,
            transcript_too_long: false,
        }
    }

    pub fn deny(reason: impl Into<String>, stage: ClassifierStage) -> Self {
        Self {
            decision: ClassifierDecision::Deny,
            reason: reason.into(),
            stage,
            thinking: None,
            model: "local".to_string(),
            latency_ms: 0.0,
            fallback: false,
            unavailable: false,
            transcript_too_long: false,
        }
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self {
            decision: ClassifierDecision::Ask,
            reason: reason.into(),
            stage: ClassifierStage::Fast,
            thinking: None,
            model: "local".to_string(),
            latency_ms: 0.0,
            fallback: false,
            unavailable: false,
            transcript_too_long: false,
        }
    }

    pub fn fallback(reason: impl Into<String>) -> Self {
        Self {
            decision: ClassifierDecision::Ask,
            reason: reason.into(),
            stage: ClassifierStage::Fast,
            thinking: None,
            model: "local".to_string(),
            latency_ms: 0.0,
            fallback: true,
            unavailable: false,
            transcript_too_long: false,
        }
    }
}

/// Auto Mode 分类器
pub struct AutoModeClassifier {
    /// 使用的分类模式：both / fast / thinking
    mode: String,
    /// 快速阶段最大 token
    fast_max_tokens: u32,
    /// LLM API base URL
    api_base: Option<String>,
    /// LLM API key
    api_key: Option<String>,
    /// 分类器模型
    model: String,
}

impl AutoModeClassifier {
    pub fn new() -> Self {
        let mode = std::env::var("STAR_AUTO_MODE_CLASSIFIER_MODE")
            .unwrap_or_else(|_| "both".to_string());
        let model = std::env::var("STAR_AUTO_MODE_CLASSIFIER_MODEL")
            .unwrap_or_else(|_| "deepseek-chat".to_string());
        let api_base = std::env::var("STAR_BASE_URL").ok();
        let api_key = std::env::var("STAR_API_KEY").ok();

        Self {
            mode,
            fast_max_tokens: 64,
            api_base,
            api_key,
            model,
        }
    }

    /// 分类工具调用
    pub async fn classify(
        &self,
        tool_name: &str,
        tool_params: &Value,
        transcript: &str,
    ) -> ClassifierResult {
        let start = std::time::Instant::now();

        // 第0层：本地规则快速检查
        if let Some(result) = self.local_rule_check(tool_name, tool_params) {
            let mut r = result;
            r.latency_ms = start.elapsed().as_millis() as f64;
            return r;
        }

        // 检查 API 可用性
        if self.api_base.is_none() || self.api_key.is_none() {
            let mut r = ClassifierResult::ask("LLM API not configured, falling back to manual approval");
            r.unavailable = true;
            r.latency_ms = start.elapsed().as_millis() as f64;
            return r;
        }

        // 检查 transcript 长度
        if transcript.len() > 100_000 {
            let mut r = ClassifierResult::ask("Transcript too long for classification");
            r.transcript_too_long = true;
            r.latency_ms = start.elapsed().as_millis() as f64;
            return r;
        }

        // Stage 1: Fast classification
        let fast_result = self.fast_classify(tool_name, tool_params, transcript).await;

        match &self.mode[..] {
            "fast" => {
                let mut r = fast_result;
                r.latency_ms = start.elapsed().as_millis() as f64;
                r
            }
            "thinking" => {
                let mut r = self.thinking_classify(tool_name, tool_params, transcript).await;
                r.latency_ms = start.elapsed().as_millis() as f64;
                r
            }
            _ => {
                // "both" mode
                if fast_result.decision == ClassifierDecision::Allow {
                    let mut r = fast_result;
                    r.latency_ms = start.elapsed().as_millis() as f64;
                    return r;
                }
                // Stage 2: Thinking classification
                let mut r = self.thinking_classify(tool_name, tool_params, transcript).await;
                r.latency_ms = start.elapsed().as_millis() as f64;
                r
            }
        }
    }

    /// 本地规则快速检查（零 LLM 开销）
    fn local_rule_check(&self, tool_name: &str, tool_params: &Value) -> Option<ClassifierResult> {
        // 读取类工具直接放行
        if matches!(tool_name, "Read" | "Grep" | "Glob" | "Search" | "LS" | "read_file" | "search" | "glob") {
            return Some(ClassifierResult::allow(
                "Read-only tool, always safe",
                ClassifierStage::Fast,
            ));
        }

        // 检查 bash 命令的危险模式
        if tool_name == "Bash" || tool_name == "shell" {
            if let Some(cmd) = tool_params.get("command").and_then(|c| c.as_str()) {
                if dangerous_patterns::is_dangerous_pattern(cmd) {
                    return Some(ClassifierResult::deny(
                        format!("Dangerous command pattern detected: {}", cmd),
                        ClassifierStage::Fast,
                    ));
                }
            }
        }

        // 写入当前目录内的文件 — 通常安全
        if tool_name == "Edit" || tool_name == "Write" || tool_name == "edit" || tool_name == "write_file" {
            if let Some(path) = tool_params.get("file_path")
                .or_else(|| tool_params.get("path"))
                .or_else(|| tool_params.get("filePath"))
                .and_then(|p| p.as_str())
            {
                // 检查是否写入系统目录
                if dangerous_patterns::is_system_path(path) {
                    return Some(ClassifierResult::deny(
                        format!("Write to system path blocked: {}", path),
                        ClassifierStage::Fast,
                    ));
                }
            }
        }

        None // 无法本地判断，需要 LLM
    }

    /// Stage 1: 快速分类（max_tokens=64, stop_sequences）
    async fn fast_classify(
        &self,
        tool_name: &str,
        tool_params: &Value,
        transcript: &str,
    ) -> ClassifierResult {
        let system_prompt = prompts::build_auto_mode_system_prompt();
        let user_content = format!(
            "Tool: {}\nParameters: {}\n\nConversation context:\n{}",
            tool_name,
            serde_json::to_string_pretty(tool_params).unwrap_or_default(),
            &transcript[..transcript.len().min(50000)]
        );

        match self.call_llm(&system_prompt, &user_content, Some(self.fast_max_tokens)).await {
            Ok(response) => {
                let response_lower = response.to_lowercase();
                if response_lower.contains("allow") {
                    ClassifierResult::allow(
                        format!("Fast classifier: {}", response.trim()),
                        ClassifierStage::Fast,
                    )
                } else if response_lower.contains("block") || response_lower.contains("deny") {
                    ClassifierResult::deny(
                        format!("Fast classifier: {}", response.trim()),
                        ClassifierStage::Fast,
                    )
                } else {
                    ClassifierResult::ask(format!("Fast classifier ambiguous: {}", response.trim()))
                }
            }
            Err(e) => {
                ClassifierResult::fallback(format!("Fast classifier API error: {}", e))
            }
        }
    }

    /// Stage 2: 深度思考分类（chain-of-thought）
    async fn thinking_classify(
        &self,
        tool_name: &str,
        tool_params: &Value,
        transcript: &str,
    ) -> ClassifierResult {
        let system_prompt = prompts::build_auto_mode_system_prompt();
        let thinking_prompt = format!(
            "{}\n\nThink step by step about whether this tool call is safe. \
             Consider: 1) What does this tool do? 2) Is it reversible? 3) \
             Does it affect external systems? 4) Is the user's intent clear?\n\n\
             Then classify_result as: BLOCK, ALLOW, or ASK",
            prompts::CLASSIFIER_THINKING_PREFIX
        );
        let user_content = format!(
            "Tool: {}\nParameters: {}\n\nConversation context:\n{}",
            tool_name,
            serde_json::to_string_pretty(tool_params).unwrap_or_default(),
            &transcript[..transcript.len().min(50000)]
        );

        match self.call_llm(&system_prompt, &format!("{}\n\n{}", thinking_prompt, user_content), None).await {
            Ok(response) => {
                let response_lower = response.to_lowercase();
                if response_lower.contains("allow") {
                    let mut r = ClassifierResult::allow(
                        format!("Thinking classifier: {}", response.trim()),
                        ClassifierStage::Thinking,
                    );
                    r.thinking = Some(response.clone());
                    r
                } else if response_lower.contains("block") || response_lower.contains("deny") {
                    let mut r = ClassifierResult::deny(
                        format!("Thinking classifier: {}", response.trim()),
                        ClassifierStage::Thinking,
                    );
                    r.thinking = Some(response.clone());
                    r
                } else {
                    let mut r = ClassifierResult::ask(format!(
                        "Thinking classifier ambiguous: {}",
                        response.trim()
                    ));
                    r.thinking = Some(response);
                    r
                }
            }
            Err(e) => ClassifierResult::fallback(format!("Thinking classifier API error: {}", e)),
        }
    }

    /// 调用 LLM API
    async fn call_llm(
        &self,
        system_prompt: &str,
        user_content: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        let api_base = self.api_base.as_ref().ok_or("API base URL not set")?;
        let api_key = self.api_key.as_ref().ok_or("API key not set")?;

        let url = if api_base.ends_with("/chat/completions") {
            api_base.clone()
        } else {
            format!("{}/chat/completions", api_base.trim_end_matches('/'))
        };

        let mut body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_content}
            ],
            "temperature": 0.0,
        });

        if let Some(tokens) = max_tokens {
            body["max_tokens"] = json!(tokens);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("HTTP error: {}", e))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        resp["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "No content in response".to_string())
    }
}
