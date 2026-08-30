use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

/// ToolUseContext - 对标claude-code的ToolUseContext
/// 
/// 管理工具执行的上下文状态
pub struct ToolUseContext {
    /// 进行中的工具ID集合
    in_progress_tool_ids: Arc<Mutex<HashSet<String>>>,
    /// 是否有可中断的工具正在执行
    has_interruptible_tool_in_progress: Arc<Mutex<bool>>,
    /// 响应长度
    response_length: Arc<Mutex<usize>>,
    /// 工具决策记录
    tool_decisions: Arc<Mutex<HashMap<String, ToolDecisionRecord>>>,
    /// 查询跟踪
    query_tracking: Option<QueryChainTracking>,
    /// 内容替换状态
    content_replacement_state: Option<ContentReplacementState>,
    /// 渲染的系统提示
    rendered_system_prompt: Option<String>,
    /// 已发现的技能名称
    discovered_skill_names: Arc<Mutex<HashSet<String>>>,
    /// 已加载的嵌套内存路径
    loaded_nested_memory_paths: Arc<Mutex<HashSet<String>>>,
    /// 动态技能目录触发器
    dynamic_skill_dir_triggers: Arc<Mutex<HashSet<String>>>,
    /// 嵌套内存附件触发器
    nested_memory_attachment_triggers: Arc<Mutex<HashSet<String>>>,
}

