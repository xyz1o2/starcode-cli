//! Auto Mode 的**可注入分类器** —— 权限判定链路上的 LLM 闸门。
//!
//! # 为什么要有这一层
//!
//! [`crate::core::policy::PolicyEngine::check`] 是权限判定的唯一收口，但它是个
//! **同步、无外部依赖**的纯逻辑模块：它不持有 LLM client，也不认识对话上下文。
//! 要让 auto 档位在"本地护栏判不出来"时由模型裁决，就得给它一条能调用模型的
//! 通道 —— 又不能把 client 直接塞进策略引擎（那会让策略层依赖网络，测试也没法
//! 离线跑）。
//!
//! 所以这里定义一个 trait：策略层只认这个接口，具体实现可以是真模型，也可以是
//! 测试里的假分类器。**依赖方向是单向的**：策略层 → trait ← 实现。
//!
//! # 与 [`crate::core::auto_mode::classifier`] 的关系
//!
//! `classifier.rs` 里的 [`crate::core::auto_mode::classifier::AutoModeClassifier`]
//! 是**旧的直连实现**：它自己用 `reqwest` 打 `/chat/completions`，模型、base_url、
//! api_key 全部从环境变量读，与本项目的 provider 解析（`ProviderStore` /
//! user settings / `small_fast_model` 副模型）**完全脱节** —— 用户配的网关和密钥
//! 它一个都不认。
//!
//! 本模块是它的替代品：走 [`crate::llm::client::StarClient`]，因此复用同一套
//! provider 解析、鉴权、错误分类与超时。`classifier.rs` 保留作历史参考与
//! 本地规则来源（[`crate::core::auto_mode::dangerous_patterns`]）。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::core::auto_mode::classifier::{ClassifierDecision, ClassifierResult, ClassifierStage};
use crate::types::{StarMessage, StarToolCall};

/// 分类器的裁决结果 —— 策略层真正消费的东西。
///
/// 与 [`ClassifierDecision`] 一一对应，但多带一层**来源说明**：用户看到的
/// 「AI 已自动批准」需要知道是谁批准的、依据是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoVerdict {
    /// 放行 —— 免确认直接执行。
    Allow { reason: String },
    /// 拒绝 —— 连用户确认都不给，直接判否。
    Deny { reason: String },
    /// 不确定 —— 退回弹确认框，交给人。
    Ask { reason: String },
    /// **本地护栏判不出来** —— 交给 LLM 分类器裁决。
    ///
    /// 这个变体只在策略层的本地判定里出现，分类器本身不会返回它：
    /// 分类器的裁决要么是 Allow/Deny/Ask，要么（失灵时）退化成 Ask。
    Undecided,
}

impl AutoVerdict {
    pub fn decision(&self) -> ClassifierDecision {
        match self {
            AutoVerdict::Allow { .. } => ClassifierDecision::Allow,
            AutoVerdict::Deny { .. } => ClassifierDecision::Deny,
            AutoVerdict::Ask { .. } | AutoVerdict::Undecided => ClassifierDecision::Ask,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            AutoVerdict::Allow { reason }
            | AutoVerdict::Deny { reason }
            | AutoVerdict::Ask { reason } => reason,
            AutoVerdict::Undecided => "",
        }
    }

    /// 这个裁决是否还需要再往下问（分类器失灵时只能问人）。
    pub fn needs_user(&self) -> bool {
        matches!(self, AutoVerdict::Ask { .. } | AutoVerdict::Undecided)
    }
}

/// 一次待分类的工具调用。
#[derive(Debug, Clone)]
pub struct ClassificationRequest {
    pub tool_name: String,
    pub tool_params: Value,
    /// 对话上下文摘要 —— 让模型知道**用户到底想要什么**。
    ///
    /// 这是分类质量的决定性输入：同一个 `rm -rf target`，用户明说"清理构建产物"
    /// 与"你自己看着办"是两件完全不同的事。没有上下文，模型只能按命令字面猜。
    pub transcript: String,
}

impl ClassificationRequest {
    pub fn new(
        tool_name: impl Into<String>,
        tool_params: Value,
        transcript: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            tool_params,
            transcript: transcript.into(),
        }
    }
}

