use crate::types::StarMessage;
use super::CompactStrategy;

/// 缓存编辑模式微压缩策略
/// 
/// 对标claude-code-main的cachedMicrocompact.ts
/// 在编辑模式下缓存微压缩结果，避免重复计算
pub struct CachedMicroCompactStrategy {
    /// 缓存的压缩结果
    cache: Option<CachedCompactResult>,
    /// 缓存有效期（消息数量）
    cache_validity_messages: usize,
    /// 缓存有效期（秒）
    cache_validity_seconds: u64,
}

/// 缓存的压缩结果
#[derive(Debug, Clone)]
struct CachedCompactResult {
    /// 输入消息的哈希
    input_hash: u64,
    /// 压缩后的消息
    compressed_messages: Vec<StarMessage>,
    /// 创建时间
    created_at: std::time::Instant,
    /// 消息数量
    message_count: usize,
}

impl CachedMicroCompactStrategy {
    pub fn new() -> Self {
        Self {
            cache: None,
            cache_validity_messages: 5,
            cache_validity_seconds: 30,
        }
    }

    /// 设置缓存有效期（消息数量）
    pub fn with_cache_validity_messages(mut self, messages: usize) -> Self {
        self.cache_validity_messages = messages;
        self
    }

    /// 设置缓存有效期（秒）
    pub fn with_cache_validity_seconds(mut self, seconds: u64) -> Self {
        self.cache_validity_seconds = seconds;
        self
    }

    /// 计算消息列表的哈希
    fn hash_messages(messages: &[StarMessage]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        
        for msg in messages {
            msg.role.hash(&mut hasher);
            if let Some(content) = &msg.content {
                content.hash(&mut hasher);
            }
            if let Some(tool_call_id) = &msg.tool_call_id {
                tool_call_id.hash(&mut hasher);
            }
        }

        hasher.finish()
    }

    /// 检查缓存是否有效
    fn is_cache_valid(&self, messages: &[StarMessage]) -> bool {
        if let Some(cache) = &self.cache {
            let current_hash = Self::hash_messages(messages);
            let current_count = messages.len();
            
            // 检查哈希是否匹配
            if cache.input_hash != current_hash {
                return false;
            }

            // 检查消息数量变化
            if current_count.abs_diff(cache.message_count) > self.cache_validity_messages {
                return false;
            }

            // 检查时间有效期
            if cache.created_at.elapsed().as_secs() > self.cache_validity_seconds {
                return false;
            }

            true
        } else {
            false
        }
    }

    /// 执行微压缩（编辑模式优化）
    fn apply_micro_compact_edit_mode(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        use super::tool_output_compact::EXEMPT_TOOLS;

        let mut result = Vec::with_capacity(messages.len());
        let mut i = 0;

        // 首先收集工具名称映射（tool_call_id -> tool_name）
        let mut tool_name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        tool_name_map.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        while i < messages.len() {
            let msg = &messages[i];

            // 如果是工具消息且包含文件内容，尝试压缩
            if msg.role == "tool" {
                // 检查工具是否豁免（如 Read、Grep、Glob 等）
                let is_exempt = msg.tool_call_id.as_ref()
                    .and_then(|id| tool_name_map.get(id))
                    .map(|tool_name| EXEMPT_TOOLS.iter().any(|e| e == tool_name))
                    .unwrap_or(false);

                if is_exempt {
                    // 豁免工具不压缩
                    result.push(msg.clone());
                    i += 1;
                    continue;
                }

                if let Some(content) = &msg.content {
                    // 检测是否是文件读取结果
                    if self.is_file_read_result(content) {
                        let compressed = self.compress_file_content(content);
                        let mut compressed_msg = msg.clone();
                        compressed_msg.content = Some(compressed);
                        result.push(compressed_msg);
                        i += 1;
                        continue;
                    }

                    // 检测是否是编辑操作结果
                    if self.is_edit_result(content) {
                        let compressed = self.compress_edit_result(content);
                        let mut compressed_msg = msg.clone();
                        compressed_msg.content = Some(compressed);
                        result.push(compressed_msg);
                        i += 1;
                        continue;
                    }
                }
            }

            // 如果是助手消息且包含长代码块，尝试压缩
            if msg.role == "assistant" {
                if let Some(content) = &msg.content {
                    if content.len() > 1000 && content.contains("```") {
                        let compressed = self.compress_code_blocks(content);
                        let mut compressed_msg = msg.clone();
                        compressed_msg.content = Some(compressed);
                        result.push(compressed_msg);
                        i += 1;
                        continue;
                    }
                }
            }

            // 默认保留原消息
            result.push(msg.clone());
            i += 1;
        }

        result
    }

