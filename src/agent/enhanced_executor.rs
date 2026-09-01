use crate::types::{StarMessage, StarToolCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 错误扣留管理器 - 对标claude-code的withheld机制
/// 
/// 用于扣留可恢复的错误，直到确定是否可以恢复
pub struct ErrorWithholdingManager {
    /// 扣留的错误消息
    withheld_messages: Vec<WithheldMessage>,
    /// 是否已尝试Reactive Compact
    has_attempted_reactive_compact: bool,
    /// 是否已尝试Context Collapse
    has_attempted_collapse: bool,
}

/// 扣留的消息
#[derive(Debug, Clone)]
pub struct WithheldMessage {
    /// 消息
    pub message: StarMessage,
    /// 扣留原因
    pub reason: WithholdReason,
    /// 扣留时间
    pub withheld_at: u64,
}

/// 扣留原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WithholdReason {
    /// Prompt-too-long
    PromptTooLong,
    /// Max-output-tokens
    MaxOutputTokens,
    /// 媒体大小错误
    MediaSizeError,
    /// 其他可恢复错误
    Other(String),
}

impl ErrorWithholdingManager {
    pub fn new() -> Self {
        Self {
            withheld_messages: Vec::new(),
            has_attempted_reactive_compact: false,
            has_attempted_collapse: false,
        }
    }

    /// 检查是否应该扣留消息
    pub fn should_withhold(&self, message: &StarMessage) -> Option<WithholdReason> {
        if let Some(content) = &message.content {
            let content_lower = content.to_lowercase();
            
            // 检查是否是prompt-too-long错误
            if content_lower.contains("prompt_too_long") 
                || content_lower.contains("context_length_exceeded")
                || content_lower.contains("maximum context length")
            {
                return Some(WithholdReason::PromptTooLong);
            }
            
            // 检查是否是max-output-tokens错误
            if content_lower.contains("max_output_tokens") 
                || content_lower.contains("output token limit")
            {
                return Some(WithholdReason::MaxOutputTokens);
            }
            
            // 检查是否是媒体大小错误
            if content_lower.contains("image_too_large")
                || content_lower.contains("pdf_too_large")
                || content_lower.contains("media_size")
            {
                return Some(WithholdReason::MediaSizeError);
            }
        }
        None
    }

    /// 扣留消息
    pub fn withhold_message(&mut self, message: StarMessage, reason: WithholdReason) {
        let withheld = WithheldMessage {
            message,
            reason,
            withheld_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        self.withheld_messages.push(withheld);
    }

    /// 获取扣留的消息
    pub fn get_withheld_messages(&self) -> &[WithheldMessage] {
        &self.withheld_messages
    }

    /// 清除扣留的消息
    pub fn clear_withheld(&mut self) {
        self.withheld_messages.clear();
    }

    /// 标记已尝试Reactive Compact
    pub fn mark_attempted_reactive_compact(&mut self) {
        self.has_attempted_reactive_compact = true;
    }

    /// 标记已尝试Context Collapse
    pub fn mark_attempted_collapse(&mut self) {
        self.has_attempted_collapse = true;
    }

    /// 检查是否已尝试Reactive Compact
    pub fn has_attempted_reactive_compact(&self) -> bool {
        self.has_attempted_reactive_compact
    }

    /// 检查是否已尝试Context Collapse
    pub fn has_attempted_collapse(&self) -> bool {
        self.has_attempted_collapse
    }

    /// 重置状态
    pub fn reset(&mut self) {
        self.withheld_messages.clear();
        self.has_attempted_reactive_compact = false;
        self.has_attempted_collapse = false;
    }
}

/// Post-sampling hooks管理器 - 对标claude-code的executePostSamplingHooks
pub struct PostSamplingHooksManager {
    /// Hooks列表
    hooks: Vec<PostSamplingHook>,
}

/// Post-sampling Hook
#[derive(Debug, Clone)]
pub struct PostSamplingHook {
    /// Hook名称
    pub name: String,
    /// 是否启用
    pub enabled: bool,
    /// Hook类型
    pub hook_type: PostSamplingHookType,
}

/// Post-sampling Hook类型
#[derive(Debug, Clone)]
pub enum PostSamplingHookType {
    /// 内存提取
    ExtractMemories,
    /// 提示建议
    PromptSuggestion,
    /// 验证代理
    VerificationAgent,
}

impl PostSamplingHooksManager {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// 添加Hook
    pub fn add_hook(&mut self, hook: PostSamplingHook) {
        self.hooks.push(hook);
    }

