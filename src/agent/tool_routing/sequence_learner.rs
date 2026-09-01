use std::collections::{HashMap, VecDeque};

/// 工具序列学习器
///
/// 记录成功的工具调用序列，用于上下文感知的工具建议
#[derive(Debug, Clone)]
pub struct ToolSequenceLearner {
    /// 成功的工具序列：(工具序列, 使用次数)
    successful_sequences: HashMap<Vec<String>, u32>,
    /// 最近使用的工具（用于上下文）
    recent_tools: VecDeque<String>,
    /// 失败的工具调用：(工具名, 错误类型, 次数)
    failed_tools: HashMap<String, HashMap<String, u32>>,
    /// 最大序列长度
    max_sequence_length: usize,
    /// 最大最近工具数
    max_recent_tools: usize,
}

impl ToolSequenceLearner {
    pub fn new() -> Self {
        Self {
            successful_sequences: HashMap::new(),
            recent_tools: VecDeque::new(),
            failed_tools: HashMap::new(),
            max_sequence_length: 5,
            max_recent_tools: 10,
        }
    }

    /// 记录成功的工具调用
    pub fn record_success(&mut self, tool_name: &str) {
        // 更新最近工具
        self.recent_tools.push_back(tool_name.to_string());
        if self.recent_tools.len() > self.max_recent_tools {
            self.recent_tools.pop_front();
        }

        // 更新序列
        let sequence = self.get_current_sequence();
        if sequence.len() >= 2 {
            *self.successful_sequences.entry(sequence).or_insert(0) += 1;
        }
    }

    /// 记录失败的工具调用
    pub fn record_failure(&mut self, tool_name: &str, error_type: &str) {
        let tool_failures = self
            .failed_tools
            .entry(tool_name.to_string())
            .or_insert_with(HashMap::new);
        *tool_failures.entry(error_type.to_string()).or_insert(0) += 1;
    }

    /// 获取当前序列
    fn get_current_sequence(&self) -> Vec<String> {
        let len = self.recent_tools.len().min(self.max_sequence_length);
        self.recent_tools
            .iter()
            .skip(self.recent_tools.len() - len)
            .cloned()
            .collect()
    }

    /// 基于上下文建议工具
    pub fn suggest_tools(&self, user_input: &str, available_tools: &[String]) -> Vec<String> {
        let mut suggestions = Vec::new();
        let input_lower = user_input.to_lowercase();

        // 1. 基于最近工具序列建议
        if let Some(sequence_suggestions) = self.suggest_from_sequence() {
            for tool in sequence_suggestions {
                if available_tools.contains(&tool) && !suggestions.contains(&tool) {
                    suggestions.push(tool);
                }
            }
        }

        // 2. 基于用户输入关键词建议
        let keyword_suggestions = self.suggest_from_keywords(&input_lower, available_tools);
        for tool in keyword_suggestions {
            if !suggestions.contains(&tool) {
                suggestions.push(tool);
            }
        }

        // 3. 避免失败的工具
        suggestions.retain(|tool| !self.should_avoid_tool(tool, &input_lower));

        suggestions
    }

    /// 基于序列建议
    fn suggest_from_sequence(&self) -> Option<Vec<String>> {
        let current_sequence = self.get_current_sequence();
        if current_sequence.is_empty() {
            return None;
        }

        // 查找以当前序列开头的成功序列
        let mut best_continuation = None;
        let mut best_count = 0;

        for (sequence, count) in &self.successful_sequences {
            if sequence.len() > current_sequence.len() {
                let prefix = &sequence[..current_sequence.len()];
                if prefix == current_sequence.as_slice() && *count > best_count {
                    let next_tool = sequence[current_sequence.len()].clone();
                    best_continuation = Some(next_tool);
                    best_count = *count;
                }
            }
        }

        best_continuation.map(|tool| vec![tool])
    }

    /// 基于关键词建议
    fn suggest_from_keywords(&self, input: &str, available_tools: &[String]) -> Vec<String> {
        let mut suggestions = Vec::new();

        // 关键词到工具的映射
        let keyword_tool_map: HashMap<&str, Vec<&str>> = [
            ("fix", vec!["Grep", "Edit", "get_diagnostics"]),
            ("bug", vec!["Grep", "Edit", "get_diagnostics"]),
            ("error", vec!["Grep", "get_diagnostics"]),
            ("test", vec!["run_tests", "get_diagnostics"]),
            ("create", vec!["Write", "create_file"]),
            ("new", vec!["Write", "create_file"]),
            ("read", vec!["Read", "view_file"]),
            ("find", vec!["Grep", "Glob"]),
            ("Grep", vec!["Grep", "Glob", "SemanticSearch"]),
            ("edit", vec!["Edit", "multi_edit"]),
            ("change", vec!["Edit", "multi_edit"]),
            ("update", vec!["Edit", "multi_edit"]),
            ("delete", vec!["Bash"]),
            ("remove", vec!["Bash"]),
            ("run", vec!["Bash", "run_tests"]),
            ("build", vec!["Bash"]),
            ("compile", vec!["Bash", "get_diagnostics"]),
        ]
        .iter()
        .cloned()
        .collect();

        for (keyword, tools) in &keyword_tool_map {
            if input.contains(keyword) {
                for tool in tools {
                    let tool_string = tool.to_string();
                    if available_tools.contains(&tool_string) && !suggestions.contains(&tool_string)
                    {
                        suggestions.push(tool_string);
                    }
                }
            }
        }

        suggestions
    }

    /// 检查是否应该避免某个工具
    fn should_avoid_tool(&self, tool_name: &str, _context: &str) -> bool {
        if let Some(failures) = self.failed_tools.get(tool_name) {
            // 如果在类似上下文中多次失败，避免使用
            let total_failures: u32 = failures.values().sum();
            if total_failures >= 3 {
                return true;
            }
        }
        false
    }

    /// 清理旧数据
    pub fn cleanup(&mut self) {
        // 保留最近的成功序列
        let threshold = 2;
        self.successful_sequences
            .retain(|_, count| *count >= threshold);

        // 清理旧的失败记录
        self.failed_tools.retain(|_, failures| {
            failures.retain(|_, count| *count >= 2);
            !failures.is_empty()
        });
    }
}
