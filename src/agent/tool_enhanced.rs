use crate::types::{StarMessage, StarToolCall};
use serde_json::Value;
use std::collections::HashMap;

/// 工具使用摘要生成器 - 对标claude-code的generateToolUseSummary
pub struct ToolUseSummaryGenerator;

impl ToolUseSummaryGenerator {
    /// 生成工具使用摘要
    pub fn generate_summary(
        tool_name: &str,
        input: &Value,
        output: Option<&str>,
    ) -> Option<String> {
        let summary = match tool_name {
            "Bash" | "bash" => {
                if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                    let short_cmd = if command.len() > 50 {
                        format!("{}...", &command[..47])
                    } else {
                        command.to_string()
                    };
                    format!("Running: {}", short_cmd)
                } else {
                    "Running bash command".to_string()
                }
            }
            "Read" | "view_file" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    format!("Reading: {}", path)
                } else {
                    "Reading file".to_string()
                }
            }
            "Edit" | "edit_file" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    format!("Editing: {}", path)
                } else {
                    "Editing file".to_string()
                }
            }
            "Write" | "create_file" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    format!("Writing: {}", path)
                } else {
                    "Writing file".to_string()
                }
            }
            "Grep" | "search_file_content" => {
                if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
                    format!("Searching: {}", query)
                } else {
                    "Searching".to_string()
                }
            }
            "Glob" | "find_by_name" => {
                if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
                    format!("Finding: {}", pattern)
                } else {
                    "Finding files".to_string()
                }
            }
            "ListDir" => {
                if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                    format!("Listing: {}", path)
                } else {
                    "Listing directory".to_string()
                }
            }
            _ => return None,
        };

        Some(summary)
    }

    /// 生成工具使用摘要消息
    pub fn generate_summary_message(
        tool_name: &str,
        input: &Value,
        output: Option<&str>,
        tool_use_id: &str,
    ) -> Option<StarMessage> {
        let summary = Self::generate_summary(tool_name, input, output)?;
        Some(StarMessage::system(&format!(
            "[Tool Summary] {}: {}",
            tool_name, summary
        )))
    }
}

/// 工具内容事件记录器 - 对标claude-code的addToolContentEvent
pub struct ToolContentEventRecorder {
    /// 事件缓冲区
    events: Vec<ToolContentEvent>,
}

/// 工具内容事件
#[derive(Debug, Clone)]
pub struct ToolContentEvent {
    /// 事件类型
    pub event_type: String,
    /// 工具名称
    pub tool_name: String,
    /// 属性
    pub attributes: HashMap<String, String>,
    /// 时间戳
    pub timestamp: u64,
}

