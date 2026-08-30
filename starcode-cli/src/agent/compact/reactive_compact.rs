use crate::types::StarMessage;
use crate::llm::client::StarClient;
use super::CompactConfig;

/// Reactive Compact配置
/// 
/// 对标claude-code-main的reactiveCompact.ts
/// 在API返回prompt-too-long错误时进行响应式压缩
#[derive(Debug, Clone)]
pub struct ReactiveCompactConfig {
    /// 是否启用Reactive Compact
    pub enabled: bool,
    /// 最大重试次数
    pub max_retries: usize,
    /// 压缩目标（原始大小的比例）
    pub target_ratio: f64,
    /// 保留最近消息数
    pub keep_recent_messages: usize,
    /// 是否保留系统消息
    pub preserve_system_messages: bool,
    /// 是否保留工具结果
    pub preserve_tool_results: bool,
}

impl Default for ReactiveCompactConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 2,
            target_ratio: 0.5,  // 压缩到50%
            keep_recent_messages: 6,
            preserve_system_messages: true,
            preserve_tool_results: false,
        }
    }
}

impl ReactiveCompactConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_REACTIVE_COMPACT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_retries = std::env::var("STAR_REACTIVE_COMPACT_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);

        let target_ratio = std::env::var("STAR_REACTIVE_COMPACT_TARGET_RATIO")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5);

        let keep_recent_messages = std::env::var("STAR_REACTIVE_COMPACT_KEEP_RECENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6);

        let preserve_system_messages = std::env::var("STAR_REACTIVE_COMPACT_PRESERVE_SYSTEM")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let preserve_tool_results = std::env::var("STAR_REACTIVE_COMPACT_PRESERVE_TOOL_RESULTS")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(false);

        Self {
            enabled,
            max_retries,
            target_ratio,
            keep_recent_messages,
            preserve_system_messages,
            preserve_tool_results,
        }
    }
}

/// Reactive Compact结果
#[derive(Debug, Clone)]
pub struct ReactiveCompactResult {
    /// 压缩后的消息
    pub messages: Vec<StarMessage>,
    /// 是否执行了压缩
    pub was_compacted: bool,
    /// 原始token数量
    pub original_token_count: usize,
    /// 压缩后token数量
    pub new_token_count: usize,
    /// 使用的策略
    pub strategy: String,
    /// 重试次数
    pub retry_count: usize,
}

/// Reactive Compact管理器
/// 
/// 在API返回prompt-too-long错误时进行响应式压缩
pub struct ReactiveCompactManager {
    config: ReactiveCompactConfig,
    compact_config: CompactConfig,
    /// 重试计数器
    retry_count: usize,
}

impl ReactiveCompactManager {
    pub fn new(compact_config: CompactConfig) -> Self {
        let config = ReactiveCompactConfig::from_env();
        Self {
            config,
            compact_config,
            retry_count: 0,
        }
    }

    /// 检查是否是prompt-too-long错误
    pub fn is_prompt_too_long_error(&self, error: &str) -> bool {
        let error_lower = error.to_lowercase();
        error_lower.contains("prompt_too_long") 
            || error_lower.contains("context_length_exceeded")
            || error_lower.contains("maximum context length")
            || error_lower.contains("token limit")
            || error_lower.contains("too many tokens")
    }

