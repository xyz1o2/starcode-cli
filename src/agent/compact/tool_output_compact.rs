use super::CompactStrategy;
use crate::types::StarMessage;
use async_trait::async_trait;

/// 不应被截断的工具列表
/// 这些工具的输出需要完整保留，因为模型需要完整内容来理解文件
/// 注意：使用starcode-cli中的实际工具名称
pub const EXEMPT_TOOLS: &[&str] = &[
    "Read",            // 主要的文件读取工具
    "read_many_files", // 批量读取文件
    "NotebookRead",    // Notebook读取
    "Grep",            // 搜索结果需要完整显示
    "Glob",            // 文件查找结果
    "ListDir",         // 目录列表
    "SemanticSearch",  // 语义搜索结果
    "ProjectMap",      // 项目地图
    "git_insight",     // Git洞察
    "git_branch",      // Git分支信息
];

/// 工具结果预算
///
/// 用于限制单个工具结果的大小
pub struct ToolResultBudget {
    pub max_chars_per_result: usize,
    pub max_lines_per_result: usize,
    /// 豁免工具列表（不会被截断）
    pub exempt_tools: Vec<String>,
}

impl ToolResultBudget {
    pub fn new() -> Self {
        Self {
            max_chars_per_result: 50000,
            max_lines_per_result: 1000,
            exempt_tools: EXEMPT_TOOLS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// 检查工具是否豁免
    pub fn is_exempt(&self, tool_name: &str) -> bool {
        self.exempt_tools.iter().any(|t| t == tool_name)
    }

    /// 对工具结果执行预算限制，必要时截断
    /// 如果工具在豁免列表中，则不截断
    pub fn enforce(&self, result: &str, tool_name: Option<&str>) -> String {
        // 检查工具是否豁免
        if let Some(name) = tool_name {
            if self.is_exempt(name) {
                return result.to_string();
            }
        }

        let lines: Vec<&str> = result.lines().collect();

        if lines.len() > self.max_lines_per_result {
            let kept = &lines[..self.max_lines_per_result];
            let omitted = lines.len() - self.max_lines_per_result;
            let mut output = kept.join("\n");
            output.push_str(&format!("\n... ({} lines omitted)", omitted));
            return output;
        }

        if result.len() > self.max_chars_per_result {
            let truncated: String = result.chars().take(self.max_chars_per_result).collect();
            let omitted = result.len() - self.max_chars_per_result;
            return format!("{}... ({} chars omitted)", truncated, omitted);
        }

        result.to_string()
    }

    /// 兼容旧接口（不指定工具名称）
    pub fn enforce_legacy(&self, result: &str) -> String {
        self.enforce(result, None)
    }
}

/// 工具输出压缩策略
///
/// 专门用于压缩大型工具输出，保留前 20 行 + 后 10 行 + "... (N more lines)"
/// 注意：Read等文件查看工具不会被压缩
pub struct ToolOutputCompactStrategy {
    /// 保留的头部行数
    head_lines: usize,
    /// 保留的尾部行数
    tail_lines: usize,
    /// 触发压缩的最小行数
    min_lines_to_compress: usize,
    /// 豁免工具列表
    exempt_tools: Vec<String>,
}

impl ToolOutputCompactStrategy {
    pub fn new() -> Self {
        Self {
            head_lines: 20,
            tail_lines: 10,
            min_lines_to_compress: 50,
            exempt_tools: EXEMPT_TOOLS.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn with_head_lines(mut self, lines: usize) -> Self {
        self.head_lines = lines;
        self
    }

    pub fn with_tail_lines(mut self, lines: usize) -> Self {
        self.tail_lines = lines;
        self
    }

    pub fn with_min_lines(mut self, lines: usize) -> Self {
        self.min_lines_to_compress = lines;
        self
    }

    /// 检查工具是否豁免
    pub fn is_exempt(&self, tool_name: &str) -> bool {
        self.exempt_tools.iter().any(|t| t == tool_name)
    }

    /// 压缩单个工具输出
    fn compress_tool_output(&self, content: &str) -> Option<String> {
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if total_lines <= self.min_lines_to_compress {
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

    /// 检查消息是否为工具输出
    fn is_tool_output(&self, msg: &StarMessage) -> bool {
        msg.role == "tool" && msg.content.is_some()
    }

    /// 从消息中提取工具名称（通过tool_call_id关联）
    fn get_tool_name_for_message(
        &self,
        msg: &StarMessage,
        messages: &[StarMessage],
    ) -> Option<String> {
        // 查找对应的tool_call消息
        if let Some(tool_call_id) = &msg.tool_call_id {
            for m in messages.iter() {
                if m.role == "assistant" {
                    if let Some(tool_calls) = &m.tool_calls {
                        for tc in tool_calls {
                            if &tc.id == tool_call_id {
                                return Some(tc.function.name.clone());
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

#[async_trait]
impl CompactStrategy for ToolOutputCompactStrategy {
    fn name(&self) -> &str {
        "tool_output_compact"
    }

    fn can_apply(&self, messages: &[StarMessage], _token_count: usize) -> bool {
        // 检查是否有大型工具输出（排除豁免工具）
        messages.iter().any(|msg| {
            if self.is_tool_output(msg) {
                // 检查工具是否豁免
                if let Some(tool_name) = self.get_tool_name_for_message(msg, messages) {
                    if self.is_exempt(&tool_name) {
                        return false;
                    }
                }
                if let Some(content) = &msg.content {
                    let line_count = content.lines().count();
                    line_count > self.min_lines_to_compress
                } else {
                    false
                }
            } else {
                false
            }
        })
    }

    fn apply(&self, messages: &[StarMessage]) -> Vec<StarMessage> {
        let mut result = Vec::with_capacity(messages.len());
        let mut changed = false;

        for msg in messages {
            if self.is_tool_output(msg) {
                // 检查工具是否豁免
                if let Some(tool_name) = self.get_tool_name_for_message(msg, messages) {
                    if self.is_exempt(&tool_name) {
                        // 豁免工具不压缩
                        result.push(msg.clone());
                        continue;
                    }
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
            crate::utils::logging::append_debug_log_line(
                "[COMPACT] Applied tool output compression",
            );
        }

        result
    }

    fn priority(&self) -> u32 {
        100 // 高优先级，首先应用
    }
}
