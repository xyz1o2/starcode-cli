use crate::core::token_limits::estimate_tokens;
use crate::llm::client::StarClient;
use crate::types::StarMessage;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 触发压缩的阈值比例（默认 95%）
const COMPRESSION_THRESHOLD_RATIO: f64 = 0.95;
/// 触发压缩的最小绝对 token 数（低于此值不压缩，避免小会话误触发）
const DEFAULT_COMPRESSION_MIN_TOKENS: usize = 30_000;
/// 压缩前预留的 buffer token 数（确保模型有空间完成当前响应）
const COMPRESSION_BUFFER_TOKENS: usize = 10_000;
/// 默认 Context Window 大小（256K，可通过 STAR_CONTEXT_WINDOW 环境变量覆盖）
const DEFAULT_CONTEXT_WINDOW: usize = 256_000;
/// 压缩检查冷却时间 (秒) - 避免频繁检查
const DEFAULT_CHECK_COOLDOWN_SECS: u64 = 60;
/// Level 1 压缩保留的最近消息数
const LEVEL1_KEEP_RECENT: usize = 8;
/// Level 2 压缩保留的最近消息数
const LEVEL2_KEEP_RECENT: usize = 6;

/// 根据上下文窗口大小动态计算压缩参数
fn dynamic_compression_params(context_window: usize) -> (usize, usize, u64) {
    // 返回 (min_tokens, buffer_tokens, cooldown_secs)
    match context_window {
        // 超大上下文 (1M+): 更宽松
        1_000_000.. => (100_000, 50_000, 120),
        // 大上下文 (200k-1M): 标准
        200_000.. => (50_000, 20_000, 90),
        // 中等上下文 (128k-200k): 稍紧凑
        128_000.. => (30_000, 15_000, 60),
        // 小上下文 (<128k): 更激进
        _ => (20_000, 10_000, 45),
    }
}

#[derive(Debug)]
pub struct CompressionResult {
    pub messages: Vec<StarMessage>,
    pub was_compacted: bool,
    pub original_token_count: usize,
    pub new_token_count: usize,
    pub threshold_tokens: usize,
    pub decision: &'static str,
}

#[derive(Debug, Clone)]
pub struct ContextCompressor {
    context_window: usize,
    /// 上次检查时间 (用于冷却)
    last_check_time: Arc<Mutex<Option<Instant>>>,
    /// 检查冷却时间 (秒)
    check_cooldown_secs: u64,
    /// 动态最小 token 阈值
    min_tokens: usize,
    /// 动态 buffer tokens
    buffer_tokens: usize,
}

impl ContextCompressor {
    pub fn new(context_window: Option<usize>) -> Self {
        let env_window = std::env::var("STAR_CONTEXT_WINDOW")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());

        // 1. 优先使用传入的 context_window
        // 2. 其次使用环境变量
        // 3. 尝试从模型缓存中获取
        // 4. 最后使用默认值
        let effective_window = context_window
            .or(env_window)
            .unwrap_or(DEFAULT_CONTEXT_WINDOW);