/// 权限判定链路上的分类器接口。
///
/// 策略层只依赖这个 trait，因此：
/// - 生产环境注入 [`LlmAutoModeClassifier`]（真模型）；
/// - 单测注入 mock（离线、确定、无 token 消耗）。
#[async_trait]
pub trait AutoModeClassifier: Send + Sync + std::fmt::Debug {
    /// 对一次工具调用做安全性裁决。
    ///
    /// 实现**必须**遵守：拿不到模型、模型报错、响应无法解析 —— 一律返回
    /// [`AutoVerdict::Ask`]，绝不返回 `Allow`。分类器是**收紧**权限的闸门，
    /// 不是放宽权限的闸门；它失灵时唯一安全的失败方向是"问人"。
    async fn classify(&self, request: &ClassificationRequest) -> AutoVerdict;

    /// 分类器当前是否可用（配置齐全、没被 circuit breaker 熔断）。
    ///
    /// 不可用时策略层直接退回弹确认，不发请求。
    fn is_available(&self) -> bool {
        true
    }
}

/// 基于 LLM 的分类器。
///
/// 走 [`crate::llm::client::StarClient`]，因此 provider / 密钥 / 副模型全部
/// 复用项目已有的解析链路 —— 与 [`crate::agent::router`] 的旁路分类是同一套路。
pub struct LlmAutoModeClassifier {
    client: crate::llm::client::StarClient,
    /// 分类用的模型名。优先用 `small_fast_model`（"小而快"的副模型）：
    /// 分类任务短、频率高，用主模型既慢又贵。
    model: Option<String>,
    /// 单次分类的超时（秒）。分类器卡住不该拖住整个审批链路。
    timeout_secs: u64,
}

