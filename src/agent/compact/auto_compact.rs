use super::{CompactConfig, CompactStrategy};
use crate::llm::client::StarClient;
use crate::types::StarMessage;
use async_trait::async_trait;
use std::time::{Duration, Instant};

/// 熔断器
///
/// 用于防止自动压缩在连续失败时反复触发
/// 当连续失败次数达到阈值时，进入冷却状态
pub struct CircuitBreaker {
    pub failure_count: u32,
    pub threshold: u32,
    pub cooldown_duration: Duration,
    pub last_failure: Option<Instant>,
    pub is_open: bool,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            failure_count: 0,
            threshold: 3,
            cooldown_duration: Duration::from_secs(300),
            last_failure: None,
            is_open: false,
        }
    }

    pub fn record_success(&mut self) {
        self.failure_count = 0;
        self.is_open = false;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());
        if self.failure_count >= self.threshold {
            self.is_open = true;
        }
    }

    pub fn should_allow(&mut self) -> bool {
        if !self.is_open {
            return true;
        }
        if let Some(last) = self.last_failure {
            if last.elapsed() >= self.cooldown_duration {
                self.is_open = false;
                self.failure_count = 0;
                return true;
            }
        }
        false
    }
}

/// 自动压缩策略
///
/// 当 token 数量超过 max_tokens 时触发
/// 保留：系统提示、最近 N 条消息、包含重要数据的工具结果
/// 压缩：旧助手消息（摘要）、旧工具结果（仅保留摘要）
pub struct AutoCompactStrategy {
    config: CompactConfig,
}

impl AutoCompactStrategy {
    pub fn new(config: CompactConfig) -> Self {
        Self { config }
    }

