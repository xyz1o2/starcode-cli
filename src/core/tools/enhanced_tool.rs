use crate::types::{StarToolCall, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// 工具验证结果
#[derive(Debug, Clone)]
pub enum ToolValidationResult {
    /// 验证通过
    Valid,
    /// 验证失败
    Invalid {
        message: String,
        error_code: Option<i32>,
    },
}

/// 工具权限结果
#[derive(Debug, Clone)]
pub enum ToolPermissionResult {
    /// 允许执行
    Allow,
    /// 需要用户确认
    Ask { message: String },
    /// 拒绝执行
    Deny { message: String },
}

/// 工具中断行为
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptBehavior {
    /// 取消工具执行
    Cancel,
    /// 阻塞等待完成
    Block,
}

/// 工具搜索/读取类型
#[derive(Debug, Clone, Default)]
pub struct ToolSearchReadType {
    /// 是否是搜索操作
    pub is_search: bool,
    /// 是否是读取操作
    pub is_read: bool,
    /// 是否是列表操作
    pub is_list: bool,
}

/// 工具活动描述
#[derive(Debug, Clone)]
pub struct ToolActivityDescription {
    /// 活动描述
    pub description: String,
    /// 目标文件/资源
    pub target: Option<String>,
}

/// 增强的Tool trait - 对标claude-code的Tool接口
pub trait EnhancedTool: Send + Sync {
    /// 工具名称
    fn name(&self) -> &str;

    /// 工具别名
    fn aliases(&self) -> Vec<&str> {
        Vec::new()
    }

    /// 搜索提示（用于延迟工具发现）
    fn search_hint(&self) -> Option<&str> {
        None
    }

    /// 是否是只读工具
    fn is_read_only(&self) -> bool;

    /// 是否是破坏性工具
    fn is_destructive(&self) -> bool {
        false
    }

    /// 是否并发安全（根据输入判断）
    ///
    /// 对标claude-code-main的Tool.isConcurrencySafe
    /// 默认只读工具是并发安全的，但可以根据输入进一步判断
    fn is_concurrency_safe(&self, input: &Value) -> bool {
        // 默认：只读工具是并发安全的
        self.is_read_only()
    }

    /// 检查工具是否可以并发执行
    ///
    /// 对标claude-code-main的StreamingToolExecutor.canExecuteTool
    fn can_execute_concurrently(&self, input: &Value, other_tools_running: bool) -> bool {
        // 如果没有其他工具在运行，总是可以执行
        if !other_tools_running {
            return true;
        }

        // 如果有其他工具在运行，只有并发安全的工具可以执行
        self.is_concurrency_safe(input)
    }

    /// 获取并发组ID
    ///
    /// 用于将并发安全的工具分组执行
    fn concurrency_group_id(&self, input: &Value) -> Option<String> {
        if self.is_concurrency_safe(input) {
            Some("read_only".to_string())
        } else {
            None // 非并发安全工具单独执行
        }
    }

    /// 中断行为
    fn interrupt_behavior(&self) -> InterruptBehavior {
        InterruptBehavior::Block
    }

    /// 是否是搜索或读取命令
    fn is_search_or_read_command(&self, input: &Value) -> ToolSearchReadType {
        ToolSearchReadType::default()
    }

    /// 最大结果大小（字符数）
    fn max_result_size_chars(&self) -> usize {
        50_000
    }

    /// 是否延迟加载
    fn should_defer(&self) -> bool {
        false
    }

    /// 是否总是加载
    fn always_load(&self) -> bool {
        false
    }

    /// 是否是MCP工具
    fn is_mcp(&self) -> bool {
        false
    }

    /// 验证输入
    fn validate_input(&self, input: &Value) -> ToolValidationResult {
        ToolValidationResult::Valid
    }

    /// 检查权限
    fn check_permissions(&self, input: &Value) -> ToolPermissionResult {
        ToolPermissionResult::Allow
    }

    /// 准备权限匹配器
    fn prepare_permission_matcher(&self, input: &Value) -> Option<Box<dyn Fn(&str) -> bool>> {
        None
    }

    /// 回填可观测输入
    fn backfill_observable_input(&self, input: &mut Value) {
        // 默认不做任何修改
    }

    /// 获取活动描述
    fn get_activity_description(&self, input: &Value) -> Option<ToolActivityDescription> {
        None
    }

    /// 获取工具使用摘要
    fn get_tool_use_summary(&self, input: &Value) -> Option<String> {
        None
    }

    /// 转换为分类器输入
    fn to_auto_classifier_input(&self, input: &Value) -> Option<String> {
        None
    }

    /// 执行工具
    async fn call(
        &self,
        input: Value,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
        progress_callback: Option<Box<dyn Fn(String) + Send + Sync>>,
    ) -> ToolResult;

    /// 创建调用
    fn create_invocation(&self, args: Value) -> Result<Box<dyn ToolInvocation>, String>;
}

/// 工具调用trait
#[async_trait]
pub trait ToolInvocation: Send + Sync {
    /// 执行工具
    async fn execute(
        &self,
        abort_signal: Option<&tokio_util::sync::CancellationToken>,
        progress_callback: Option<Box<dyn Fn(String) + Send + Sync>>,
    ) -> Result<ToolExecutionResult, Box<dyn std::error::Error + Send + Sync>>;

    /// 是否需要确认
    async fn should_confirm_execute(
        &self,
        abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Option<ToolConfirmationDetails>, Box<dyn std::error::Error + Send + Sync>>;

    /// 归一化确认结果
    fn normalize_confirmation_outcome(
        &self,
        outcome: crate::types::ToolConfirmationOutcome,
    ) -> crate::types::ToolConfirmationOutcome {
        outcome
    }
}

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    /// 输出内容
    pub output: String,
    /// 返回显示内容
    pub return_display: Option<String>,
    /// 错误信息
    pub error: Option<ToolError>,
    /// 附加数据
    pub data: Option<Value>,
}

/// 工具错误
#[derive(Debug, Clone)]
pub struct ToolError {
    /// 错误消息
    pub message: String,
    /// 错误代码
    pub code: Option<i32>,
}

/// 工具确认详情
pub struct ToolConfirmationDetails {
    /// 标题
    pub title: String,
    /// 提示信息
    pub prompt: String,
    /// 确认回调
    pub on_confirm: Box<dyn Fn(crate::types::ToolConfirmationOutcome) + Send + Sync>,
}

impl std::fmt::Debug for ToolConfirmationDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolConfirmationDetails")
            .field("title", &self.title)
            .field("prompt", &self.prompt)
            .field("on_confirm", &"<closure>")
            .finish()
    }
}

