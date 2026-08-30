use crate::types::{StarMessage, StarToolCall};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 流式回退管理器 - 对标claude-code的流式回退机制
pub struct StreamFallbackManager {
    /// 是否正在回退
    is_fallback_active: bool,
    /// 原始模型
    original_model: Option<String>,
    /// 回退模型
    fallback_model: Option<String>,
    /// 回退尝试次数
    fallback_attempts: usize,
    /// 最大回退尝试次数
    max_fallback_attempts: usize,
}

impl StreamFallbackManager {
    pub fn new() -> Self {
        Self {
            is_fallback_active: false,
            original_model: None,
            fallback_model: None,
            fallback_attempts: 0,
            max_fallback_attempts: 3,
        }
    }

    /// 检查是否是可回退的错误
    pub fn is_fallback_eligible_error(error: &str) -> bool {
        let error_lower = error.to_lowercase();
        error_lower.contains("overloaded")
            || error_lower.contains("rate_limit")
            || error_lower.contains("rate limit")
            || error_lower.contains("529")
            || error_lower.contains("503")
            || error_lower.contains("502")
            || error_lower.contains("service_unavailable")
            || error_lower.contains("too many requests")
    }

    /// 开始回退
    pub fn start_fallback(&mut self, original_model: &str, fallback_model: &str) {
        self.is_fallback_active = true;
        self.original_model = Some(original_model.to_string());
        self.fallback_model = Some(fallback_model.to_string());
        self.fallback_attempts += 1;
    }

    /// 结束回退
    pub fn end_fallback(&mut self) {
        self.is_fallback_active = false;
    }

    /// 检查是否可以回退
    pub fn can_fallback(&self) -> bool {
        self.fallback_attempts < self.max_fallback_attempts
    }

    /// 获取回退模型
    pub fn get_fallback_model(&self) -> Option<&str> {
        self.fallback_model.as_deref()
    }

    /// 检查是否正在回退
    pub fn is_fallback_active(&self) -> bool {
        self.is_fallback_active
    }

    /// 创建回退结果消息
    pub fn create_fallback_result_message(&self, original_model: &str, fallback_model: &str) -> StarMessage {
        StarMessage::system(&format!(
            "[Model Fallback] Switched to {} due to high demand for {}",
            fallback_model, original_model
        ))
    }
}

/// Tombstone消息管理器 - 对标claude-code的tombstone消息
pub struct TombstoneManager;

impl TombstoneManager {
    /// 创建tombstone消息
    /// 用于标记孤立的消息，让UI和转录可以移除它们
    pub fn create_tombstone_message(original_message: &StarMessage) -> StarMessage {
        let content = format!(
            "[Tombstone] Original message removed: {}",
            original_message.content.as_deref().unwrap_or("").chars().take(50).collect::<String>()
        );
        StarMessage::system(&content)
    }

    /// 批量创建tombstone消息
    pub fn create_tombstone_messages(messages: &[StarMessage]) -> Vec<StarMessage> {
        messages.iter().map(|m| Self::create_tombstone_message(m)).collect()
    }

    /// 检查是否是tombstone消息
    pub fn is_tombstone_message(message: &StarMessage) -> bool {
        message.content.as_ref().map_or(false, |c| c.starts_with("[Tombstone]"))
    }
}

/// 消息Backfill管理器 - 对标claude-code的backfillObservableInput
pub struct MessageBackfillManager;

impl MessageBackfillManager {
    /// 回填可观测输入
    /// 在消息被yield之前，为SDK流输出和转录序列化添加遗留/派生字段
    pub fn backfill_observable_input(
        tool: &dyn BackfillCapableTool,
        input: &mut Value,
    ) {
        if let Some(obj) = input.as_object_mut() {
            tool.backfill_observable_input(obj);
        }
    }

    /// 检查是否有新增字段
    pub fn has_added_fields(original: &Value, backfilled: &Value) -> bool {
        if let (Some(orig_obj), Some(back_obj)) = (original.as_object(), backfilled.as_object()) {
            for key in back_obj.keys() {
                if !orig_obj.contains_key(key) {
                    return true;
                }
            }
        }
        false
    }