impl std::fmt::Debug for LlmAutoModeClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmAutoModeClassifier")
            .field("model", &self.model)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl LlmAutoModeClassifier {
    pub fn new(
        client: crate::llm::client::StarClient,
        model: Option<String>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            client,
            model,
            timeout_secs: timeout_secs.max(1),
        }
    }

    /// 从环境变量读超时：`STAR_AUTO_MODE_CLASSIFIER_TIMEOUT`（秒）。
    ///
    /// 默认 2 秒而不是原来的 10 秒 —— 分类是"这条调用危不危险"的粗判，
    /// 不是完整推理：一次正常的裁决模型 1 秒内就该回话。10 秒意味着 provider
    /// 慢的时候每次工具调用都白等 10 秒才弹框（现场日志：108 次
    /// `classifier timed out`，同一条命令因此「时而放行、时而弹框」）。
    /// 超时竖直收紧后，失灵路径退化成弹框的代价从 10s 降到 2s。
    pub fn timeout_from_env() -> u64 {
        std::env::var("STAR_AUTO_MODE_CLASSIFIER_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
    }

    /// 构造分类提示词。
    ///
    /// 三段式，与 [`crate::core::auto_mode::prompts::build_auto_mode_system_prompt`]
    /// 的规则口径一致，但重排成"先看用户意图，再判工具"的顺序 —— 分类的难点
    /// 从不是"这个命令危不危险"，而是"这次调用是不是用户要的"。
    fn build_prompt(request: &ClassificationRequest) -> (String, String) {
        let system = crate::core::auto_mode::prompts::build_auto_mode_system_prompt();
        let user = format!(
            "Tool: {}\nParameters: {}\n\nConversation context:\n{}\n\n\
             Reply with exactly one word: ALLOW, BLOCK, or ASK, then a brief reason.",
            request.tool_name,
            serde_json::to_string_pretty(&request.tool_params).unwrap_or_else(|_| "{}".to_string()),
            request.transcript.chars().take(8000).collect::<String>(),
        );
        (system, user)
    }

    /// 把模型的自由文本解析成裁决。
    ///
    /// 解析必须**保守**。三条规则：
    ///
    /// 1. 只认首词 —— 模型常写 "ALLOW — this is a read-only operation"，
    ///    也可能写 "I would BLOCK this"。取首词能同时吃下这两种。
    /// 2. **冲突即问人** —— 首词说 ALLOW 但正文里出现 BLOCK/DENY，说明模型
    ///    自相矛盾（"ALLOW is not appropriate here, BLOCK"）。这种情况绝不能
    ///    放行：矛盾信号意味着我们没真正读懂模型的判断。
    /// 3. `contains("allow")` 绝不用于判定 —— "not allowed" 会被判成放行，
    ///    那是权限旁路。
    ///
    /// # 首词之外还认什么
    ///
    /// 只认首词不够 —— 现实里的模型经常这么回：
    /// - `**ALLOW**`（markdown 加粗）
    /// - `ALLOW`（带代码块围栏）
    /// - `Verdict: ALLOW` / `Decision: ALLOW`（带前缀标签）
    /// - `ALLOW\nThis is a read-only operation`（首词 + 换行 + 理由）
    ///
    /// 这些都不是「模型不听话」，而是**格式噪声**。把它们一律判成
    /// `unrecognized` 会让熔断器跳闸，之后每次工具调用都退化成「问人」——
    /// 现场：ARTEX 的日志里每次会话都有
    /// `[AutoMode] classifier circuit breaker tripped`（12 次）。
    ///
    /// 所以：**剥掉 markdown 装饰后，在前几个词里找裁决词**。
    /// 找不到才算 unrecognized（保守方向不变 —— 认不出就问人，绝不放行）。
    pub fn parse_verdict(response: &str) -> AutoVerdict {
        let text = response.trim();
        let first = Self::extract_verdict_word(text);

        let reason = text.lines().next().unwrap_or(text).trim().to_string();

        // 冲突检测：正文里出现了与首词相反的裁决词。
        //
        // 只在首词是 allow/block 时做 —— 首词是 ask 时没有"相反"可言。
        let lower = text.to_ascii_lowercase();
        let mentions_block = lower.contains("block") || lower.contains("deny");
        let mentions_allow = lower.contains("allow");

        let conflicting = match first.as_str() {
            "allow" => mentions_block,
            "block" | "deny" => mentions_allow,
            _ => false,
        };

        if conflicting {
            return AutoVerdict::Ask {
                reason: format!("AI classifier response was contradictory: {}", text),
            };
        }

        match first.as_str() {
            "allow" => AutoVerdict::Allow {
                reason: if reason.is_empty() {
                    "AI classifier: allowed".to_string()
                } else {
                    reason
                },
            },
            "block" | "deny" => AutoVerdict::Deny {
                reason: if reason.is_empty() {
                    "AI classifier: blocked".to_string()
                } else {
                    reason
                },
            },
            "ask" => AutoVerdict::Ask {
                reason: if reason.is_empty() {
                    "AI classifier: needs confirmation".to_string()
                } else {
                    reason
                },
            },
            // 认不出来 = 不确定 = 问人。绝不放行。
            _ => AutoVerdict::Ask {
                reason: format!("AI classifier response unrecognized: {}", text),
            },
        }
    }

    /// 从模型回复里**取出**裁决词。
    ///
    /// 处理现实里常见的格式噪声：
    /// - markdown 装饰：`**ALLOW**`、`_BLOCK_`、`` `ASK` ``
    /// - 代码块围栏：` ```ALLOW ``` `
    /// - 标签前缀：`Verdict: ALLOW`、`Decision - BLOCK`
    /// - 列表项：`- ALLOW`
    ///
    /// 找不到就返回空串（调用方判成 unrecognized → 问人，保守方向不变）。
    ///
    /// **只看前几个词**：裁决词后面跟着理由（"ALLOW — 这是只读操作"），
    /// 理由里可能也出现 allow/block，所以只在前面找。
    fn extract_verdict_word(text: &str) -> String {
        /// 裁决词前面允许出现的标签（最多 3 个词）。
        const LABELS: [&str; 6] = [
            "verdict", "decision", "answer", "result", "judgment", "choice",
        ];

        let mut cleaned = text.trim();
        // 剥掉代码块围栏。
        cleaned = cleaned.trim_start_matches('`').trim_end_matches('`');
        cleaned = cleaned.trim();

        let mut words: Vec<String> = cleaned
            .split_whitespace()
            .take(4)
            .map(|w| {
                w.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                    .to_ascii_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect();

        // 跳过标签词：`Verdict: ALLOW` → 跳过 `verdict` 找 `allow`。
        while words
            .first()
            .map(|w| LABELS.contains(&w.as_str()))
            .unwrap_or(false)
        {
            words.remove(0);
        }

        words
            .into_iter()
            .find(|w| matches!(w.as_str(), "allow" | "block" | "deny" | "ask"))
            .unwrap_or_default()
    }
}

#[async_trait]
impl AutoModeClassifier for LlmAutoModeClassifier {
    async fn classify(&self, request: &ClassificationRequest) -> AutoVerdict {
        let (system, user) = Self::build_prompt(request);

        let call = async {
            let messages = vec![StarMessage::system(system), StarMessage::user(user)];
            let response = self
                .client
                .chat(messages, None, self.model.clone(), None)
                .await?;
            let content = response
                .choices
                .first()
                .and_then(|c| c.message.content.as_deref())
                .unwrap_or("")
                .to_string();
            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(content)
        };

        match tokio::time::timeout(std::time::Duration::from_secs(self.timeout_secs), call).await {
            Ok(Ok(content)) => {
                if content.trim().is_empty() {
                    // 空响应 = 没得到裁决 = 问人。
                    AutoVerdict::Ask {
                        reason: "AI classifier returned an empty response".to_string(),
                    }
                } else {
                    let verdict = Self::parse_verdict(&content);
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[AutoMode] classifier verdict for {}: {:?}",
                        request.tool_name, verdict
                    ));
                    verdict
                }
            }
            Ok(Err(e)) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AutoMode] classifier call failed for {}: {}",
                    request.tool_name, e
                ));
                AutoVerdict::Ask {
                    reason: format!("AI classifier unavailable: {}", e),
                }
            }
            Err(_) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[AutoMode] classifier timed out after {}s for {}",
                    self.timeout_secs, request.tool_name
                ));
                AutoVerdict::Ask {
                    reason: format!("AI classifier timed out after {}s", self.timeout_secs),
                }
            }
        }
    }
}