        let check_cooldown_secs = std::env::var("STAR_COMPRESS_CHECK_COOLDOWN")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CHECK_COOLDOWN_SECS);

        // 根据上下文窗口大小动态计算参数
        let (min_tokens, buffer_tokens, dynamic_cooldown) =
            dynamic_compression_params(effective_window);

        // 使用环境变量覆盖冷却时间（如果设置了）
        let final_cooldown = if std::env::var("STAR_COMPRESS_CHECK_COOLDOWN").is_ok() {
            check_cooldown_secs
        } else {
            dynamic_cooldown
        };

        Self {
            context_window: effective_window,
            last_check_time: Arc::new(Mutex::new(None)),
            check_cooldown_secs: final_cooldown,
            min_tokens,
            buffer_tokens,
        }
    }

    /// 使用模型名称创建，自动从 API 缓存中查找上下文窗口
    pub fn new_with_model(model_name: &str) -> Self {
        // 1. 尝试从 API /models 缓存中获取（由 list_models_for_client 填充）
        let cached =
            crate::agent::model_catalog::get_cached_context_window(model_name).map(|v| v as usize);

        // 2. 使用默认值
        let context_window = cached.unwrap_or(DEFAULT_CONTEXT_WINDOW);

        let compressor = Self::new(Some(context_window));

        crate::utils::logging::append_debug_log_line(&format!(
            "[CTX] ContextCompressor for '{}': window={} tokens, min_tokens={}, buffer={}, cooldown={}s (from_api={})",
            model_name, context_window, compressor.min_tokens, compressor.buffer_tokens, compressor.check_cooldown_secs, cached.is_some()
        ));

        compressor
    }

    /// 检查是否在冷却期内 (跳过压缩检查)
    fn is_in_cooldown(&self) -> bool {
        if let Ok(guard) = self.last_check_time.lock() {
            if let Some(last) = *guard {
                return last.elapsed() < Duration::from_secs(self.check_cooldown_secs);
            }
        }
        false
    }

    /// 更新检查时间
    fn update_check_time(&self) {
        if let Ok(mut guard) = self.last_check_time.lock() {
            *guard = Some(Instant::now());
        }
    }

    pub fn context_window(&self) -> usize {
        self.context_window
    }

    fn calculate_tokens(&self, messages: &[StarMessage]) -> usize {
        messages
            .iter()
            .map(|m| {
                let content_len = m.content.as_ref().map(|c| estimate_tokens(c)).unwrap_or(0);
                let tool_len = if let Some(calls) = &m.tool_calls {
                    calls.len() * 50
                } else {
                    0
                };
                content_len + tool_len
            })
            .sum()
    }

    fn perform_level_1_compression(
        &self,
        messages: Vec<StarMessage>,
        keep_recent: usize,
    ) -> (Vec<StarMessage>, bool) {
        let len = messages.len();
        if len <= keep_recent {
            return (messages, false);
        }

        let split_idx = len.saturating_sub(keep_recent);
        let mut new_messages = Vec::with_capacity(len);
        let mut changed = false;

        // Level 1 截断长度：工具输出保留前 N 字符
        let truncate_len = std::env::var("STAR_COMPRESS_TRUNCATE_CHARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(300);

        for (i, msg) in messages.iter().enumerate() {
            if i < split_idx && msg.role == "tool" {
                if let Some(content) = &msg.content {
                    if content.len() > truncate_len {
                        let safe_end = content
                            .char_indices()
                            .nth(truncate_len)
                            .map(|(i, _)| i)
                            .unwrap_or(content.len());
                        let truncated = format!(
                            "{}...\n[Truncated by ContextCompressor L1]",
                            &content[..safe_end]
                        );
                        let mut new_msg = msg.clone();
                        new_msg.content = Some(truncated);
                        new_messages.push(new_msg);
                        changed = true;
                        continue;
                    }
                }
            }
            new_messages.push(msg.clone());
        }

        (new_messages, changed)
    }

    async fn generate_au2_summary(
        &self,
        messages: &[StarMessage],
        client: &StarClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let au2_prompt = crate::core::prompts::summary::summary_prompt();
        // Build prompt message
        let mut history_text = String::new();
        for msg in messages {
            let role = &msg.role;
            let content = msg.content.as_deref().unwrap_or("[No Content]");
            let tool_info = if let Some(calls) = &msg.tool_calls {
                format!(" [Tool Calls: {}]", calls.len())
            } else {
                String::new()
            };
            history_text.push_str(&format!("\n{}: {}{}\n", role, content, tool_info));
        }

        let prompt_message = StarMessage::user(format!(
            "{}\n\n## Conversation History:\n{}",
            au2_prompt, history_text
        ));

        let response = client.chat(vec![prompt_message], None, None, None).await?;

        if let Some(choice) = response.choices.first() {
            if let Some(content) = &choice.message.content {
                return Ok(content.clone());
            }
        }

        Err("Empty response from LLM".into())
    }

    /// 强制执行压缩逻辑
    pub async fn force_compress(
        &self,
        messages: Vec<StarMessage>,
        client: Option<&StarClient>,
    ) -> Result<CompressionResult, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let total_tokens = self.calculate_tokens(&messages);

        if messages.len() <= 5 {
            return Ok(CompressionResult {
                messages,
                was_compacted: false,
                original_token_count: total_tokens,
                new_token_count: total_tokens,
                threshold_tokens: 0,
                decision: "force_skipped_insufficient_messages",
            });
        }

        // Apply Level 2 directly for force compress
        // We assume index 0 is system prompt
        let _system_msg = if !messages.is_empty() && messages[0].role == "system" {
            messages[0].clone()
        } else {
            StarMessage::system("".to_string())
        };

        let keep_count = LEVEL1_KEEP_RECENT;
        let split_idx = messages.len().saturating_sub(keep_count);

        // Ensure split_idx respects system prompt (don't split before index 1)
        let split_idx = if split_idx < 1 { 1 } else { split_idx };

        let recent_messages = messages[split_idx..].to_vec();

        let mut summary_text = format!(
            "(User Manual Compression: Omitted {} to {} messages. Summary: User interacted multiple times, tools executed. Recent context preserved.)",
            1, split_idx
        );

        if let Some(client) = client {
            let messages_to_summarize = &messages[0..split_idx];
            if let Ok(generated_summary) = self
                .generate_au2_summary(messages_to_summarize, client)
                .await
            {
                summary_text = generated_summary;
                crate::utils::logging::append_debug_log_line(
                    "✅ Forced AU2 Compression successful.",
                );
            }
        }

        let summary_msg = StarMessage::system(format!(
            "Conversation Summary (AU2 Compressed):\n{}",
            summary_text
        ));

        let mut new_messages = Vec::new();
        // If we have a system prompt, keep it at index 0
        if !messages.is_empty() && messages[0].role == "system" {
            new_messages.push(messages[0].clone());
        }
        new_messages.push(summary_msg);
        new_messages.extend(recent_messages);

        let new_total_tokens = self.calculate_tokens(&new_messages);

        Ok(CompressionResult {
            messages: new_messages,
            was_compacted: true,
            original_token_count: total_tokens,
            new_token_count: new_total_tokens,
            threshold_tokens: 0,
            decision: "force_au2_summary",
        })
    }

    /// 检查是否需要压缩，如果需要则执行压缩逻辑
    pub async fn compress_if_needed(
        &self,
        messages: Vec<StarMessage>,
        client: Option<&StarClient>,
    ) -> Result<CompressionResult, Box<dyn std::error::Error + Send + Sync + 'static>> {
        // 冷却检查: 如果在冷却期内，跳过压缩检查
        if self.is_in_cooldown() {
            let total_tokens = self.calculate_tokens(&messages);
            return Ok(CompressionResult {
                messages,
                was_compacted: false,
                original_token_count: total_tokens,
                new_token_count: total_tokens,
                threshold_tokens: 0,
                decision: "cooldown_skip",
            });
        }

        let total_tokens = self.calculate_tokens(&messages);

        // 更新检查时间
        self.update_check_time();

        let ratio = std::env::var("STAR_COMPRESSION_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(COMPRESSION_THRESHOLD_RATIO);

        // 使用动态参数（根据上下文窗口大小自动调整）
        let min_tokens = std::env::var("STAR_COMPRESSION_MIN_TOKENS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(self.min_tokens);

        let ratio_threshold = (self.context_window as f64 * ratio) as usize;
        // 对齐 Claude Code：threshold = window - buffer（约 95-98%）
        let ratio_threshold =
            ratio_threshold.max(self.context_window.saturating_sub(self.buffer_tokens));
        let threshold = ratio_threshold.max(min_tokens);

        if total_tokens < threshold {
            return Ok(CompressionResult {
                messages,
                was_compacted: false,
                original_token_count: total_tokens,
                new_token_count: total_tokens,
                threshold_tokens: threshold,
                decision: "below_threshold",
            });
        }

        // ============ 执行压缩逻辑 ============
        crate::utils::logging::append_debug_log_line(&format!(
            "⚠️ Context token usage {} exceeds threshold {}. Triggering compression.",
            total_tokens, threshold
        ));

        if messages.len() <= 5 {
            return Ok(CompressionResult {
                messages,
                was_compacted: false,
                original_token_count: total_tokens,
                new_token_count: total_tokens,
                threshold_tokens: threshold,
                decision: "insufficient_messages",
            });
        }

        // --- Level 1 ---
        let (messages_l1, changed_l1) =
            self.perform_level_1_compression(messages.clone(), LEVEL1_KEEP_RECENT);
        let tokens_l1 = self.calculate_tokens(&messages_l1);

        if changed_l1 && tokens_l1 < threshold {
            crate::utils::logging::append_debug_log_line(
                "✅ Level 1 compression (truncation) successful.",
            );
            return Ok(CompressionResult {
                messages: messages_l1,
                was_compacted: true,
                original_token_count: total_tokens,
                new_token_count: tokens_l1,
                threshold_tokens: threshold,
                decision: "level1_truncation",
            });
        }

        // --- Level 2 (Summarization - AU2 8-Section Strategy) ---
        // Use messages_l1 as base (already truncated, might help summarizer too)
        let keep_count = LEVEL2_KEEP_RECENT;
        let split_idx = messages_l1.len().saturating_sub(keep_count);
        // Ensure we don't cut off system prompt if it exists at index 0
        let mut split_idx = if split_idx < 1 { 1 } else { split_idx };

        // Adjust split point to avoid keeping orphaned tool messages whose
        // corresponding assistant tool_calls are in the summarized portion.
        // DeepSeek requires each tool message to follow an assistant with tool_calls,
        // so a tool message at the start of the "keep" section is invalid.
        while split_idx < messages_l1.len() && messages_l1[split_idx].role == "tool" {
            split_idx += 1;
        }

        let recent_messages = messages_l1[split_idx..].to_vec();

        let mut summary_text = format!(
            "(Auto Compression Level 2: Omitted {} to {} messages.)",
            1, split_idx
        );

        if let Some(client) = client {
            let messages_to_summarize = &messages_l1[0..split_idx];
            if let Ok(generated_summary) = self
                .generate_au2_summary(messages_to_summarize, client)
                .await
            {
                summary_text = generated_summary;
                crate::utils::logging::append_debug_log_line(
                    "✅ Level 2 compression (AU2) successful.",
                );
            } else {
                crate::utils::logging::append_debug_log_line(
                    "❌ Level 2 compression LLM call failed. Using placeholder.",
                );
            }
        }

        let summary_msg = StarMessage::system(format!(
            "Conversation Summary (AU2 Compressed):\n{}",
            summary_text
        ));

        let mut new_messages = Vec::new();
        if !messages_l1.is_empty() && messages_l1[0].role == "system" {
            new_messages.push(messages_l1[0].clone());
        }
        new_messages.push(summary_msg);
        new_messages.extend(recent_messages);

        let new_total_tokens = self.calculate_tokens(&new_messages);

        // Validate: if Level 2 summary didn't actually reduce tokens (or made things worse),
        // fall back to Level 1 simple truncation which is guaranteed to reduce
        if new_total_tokens >= total_tokens {
            crate::utils::logging::append_debug_log_line(&format!(
                "⚠️ Level 2 compression INEFFECTIVE: new={} >= old={}. Falling back to Level 1.",
                new_total_tokens, total_tokens
            ));
            return Ok(CompressionResult {
                messages: messages_l1,
                was_compacted: changed_l1,
                original_token_count: total_tokens,
                new_token_count: tokens_l1,
                threshold_tokens: threshold,
                decision: "level1_fallback_from_ineffective_l2",
            });
        }

        Ok(CompressionResult {
            messages: new_messages,
            was_compacted: true,
            original_token_count: total_tokens,
            new_token_count: new_total_tokens,
            threshold_tokens: threshold,
            decision: "level2_summary",
        })
    }
}