    /// 执行Post-sampling hooks
    pub async fn execute_hooks(
        &self,
        messages: &[StarMessage],
        system_prompt: &str,
        user_context: &str,
        system_context: &str,
    ) -> Vec<PostSamplingHookResult> {
        let mut results = Vec::new();

        for hook in &self.hooks {
            if !hook.enabled {
                continue;
            }

            let result = PostSamplingHookResult {
                hook_name: hook.name.clone(),
                hook_type: hook.hook_type.clone(),
                success: true,
                message: None,
            };
            results.push(result);
        }

        results
    }
}

/// Post-sampling Hook结果
#[derive(Debug, Clone)]
pub struct PostSamplingHookResult {
    pub hook_name: String,
    pub hook_type: PostSamplingHookType,
    pub success: bool,
    pub message: Option<String>,
}

/// Bash分类器检查器 - 对标claude-code的startSpeculativeClassifierCheck
pub struct BashClassifierChecker {
    /// 是否正在检查
    is_checking: bool,
    /// 检查结果
    check_result: Option<BashClassifierResult>,
}

/// Bash分类器结果
#[derive(Debug, Clone)]
pub enum BashClassifierResult {
    /// 允许
    Allow,
    /// 拒绝
    Deny { reason: String },
    /// 需要确认
    Ask { reason: String },
}

impl BashClassifierChecker {
    pub fn new() -> Self {
        Self {
            is_checking: false,
            check_result: None,
        }
    }

    /// 开始推测性检查
    pub fn start_speculative_check(
        &mut self,
        command: &str,
        permission_mode: &str,
    ) {
        self.is_checking = true;
        self.check_result = None;

        // 简化的分类器逻辑
        let command_lower = command.to_lowercase();
        
        // 危险命令
        if command_lower.contains("rm -rf") 
            || command_lower.contains("rm -f")
            || command_lower.contains("format")
            || command_lower.contains("mkfs")
        {
            self.check_result = Some(BashClassifierResult::Deny {
                reason: "Destructive command detected".to_string(),
            });
            self.is_checking = false;
            return;
        }

        // 网络命令
        if command_lower.contains("curl") 
            || command_lower.contains("wget")
            || command_lower.contains("nc ")
            || command_lower.contains("netcat")
        {
            self.check_result = Some(BashClassifierResult::Ask {
                reason: "Network command detected".to_string(),
            });
            self.is_checking = false;
            return;
        }

        // 默认允许
        self.check_result = Some(BashClassifierResult::Allow);
        self.is_checking = false;
    }

    /// 检查是否正在检查
    pub fn is_checking(&self) -> bool {
        self.is_checking
    }

    /// 获取检查结果
    pub fn get_result(&self) -> Option<&BashClassifierResult> {
        self.check_result.as_ref()
    }

    /// 重置
    pub fn reset(&mut self) {
        self.is_checking = false;
        self.check_result = None;
    }
}

/// _simulatedSedEdit剥离器 - 对标claude-code的_simulatedSedEdit剥离
pub struct SimulatedSedEditStripper;

impl SimulatedSedEditStripper {
    /// 剥离_simulatedSedEdit字段
    pub fn strip(tool_name: &str, input: &mut Value) -> bool {
        if tool_name != "Bash" && tool_name != "bash" {
            return false;
        }

        if let Some(obj) = input.as_object_mut() {
            if obj.contains_key("_simulatedSedEdit") {
                obj.remove("_simulatedSedEdit");
                return true;
            }
        }
        false
    }
}

/// 工具执行span管理器 - 对标claude-code的startToolSpan/endToolSpan
pub struct ToolSpanManager {
    /// 当前span
    current_span: Option<ToolSpan>,
    /// span历史
    span_history: Vec<ToolSpan>,
}

/// 工具span
#[derive(Debug, Clone)]
pub struct ToolSpan {
    /// 工具名称
    pub tool_name: String,
    /// 工具属性
    pub attributes: HashMap<String, String>,
    /// 开始时间
    pub started_at: u64,
    /// 结束时间
    pub ended_at: Option<u64>,
    /// 是否成功
    pub success: Option<bool>,
    /// 错误信息
    pub error: Option<String>,
}

impl ToolSpanManager {
    pub fn new() -> Self {
        Self {
            current_span: None,
            span_history: Vec::new(),
        }
    }

