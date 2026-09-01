use super::CompactStrategy;
use crate::types::StarMessage;

/// 会话记忆压缩策略
///
/// 对标claude-code-main的sessionMemoryCompact.ts
/// 将会话记忆压缩为摘要形式
pub struct SessionMemoryCompactStrategy {
    /// 保留最近的轮次数
    recent_turns_to_keep: usize,
    /// 每轮保留的最大消息数
    messages_per_turn: usize,
    /// 是否保留工具调用详情
    keep_tool_details: bool,
}

impl SessionMemoryCompactStrategy {
    pub fn new() -> Self {
        Self {
            recent_turns_to_keep: 3,
            messages_per_turn: 2,
            keep_tool_details: false,
        }
    }

    /// 设置保留最近的轮次数
    pub fn with_recent_turns(mut self, turns: usize) -> Self {
        self.recent_turns_to_keep = turns;
        self
    }

    /// 设置每轮保留的最大消息数
    pub fn with_messages_per_turn(mut self, messages: usize) -> Self {
        self.messages_per_turn = messages;
        self
    }

    /// 设置是否保留工具调用详情
    pub fn with_keep_tool_details(mut self, keep: bool) -> Self {
        self.keep_tool_details = keep;
        self
    }

    /// 识别轮次边界
    fn identify_turns(&self, messages: &[StarMessage]) -> Vec<TurnInfo> {
        let mut turns = Vec::new();
        let mut current_turn = TurnInfo {
            start_index: 0,
            end_index: 0,
            has_user_message: false,
            has_assistant_message: false,
            has_tool_calls: false,
            message_count: 0,
        };

        for (i, msg) in messages.iter().enumerate() {
            // 用户消息标志着新轮次的开始
            if msg.role == "user" && current_turn.message_count > 0 {
                turns.push(current_turn);
                current_turn = TurnInfo {
                    start_index: i,
                    end_index: i,
                    has_user_message: true,
                    has_assistant_message: false,
                    has_tool_calls: false,
                    message_count: 1,
                };
            } else {
                current_turn.end_index = i;
                current_turn.message_count += 1;

                match msg.role.as_str() {
                    "user" => current_turn.has_user_message = true,
                    "assistant" => {
                        current_turn.has_assistant_message = true;
                        if msg.tool_calls.is_some() {
                            current_turn.has_tool_calls = true;
                        }
                    }
                    "tool" => current_turn.has_tool_calls = true,
                    _ => {}
                }
            }
        }

        // 添加最后一个轮次
        if current_turn.message_count > 0 {
            turns.push(current_turn);
        }

        turns
    }

    /// 压缩单个轮次
    fn compress_turn(&self, messages: &[StarMessage], turn: &TurnInfo) -> Vec<StarMessage> {
        let mut result = Vec::new();
        let turn_messages = &messages[turn.start_index..=turn.end_index];

        // 如果轮次消息数小于限制，保留所有消息
        if turn_messages.len() <= self.messages_per_turn {
            return turn_messages.to_vec();
        }

        // 保留用户消息
        if let Some(user_msg) = turn_messages.iter().find(|m| m.role == "user") {
            result.push(user_msg.clone());
        }

        // 保留助手的关键回复
        if let Some(assistant_msg) = turn_messages.iter().find(|m| m.role == "assistant") {
            let compressed = self.compress_assistant_message(assistant_msg);
            result.push(compressed);
        }

        // 如果保留工具详情，添加工具消息摘要
        if self.keep_tool_details {
            let tool_msgs: Vec<&StarMessage> =
                turn_messages.iter().filter(|m| m.role == "tool").collect();

            if !tool_msgs.is_empty() {
                let summary = self.summarize_tool_messages(&tool_msgs);
                let mut summary_msg = StarMessage::system(&summary);
                result.push(summary_msg);
            }
        }

        result
    }

    /// 压缩助手消息
    fn compress_assistant_message(&self, message: &StarMessage) -> StarMessage {
        let mut compressed = message.clone();

        if let Some(content) = &message.content {
            if content.len() > 500 {
                // 保留前300字符和关键信息
                let truncated = self.extract_key_content(content);
                compressed.content = Some(truncated);
            }
        }

        // 移除工具调用详情
        if !self.keep_tool_details {
            compressed.tool_calls = None;
        }

        compressed
    }

    /// 提取关键内容
    fn extract_key_content(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();

        if lines.len() <= 10 {
            return content.to_string();
        }

        // 提取第一段（通常是摘要）
        let first_paragraph: String = lines
            .iter()
            .take_while(|line| !line.trim().is_empty())
            .cloned()
            .collect::<Vec<&str>>()
            .join("\n");

        // 提取最后一段（通常是结论）
        let last_paragraph: String = lines
            .iter()
            .rev()
            .take_while(|line| !line.trim().is_empty())
            .cloned()
            .collect::<Vec<&str>>()
            .into_iter()
            .rev()
            .collect::<Vec<&str>>()
            .join("\n");

        format!(
            "{}\n\n[... content summarized ...]\n\n{}",
            first_paragraph, last_paragraph
        )
    }

    /// 总结工具消息
    fn summarize_tool_messages(&self, messages: &[&StarMessage]) -> String {
        let mut tool_names = Vec::new();
        let mut has_errors = false;

        for msg in messages {
            if let Some(content) = &msg.content {
                if content.contains("error") || content.contains("Error") {
                    has_errors = true;
                }
            }

            if let Some(tool_call_id) = &msg.tool_call_id {
                tool_names.push(tool_call_id.clone());
            }
        }

        let mut summary = format!("Tools used: {}", tool_names.join(", "));
        if has_errors {
            summary.push_str(" (with errors)");
        }

        summary
    }
}

/// 轮次信息
#[derive(Debug)]
struct TurnInfo {
    start_index: usize,
    end_index: usize,
    has_user_message: bool,
    has_assistant_message: bool,
    has_tool_calls: bool,
    message_count: usize,
}

impl CompactStrategy for SessionMemoryCompactStrategy {
    fn name(&self) -> &str {
        "session_memory_compact"
    }

    fn can_apply(&self, messages: &[StarMessage], _token_count: usize) -> bool {
        // 只有当有足够的轮次时才应用
        let turns = self.identify_turns(messages);
        turns.len() > self.recent_turns_to_keep + 1
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        let turns = self.identify_turns(messages);

        if turns.len() <= self.recent_turns_to_keep {
            return messages.to_vec();
        }

        let mut result = Vec::new();
        let total_turns = turns.len();

        // 压缩早期轮次
        for (i, turn) in turns.iter().enumerate() {
            if i < total_turns - self.recent_turns_to_keep {
                // 早期轮次 - 压缩
                let compressed = self.compress_turn(messages, turn);
                result.extend(compressed);
            } else {
                // 最近轮次 - 保留完整内容
                let turn_messages = &messages[turn.start_index..=turn.end_index];
                result.extend(turn_messages.to_vec());
            }
        }

        // 添加轮次边界标记
        if !result.is_empty() {
            let marker = StarMessage::system(&format!(
                "[Session memory: {} turns compressed, {} recent turns preserved]",
                total_turns - self.recent_turns_to_keep,
                self.recent_turns_to_keep
            ));
            result.insert(0, marker);
        }

        result
    }

    fn priority(&self) -> u32 {
        25 // 优先级较低，在其他压缩策略之后
    }
}