/// 分类器的**熔断包装**：连续失败 N 次后停止调用模型。
///
/// # 为什么需要
///
/// 分类器失灵时唯一安全的失败方向是"问人"（Ask）—— 这本身没问题。但 provider
/// 挂掉时，每一次工具调用都会等一次完整的超时（默认 10 秒）。一个跑 50 步的
/// 任务就是 500 秒纯等待，而且**每一次都必然失败**。
///
/// 熔断让这件事只发生一次：连续失败到阈值就跳过后续调用，直接问人。
/// 半开（隔一段时间放一次探测）让 provider 恢复后能自动回到自主判定。
///
/// 熔断**只收紧不放宽**：熔断期间的判定是"问人"，不是"放行"。
pub struct CircuitBreakingClassifier {
    inner: Arc<dyn AutoModeClassifier>,
    /// 连续失败次数上限。
    failure_threshold: u32,
    /// 当前连续失败次数。
    consecutive_failures: std::sync::atomic::AtomicU32,
    /// 熔断后的冷却时间（秒）；冷却结束放一次探测（半开）。
    cooldown_secs: u64,
    /// 上次熔断的时刻（epoch 秒）。
    tripped_at: std::sync::Mutex<Option<u64>>,
}

impl std::fmt::Debug for CircuitBreakingClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreakingClassifier")
            .field("failure_threshold", &self.failure_threshold)
            .field("cooldown_secs", &self.cooldown_secs)
            .field(
                "consecutive_failures",
                &self
                    .consecutive_failures
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish()
    }
}

impl CircuitBreakingClassifier {
    pub fn new(
        inner: Arc<dyn AutoModeClassifier>,
        failure_threshold: u32,
        cooldown_secs: u64,
    ) -> Self {
        Self {
            inner,
            failure_threshold: failure_threshold.max(1),
            consecutive_failures: std::sync::atomic::AtomicU32::new(0),
            cooldown_secs: cooldown_secs.max(1),
            tripped_at: std::sync::Mutex::new(None),
        }
    }