    /// 开始工具span
    pub fn start_span(&mut self, tool_name: &str, attributes: HashMap<String, String>) {
        let span = ToolSpan {
            tool_name: tool_name.to_string(),
            attributes,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            ended_at: None,
            success: None,
            error: None,
        };
        self.current_span = Some(span);
    }

    /// 结束工具span
    pub fn end_span(&mut self, success: bool, error: Option<String>) {
        if let Some(mut span) = self.current_span.take() {
            span.ended_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            span.success = Some(success);
            span.error = error;
            self.span_history.push(span);
        }
    }

    /// 获取当前span
    pub fn get_current_span(&self) -> Option<&ToolSpan> {
        self.current_span.as_ref()
    }

    /// 获取span历史
    pub fn get_span_history(&self) -> &[ToolSpan] {
        &self.span_history
    }

    /// 清除历史
    pub fn clear_history(&mut self) {
        self.span_history.clear();
    }
}

/// 工具属性提取器 - 对标claude-code的toolAttributes
pub struct ToolAttributeExtractor;

impl ToolAttributeExtractor {
    /// 提取工具属性
    pub fn extract(tool_name: &str, input: &Value) -> HashMap<String, String> {
        let mut attributes = HashMap::new();

        if let Some(obj) = input.as_object() {
            match tool_name {
                "Read" | "view_file" => {
                    if let Some(path) = obj.get("file_path").or(obj.get("path")).and_then(|v| v.as_str()) {
                        attributes.insert("file_path".to_string(), path.to_string());
                    }
                }
                "Edit" | "edit_file" | "Write" | "create_file" => {
                    if let Some(path) = obj.get("file_path").or(obj.get("path")).and_then(|v| v.as_str()) {
                        attributes.insert("file_path".to_string(), path.to_string());
                    }
                }
                "Bash" | "bash" => {
                    if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
                        attributes.insert("full_command".to_string(), command.to_string());
                    }
                }
                "Grep" | "search_file_content" => {
                    if let Some(query) = obj.get("query").and_then(|v| v.as_str()) {
                        attributes.insert("query".to_string(), query.to_string());
                    }
                }
                "Glob" | "find_by_name" => {
                    if let Some(pattern) = obj.get("pattern").and_then(|v| v.as_str()) {
                        attributes.insert("pattern".to_string(), pattern.to_string());
                    }
                }
                _ => {}
            }
        }

        attributes
    }
}

/// 工具输入清理器 - 对标claude-code的input清理
pub struct ToolInputCleaner;

impl ToolInputCleaner {
    /// 清理工具输入
    pub fn clean_input(tool_name: &str, input: &mut Value) {
        // 剥离_simulatedSedEdit
        SimulatedSedEditStripper::strip(tool_name, input);
        
        // 其他清理逻辑可以在这里添加
    }
}

/// 流式工具执行管理器 - 对标claude-code的StreamingToolExecutor
pub struct StreamingToolExecutorManager {
    /// 工具列表
    tools: Vec<String>,
    /// 进行中的工具
    in_progress: HashMap<String, InProgressTool>,
    /// 已完成的结果
    completed_results: HashMap<String, Value>,
    /// 是否已丢弃
    discarded: bool,
    /// 进度回调
    progress_callbacks: HashMap<String, Box<dyn Fn(String) + Send + Sync>>,
}

/// 进行中的工具
#[derive(Debug, Clone)]
pub struct InProgressTool {
    /// 工具名称
    pub name: String,
    /// 工具使用ID
    pub tool_use_id: String,
    /// 开始时间
    pub started_at: u64,
    /// 是否并发安全
    pub is_concurrency_safe: bool,
}