/// 工具权限身份
pub struct ToolPermissionIdentity {
    /// 工具名称
    pub tool_name: String,
    /// 输入签名
    pub input_signature: String,
}

/// 工具结果处理trait
pub trait ToolResultProcessor {
    /// 处理工具结果
    fn process_result(&self, result: &ToolResult) -> ProcessedToolResult;

    /// 生成预览
    fn generate_preview(&self, content: &str, max_bytes: usize) -> (String, bool);
}

/// 处理后的工具结果
#[derive(Debug, Clone)]
pub struct ProcessedToolResult {
    /// 原始内容
    pub original_content: String,
    /// 处理后的内容
    pub processed_content: String,
    /// 是否被截断
    pub is_truncated: bool,
    /// 是否被持久化
    pub is_persisted: bool,
    /// 持久化文件路径
    pub persisted_path: Option<String>,
}

/// 工具使用摘要生成器
pub struct ToolUseSummaryGenerator;

impl ToolUseSummaryGenerator {
    /// 生成工具使用摘要
    pub fn generate_summary(tool_name: &str, input: &Value, output: &str) -> Option<String> {
        // 根据工具类型生成摘要（使用starcode-cli中的实际工具名称）
        match tool_name {
            "Bash" => {
                if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
                    let short_cmd = crate::utils::string_utils::truncate_with_ellipsis(command, 50);
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
            "Edit" | "smart_edit" | "multi_edit" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    Some(format!("Editing: {}", path))
                } else {
                    Some("Editing file".to_string())
                }
            }
            "Write" => {
                if let Some(path) = input.get("file_path").and_then(|v| v.as_str()) {
                    Some(format!("Writing: {}", path))
                } else {
                    Some("Writing file".to_string())
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
            "ListDir" => {
                if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                    Some(format!("Listing: {}", path))
                } else {
                    Some("Listing directory".to_string())
                }
            }
            _ => None,
        }
    }
}

/// 合成消息检测器
pub struct SyntheticMessageDetector;

impl SyntheticMessageDetector {
    /// 合成消息常量
    pub const INTERRUPT_MESSAGE: &'static str = "[Request interrupted by user]";
    pub const INTERRUPT_MESSAGE_FOR_TOOL_USE: &'static str =
        "[Request interrupted by user for tool use]";
    pub const CANCEL_MESSAGE: &'static str = "The user doesn't want to take this action right now. STOP what you are doing and wait for the user to tell you how to proceed.";
    pub const REJECT_MESSAGE: &'static str = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.";
    pub const NO_RESPONSE_REQUESTED: &'static str = "No response requested.";

