use super::CompactStrategy;
use crate::types::StarMessage;
use async_trait::async_trait;

/// 微压缩策略
///
/// 轻量级压缩，更频繁地触发（每 N 条消息）
/// 只压缩大型工具输出（截断到前/后 N 行）
/// 保留：所有用户消息、所有助手消息、工具调用元数据
pub struct MicroCompactStrategy {
    /// 触发压缩的工具输出行数阈值
    tool_output_line_threshold: usize,
    /// 保留的头部行数
    head_lines: usize,
    /// 保留的尾部行数
    tail_lines: usize,
    /// 消息计数器（用于触发频率控制）
    message_counter: std::sync::atomic::AtomicUsize,
    /// 触发频率（每 N 条消息）
    trigger_frequency: usize,
}

impl MicroCompactStrategy {
    pub fn new() -> Self {
        Self {
            tool_output_line_threshold: 100,
            head_lines: 30,
            tail_lines: 15,
            message_counter: std::sync::atomic::AtomicUsize::new(0),
            trigger_frequency: 10,
        }
    }

    pub fn with_tool_output_threshold(mut self, lines: usize) -> Self {
        self.tool_output_line_threshold = lines;
        self
    }

    pub fn with_trigger_frequency(mut self, frequency: usize) -> Self {
        self.trigger_frequency = frequency;
        self
    }

    /// 检查是否应该触发压缩
    fn should_trigger(&self) -> bool {
        let count = self
            .message_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        count % self.trigger_frequency == 0
    }

    /// 压缩单个工具输出
    fn compress_tool_output(&self, content: &str) -> Option<String> {
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if total_lines <= self.tool_output_line_threshold {
            return None;
        }

        let mut result = String::new();

        // 添加头部行
        for line in lines.iter().take(self.head_lines) {
            result.push_str(line);
            result.push('\n');
        }

        // 添加省略提示
        let omitted_lines = total_lines - self.head_lines - self.tail_lines;
        result.push_str(&format!("\n... ({} more lines) ...\n\n", omitted_lines));

        // 添加尾部行
        for line in lines.iter().skip(total_lines - self.tail_lines) {
            result.push_str(line);
            result.push('\n');
        }

        Some(result)
    }
}

#[async_trait]
impl CompactStrategy for MicroCompactStrategy {
    fn name(&self) -> &str {
        "micro_compact"
    }

    fn can_apply(&self, messages: &[StarMessage], _token_count: usize) -> bool {
        // 检查是否应该触发
        if !self.should_trigger() {
            return false;
        }

        // 检查是否有大型工具输出
        messages.iter().any(|msg| {
            if msg.role == "tool" {
                if let Some(content) = &msg.content {
                    let line_count = content.lines().count();
                    line_count > self.tool_output_line_threshold
                } else {
                    false
                }
            } else {
                false
            }
        })
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        use super::tool_output_compact::EXEMPT_TOOLS;

        let mut result = Vec::with_capacity(messages.len());
        let mut changed = false;

        // 首先收集工具名称映射（tool_call_id -> tool_name）
        let mut tool_name_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in messages.iter() {
            if msg.role == "assistant" {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        tool_name_map.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        for msg in messages {
            if msg.role == "tool" {
                // 检查工具是否豁免（如 Read、Grep、Glob 等）
                let is_exempt = msg
                    .tool_call_id
                    .as_ref()
                    .and_then(|id| tool_name_map.get(id))
                    .map(|tool_name| EXEMPT_TOOLS.iter().any(|e| e == tool_name))
                    .unwrap_or(false);

                // 豁免工具不压缩
                if is_exempt {
                    result.push(msg.clone());
                    continue;
                }

                if let Some(content) = &msg.content {
                    if let Some(compressed) = self.compress_tool_output(content) {
                        let mut new_msg = msg.clone();
                        new_msg.content = Some(compressed);
                        result.push(new_msg);
                        changed = true;
                        continue;
                    }
                }
            }
            result.push(msg.clone());
        }

        if changed {
            crate::utils::logging::append_debug_log_line("[COMPACT] Applied micro compression");
        }

        result
    }

    fn priority(&self) -> u32 {
        200 // 中等优先级
    }
}