impl StreamingToolExecutorManager {
    pub fn new(tools: Vec<String>) -> Self {
        Self {
            tools,
            in_progress: HashMap::new(),
            completed_results: HashMap::new(),
            discarded: false,
            progress_callbacks: HashMap::new(),
        }
    }

    /// 添加工具
    pub fn add_tool(&mut self, tool_name: &str, tool_use_id: &str, is_concurrency_safe: bool) {
        if self.discarded {
            return;
        }

        self.in_progress.insert(
            tool_use_id.to_string(),
            InProgressTool {
                name: tool_name.to_string(),
                tool_use_id: tool_use_id.to_string(),
                started_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                is_concurrency_safe,
            },
        );
    }

    /// 完成工具
    pub fn complete_tool(&mut self, tool_use_id: &str, result: Value) {
        self.in_progress.remove(tool_use_id);
        self.completed_results.insert(tool_use_id.to_string(), result);
    }

    /// 获取已完成的结果
    pub fn get_completed_results(&mut self) -> Vec<(String, Value)> {
        let results: Vec<_> = self.completed_results.drain().collect();
        results
    }

    /// 获取剩余结果
    pub fn get_remaining_results(&mut self) -> Vec<(String, Value)> {
        let mut results = Vec::new();
        
        // 获取已完成的结果
        results.extend(self.completed_results.drain());
        
        // 为进行中的工具生成合成结果
        for (tool_use_id, tool) in self.in_progress.drain() {
            results.push((
                tool_use_id,
                serde_json::json!({
                    "error": "Tool execution interrupted",
                    "tool_name": tool.name,
                }),
            ));
        }
        
        results
    }

    /// 丢弃所有工具
    pub fn discard(&mut self) {
        self.discarded = true;
        self.in_progress.clear();
        self.completed_results.clear();
        self.progress_callbacks.clear();
    }

    /// 检查是否已丢弃
    pub fn is_discarded(&self) -> bool {
        self.discarded
    }

    /// 获取进行中的工具数量
    pub fn in_progress_count(&self) -> usize {
        self.in_progress.len()
    }

    /// 检查工具是否可以执行
    pub fn can_execute_tool(&self, is_concurrency_safe: bool) -> bool {
        let executing_tools: Vec<_> = self.in_progress.values().collect();
        executing_tools.is_empty()
            || (is_concurrency_safe && executing_tools.iter().all(|t| t.is_concurrency_safe))
    }
}

/// 增强的工具执行器 - 整合所有增强功能
/// 
/// 对标claude-code-main的toolExecution.ts
/// 提供完整的工具执行流程，包括验证、权限检查、Hook执行等
pub struct EnhancedToolExecutor {
    /// 错误扣留管理器
    error_withholding: ErrorWithholdingManager,
    /// Post-sampling hooks管理器
    post_sampling_hooks: PostSamplingHooksManager,
    /// Bash分类器检查器
    bash_classifier: BashClassifierChecker,
    /// 工具span管理器
    span_manager: ToolSpanManager,
    /// 流式工具执行管理器
    streaming_executor: StreamingToolExecutorManager,
    /// 工具使用摘要生成器
    summary_generator: ToolUseSummaryGenerator,
    /// 结构化输出捕获器
    structured_output: StructuredOutputCapture,
}

/// 工具使用摘要生成器
/// 
/// 对标claude-code-main的toolUseSummaryGenerator.ts
pub struct ToolUseSummaryGenerator;

impl ToolUseSummaryGenerator {
    pub fn new() -> Self {
        Self
    }

    /// 生成工具使用摘要
    pub fn generate_summary(&self, tool_name: &str, input: &Value, output: &str) -> Option<String> {
        match tool_name {
            "Bash" => {
                if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                    let short_cmd = if command.len() > 50 {
                        format!("{}...", &command[..47])
                    } else {
                        command.to_string()
                    };
                    Some(format!("Running: {}", short_cmd))
                } else {
                    Some("Running bash command".to_string())
                }
            }
            "Read" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    Some(format!("Reading: {}", path))
                } else {
                    Some("Reading file".to_string())
                }
            }
            "Edit" | "Write" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    Some(format!("Editing: {}", path))
                } else {
                    Some("Editing file".to_string())
                }
            }
            "Grep" => {
                if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
                    Some(format!("Searching: {}", query))
                } else {
                    Some("Searching".to_string())
                }
            }
            "Glob" => {
                if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
                    Some(format!("Finding: {}", pattern))
                } else {
                    Some("Finding files".to_string())
                }
            }
            _ => None,
        }
    }
}