    /// 检查是否是合成消息
    pub fn is_synthetic_message(content: &str) -> bool {
        let trimmed = content.trim();
        trimmed == Self::INTERRUPT_MESSAGE
            || trimmed == Self::INTERRUPT_MESSAGE_FOR_TOOL_USE
            || trimmed == Self::CANCEL_MESSAGE
            || trimmed == Self::REJECT_MESSAGE
            || trimmed == Self::NO_RESPONSE_REQUESTED
    }

    /// 检查是否是拒绝消息
    pub fn is_reject_message(content: &str) -> bool {
        content.starts_with(Self::REJECT_MESSAGE)
    }

    /// 构建拒绝消息
    pub fn build_reject_message(reason: Option<&str>) -> String {
        if let Some(reason) = reason {
            format!("{} The user said:\n{}", Self::REJECT_MESSAGE, reason)
        } else {
            Self::REJECT_MESSAGE.to_string()
        }
    }

    /// 构建权限拒绝消息
    pub fn build_denial_message(tool_name: &str) -> String {
        format!(
            "Permission to use {} has been denied. {}",
            tool_name,
            Self::DENIAL_WORKAROUND_GUIDANCE
        )
    }

    /// 权限拒绝工作指导
    pub const DENIAL_WORKAROUND_GUIDANCE: &'static str = 
        "IMPORTANT: You *may* attempt to accomplish this action using other tools that might naturally be used to accomplish this goal, \
         e.g. using head instead of cat. But you *should not* attempt to work around this denial in malicious ways, \
         e.g. do not use your ability to run tests to execute non-test actions. \
         You should only try to work around this restriction in reasonable ways that do not attempt to bypass the intent behind this denial. \
         If you believe this capability is essential to complete the user's request, STOP and explain to the user \
         what you were trying to do and why you need this permission. Let the user decide how to proceed.";
}

/// 消息ID生成器
pub struct MessageIdGenerator;

impl MessageIdGenerator {
    /// 生成短消息ID
    pub fn derive_short_id(uuid: &str) -> String {
        // 取UUID的前10个十六进制字符（跳过破折号）
        let hex: String = uuid.chars().filter(|c| *c != '-').take(10).collect();
        // 转换为base36，取6个字符
        if let Ok(num) = u64::from_str_radix(&hex, 16) {
            let base36 = Self::to_base36(num);
            base36.chars().take(6).collect()
        } else {
            hex.chars().take(6).collect()
        }
    }

    /// 转换为base36
    fn to_base36(mut num: u64) -> String {
        if num == 0 {
            return "0".to_string();
        }
        let chars = "0123456789abcdefghijklmnopqrstuvwxyz";
        let mut result = String::new();
        while num > 0 {
            let idx = (num % 36) as usize;
            result.insert(0, chars.chars().nth(idx).unwrap_or('0'));
            num /= 36;
        }
        result
    }
}

/// 查询跟踪
#[derive(Debug, Clone)]
pub struct QueryChainTracking {
    /// 链ID
    pub chain_id: String,
    /// 深度
    pub depth: usize,
}

/// 内容替换状态
#[derive(Debug, Clone)]
pub struct ContentReplacementState {
    /// 已见ID
    pub seen_ids: std::collections::HashSet<String>,
    /// 替换映射
    pub replacements: std::collections::HashMap<String, String>,
}

impl ContentReplacementState {
    pub fn new() -> Self {
        Self {
            seen_ids: std::collections::HashSet::new(),
            replacements: std::collections::HashMap::new(),
        }
    }

    /// 克隆状态（用于缓存共享的fork）
    pub fn clone_for_fork(&self) -> Self {
        Self {
            seen_ids: self.seen_ids.clone(),
            replacements: self.replacements.clone(),
        }
    }
}

/// 工具决策记录
#[derive(Debug, Clone)]
pub struct ToolDecisionRecord {
    /// 来源
    pub source: String,
    /// 决策
    pub decision: ToolDecision,
    /// 时间戳
    pub timestamp: u64,
}

/// 工具决策
#[derive(Debug, Clone)]
pub enum ToolDecision {
    Accept,
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_message_detection() {
        assert!(SyntheticMessageDetector::is_synthetic_message(
            "[Request interrupted by user]"
        ));
        assert!(SyntheticMessageDetector::is_synthetic_message(
            SyntheticMessageDetector::CANCEL_MESSAGE
        ));
        assert!(!SyntheticMessageDetector::is_synthetic_message(
            "Hello world"
        ));
    }

    #[test]
    fn test_message_id_generation() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let short_id = MessageIdGenerator::derive_short_id(uuid);
        assert_eq!(short_id.len(), 6);
    }

    #[test]
    fn test_base36_conversion() {
        assert_eq!(MessageIdGenerator::to_base36(0), "0");
        assert_eq!(MessageIdGenerator::to_base36(35), "z");
        assert_eq!(MessageIdGenerator::to_base36(36), "10");
    }
}