    /// 从环境变量读熔断参数：`STAR_AUTO_MODE_CLASSIFIER_BREAKER_THRESHOLD`、
    /// `STAR_AUTO_MODE_CLASSIFIER_BREAKER_COOLDOWN`。
    pub fn params_from_env() -> (u32, u64) {
        let threshold = std::env::var("STAR_AUTO_MODE_CLASSIFIER_BREAKER_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let cooldown = std::env::var("STAR_AUTO_MODE_CLASSIFIER_BREAKER_COOLDOWN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        (threshold, cooldown)
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// 当前是否处于熔断状态（不含半开判定）。
    fn is_tripped(&self) -> bool {
        let mut tripped_at = self.tripped_at.lock().unwrap_or_else(|e| e.into_inner());
        match *tripped_at {
            Some(at) => {
                if Self::now_secs().saturating_sub(at) >= self.cooldown_secs {
                    // 冷却结束 → 半开：放一次探测（把熔断时刻清掉）。
                    *tripped_at = None;
                    false
                } else {
                    true
                }
            }
            None => false,
        }
    }

    fn record_failure(&self) {
        let failures = self
            .consecutive_failures
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if failures >= self.failure_threshold {
            let mut tripped_at = self.tripped_at.lock().unwrap_or_else(|e| e.into_inner());
            *tripped_at = Some(Self::now_secs());
            crate::utils::logging::append_debug_log_line(&format!(
                "[AutoMode] classifier circuit breaker tripped after {} consecutive failures",
                failures
            ));
        }
    }

    fn record_success(&self) {
        self.consecutive_failures
            .store(0, std::sync::atomic::Ordering::SeqCst);
        let mut tripped_at = self.tripped_at.lock().unwrap_or_else(|e| e.into_inner());
        *tripped_at = None;
    }
}

#[async_trait]
impl AutoModeClassifier for CircuitBreakingClassifier {
    async fn classify(&self, request: &ClassificationRequest) -> AutoVerdict {
        if self.is_tripped() {
            // 熔断中：不发请求，直接问人。
            return AutoVerdict::Ask {
                reason: "AI classifier circuit breaker is open — asking the user instead"
                    .to_string(),
            };
        }

        let verdict = self.inner.classify(request).await;

        // 只有"Ask"算失败 —— 模型明确说"不确定"是**正常裁决**，不是故障。
        // 把两者混在一起会让一次正常的犹豫把熔断器打开。
        let reason = verdict.reason();
        let failed = matches!(verdict, AutoVerdict::Ask { .. })
            && (reason.contains("unavailable")
                || reason.contains("timed out")
                || reason.contains("unrecognized")
                || reason.contains("empty response"));

        if failed {
            self.record_failure();
        } else {
            self.record_success();
        }

        verdict
    }

    fn is_available(&self) -> bool {
        self.inner.is_available() && !self.is_tripped()
    }
}

/// 分类结果的调试信息，供 `/permissions` 之类的诊断命令回显。
#[derive(Debug, Clone)]
pub struct ClassifierStats {
    pub total_classifications: u64,
    pub allowed: u64,
    pub denied: u64,
    pub asked: u64,
    pub fallback_count: u64,
    pub avg_latency_ms: f64,
}

impl Default for ClassifierStats {
    fn default() -> Self {
        Self {
            total_classifications: 0,
            allowed: 0,
            denied: 0,
            asked: 0,
            fallback_count: 0,
            avg_latency_ms: 0.0,
        }
    }
}

/// 记录分类结果的计数器。策略层持有它，用于 `/permissions` 的诊断输出。
#[derive(Debug, Default)]
pub struct ClassifierCounter {
    stats: std::sync::Mutex<ClassifierStats>,
}

impl ClassifierCounter {
    pub fn record(&self, result: &ClassifierResult, latency_ms: f64) {
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.total_classifications += 1;
        match result.decision {
            ClassifierDecision::Allow => stats.allowed += 1,
            ClassifierDecision::Deny => stats.denied += 1,
            ClassifierDecision::Ask => stats.asked += 1,
        }
        if result.fallback {
            stats.fallback_count += 1;
        }
        let n = stats.total_classifications as f64;
        stats.avg_latency_ms = (stats.avg_latency_ms * (n - 1.0) + latency_ms) / n;
    }

    pub fn snapshot(&self) -> ClassifierStats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// 构造一个永不判错的 mock 分类器 —— 单测用。
///
/// `verdicts` 按调用顺序消费；用完以后一律 `Ask`（问人），模拟"分类器没话说"。
pub fn mock_classifier(verdicts: Vec<AutoVerdict>) -> Arc<dyn AutoModeClassifier> {
    Arc::new(MockAutoModeClassifier {
        verdicts: std::sync::Mutex::new(verdicts.into_iter().collect()),
    })
}

#[derive(Debug)]
struct MockAutoModeClassifier {
    verdicts: std::sync::Mutex<Vec<AutoVerdict>>,
}

#[async_trait]
impl AutoModeClassifier for MockAutoModeClassifier {
    async fn classify(&self, _request: &ClassificationRequest) -> AutoVerdict {
        self.verdicts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop()
            .unwrap_or(AutoVerdict::Ask {
                reason: "mock classifier exhausted".to_string(),
            })
    }
}

/// 便捷别名：把一次工具调用与可选的 transcript 组装成分类请求。
pub fn request_for(
    tool_name: &str,
    args: &Value,
    tool_call: Option<&StarToolCall>,
    transcript: &str,
) -> ClassificationRequest {
    let mut params = args.clone();
    if params.is_null() {
        params = Value::Object(Default::default());
    }
    if let Some(call) = tool_call {
        if let Ok(parsed) = serde_json::from_str::<Value>(&call.function.arguments) {
            params = parsed;
        }
    }
    ClassificationRequest::new(tool_name, params, transcript)
}

/// 分类阶段的展示名（诊断用）。
pub fn stage_label(stage: ClassifierStage) -> &'static str {
    match stage {
        ClassifierStage::Fast => "fast",
        ClassifierStage::Thinking => "thinking",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ClassificationRequest {
        ClassificationRequest::new(
            "Bash",
            serde_json::json!({"command": "cargo test --lib"}),
            "user: run the tests\n",
        )
    }

    /// 解析必须只认首词 —— `contains("allow")` 会把 "not allowed" 判成放行，
    /// 那是权限旁路。
    #[test]
    fn parse_verdict_only_trusts_the_first_word() {
        assert!(matches!(
            LlmAutoModeClassifier::parse_verdict("ALLOW — read-only test run"),
            AutoVerdict::Allow { .. }
        ));
        assert!(matches!(
            LlmAutoModeClassifier::parse_verdict("BLOCK"),
            AutoVerdict::Deny { .. }
        ));
        assert!(matches!(
            LlmAutoModeClassifier::parse_verdict("ASK"),
            AutoVerdict::Ask { .. }
        ));
    }

    /// "not allowed" / "this is not permitted" 绝不能被解析成放行。
    #[test]
    fn a_negated_allow_is_never_an_allow() {
        for text in [
            "not allowed",
            "this is not allowed to run",
            "ALLOW is not appropriate here, BLOCK",
            "I would BLOCK this",
        ] {
            let verdict = LlmAutoModeClassifier::parse_verdict(text);
            assert!(
                !matches!(verdict, AutoVerdict::Allow { .. }),
                "{text:?} 被判成了放行"
            );
        }
    }

    /// **格式噪声不能当成「模型不听话」。**
    ///
    /// 现场事故：ARTEX 的日志里每次会话都有
    /// `[AutoMode] classifier circuit breaker tripped`（12 次）——
    /// 分类器被熔断之后，auto 档位每次工具调用都退化成「问人」，
    /// 用户抱怨「权限老是弹窗」。
    ///
    /// 根因之一是判词解析太严：模型回 `**ALLOW**`、`Verdict: ALLOW`、
    /// `\`\`ALLOW\`\`` 这类格式时，首词不是裁决词 → `unrecognized` →
    /// 熔断。这些是**格式**问题，不是判断问题。
    #[test]
    fn markdown_and_label_noise_still_parses() {
        for text in [
            "**ALLOW** — read-only operation",
            "_BLOCK_ — this deletes files",
            "`ASK` — needs user intent",
            "Verdict: ALLOW",
            "Decision: BLOCK",
            "```ALLOW```",
            "- ALLOW",
            "Result: ASK",
        ] {
            let verdict = LlmAutoModeClassifier::parse_verdict(text);
            let ok = match text.to_ascii_lowercase().as_str() {
                t if t.contains("allow") => matches!(verdict, AutoVerdict::Allow { .. }),
                t if t.contains("block") => matches!(verdict, AutoVerdict::Deny { .. }),
                _ => matches!(verdict, AutoVerdict::Ask { .. }),
            };
            assert!(ok, "{text:?} 解析成了 {verdict:?}");
        }
    }

    /// 但**否定**与**矛盾**仍然要挡住 —— 格式宽松不能变成语义宽松。
    #[test]
    fn the_noise_tolerance_does_not_weaken_the_negation_guard() {
        for text in [
            "not allowed",
            "Verdict: not allowed",
            "ALLOW is not appropriate here, BLOCK",
        ] {
            let verdict = LlmAutoModeClassifier::parse_verdict(text);
            assert!(
                !matches!(verdict, AutoVerdict::Allow { .. }),
                "{text:?} 被判成了放行"
            );
        }
    }

    /// 认不出来 → 问人。绝不猜成放行。
    #[test]
    fn an_unrecognized_response_asks_the_user() {
        let verdict = LlmAutoModeClassifier::parse_verdict("maybe? who knows");
        assert!(matches!(verdict, AutoVerdict::Ask { .. }));
        assert!(verdict.needs_user());
    }

    /// 空响应 → 问人。
    #[test]
    fn an_empty_response_asks_the_user() {
        let verdict = LlmAutoModeClassifier::parse_verdict("   ");
        assert!(matches!(verdict, AutoVerdict::Ask { .. }));
    }

    /// 熔断器：连续失败到阈值后跳过调用，之后冷却结束恢复（半开）。
    #[tokio::test]
    async fn the_circuit_breaker_opens_after_repeated_failures() {
        let breaker = CircuitBreakingClassifier::new(
            mock_classifier(vec![
                AutoVerdict::Ask {
                    reason: "AI classifier unavailable: boom".to_string(),
                },
                AutoVerdict::Ask {
                    reason: "AI classifier unavailable: boom".to_string(),
                },
            ]),
            2,
            60,
        );

        // 前两次：调用分类器（失败）。
        let v1 = breaker.classify(&request()).await;
        assert!(v1.needs_user());
        let v2 = breaker.classify(&request()).await;
        assert!(v2.needs_user());

        // 第三次：熔断已开，不再调用内层。
        let v3 = breaker.classify(&request()).await;
        assert!(
            v3.reason().contains("circuit breaker"),
            "熔断后应当直接问人，实际：{}",
            v3.reason()
        );
        assert!(!breaker.is_available());
    }

    /// 熔断只对**故障**生效 —— 模型正常说"不确定"不算失败，
    /// 否则一次正常的犹豫就会把熔断器打开。
    #[tokio::test]
    async fn a_normal_unsure_verdict_does_not_trip_the_breaker() {
        let breaker = CircuitBreakingClassifier::new(
            mock_classifier(vec![
                AutoVerdict::Ask {
                    reason: "not sure".to_string(),
                },
                AutoVerdict::Ask {
                    reason: "not sure".to_string(),
                },
                AutoVerdict::Ask {
                    reason: "not sure".to_string(),
                },
            ]),
            2,
            60,
        );

        for _ in 0..3 {
            breaker.classify(&request()).await;
        }
        assert!(breaker.is_available(), "模型正常说'不确定'不该触发熔断");
    }

    /// 一次成功就清零连续失败计数。
    #[tokio::test]
    async fn a_success_resets_the_failure_counter() {
        let breaker = CircuitBreakingClassifier::new(
            mock_classifier(vec![
                AutoVerdict::Ask {
                    reason: "AI classifier unavailable: boom".to_string(),
                },
                AutoVerdict::Allow {
                    reason: "fine".to_string(),
                },
                AutoVerdict::Ask {
                    reason: "AI classifier unavailable: boom".to_string(),
                },
            ]),
            2,
            60,
        );

        breaker.classify(&request()).await;
        breaker.classify(&request()).await;
        assert!(breaker.is_available(), "中间的成功应当清零失败计数");
    }

    /// mock 分类器用完就退化成问人 —— 模拟"分类器没话说"。
    #[tokio::test]
    async fn the_mock_classifier_falls_back_to_asking() {
        let classifier = mock_classifier(vec![]);
        let verdict = classifier.classify(&request()).await;
        assert!(matches!(verdict, AutoVerdict::Ask { .. }));
    }
}