impl ToolUseContext {
    pub fn new() -> Self {
        Self {
            in_progress_tool_ids: Arc::new(Mutex::new(HashSet::new())),
            has_interruptible_tool_in_progress: Arc::new(Mutex::new(false)),
            response_length: Arc::new(Mutex::new(0)),
            tool_decisions: Arc::new(Mutex::new(HashMap::new())),
            query_tracking: None,
            content_replacement_state: None,
            rendered_system_prompt: None,
            discovered_skill_names: Arc::new(Mutex::new(HashSet::new())),
            loaded_nested_memory_paths: Arc::new(Mutex::new(HashSet::new())),
            dynamic_skill_dir_triggers: Arc::new(Mutex::new(HashSet::new())),
            nested_memory_attachment_triggers: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 设置进行中的工具ID
    pub async fn set_in_progress_tool_ids<F>(&self, f: F)
    where
        F: FnOnce(&HashSet<String>) -> HashSet<String>,
    {
        let mut ids = self.in_progress_tool_ids.lock().await;
        *ids = f(&ids);
    }

    /// 获取进行中的工具ID
    pub async fn get_in_progress_tool_ids(&self) -> HashSet<String> {
        self.in_progress_tool_ids.lock().await.clone()
    }

    /// 设置是否有可中断的工具正在执行
    pub async fn set_has_interruptible_tool_in_progress(&self, v: bool) {
        let mut has = self.has_interruptible_tool_in_progress.lock().await;
        *has = v;
    }

    /// 获取是否有可中断的工具正在执行
    pub async fn get_has_interruptible_tool_in_progress(&self) -> bool {
        *self.has_interruptible_tool_in_progress.lock().await
    }

    /// 设置响应长度
    pub async fn set_response_length<F>(&self, f: F)
    where
        F: FnOnce(usize) -> usize,
    {
        let mut length = self.response_length.lock().await;
        *length = f(*length);
    }

    /// 获取响应长度
    pub async fn get_response_length(&self) -> usize {
        *self.response_length.lock().await
    }

    /// 记录工具决策
    pub async fn record_tool_decision(
        &self,
        tool_use_id: String,
        source: String,
        decision: ToolDecision,
    ) {
        let mut decisions = self.tool_decisions.lock().await;
        decisions.insert(
            tool_use_id,
            ToolDecisionRecord {
                source,
                decision,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            },
        );
    }

    /// 获取工具决策
    pub async fn get_tool_decision(&self, tool_use_id: &str) -> Option<ToolDecisionRecord> {
        self.tool_decisions.lock().await.get(tool_use_id).cloned()
    }

    /// 设置查询跟踪
    pub fn set_query_tracking(&mut self, tracking: QueryChainTracking) {
        self.query_tracking = Some(tracking);
    }

    /// 获取查询跟踪
    pub fn get_query_tracking(&self) -> Option<&QueryChainTracking> {
        self.query_tracking.as_ref()
    }

    /// 设置内容替换状态
    pub fn set_content_replacement_state(&mut self, state: ContentReplacementState) {
        self.content_replacement_state = Some(state);
    }

    /// 获取内容替换状态
    pub fn get_content_replacement_state(&self) -> Option<&ContentReplacementState> {
        self.content_replacement_state.as_ref()
    }

    /// 克隆内容替换状态（用于缓存共享的fork）
    pub fn clone_content_replacement_state(&self) -> Option<ContentReplacementState> {
        self.content_replacement_state.as_ref().map(|s| s.clone_for_fork())
    }

    /// 设置渲染的系统提示
    pub fn set_rendered_system_prompt(&mut self, prompt: String) {
        self.rendered_system_prompt = Some(prompt);
    }

    /// 获取渲染的系统提示
    pub fn get_rendered_system_prompt(&self) -> Option<&str> {
        self.rendered_system_prompt.as_deref()
    }

    /// 添加已发现的技能名称
    pub async fn add_discovered_skill_name(&self, name: String) {
        let mut names = self.discovered_skill_names.lock().await;
        names.insert(name);
    }

    /// 获取已发现的技能名称
    pub async fn get_discovered_skill_names(&self) -> HashSet<String> {
        self.discovered_skill_names.lock().await.clone()
    }

    /// 添加已加载的嵌套内存路径
    pub async fn add_loaded_nested_memory_path(&self, path: String) {
        let mut paths = self.loaded_nested_memory_paths.lock().await;
        paths.insert(path);
    }

    /// 检查是否已加载嵌套内存路径
    pub async fn has_loaded_nested_memory_path(&self, path: &str) -> bool {
        self.loaded_nested_memory_paths.lock().await.contains(path)
    }

    /// 添加动态技能目录触发器
    pub async fn add_dynamic_skill_dir_trigger(&self, trigger: String) {
        let mut triggers = self.dynamic_skill_dir_triggers.lock().await;
        triggers.insert(trigger);
    }

    /// 添加嵌套内存附件触发器
    pub async fn add_nested_memory_attachment_trigger(&self, trigger: String) {
        let mut triggers = self.nested_memory_attachment_triggers.lock().await;
        triggers.insert(trigger);
    }
}

/// 查询链跟踪
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
    pub seen_ids: HashSet<String>,
    /// 替换映射
    pub replacements: HashMap<String, String>,
}

impl ContentReplacementState {
    pub fn new() -> Self {
        Self {
            seen_ids: HashSet::new(),
            replacements: HashMap::new(),
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

/// 工具结果摘要生成器
pub struct ToolResultSummaryGenerator;

impl ToolResultSummaryGenerator {
    /// 生成工具结果摘要（使用starcode-cli中的实际工具名称）
    pub fn generate_summary(
        tool_name: &str,
        input: &serde_json::Value,
        output: &str,
    ) -> Option<String> {
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
    pub const INTERRUPT_MESSAGE_FOR_TOOL_USE: &'static str = "[Request interrupted by user for tool use]";
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

/// 检查消息是否包含工具调用
pub fn has_tool_calls_in_last_assistant_turn(messages: &[crate::types::StarMessage]) -> bool {
    for msg in messages.iter().rev() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                return !tool_calls.is_empty();
            }
            return false;
        }
    }
    false
}

/// 获取最后一条助手消息
pub fn get_last_assistant_message(messages: &[crate::types::StarMessage]) -> Option<&crate::types::StarMessage> {
    messages.iter().rev().find(|msg| msg.role == "assistant")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_message_detection() {
        assert!(SyntheticMessageDetector::is_synthetic_message("[Request interrupted by user]"));
        assert!(SyntheticMessageDetector::is_synthetic_message(SyntheticMessageDetector::CANCEL_MESSAGE));
        assert!(!SyntheticMessageDetector::is_synthetic_message("Hello world"));
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