    /// 克隆并回填
    pub fn clone_and_backfill(
        tool: &dyn BackfillCapableTool,
        input: &Value,
    ) -> Option<Value> {
        let mut input_copy = input.clone();
        Self::backfill_observable_input(tool, &mut input_copy);
        
        if Self::has_added_fields(input, &input_copy) {
            Some(input_copy)
        } else {
            None
        }
    }
}

/// 可回填工具trait
pub trait BackfillCapableTool {
    /// 回填可观测输入
    fn backfill_observable_input(&self, input: &mut serde_json::Map<String, Value>);
}

/// 延迟工具发现提示生成器 - 对标claude-code的buildSchemaNotSentHint
pub struct DeferredToolHintGenerator;

impl DeferredToolHintGenerator {
    /// 构建延迟工具发现提示
    /// 当工具未被发现时，生成提示信息
    pub fn build_schema_not_sent_hint(
        tool_name: &str,
        messages: &[StarMessage],
        available_tools: &[String],
    ) -> Option<String> {
        // 检查工具是否在可用工具列表中
        if available_tools.iter().any(|t| t == tool_name) {
            return None;
        }

        // 检查是否是已知的延迟工具
        let deferred_tools = Self::get_deferred_tools();
        if !deferred_tools.contains(&tool_name.to_string()) {
            return None;
        }

        // 检查是否已经发现了该工具
        let discovered_tools = Self::extract_discovered_tool_names(messages);
        if discovered_tools.contains(tool_name) {
            return None;
        }

        Some(format!(
            "\n\nTool \"{}\" is deferred-loading and needs to be discovered before use.\n\
             When using OpenAI-compatible models (DeepSeek, Ollama, etc.), follow these steps:\n\
             1. First discover the tool with SearchExtraTools: SearchExtraTools(\"select:{}\")\n\
             2. Then call {} tool\n\
             \nExample:\n\
             SearchExtraTools(\"select:{}\") → {}({{ ... }})\n\
             \nImportant notes:\n\
             • Use camelCase parameter names (e.g., taskId), not snake_case (task_id)\n\
             • All task tools (TaskGet, TaskCreate, TaskUpdate, TaskList) need to be discovered first\n\
             • You can discover them all at once: SearchExtraTools(\"select:TaskGet,TaskCreate,TaskUpdate,TaskList\")\n",
            tool_name, tool_name, tool_name, tool_name, tool_name
        ))
    }

    /// 获取延迟工具列表
    fn get_deferred_tools() -> Vec<String> {
        vec![
            "TeamCreate".to_string(),
            "TeamDelete".to_string(),
            "SendMessage".to_string(),
            "CronCreate".to_string(),
            "CronDelete".to_string(),
            "CronList".to_string(),
        ]
    }

    /// 提取已发现的工具名称
    fn extract_discovered_tool_names(messages: &[StarMessage]) -> HashSet<String> {
        let mut discovered = HashSet::new();
        for msg in messages {
            if let Some(content) = &msg.content {
                if content.contains("Tool loaded") || content.contains("tool loaded") {
                    // 简化处理：从内容中提取工具名称
                    for tool_name in Self::get_deferred_tools() {
                        if content.contains(&tool_name) {
                            discovered.insert(tool_name);
                        }
                    }
                }
            }
        }
        discovered
    }
}

/// 流式权限检查管理器 - 对标claude-code的streamedCheckPermissionsAndCallTool
pub struct StreamPermissionChecker;

impl StreamPermissionChecker {
    /// 流式权限检查和工具调用
    /// 在流式执行时检查权限并调用工具
    pub async fn check_and_call_tool(
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        can_use_tool: &dyn Fn(&str, &Value) -> bool,
        on_progress: Option<Box<dyn Fn(String) + Send + Sync>>,
    ) -> StreamPermissionResult {
        // 检查权限
        if !can_use_tool(tool_name, input) {
            return StreamPermissionResult::Denied {
                message: format!("Permission denied for tool: {}", tool_name),
            };
        }

        // 报告进度
        if let Some(callback) = &on_progress {
            callback(format!("Executing tool: {}", tool_name));
        }

        // 工具执行由调用者完成
        StreamPermissionResult::Allowed
    }
}