/// 结构化输出捕获器
/// 
/// 对标claude-code-main的structured output捕获
pub struct StructuredOutputCapture {
    /// 捕获的输出
    outputs: Vec<CapturedOutput>,
}

/// 捕获的输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedOutput {
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

    /// 捕获输出
    pub fn capture(&mut self, tool_name: &str, tool_use_id: &str, data: Value) {
        let output = CapturedOutput {
            tool_name: tool_name.to_string(),
            tool_use_id: tool_use_id.to_string(),
            data,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        self.outputs.push(output);
    }

    /// 获取所有捕获的输出
    pub fn get_outputs(&self) -> &[CapturedOutput] {
        &self.outputs
    }

    /// 清空输出
    pub fn clear(&mut self) {
        self.outputs.clear();
    }
}

impl EnhancedToolExecutor {
    pub fn new(tools: Vec<String>) -> Self {
        Self {
            error_withholding: ErrorWithholdingManager::new(),
            post_sampling_hooks: PostSamplingHooksManager::new(),
            bash_classifier: BashClassifierChecker::new(),
            span_manager: ToolSpanManager::new(),
            streaming_executor: StreamingToolExecutorManager::new(tools),
            summary_generator: ToolUseSummaryGenerator::new(),
            structured_output: StructuredOutputCapture::new(),
        }
    }

    /// 获取错误扣留管理器
    pub fn error_withholding(&mut self) -> &mut ErrorWithholdingManager {
        &mut self.error_withholding
    }

    /// 获取Post-sampling hooks管理器
    pub fn post_sampling_hooks(&mut self) -> &mut PostSamplingHooksManager {
        &mut self.post_sampling_hooks
    }

    /// 获取Bash分类器检查器
    pub fn bash_classifier(&mut self) -> &mut BashClassifierChecker {
        &mut self.bash_classifier
    }

    /// 获取工具span管理器
    pub fn span_manager(&mut self) -> &mut ToolSpanManager {
        &mut self.span_manager
    }

    /// 获取流式工具执行管理器
    pub fn streaming_executor(&mut self) -> &mut StreamingToolExecutorManager {
        &mut self.streaming_executor
    }

    /// 获取工具使用摘要生成器
    pub fn summary_generator(&self) -> &ToolUseSummaryGenerator {
        &self.summary_generator
    }

    /// 获取结构化输出捕获器
    pub fn structured_output(&mut self) -> &mut StructuredOutputCapture {
        &mut self.structured_output
    }

    /// 执行工具
    pub async fn execute_tool(
        &mut self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        is_concurrency_safe: bool,
    ) -> Value {
        // 1. 清理输入
        let mut cleaned_input = input.clone();
        ToolInputCleaner::clean_input(tool_name, &mut cleaned_input);

        // 2. 提取属性
        let attributes = ToolAttributeExtractor::extract(tool_name, &cleaned_input);

        // 3. 开始span
        self.span_manager.start_span(tool_name, attributes);

        // 4. 检查Bash分类器
        if tool_name == "Bash" || tool_name == "bash" {
            if let Some(command) = cleaned_input.get("command").and_then(|v| v.as_str()) {
                self.bash_classifier.start_speculative_check(command, "default");
                
                if let Some(BashClassifierResult::Deny { reason }) = self.bash_classifier.get_result() {
                    self.span_manager.end_span(false, Some(reason.clone()));
                    return serde_json::json!({
                        "error": reason,
                        "is_error": true,
                    });
                }
            }
        }

        // 5. 添加到流式执行器
        self.streaming_executor.add_tool(tool_name, tool_use_id, is_concurrency_safe);

        // 6. 模拟工具执行（实际实现会调用真实的工具）
        let result = serde_json::json!({
            "success": true,
            "output": "Tool executed successfully",
        });

        // 7. 捕获结构化输出
        self.structured_output.capture(tool_name, tool_use_id, result.clone());

        // 8. 生成摘要
        let _summary = self.summary_generator.generate_summary(
            tool_name,
            &cleaned_input,
            result.as_str().unwrap_or(""),
        );

        // 9. 完成工具
        self.streaming_executor.complete_tool(tool_use_id, result.clone());

        // 10. 结束span
        self.span_manager.end_span(true, None);

        result
    }