    /// 生成对话摘要
    async fn generate_summary(
        &self,
        messages: &[StarMessage],
        client: &StarClient,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let summary_prompt = crate::core::prompts::summary::summary_prompt();

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
            summary_prompt, history_text
        ));

        let response = client.chat(vec![prompt_message], None, None, None).await?;

        if let Some(choice) = response.choices.first() {
            if let Some(content) = &choice.message.content {
                return Ok(content.clone());
            }
        }

        Err("LLM returned empty response".into())
    }

    /// 确定需要保留的最近消息数
    fn get_keep_recent_count(&self) -> usize {
        // 保留最近的 6 条消息（3 轮对话）
        6
    }

    /// 检查消息是否包含重要数据（不应被压缩）
    fn has_important_data(&self, msg: &StarMessage) -> bool {
        // 系统消息通常重要
        if msg.role == "system" {
            return true;
        }

        // 包含错误信息的工具结果
        if msg.role == "tool" {
            if let Some(content) = &msg.content {
                let lower = content.to_lowercase();
                if lower.contains("error") || lower.contains("failed") || lower.contains("failure")
                {
                    return true;
                }
            }
        }

        false
    }

    /// 从被压缩的旧消息中提取关键信息，生成压缩摘要。
    /// 保留涉及文件、关键操作/决策与使用工具，避免压缩后信息丢失。
    fn extract_key_summary(omitted: &[StarMessage]) -> String {
        if omitted.is_empty() {
            return String::new();
        }

        let mut key_files: Vec<String> = Vec::new();
        let mut key_actions: Vec<String> = Vec::new();
        let mut tools: Vec<String> = Vec::new();

        for m in omitted {
            if let Some(c) = m.content.as_deref() {
                for line in c.lines() {
                    let trimmed = line.trim();
                    // 涉及文件路径
                    let is_code_ext = trimmed.ends_with(".rs")
                        || trimmed.ends_with(".ts")
                        || trimmed.ends_with(".js")
                        || trimmed.ends_with(".py")
                        || trimmed.ends_with(".md")
                        || trimmed.ends_with(".toml")
                        || trimmed.ends_with(".json")
                        || trimmed.ends_with(".go")
                        || trimmed.ends_with(".c")
                        || trimmed.ends_with(".cpp")
                        || trimmed.ends_with(".h")
                        || trimmed.ends_with(".java");
                    if (trimmed.contains("src/")
                        || trimmed.contains("path/")
                        || trimmed.starts_with("./")
                        || trimmed.contains(":/"))
                        && is_code_ext
                    {
                        let file = trimmed
                            .split_whitespace()
                            .next()
                            .unwrap_or(trimmed)
                            .trim_matches(|ch: char| {
                                ch == '`' || ch == '"' || ch == '\'' || ch == ','
                            });
                        if !key_files.contains(&file.to_string()) && key_files.len() < 20 {
                            key_files.push(file.to_string());
                        }
                    }
                    // 关键操作/决策：以动词开头或含决策性关键词的中等长度行
                    if (trimmed.starts_with("修复")
                        || trimmed.starts_with("决定")
                        || trimmed.starts_with("选择")
                        || trimmed.starts_with("方案")
                        || trimmed.starts_with("修改")
                        || trimmed.starts_with("新增")
                        || trimmed.starts_with("添加")
                        || trimmed.starts_with("删除")
                        || trimmed.starts_with("重构")
                        || trimmed.starts_with("实现")
                        || trimmed.starts_with("创建"))
                        && trimmed.len() > 6
                        && trimmed.len() < 160
                    {
                        if !key_actions.contains(&trimmed.to_string()) && key_actions.len() < 12 {
                            key_actions.push(trimmed.to_string());
                        }
                    }
                }
            }
            // 使用工具
            if let Some(tcs) = &m.tool_calls {
                for tc in tcs {
                    if !tools.contains(&tc.function.name) {
                        tools.push(tc.function.name.clone());
                    }
                }
            }
        }

        let mut lines: Vec<String> = Vec::new();
        if !key_files.is_empty() {
            lines.push("涉及文件：".to_string());
            for f in key_files.iter().take(15) {
                lines.push(format!("- {}", f));
            }
        }
        if !key_actions.is_empty() {
            lines.push("关键操作/决策：".to_string());
            for a in key_actions.iter().take(10) {
                lines.push(format!("- {}", a));
            }
        }
        if !tools.is_empty() {
            lines.push(format!("使用工具：{}", tools.join(", ")));
        }

        if lines.is_empty() {
            String::new()
        } else {
            format!("\n[SUMMARY]\n{}", lines.join("\n"))
        }
    }
}

#[async_trait]
impl CompactStrategy for AutoCompactStrategy {
    fn name(&self) -> &str {
        "auto_compact"
    }

    fn can_apply(&self, _messages: &[StarMessage], token_count: usize) -> bool {
        // 检查是否超过最大 token 数
        token_count > self.config.max_tokens
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        // 注意：这个方法不使用 LLM，只进行简单的截断
        // 实际的 LLM 摘要需要通过 AutoCompactManager 处理
        let keep_recent = self.get_keep_recent_count();

        if messages.len() <= keep_recent {
            return messages.to_vec();
        }

        let split_idx = messages.len() - keep_recent;
        let mut result = Vec::with_capacity(messages.len());

        // 保留系统消息
        if !messages.is_empty() && messages[0].role == "system" {
            result.push(messages[0].clone());
        }

        // 添加压缩提示（含关键信息提取摘要，避免压缩后丢失项目分析/文件路径/决策）
        let omitted_msgs = &messages[..split_idx];
        let key_summary = Self::extract_key_summary(omitted_msgs);
        result.push(StarMessage::system(format!(
            "[COMPACT] Context compressed. Omitted {} messages. Kept recent {} messages.\n{}",
            split_idx - 1,
            keep_recent,
            key_summary
        )));

        // Preserve ALL user messages from the omitted portion
        // This ensures user requests are not lost after compression
        for msg in &messages[1..split_idx] {
            if msg.role == "user" {
                result.push(msg.clone());
            }
        }

        // 保留被省略区间里【最近】的几条 tool 消息（工具输出内容）。
        // 之前 tool 消息被全部丢弃，导致 agent 在压缩后拿到"[COMPACT]"提示却
        // 误以为已读的文件/搜索结果仍在 —— 内容其实已被清空。
        // 这里只保留最近 MAX 条，避免完全抵消压缩的 token 收益。
        const MAX_KEEP_TOOL_MSGS: usize = 4;
        let omitted_tools: Vec<StarMessage> = messages[1..split_idx]
            .iter()
            .filter(|m| m.role == "tool")
            .rev()
            .take(MAX_KEEP_TOOL_MSGS)
            .cloned()
            .collect();
        if !omitted_tools.is_empty() {
            result.push(StarMessage::system(format!(
                "## Recent Tool Output (retained)\nRetained the latest {} tool result(s) from the compressed portion because their content may still be needed (file contents / search results).",
                omitted_tools.len()
            )));
            for t in omitted_tools.into_iter().rev() {
                result.push(t);
            }
        }

        // 保留最近的消息
        result.extend_from_slice(&messages[split_idx..]);

        crate::utils::logging::append_debug_log_line(&format!(
            "[COMPACT] Applied auto compression: {} → {} messages (preserved {} user messages)",
            messages.len(),
            result.len(),
            messages[1..split_idx]
                .iter()
                .filter(|m| m.role == "user")
                .count()
        ));

        result
    }

