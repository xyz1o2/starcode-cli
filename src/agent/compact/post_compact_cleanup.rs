use crate::types::StarMessage;

/// 压缩后清理
/// 
/// 对标claude-code-main的postCompactCleanup.ts
/// 在压缩执行后进行清理工作
pub struct PostCompactCleanup {
    /// 是否清理孤立的工具调用
    cleanup_orphaned_tool_calls: bool,
    /// 是否清理空消息
    cleanup_empty_messages: bool,
    /// 是否重新计算消息引用
    recalculate_references: bool,
    /// 是否添加压缩标记
    add_compaction_marker: bool,
}

impl PostCompactCleanup {
    pub fn new() -> Self {
        Self {
            cleanup_orphaned_tool_calls: true,
            cleanup_empty_messages: true,
            recalculate_references: true,
            add_compaction_marker: true,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        let cleanup_orphaned = std::env::var("STAR_CLEANUP_ORPHANED_TOOL_CALLS")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let cleanup_empty = std::env::var("STAR_CLEANUP_EMPTY_MESSAGES")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let recalculate = std::env::var("STAR_RECALCULATE_REFERENCES")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let add_marker = std::env::var("STAR_ADD_COMPACTION_MARKER")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            cleanup_orphaned_tool_calls: cleanup_orphaned,
            cleanup_empty_messages: cleanup_empty,
            recalculate_references: recalculate,
            add_compaction_marker: add_marker,
        }
    }

    /// 执行压缩后清理
    pub fn cleanup(&self, messages: Vec<StarMessage>, was_compacted: bool) -> Vec<StarMessage> {
        if !was_compacted {
            return messages;
        }

        let mut result = messages;

        // 1. 清理空消息
        if self.cleanup_empty_messages {
            result = self.remove_empty_messages(result);
        }

        // 2. 清理孤立的工具调用
        if self.cleanup_orphaned_tool_calls {
            result = self.remove_orphaned_tool_calls(result);
        }

        // 3. 重新计算消息引用
        if self.recalculate_references {
            result = self.recalculate_message_references(result);
        }

        // 4. 添加压缩标记
        if self.add_compaction_marker {
            result = self.add_compaction_marker(result);
        }

        result
    }

    /// 移除空消息
    fn remove_empty_messages(&self, messages: Vec<StarMessage>) -> Vec<StarMessage> {
        messages.into_iter()
            .filter(|msg| {
                // 保留系统消息（即使为空，可能包含重要配置）
                if msg.role == "system" {
                    return true;
                }

                // 检查是否有内容
                let has_content = msg.content.as_ref().map_or(false, |c| !c.trim().is_empty());
                let has_tool_calls = msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                let has_tool_call_id = msg.tool_call_id.is_some();

                has_content || has_tool_calls || has_tool_call_id
            })
            .collect()
    }

    /// 移除孤立的工具调用
    fn remove_orphaned_tool_calls(&self, messages: Vec<StarMessage>) -> Vec<StarMessage> {
        // 收集所有工具调用ID
        let tool_call_ids: std::collections::HashSet<String> = messages.iter()
            .filter_map(|msg| msg.tool_call_id.clone())
            .collect();

        // 收集所有助手消息中声明的工具调用ID
        let declared_tool_call_ids: std::collections::HashSet<String> = messages.iter()
            .filter(|msg| msg.role == "assistant")
            .filter_map(|msg| msg.tool_calls.as_ref())
            .flat_map(|tc| tc.iter())
            .map(|tc| tc.id.clone())
            .collect();

        // 找出孤立的工具调用（有声明但无结果）
        let orphaned_ids: Vec<String> = declared_tool_call_ids.difference(&tool_call_ids)
            .cloned()
            .collect();

        if orphaned_ids.is_empty() {
            return messages;
        }

        // 移除孤立的工具调用
        messages.into_iter()
            .map(|mut msg| {
                if msg.role == "assistant" {
                    if let Some(tool_calls) = msg.tool_calls {
                        let filtered: Vec<_> = tool_calls.into_iter()
                            .filter(|tc| !orphaned_ids.contains(&tc.id))
                            .collect();
                        
                        if filtered.is_empty() {
                            msg.tool_calls = None;
                        } else {
                            msg.tool_calls = Some(filtered);
                        }
                    }
                }
                msg
            })
            .collect()
    }