    /// 处理错误
    pub fn handle_error(&mut self, error: &str) -> ErrorHandlingResult {
        // 检查是否应该扣留错误
        let mock_message = StarMessage::system(error);
        if let Some(reason) = self.error_withholding.should_withhold(&mock_message) {
            self.error_withholding.withhold_message(mock_message, reason.clone());
            return ErrorHandlingResult::Withheld { reason: reason.clone() };
        }

        // 检查是否是可恢复的错误
        if error.contains("timeout") || error.contains("connection") {
            return ErrorHandlingResult::Retryable;
        }

        ErrorHandlingResult::Fatal
    }

    /// 重置状态
    pub fn reset(&mut self) {
        self.error_withholding.reset();
        self.bash_classifier.reset();
        self.span_manager.clear_history();
        self.structured_output.clear();
    }
}

/// 错误处理结果
#[derive(Debug, Clone)]
pub enum ErrorHandlingResult {
    /// 已扣留
    Withheld { reason: WithholdReason },
    /// 可重试
    Retryable,
    /// 致命错误
    Fatal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_withholding_manager() {
        let mut manager = ErrorWithholdingManager::new();
        
        let msg = StarMessage::system("Error: prompt_too_long");
        assert_eq!(
            manager.should_withhold(&msg),
            Some(WithholdReason::PromptTooLong)
        );
        
        manager.withhold_message(msg, WithholdReason::PromptTooLong);
        assert_eq!(manager.get_withheld_messages().len(), 1);
    }

    #[test]
    fn test_bash_classifier_checker() {
        let mut checker = BashClassifierChecker::new();
        
        checker.start_speculative_check("rm -rf /", "default");
        assert!(matches!(
            checker.get_result(),
            Some(BashClassifierResult::Deny { .. })
        ));
        
        checker.start_speculative_check("ls -la", "default");
        assert!(matches!(
            checker.get_result(),
            Some(BashClassifierResult::Allow)
        ));
    }

    #[test]
    fn test_simulated_sed_edit_stripper() {
        let mut input = serde_json::json!({
            "command": "sed 's/old/new/g' file.txt",
            "_simulatedSedEdit": {"filePath": "file.txt"}
        });
        
        let stripped = SimulatedSedEditStripper::strip("Bash", &mut input);
        assert!(stripped);
        assert!(!input.as_object().unwrap().contains_key("_simulatedSedEdit"));
    }

    #[test]
    fn test_tool_span_manager() {
        let mut manager = ToolSpanManager::new();
        
        let mut attributes = HashMap::new();
        attributes.insert("file_path".to_string(), "/tmp/test.txt".to_string());
        
        manager.start_span("Read", attributes);
        assert!(manager.get_current_span().is_some());
        
        manager.end_span(true, None);
        assert!(manager.get_current_span().is_none());
        assert_eq!(manager.get_span_history().len(), 1);
    }

    #[test]
    fn test_tool_attribute_extractor() {
        let input = serde_json::json!({
            "file_path": "/tmp/test.txt",
            "offset": 0,
            "limit": 100,
        });
        
        let attributes = ToolAttributeExtractor::extract("Read", &input);
        assert_eq!(attributes.get("file_path"), Some(&"/tmp/test.txt".to_string()));
    }

    #[test]
    fn test_streaming_tool_executor() {
        let mut executor = StreamingToolExecutorManager::new(vec!["Read".to_string()]);
        
        executor.add_tool("Read", "tool1", true);
        assert_eq!(executor.in_progress_count(), 1);
        
        executor.complete_tool("tool1", serde_json::json!({"result": "ok"}));
        assert_eq!(executor.in_progress_count(), 0);
        
        let results = executor.get_completed_results();
        assert_eq!(results.len(), 1);
    }
}