    /// 检查是否可以重试
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.config.max_retries
    }

    /// 重置重试计数器
    pub fn reset_retry_count(&mut self) {
        self.retry_count = 0;
    }

    /// 执行响应式压缩
    pub async fn reactive_compact(
        &mut self,
        messages: &[StarMessage],
        client: &StarClient,
        has_attempted: bool,
    ) -> Option<ReactiveCompactResult> {
        if !self.config.enabled {
            return None;
        }

        // 检查是否已经尝试过
        if has_attempted && !self.can_retry() {
            crate::utils::logging::append_debug_log_line(
                "[REACTIVE_COMPACT] Already attempted and max retries reached, skipping"
            );
            return None;
        }

        // 增加重试计数器
        self.retry_count += 1;

        crate::utils::logging::append_debug_log_line(
            &format!("[REACTIVE_COMPACT] Attempting reactive compression (attempt {}/{})", 
                self.retry_count, self.config.max_retries)
        );

        // 计算目标token数
        let current_tokens = super::token_counter::count_tokens(messages);
        let target_tokens = (current_tokens as f64 * self.config.target_ratio) as usize;

        // 尝试压缩
        let result = self.compress_messages(messages, client, target_tokens).await;

        match result {
            Ok(compressed_messages) => {
                let new_tokens = super::token_counter::count_tokens(&compressed_messages);
                
                crate::utils::logging::append_debug_log_line(&format!(
                    "[REACTIVE_COMPACT] Compression successful: {} → {} tokens ({}% reduction)",
                    current_tokens,
                    new_tokens,
                    ((current_tokens - new_tokens) as f64 / current_tokens as f64 * 100.0) as u32
                ));

                // 重置重试计数器
                self.retry_count = 0;

                Some(ReactiveCompactResult {
                    messages: compressed_messages,
                    was_compacted: true,
                    original_token_count: current_tokens,
                    new_token_count: new_tokens,
                    strategy: "reactive_compact".to_string(),
                    retry_count: self.retry_count,
                })
            }
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[REACTIVE_COMPACT] Compression failed: {}",
                    e
                ));
                None
            }
        }
    }

    /// 压缩消息
    async fn compress_messages(
        &self,
        messages: &[StarMessage],
        client: &StarClient,
        target_tokens: usize,
    ) -> Result<Vec<StarMessage>, Box<dyn std::error::Error + Send + Sync>> {
        // 分离系统消息、历史消息和最近消息
        let (system_messages, history_messages, recent_messages) = self.separate_messages(messages);
        
        // 构建压缩提示
        let prompt = self.build_compression_prompt(&history_messages, target_tokens);
        
        // 调用LLM进行压缩
        let response = client.chat(
            vec![StarMessage::user(prompt)],
            None,
            None,
            None,
        ).await?;

        // 解析响应
        if let Some(choice) = response.choices.first() {
            if let Some(content) = &choice.message.content {
                // 将压缩后的内容作为摘要消息
                let summary = StarMessage::system(format!(
                    "[REACTIVE_COMPACT] Previous context was compressed due to token limit.\n\nSummary:\n{}",
                    content
                ));
                
                // 构建压缩后的消息列表
                let mut result = Vec::new();
                
                // 添加系统消息
                if self.config.preserve_system_messages {
                    result.extend(system_messages);
                }
                
                // 添加摘要
                result.push(summary);
                
                // 添加最近的消息
                result.extend(recent_messages);
                
                return Ok(result);
            }
        }

        Err("Failed to generate compression summary".into())
    }

    /// 分离消息
    fn separate_messages(&self, messages: &[StarMessage]) -> (Vec<StarMessage>, Vec<StarMessage>, Vec<StarMessage>) {
        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                system_messages.push(msg.clone());
            } else {
                other_messages.push(msg.clone());
            }
        }

        // 分离最近消息和历史消息
        let keep_count = self.config.keep_recent_messages;
        let split_point = other_messages.len().saturating_sub(keep_count);
        
        let history_messages = other_messages[..split_point].to_vec();
        let recent_messages = other_messages[split_point..].to_vec();

        (system_messages, history_messages, recent_messages)
    }

    /// 构建压缩提示
    fn build_compression_prompt(&self, messages: &[StarMessage], target_tokens: usize) -> String {
        let mut history = String::new();
        for msg in messages {
            let role = &msg.role;
            let content = msg.content.as_deref().unwrap_or("[no content]");
            history.push_str(&format!("{}: {}\n\n", role, content));
        }

        format!(
            r#"Please summarize the following conversation history in a concise way.
The summary should capture the key points, decisions, and context needed to continue the conversation.

Target length: approximately {} tokens

Conversation history:
{}

Summary:"#,
            target_tokens,
            history
        )
    }

    /// 压缩单个消息（用于工具结果压缩）
    pub fn compress_single_message(&self, message: &StarMessage, max_tokens: usize) -> StarMessage {
        let content = message.content.as_deref().unwrap_or("");
        let current_tokens = content.len() / 4; // 粗略估算
        
        if current_tokens <= max_tokens {
            return message.clone();
        }

        // 截断内容
        let target_chars = max_tokens * 4;
        let truncated = if content.len() > target_chars {
            format!("{}...\n[Content truncated to fit token limit]", &content[..target_chars])
        } else {
            content.to_string()
        };

        StarMessage {
            role: message.role.clone(),
            content: Some(truncated),
            tool_call_id: message.tool_call_id.clone(),
            tool_calls: message.tool_calls.clone(),
            reasoning_content: message.reasoning_content.clone(),
            cache_control: message.cache_control.clone(),
        }
    }

    /// 构建压缩后的消息
    pub fn build_post_compact_messages(&self, result: &ReactiveCompactResult) -> Vec<StarMessage> {
        let mut messages = Vec::new();
        
        // 添加压缩边界消息
        messages.push(StarMessage::system(format!(
            "[COMPACT] Context was reactively compressed due to prompt-too-long error. \
             Original: {} tokens → Now: {} tokens. \
             Continue the task based on the summarized context above.",
            result.original_token_count,
            result.new_token_count
        )));
        
        // 添加压缩后的消息
        messages.extend(result.messages.clone());
        
        messages
    }
}

/// 检查是否是prompt-too-long错误
pub fn is_prompt_too_long_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();
    error_lower.contains("prompt_too_long") 
        || error_lower.contains("context_length_exceeded")
        || error_lower.contains("maximum context length")
        || error_lower.contains("token limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prompt_too_long_error() {
        assert!(is_prompt_too_long_error("Error: prompt_too_long"));
        assert!(is_prompt_too_long_error("context_length_exceeded"));
        assert!(is_prompt_too_long_error("Maximum context length is 200000 tokens"));
        assert!(!is_prompt_too_long_error("Some other error"));
    }
}
