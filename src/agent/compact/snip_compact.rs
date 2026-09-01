use super::CompactStrategy;
use crate::types::StarMessage;
use async_trait::async_trait;

/// 激进压缩策略
///
/// 当接近上下文限制时触发
/// 用摘要段落替换旧对话
/// 保留：系统提示 + 最后 10 条消息 + 摘要
pub struct SnipCompactStrategy {
    /// 保留的最近消息数
    keep_recent: usize,
    /// 触发压缩的 token 阈值
    token_threshold: usize,
}

impl SnipCompactStrategy {
    pub fn new() -> Self {
        Self {
            keep_recent: 10,
            // Default: trigger at 80% of typical context window (150K * 0.8 = 120K tokens)
            token_threshold: 120_000,
        }
    }

    pub fn with_keep_recent(mut self, count: usize) -> Self {
        self.keep_recent = count;
        self
    }

    pub fn with_token_threshold(mut self, threshold: usize) -> Self {
        self.token_threshold = threshold;
        self
    }

    /// 生成简单的摘要（不使用 LLM）
    /// 使用英文作为通用语言，避免硬编码特定语言
    fn generate_simple_summary(&self, messages: &[StarMessage]) -> String {
        let mut user_messages = Vec::new();
        let mut tool_calls_count = 0;
        let mut errors_count = 0;
        let mut last_user_request = String::new();
        let mut files_mentioned = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "user" => {
                    if let Some(content) = &msg.content {
                        last_user_request = content.clone();
                        user_messages.push(content.clone());
                    }
                }
                "assistant" => {
                    if msg.tool_calls.is_some() {
                        tool_calls_count += 1;
                    }
                }
                "tool" => {
                    if let Some(content) = &msg.content {
                        if content.to_lowercase().contains("error")
                            || content.to_lowercase().contains("fail")
                        {
                            errors_count += 1;
                        }
                        // Extract file paths from tool results
                        for line in content.lines() {
                            if line.contains(".rs")
                                || line.contains(".js")
                                || line.contains(".ts")
                                || line.contains(".py")
                                || line.contains(".go")
                                || line.contains(".java")
                            {
                                let path: String = line.chars().take(100).collect();
                                if !files_mentioned.contains(&path) {
                                    files_mentioned.push(path);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let mut summary = String::new();
        summary.push_str("## Conversation Summary\n\n");

        // Current task state
        summary.push_str("### Current Task\n");
        if !last_user_request.is_empty() {
            let preview: String = last_user_request.chars().take(200).collect();
            summary.push_str(&format!("- Latest request: {}\n", preview));
        }
        summary.push_str("- Status: Continue after context compression\n");

        // User requests
        summary.push_str("\n### User Requests\n");
        if user_messages.is_empty() {
            summary.push_str("- No user messages\n");
        } else {
            for (i, msg) in user_messages.iter().enumerate().take(3) {
                let preview: String = msg.chars().take(100).collect();
                summary.push_str(&format!("{}. {}\n", i + 1, preview));
            }
            if user_messages.len() > 3 {
                summary.push_str(&format!(
                    "... and {} more messages\n",
                    user_messages.len() - 3
                ));
            }
        }

        // Files mentioned
        if !files_mentioned.is_empty() {
            summary.push_str("\n### Files Referenced\n");
            for path in files_mentioned.iter().take(5) {
                summary.push_str(&format!("- {}\n", path));
            }
        }

        // Statistics
        summary.push_str("\n### Statistics\n");
        summary.push_str(&format!("- Tool calls: {}\n", tool_calls_count));
        summary.push_str(&format!("- Errors: {}\n", errors_count));
        summary.push_str(&format!("- Total messages: {}\n", messages.len()));

        // Next steps
        summary.push_str("\n### Next Steps\n");
        summary.push_str("- Continue executing the user's request based on the summary above\n");
        summary.push_str("- If the task is complete, provide a summary\n");

        summary
    }
}

#[async_trait]
impl CompactStrategy for SnipCompactStrategy {
    fn name(&self) -> &str {
        "snip_compact"
    }

    fn can_apply(&self, messages: &[StarMessage], token_count: usize) -> bool {
        // Only trigger when token count is high AND there are enough messages
        // This prevents premature compression on short conversations
        token_count >= self.token_threshold && messages.len() >= self.keep_recent + 5
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        if messages.len() <= self.keep_recent {
            return messages.to_vec();
        }

        let split_idx = messages.len() - self.keep_recent;

        // 确定要压缩的消息（排除系统消息）
        let start_idx = if !messages.is_empty() && messages[0].role == "system" {
            1
        } else {
            0
        };

        let messages_to_summarize = &messages[start_idx..split_idx];

        if messages_to_summarize.is_empty() {
            return messages.to_vec();
        }

        // 生成简单摘要
        let summary = self.generate_simple_summary(messages_to_summarize);

        // 构建新消息列表
        let mut result = Vec::with_capacity(messages.len());

        // 保留系统消息
        if !messages.is_empty() && messages[0].role == "system" {
            result.push(messages[0].clone());
        }

        // 添加摘要消息
        result.push(StarMessage::system(format!(
            "## Context Compressed\n\n{}\n\n---\n**IMPORTANT**: The user's original messages have been summarized above. \
             Continue executing the task based on this summary. If you need to reference specific user requests, \
             use the information from the summary above.",
            summary
        )));

        // Preserve ALL user messages from the summarized portion
        // This ensures user requests are not lost after compression
        for msg in &messages[start_idx..split_idx] {
            if msg.role == "user" {
                result.push(msg.clone());
            }
        }

        // 保留被省略区间里【最近】的几条 tool 消息（工具输出内容）。
        // 之前 tool 消息被全部丢弃，agent 压缩后误以为已读的文件/搜索结果仍在，
        // 但内容其实已被清空。这里只保留最近 MAX 条以控制 token 占用。
        const MAX_KEEP_TOOL_MSGS: usize = 4;
        let omitted_tools: Vec<StarMessage> = messages[start_idx..split_idx]
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
            "[COMPACT] Applied snip compression: {} → {} messages (preserved {} user messages)",
            messages.len(),
            result.len(),
            messages[start_idx..split_idx]
                .iter()
                .filter(|m| m.role == "user")
                .count()
        ));

        result
    }

    fn priority(&self) -> u32 {
        400 // 最低优先级，作为最后手段
    }
}