impl ToolContentEventRecorder {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
        }
    }

    /// 记录工具内容事件
    pub fn record_event(
        &mut self,
        tool_name: &str,
        input: &Value,
        output: &Value,
    ) {
        let mut attributes = HashMap::new();

        match tool_name {
            "Read" | "view_file" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    attributes.insert("file_path".to_string(), path.to_string());
                }
                if let Some(content) = output.get("content").and_then(|v| v.as_str()) {
                    attributes.insert("content_length".to_string(), content.len().to_string());
                }
            }
            "Edit" | "edit_file" | "Write" | "create_file" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    attributes.insert("file_path".to_string(), path.to_string());
                }
                if let Some(diff) = output.get("diff").and_then(|v| v.as_str()) {
                    attributes.insert("diff_length".to_string(), diff.len().to_string());
                }
            }
            "Bash" | "bash" => {
                if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                    attributes.insert("bash_command".to_string(), command.to_string());
                }
                if let Some(output_str) = output.get("output").and_then(|v| v.as_str()) {
                    attributes.insert("output_length".to_string(), output_str.len().to_string());
                }
            }
            _ => {}
        }

        if !attributes.is_empty() {
            self.events.push(ToolContentEvent {
                event_type: "tool.output".to_string(),
                tool_name: tool_name.to_string(),
                attributes,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }
    }

    /// 获取所有事件
    pub fn get_events(&self) -> &[ToolContentEvent] {
        &self.events
    }

    /// 清除事件
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

/// 结构化输出捕获器 - 对标claude-code的structured_output
pub struct StructuredOutputCapture {
    /// 捕获的输出
    outputs: Vec<StructuredOutput>,
}

/// 结构化输出
#[derive(Debug, Clone)]
pub struct StructuredOutput {
    /// 工具名称
    pub tool_name: String,
    /// 工具使用ID
    pub tool_use_id: String,
    /// 输出数据
    pub data: Value,
    /// 时间戳
    pub timestamp: u64,
}

impl StructuredOutputCapture {
    pub fn new() -> Self {
        Self {
            outputs: Vec::new(),
        }
    }

    /// 捕获结构化输出
    pub fn capture(&mut self, tool_name: &str, tool_use_id: &str, output: &Value) {
        if let Some(structured) = output.get("structured_output") {
            self.outputs.push(StructuredOutput {
                tool_name: tool_name.to_string(),
                tool_use_id: tool_use_id.to_string(),
                data: structured.clone(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }
    }

    /// 获取所有捕获的输出
    pub fn get_outputs(&self) -> &[StructuredOutput] {
        &self.outputs
    }

    /// 清除输出
    pub fn clear(&mut self) {
        self.outputs.clear();
    }
}

/// Langfuse观察记录器 - 对标claude-code的recordToolObservation
pub struct LangfuseObserver {
    /// 是否启用
    enabled: bool,
    /// 观察记录
    observations: Vec<ToolObservation>,
}

/// 工具观察
#[derive(Debug, Clone)]
pub struct ToolObservation {
    /// 工具名称
    pub tool_name: String,
    /// 工具使用ID
    pub tool_use_id: String,
    /// 输入
    pub input: String,
    /// 输出
    pub output: String,
    /// 开始时间
    pub start_time: u64,
    /// 是否错误
    pub is_error: bool,
}

impl LangfuseObserver {
    pub fn new() -> Self {
        Self {
            enabled: std::env::var("LANGFUSE_SECRET_KEY").is_ok(),
            observations: Vec::new(),
        }
    }

    /// 记录工具观察
    pub fn record_observation(
        &mut self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        output: &str,
        is_error: bool,
    ) {
        if !self.enabled {
            return;
        }

        self.observations.push(ToolObservation {
            tool_name: tool_name.to_string(),
            tool_use_id: tool_use_id.to_string(),
            input: serde_json::to_string(input).unwrap_or_default(),
            output: output.to_string(),
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            is_error,
        });
    }

    /// 获取所有观察
    pub fn get_observations(&self) -> &[ToolObservation] {
        &self.observations
    }

    /// 清除观察
    pub fn clear(&mut self) {
        self.observations.clear();
    }

    /// 刷新（发送到Langfuse）
    pub async fn flush(&self) {
        // 实际实现会发送到Langfuse服务器
        // 这里只是占位
    }
}

/// 工具结果大小计算器 - 对标claude-code的toolResultSizeBytes
pub struct ToolResultSizeCalculator;

impl ToolResultSizeCalculator {
    /// 计算工具结果大小
    pub fn calculate(content: &Value) -> usize {
        match content {
            Value::String(s) => s.len(),
            Value::Array(arr) => {
                let mut size = 0;
                for item in arr {
                    size += Self::calculate(item);
                }
                size
            }
            Value::Object(obj) => {
                let mut size = 0;
                for (key, value) in obj {
                    size += key.len();
                    size += Self::calculate(value);
                }
                size
            }
            _ => 0,
        }
    }

    /// 计算工具结果块大小
    pub fn calculate_block(content: &str) -> usize {
        content.len()
    }
}

/// Git commit ID提取器 - 对标claude-code的parseGitCommitId
pub struct GitCommitIdParser;

impl GitCommitIdParser {
    /// 从git commit输出中提取commit ID
    pub fn parse(output: &str) -> Option<String> {
        // 检查是否是git commit输出
        if !output.contains("[") || !output.contains("]") {
            return None;
        }

        // 查找commit ID模式
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("[") && trimmed.contains("]") {
                // 提取commit ID
                let start = trimmed.find('[')? + 1;
                let end = trimmed.find(']')?;
                if start < end {
                    let commit_id = &trimmed[start..end];
                    // 验证是否是有效的commit ID（40个十六进制字符）
                    if commit_id.len() >= 7 && commit_id.len() <= 40 && commit_id.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Some(commit_id.to_string());
                    }
                }
            }
        }

        // 查找其他模式
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("commit ") || trimmed.starts_with("Commit ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    let commit_id = parts[1];
                    if commit_id.len() >= 7 && commit_id.len() <= 40 && commit_id.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Some(commit_id.to_string());
                    }
                }
            }
        }

        None
    }
}