    fn priority(&self) -> u32 {
        300 // 较低优先级，作为后备
    }
}

/// 自动压缩管理器
///
/// 使用 LLM 生成摘要的完整自动压缩
pub struct AutoCompactManager {
    strategy: AutoCompactStrategy,
}

impl AutoCompactManager {
    pub fn new(config: CompactConfig) -> Self {
        Self {
            strategy: AutoCompactStrategy::new(config),
        }
    }

    /// 执行自动压缩（使用 LLM 生成摘要）
    pub async fn compact_with_llm(
        &self,
        messages: &[StarMessage],
        client: &StarClient,
    ) -> Result<Vec<StarMessage>, Box<dyn std::error::Error + Send + Sync>> {
        let keep_recent = self.strategy.get_keep_recent_count();

        if messages.len() <= keep_recent {
            return Ok(messages.to_vec());
        }

        let split_idx = messages.len() - keep_recent;

        // 确定要压缩的消息（排除系统消息）
        let start_idx = if !messages.is_empty() && messages[0].role == "system" {
            1
        } else {
            0
        };

        let messages_to_summarize = &messages[start_idx..split_idx];

        if messages_to_summarize.is_empty() {
            return Ok(messages.to_vec());
        }

        // 生成摘要
        let summary = self
            .strategy
            .generate_summary(messages_to_summarize, client)
            .await?;

        // 构建新消息列表
        let mut result = Vec::with_capacity(messages.len());

        // 保留系统消息
        if !messages.is_empty() && messages[0].role == "system" {
            result.push(messages[0].clone());
        }

        // 添加摘要消息
        result.push(StarMessage::system(format!(
            "## Context Compressed (Auto)\n\n{}\n\n---\n\
             **IMPORTANT**: The user's original messages have been summarized above. \
             Continue executing the task based on this summary.",
            summary
        )));

        // Preserve ALL user messages from the summarized portion
        for msg in &messages[start_idx..split_idx] {
            if msg.role == "user" {
                result.push(msg.clone());
            }
        }

        // 保留最近的消息
        result.extend_from_slice(&messages[split_idx..]);

        crate::utils::logging::append_debug_log_line(&format!(
            "[COMPACT] Applied auto compression with LLM summary: {} → {} messages",
            messages.len(),
            result.len()
        ));

        Ok(result)
    }

    /// 检查是否需要压缩
    pub fn needs_compaction(&self, token_count: usize) -> bool {
        self.strategy.can_apply(&[], token_count)
    }

    /// 获取配置
    pub fn config(&self) -> &CompactConfig {
        &self.strategy.config
    }
}