    /// 重新计算消息引用
    fn recalculate_message_references(&self, messages: Vec<StarMessage>) -> Vec<StarMessage> {
        // 在Rust实现中，消息索引通过Vec的位置隐式维护
        // 这里我们只进行基本的消息清理和规范化
        let mut result = Vec::with_capacity(messages.len());

        for msg in messages {
            // 确保消息的基本字段完整
            let mut cleaned_msg = msg;
            
            // 如果内容为空，设置为空字符串
            if cleaned_msg.content.is_none() {
                cleaned_msg.content = Some(String::new());
            }

            result.push(cleaned_msg);
        }

        result
    }

    /// 添加压缩标记
    fn add_compaction_marker(&self, mut messages: Vec<StarMessage>) -> Vec<StarMessage> {
        if messages.is_empty() {
            return messages;
        }

        // 在消息开头添加压缩标记
        let marker = StarMessage::system(
            "[Context was compacted to fit within token limits. Some earlier messages may have been summarized or removed.]"
        );
        
        messages.insert(0, marker);
        messages
    }

    /// 清理工具结果中的敏感信息
    pub fn sanitize_tool_results(&self, messages: Vec<StarMessage>) -> Vec<StarMessage> {
        messages.into_iter()
            .map(|mut msg| {
                if msg.role == "tool" {
                    if let Some(content) = msg.content {
                        // 移除可能的敏感信息
                        let sanitized = self.sanitize_content(&content);
                        msg.content = Some(sanitized);
                    }
                }
                msg
            })
            .collect()
    }

    /// 清理内容中的敏感信息
    fn sanitize_content(&self, content: &str) -> String {
        let mut result = content.to_string();

        // 简化的API密钥模式匹配（不使用正则表达式）
        let sensitive_patterns = [
            ("api_key=", "[REDACTED]"),
            ("api-key=", "[REDACTED]"),
            ("API_KEY=", "[REDACTED]"),
            ("secret=", "[REDACTED]"),
            ("SECRET=", "[REDACTED]"),
            ("token=", "[REDACTED]"),
            ("TOKEN=", "[REDACTED]"),
            ("password=", "[REDACTED]"),
            ("PASSWORD=", "[REDACTED]"),
        ];

        for (pattern, replacement) in &sensitive_patterns {
            if result.contains(pattern) {
                // 简单的模式替换
                let parts: Vec<&str> = result.split(pattern).collect();
                if parts.len() > 1 {
                    let mut new_result = parts[0].to_string();
                    for i in 1..parts.len() {
                        // 替换等号后的值
                        let value_part = parts[i];
                        if let Some(end_pos) = value_part.find(|c: char| c.is_whitespace() || c == ',' || c == ';') {
                            new_result.push_str(pattern);
                            new_result.push_str(replacement);
                            new_result.push_str(&value_part[end_pos..]);
                        } else {
                            new_result.push_str(pattern);
                            new_result.push_str(replacement);
                        }
                    }
                    result = new_result;
                }
            }
        }

        // 移除文件路径中的用户目录
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if !home_dir.is_empty() {
            result = result.replace(&home_dir, "~");
        }

        result
    }
}

/// 压缩清理管理器
/// 
/// 管理压缩后的清理流程
pub struct PostCompactCleanupManager {
    cleanup: PostCompactCleanup,
    /// 清理统计
    stats: CleanupStats,
}

/// 清理统计
#[derive(Debug, Default)]
pub struct CleanupStats {
    pub total_cleanups: u64,
    pub empty_messages_removed: u64,
    pub orphaned_tool_calls_removed: u64,
    pub messages_recalculated: u64,
}

impl PostCompactCleanupManager {
    pub fn new() -> Self {
        Self {
            cleanup: PostCompactCleanup::new(),
            stats: CleanupStats::default(),
        }
    }

    /// 执行清理并更新统计
    pub fn cleanup_with_stats(&mut self, messages: Vec<StarMessage>, was_compacted: bool) -> Vec<StarMessage> {
        let initial_count = messages.len();
        let result = self.cleanup.cleanup(messages, was_compacted);
        let final_count = result.len();

        if was_compacted {
            self.stats.total_cleanups += 1;
            self.stats.empty_messages_removed += (initial_count - final_count) as u64;
        }

        result
    }

    /// 获取统计信息
    pub fn stats(&self) -> &CleanupStats {
        &self.stats
    }

    /// 重置统计信息
    pub fn reset_stats(&mut self) {
        self.stats = CleanupStats::default();
    }
}