    /// 检测是否是文件读取结果
    fn is_file_read_result(&self, content: &str) -> bool {
        content.len() > 500 
            && (content.contains("line ") || content.contains("Line "))
            && (content.contains(":") || content.contains("|"))
    }

    /// 压缩文件内容
    fn compress_file_content(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        
        if lines.len() <= 20 {
            return content.to_string();
        }

        // 保留前10行和后5行，中间用摘要替代
        let mut result = String::new();
        for line in lines.iter().take(10) {
            result.push_str(line);
            result.push('\n');
        }
        
        result.push_str(&format!(
            "\n[... {} lines omitted ...]\n\n",
            lines.len() - 15
        ));
        
        for line in lines.iter().skip(lines.len() - 5) {
            result.push_str(line);
            result.push('\n');
        }

        result
    }

    /// 检测是否是编辑操作结果
    fn is_edit_result(&self, content: &str) -> bool {
        content.contains("Successfully edited") 
            || content.contains("File updated")
            || content.contains("Changes applied")
    }

    /// 压缩编辑结果
    fn compress_edit_result(&self, content: &str) -> String {
        // 编辑结果通常可以大幅压缩
        if content.len() > 200 {
            let first_line = content.lines().next().unwrap_or("");
            format!("{} [details omitted]", first_line)
        } else {
            content.to_string()
        }
    }

    /// 压缩代码块
    fn compress_code_blocks(&self, content: &str) -> String {
        let mut result = String::new();
        let mut in_code_block = false;
        let mut code_block_lines = 0;
        let mut code_block_content = String::new();

        for line in content.lines() {
            if line.starts_with("```") {
                if in_code_block {
                    // 结束代码块
                    if code_block_lines > 20 {
                        result.push_str("```\n");
                        result.push_str(&format!(
                            "[... {} lines of code omitted ...]\n",
                            code_block_lines - 10
                        ));
                        // 保留最后几行
                        let last_lines: Vec<&str> = code_block_content.lines()
                            .skip(code_block_lines - 5)
                            .collect();
                        for last_line in last_lines {
                            result.push_str(last_line);
                            result.push('\n');
                        }
                        result.push_str("```\n");
                    } else {
                        result.push_str(&code_block_content);
                        result.push_str("```\n");
                    }
                    in_code_block = false;
                    code_block_lines = 0;
                    code_block_content.clear();
                } else {
                    // 开始代码块
                    in_code_block = true;
                    result.push_str(line);
                    result.push('\n');
                }
            } else if in_code_block {
                code_block_lines += 1;
                code_block_content.push_str(line);
                code_block_content.push('\n');
                
                // 保留前5行
                if code_block_lines <= 5 {
                    result.push_str(line);
                    result.push('\n');
                }
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }
}

impl CompactStrategy for CachedMicroCompactStrategy {
    fn name(&self) -> &str {
        "cached_micro_compact"
    }

    fn can_apply(&self, messages: &[StarMessage], _token_count: usize) -> bool {
        // 检查是否有工具消息或长助手消息
        messages.iter().any(|m| {
            (m.role == "tool" && m.content.as_ref().map_or(false, |c| c.len() > 500))
                || (m.role == "assistant" && m.content.as_ref().map_or(false, |c| c.len() > 1000))
        })
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        // 检查缓存
        if self.is_cache_valid(messages) {
            if let Some(cache) = &self.cache {
                return cache.compressed_messages.clone();
            }
        }

        // 执行压缩
        self.apply_micro_compact_edit_mode(messages)
    }

    fn priority(&self) -> u32 {
        15 // 优先级介于工具输出压缩和微压缩之间
    }
}