/// 流式权限结果
#[derive(Debug, Clone)]
pub enum StreamPermissionResult {
    /// 允许执行
    Allowed,
    /// 拒绝执行
    Denied {
        message: String,
    },
    /// 需要确认
    NeedsConfirmation {
        message: String,
    },
}

/// 流式工具执行器增强 - 对标claude-code的StreamingToolExecutor
pub struct EnhancedStreamingToolExecutor {
    /// 工具定义
    tools: Vec<String>,
    /// 进行中的工具
    in_progress: HashMap<String, InProgressTool>,
    /// 已完成的结果
    completed_results: HashMap<String, Value>,
    /// 是否已丢弃
    discarded: bool,
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

impl EnhancedStreamingToolExecutor {
    pub fn new(tools: Vec<String>) -> Self {
        Self {
            tools,
            in_progress: HashMap::new(),
            completed_results: HashMap::new(),
            discarded: false,
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
    pub fn get_completed_results(&self) -> &HashMap<String, Value> {
        &self.completed_results
    }

    /// 丢弃所有待执行和正在执行的工具
    pub fn discard(&mut self) {
        self.discarded = true;
        self.in_progress.clear();
        self.completed_results.clear();
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

/// 模型回退错误
#[derive(Debug, Clone)]
pub struct FallbackTriggeredError {
    /// 原始模型
    pub original_model: String,
    /// 回退模型
    pub fallback_model: String,
    /// 错误消息
    pub error_message: String,
}

impl std::fmt::Display for FallbackTriggeredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Fallback from {} to {}: {}",
            self.original_model, self.fallback_model, self.error_message
        )
    }
}

impl std::error::Error for FallbackTriggeredError {}

/// 查询配置
#[derive(Debug, Clone)]
pub struct QueryConfig {
    /// 是否是Ant用户
    pub is_ant: bool,
    /// 是否启用流式工具执行
    pub streaming_tool_execution: bool,
    /// 是否启用快速模式
    pub fast_mode_enabled: bool,
    /// 会话ID
    pub session_id: String,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            is_ant: false,
            streaming_tool_execution: true,
            fast_mode_enabled: false,
            session_id: String::new(),
        }
    }
}

impl QueryConfig {
    pub fn from_env() -> Self {
        Self {
            is_ant: std::env::var("USER_TYPE").map(|v| v == "ant").unwrap_or(false),
            streaming_tool_execution: true,
            fast_mode_enabled: false,
            session_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// 查询检查点
pub struct QueryCheckpoint;

impl QueryCheckpoint {
    /// 记录检查点
    pub fn checkpoint(name: &str) {
        crate::utils::logging::append_debug_log_line(&format!("[CHECKPOINT] {}", name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_fallback_manager() {
        let mut manager = StreamFallbackManager::new();
        assert!(!manager.is_fallback_active());
        assert!(manager.can_fallback());
        
        manager.start_fallback("model-a", "model-b");
        assert!(manager.is_fallback_active());
        assert_eq!(manager.get_fallback_model(), Some("model-b"));
        
        manager.end_fallback();
        assert!(!manager.is_fallback_active());
    }

    #[test]
    fn test_tombstone_manager() {
        let msg = StarMessage::user("test content");
        let tombstone = TombstoneManager::create_tombstone_message(&msg);
        assert!(TombstoneManager::is_tombstone_message(&tombstone));
    }

    #[test]
    fn test_enhanced_streaming_executor() {
        let mut executor = EnhancedStreamingToolExecutor::new(vec!["Read".to_string()]);
        
        executor.add_tool("Read", "tool1", true);
        assert_eq!(executor.in_progress_count(), 1);
        assert!(executor.can_execute_tool(true));
        
        executor.complete_tool("tool1", serde_json::json!({"result": "ok"}));
        assert_eq!(executor.in_progress_count(), 0);
        assert!(executor.get_completed_results().contains_key("tool1"));
    }

    #[test]
    fn test_deferred_tool_hint() {
        let hint = DeferredToolHintGenerator::build_schema_not_sent_hint(
            "TeamCreate",
            &[],
            &["Read".to_string(), "Edit".to_string()],
        );
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("deferred-loading"));
    }
}