/// OTLP日志记录器 - 对标claude-code的logOTelEvent
pub struct OTLPLogger {
    /// 是否启用
    enabled: bool,
    /// 日志缓冲区
    logs: Vec<OTLPLogEntry>,
}

/// OTLP日志条目
#[derive(Debug, Clone)]
pub struct OTLPLogEntry {
    /// 事件名称
    pub event_name: String,
    /// 属性
    pub attributes: HashMap<String, String>,
    /// 时间戳
    pub timestamp: u64,
}

impl OTLPLogger {
    pub fn new() -> Self {
        Self {
            enabled: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok(),
            logs: Vec::new(),
        }
    }

    /// 记录事件
    pub fn log_event(&mut self, event_name: &str, attributes: HashMap<String, String>) {
        if !self.enabled {
            return;
        }

        self.logs.push(OTLPLogEntry {
            event_name: event_name.to_string(),
            attributes,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
    }

    /// 记录工具结果事件
    pub fn log_tool_result(
        &mut self,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
        tool_result_size_bytes: usize,
        decision_source: Option<&str>,
        decision_type: Option<&str>,
    ) {
        let mut attributes = HashMap::new();
        attributes.insert("tool_name".to_string(), tool_name.to_string());
        attributes.insert("success".to_string(), success.to_string());
        attributes.insert("duration_ms".to_string(), duration_ms.to_string());
        attributes.insert("tool_result_size_bytes".to_string(), tool_result_size_bytes.to_string());
        
        if let Some(source) = decision_source {
            attributes.insert("decision_source".to_string(), source.to_string());
        }
        if let Some(decision) = decision_type {
            attributes.insert("decision_type".to_string(), decision.to_string());
        }

        self.log_event("tool_result", attributes);
    }

    /// 获取所有日志
    pub fn get_logs(&self) -> &[OTLPLogEntry] {
        &self.logs
    }

    /// 清除日志
    pub fn clear(&mut self) {
        self.logs.clear();
    }

    /// 刷新（发送到OTLP服务器）
    pub async fn flush(&self) {
        // 实际实现会发送到OTLP服务器
        // 这里只是占位
    }
}

/// 工具执行增强器 - 整合所有增强功能
pub struct ToolExecutionEnhancer {
    /// 工具使用摘要生成器
    summary_generator: ToolUseSummaryGenerator,
    /// 工具内容事件记录器
    content_recorder: ToolContentEventRecorder,
    /// 结构化输出捕获器
    output_capture: StructuredOutputCapture,
    /// Langfuse观察记录器
    langfuse_observer: LangfuseObserver,
    /// 工具结果大小计算器
    size_calculator: ToolResultSizeCalculator,
    /// Git commit ID提取器
    commit_parser: GitCommitIdParser,
    /// OTLP日志记录器
    otlp_logger: OTLPLogger,
}

impl ToolExecutionEnhancer {
    pub fn new() -> Self {
        Self {
            summary_generator: ToolUseSummaryGenerator,
            content_recorder: ToolContentEventRecorder::new(),
            output_capture: StructuredOutputCapture::new(),
            langfuse_observer: LangfuseObserver::new(),
            size_calculator: ToolResultSizeCalculator,
            commit_parser: GitCommitIdParser,
            otlp_logger: OTLPLogger::new(),
        }
    }

    /// 处理工具执行结果
    pub fn process_tool_result(
        &mut self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        output: &Value,
        success: bool,
        duration_ms: u64,
    ) {
        // 1. 记录工具内容事件
        self.content_recorder.record_event(tool_name, input, output);

        // 2. 捕获结构化输出
        self.output_capture.capture(tool_name, tool_use_id, output);

        // 3. 记录Langfuse观察
        let output_str = serde_json::to_string(output).unwrap_or_default();
        self.langfuse_observer.record_observation(
            tool_name,
            tool_use_id,
            input,
            &output_str,
            !success,
        );

        // 4. 计算工具结果大小
        let result_size = ToolResultSizeCalculator::calculate(output);

        // 5. 提取Git commit ID（如果是git commit）
        let mut git_commit_id = None;
        if tool_name == "Bash" || tool_name == "bash" {
            if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                if command.contains("git commit") {
                    if let Some(output_str) = output.get("output").and_then(|v| v.as_str()) {
                        git_commit_id = GitCommitIdParser::parse(output_str);
                    }
                }
            }
        }

        // 6. 记录OTLP日志
        self.otlp_logger.log_tool_result(
            tool_name,
            success,
            duration_ms,
            result_size,
            None,
            None,
        );
    }

    /// 获取工具使用摘要
    pub fn get_tool_summary(&self, tool_name: &str, input: &Value, output: Option<&str>) -> Option<String> {
        ToolUseSummaryGenerator::generate_summary(tool_name, input, output)
    }

    /// 获取内容事件
    pub fn get_content_events(&self) -> &[ToolContentEvent] {
        self.content_recorder.get_events()
    }

    /// 获取结构化输出
    pub fn get_structured_outputs(&self) -> &[StructuredOutput] {
        self.output_capture.get_outputs()
    }

    /// 获取Langfuse观察
    pub fn get_langfuse_observations(&self) -> &[ToolObservation] {
        self.langfuse_observer.get_observations()
    }

    /// 获取OTLP日志
    pub fn get_otlp_logs(&self) -> &[OTLPLogEntry] {
        self.otlp_logger.get_logs()
    }

    /// 刷新所有缓冲区
    pub async fn flush_all(&self) {
        self.langfuse_observer.flush().await;
        self.otlp_logger.flush().await;
    }

    /// 清除所有缓冲区
    pub fn clear_all(&mut self) {
        self.content_recorder.clear();
        self.output_capture.clear();
        self.langfuse_observer.clear();
        self.otlp_logger.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_use_summary_generator() {
        let input = serde_json::json!({
            "file_path": "/tmp/test.txt"
        });
        
        let summary = ToolUseSummaryGenerator::generate_summary("Read", &input, None);
        assert_eq!(summary, Some("Reading: /tmp/test.txt".to_string()));
    }

    #[test]
    fn test_tool_content_event_recorder() {
        let mut recorder = ToolContentEventRecorder::new();
        let input = serde_json::json!({"file_path": "/tmp/test.txt"});
        let output = serde_json::json!({"content": "test content"});
        
        recorder.record_event("Read", &input, &output);
        assert_eq!(recorder.get_events().len(), 1);
    }

    #[test]
    fn test_structured_output_capture() {
        let mut capture = StructuredOutputCapture::new();
        let output = serde_json::json!({
            "structured_output": {"key": "value"}
        });
        
        capture.capture("test_tool", "tool1", &output);
        assert_eq!(capture.get_outputs().len(), 1);
    }

    #[test]
    fn test_tool_result_size_calculator() {
        let content = serde_json::json!({
            "key": "value",
            "nested": {"a": "b"}
        });
        
        let size = ToolResultSizeCalculator::calculate(&content);
        assert!(size > 0);
    }

    #[test]
    fn test_git_commit_id_parser() {
        let output = "[abc1234] feat: add new feature";
        let commit_id = GitCommitIdParser::parse(output);
        assert_eq!(commit_id, Some("abc1234".to_string()));
    }

    #[test]
    fn test_otlp_logger() {
        let mut logger = OTLPLogger::new();
        let mut attributes = HashMap::new();
        attributes.insert("key".to_string(), "value".to_string());
        
        logger.log_event("test_event", attributes);
        assert_eq!(logger.get_logs().len(), 1);
    }
}
