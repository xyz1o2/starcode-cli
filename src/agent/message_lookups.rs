use crate::types::{StarMessage, StarToolCall};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// 消息查找表 - 对标claude-code的MessageLookups
///
/// 预计算的消息关系查找表，用于高效O(1)访问
#[derive(Debug, Clone)]
pub struct MessageLookups {
    /// 兄弟工具ID映射（同一消息中的所有工具ID）
    pub sibling_tool_use_ids: HashMap<String, HashSet<String>>,
    /// 进度消息映射
    pub progress_messages_by_tool_use_id: HashMap<String, Vec<ProgressMessage>>,
    /// 进行中的Hook计数
    pub in_progress_hook_counts: HashMap<String, HashMap<String, usize>>,
    /// 已完成的Hook计数
    pub resolved_hook_counts: HashMap<String, HashMap<String, usize>>,
    /// 工具结果映射
    pub tool_result_by_tool_use_id: HashMap<String, usize>,
    /// 工具调用映射
    pub tool_use_by_tool_use_id: HashMap<String, StarToolCall>,
    /// 归一化消息数量
    pub normalized_message_count: usize,
    /// 已解决的工具ID
    pub resolved_tool_use_ids: HashSet<String>,
    /// 错误的工具ID
    pub errored_tool_use_ids: HashSet<String>,
}

/// 空Lookups常量 - 用于静态渲染上下文
pub static EMPTY_LOOKUPS: OnceLock<MessageLookups> = OnceLock::new();

/// 空字符串集合常量 - 避免每次分配
pub static EMPTY_STRING_SET: OnceLock<HashSet<String>> = OnceLock::new();

/// 获取空Lookups
pub fn get_empty_lookups() -> &'static MessageLookups {
    EMPTY_LOOKUPS.get_or_init(|| MessageLookups {
        sibling_tool_use_ids: HashMap::new(),
        progress_messages_by_tool_use_id: HashMap::new(),
        in_progress_hook_counts: HashMap::new(),
        resolved_hook_counts: HashMap::new(),
        tool_result_by_tool_use_id: HashMap::new(),
        tool_use_by_tool_use_id: HashMap::new(),
        normalized_message_count: 0,
        resolved_tool_use_ids: HashSet::new(),
        errored_tool_use_ids: HashSet::new(),
    })
}

/// 获取空字符串集合
pub fn get_empty_string_set() -> &'static HashSet<String> {
    EMPTY_STRING_SET.get_or_init(HashSet::new)
}

/// 进度消息
#[derive(Debug, Clone)]
pub struct ProgressMessage {
    /// 工具使用ID
    pub tool_use_id: String,
    /// 父工具使用ID
    pub parent_tool_use_id: String,
    /// 进度数据
    pub data: ProgressData,
}

/// 进度数据
#[derive(Debug, Clone)]
pub enum ProgressData {
    /// 工具进度
    ToolProgress { tool_name: String, message: String },
    /// Hook进度
    HookProgress {
        hook_event: String,
        hook_name: String,
    },
}

impl MessageLookups {
    /// 构建消息查找表
    pub fn build(messages: &[StarMessage]) -> Self {
        let mut sibling_tool_use_ids: HashMap<String, HashSet<String>> = HashMap::new();
        let mut progress_messages_by_tool_use_id: HashMap<String, Vec<ProgressMessage>> =
            HashMap::new();
        let mut in_progress_hook_counts: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut resolved_hook_counts: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut tool_result_by_tool_use_id: HashMap<String, usize> = HashMap::new();
        let mut tool_use_by_tool_use_id: HashMap<String, StarToolCall> = HashMap::new();
        let mut resolved_tool_use_ids: HashSet<String> = HashSet::new();
        let mut errored_tool_use_ids: HashSet<String> = HashSet::new();

        // 第一遍：收集所有工具ID和兄弟关系
        let mut tool_ids_by_message: HashMap<String, HashSet<String>> = HashMap::new();
        let mut message_id_by_tool_id: HashMap<String, String> = HashMap::new();

        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "assistant" {
                if let Some(tool_calls) = &msg.tool_calls {
                    let message_id = msg
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("msg_{}", idx));
                    let mut tool_ids = HashSet::new();

                    for tc in tool_calls {
                        tool_ids.insert(tc.id.clone());
                        message_id_by_tool_id.insert(tc.id.clone(), message_id.clone());
                        tool_use_by_tool_use_id.insert(tc.id.clone(), tc.clone());
                    }

                    tool_ids_by_message.insert(message_id, tool_ids);
                }
            }
        }

        // 构建兄弟工具ID映射
        for (tool_id, message_id) in &message_id_by_tool_id {
            if let Some(siblings) = tool_ids_by_message.get(message_id) {
                sibling_tool_use_ids.insert(tool_id.clone(), siblings.clone());
            }
        }

        // 第二遍：收集工具结果和Hook信息
        for (idx, msg) in messages.iter().enumerate() {
            if msg.role == "tool" {
                if let Some(tool_call_id) = &msg.tool_call_id {
                    tool_result_by_tool_use_id.insert(tool_call_id.clone(), idx);
                    resolved_tool_use_ids.insert(tool_call_id.clone());

                    // 检查是否是错误结果
                    if let Some(content) = &msg.content {
                        if content.contains("error") || content.contains("Error") {
                            errored_tool_use_ids.insert(tool_call_id.clone());
                        }
                    }
                }
            }
        }

        // 检测孤立工具（没有匹配结果的工具调用）
        for tool_id in tool_use_by_tool_use_id.keys() {
            if !resolved_tool_use_ids.contains(tool_id) {
                // 如果是最后一个助手消息中的工具，可能还在执行中
                // 否则标记为错误
                let is_last_message_tool = messages
                    .last()
                    .and_then(|m| m.tool_calls.as_ref())
                    .map(|tcs| tcs.iter().any(|tc| &tc.id == tool_id))
                    .unwrap_or(false);

                if !is_last_message_tool {
                    errored_tool_use_ids.insert(tool_id.clone());
                }
            }
        }

        Self {
            sibling_tool_use_ids,
            progress_messages_by_tool_use_id,
            in_progress_hook_counts,
            resolved_hook_counts,
            tool_result_by_tool_use_id,
            tool_use_by_tool_use_id,
            normalized_message_count: messages.len(),
            resolved_tool_use_ids,
            errored_tool_use_ids,
        }
    }

    /// 获取兄弟工具ID
    pub fn get_sibling_tool_use_ids(&self, tool_use_id: &str) -> HashSet<String> {
        self.sibling_tool_use_ids
            .get(tool_use_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 获取进度消息
    pub fn get_progress_messages(&self, tool_use_id: &str) -> Vec<ProgressMessage> {
        self.progress_messages_by_tool_use_id
            .get(tool_use_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 检查是否有未完成的Hooks
    pub fn has_unresolved_hooks(&self, tool_use_id: &str, hook_event: &str) -> bool {
        let in_progress = self
            .in_progress_hook_counts
            .get(tool_use_id)
            .and_then(|m| m.get(hook_event))
            .copied()
            .unwrap_or(0);

        let resolved = self
            .resolved_hook_counts
            .get(tool_use_id)
            .and_then(|m| m.get(hook_event))
            .copied()
            .unwrap_or(0);

        in_progress > resolved
    }

    /// 获取工具结果
    pub fn get_tool_result_index(&self, tool_use_id: &str) -> Option<usize> {
        self.tool_result_by_tool_use_id.get(tool_use_id).copied()
    }

    /// 获取工具调用
    pub fn get_tool_use(&self, tool_use_id: &str) -> Option<&StarToolCall> {
        self.tool_use_by_tool_use_id.get(tool_use_id)
    }

    /// 检查工具是否已解决
    pub fn is_tool_resolved(&self, tool_use_id: &str) -> bool {
        self.resolved_tool_use_ids.contains(tool_use_id)
    }

    /// 检查工具是否有错误
    pub fn is_tool_errored(&self, tool_use_id: &str) -> bool {
        self.errored_tool_use_ids.contains(tool_use_id)
    }

    /// 获取孤立工具ID列表
    pub fn get_orphaned_tool_ids(&self) -> HashSet<String> {
        let mut orphaned = HashSet::new();
        for tool_id in self.tool_use_by_tool_use_id.keys() {
            if !self.resolved_tool_use_ids.contains(tool_id) {
                orphaned.insert(tool_id.clone());
            }
        }
        orphaned
    }
}

/// 工具执行活动跟踪器
pub struct ToolActivityTracker {
    /// 活动计数
    activity_count: usize,
    /// 最后活动时间
    last_activity_time: Option<std::time::Instant>,
    /// 活动类型
    activity_type: String,
}

impl ToolActivityTracker {
    pub fn new() -> Self {
        Self {
            activity_count: 0,
            last_activity_time: None,
            activity_type: String::new(),
        }
    }

    /// 开始活动
    pub fn start_activity(&mut self, activity_type: &str) {
        self.activity_count += 1;
        self.last_activity_time = Some(std::time::Instant::now());
        self.activity_type = activity_type.to_string();
    }

    /// 停止活动
    pub fn stop_activity(&mut self, activity_type: &str) {
        if self.activity_type == activity_type {
            self.last_activity_time = None;
        }
    }

    /// 获取活动持续时间
    pub fn get_duration(&self) -> Option<std::time::Duration> {
        self.last_activity_time.map(|t| t.elapsed())
    }

    /// 获取活动计数
    pub fn get_count(&self) -> usize {
        self.activity_count
    }
}

/// PostToolUse Hook管理器
pub struct PostToolUseHookManager {
    /// Hook列表
    hooks: Vec<PostToolUseHook>,
}

/// PostToolUse Hook
#[derive(Debug, Clone)]
pub struct PostToolUseHook {
    /// Hook名称
    pub name: String,
    /// Hook事件
    pub event: String,
    /// 是否启用
    pub enabled: bool,
}

impl PostToolUseHookManager {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// 添加Hook
    pub fn add_hook(&mut self, hook: PostToolUseHook) {
        self.hooks.push(hook);
    }

    /// 执行PostToolUse hooks
    pub async fn run_post_tool_use_hooks(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        output: &Value,
        success: bool,
    ) -> Vec<HookResult> {
        let mut results = Vec::new();

        for hook in &self.hooks {
            if !hook.enabled {
                continue;
            }

            let result = HookResult {
                hook_name: hook.name.clone(),
                hook_event: hook.event.clone(),
                success: true,
                message: None,
            };
            results.push(result);
        }

        results
    }

    /// 执行PostToolUseFailure hooks
    pub async fn run_post_tool_use_failure_hooks(
        &self,
        tool_name: &str,
        tool_use_id: &str,
        input: &Value,
        error: &str,
        is_interrupt: bool,
    ) -> Vec<HookResult> {
        let mut results = Vec::new();

        for hook in &self.hooks {
            if !hook.enabled {
                continue;
            }

            let result = HookResult {
                hook_name: hook.name.clone(),
                hook_event: hook.event.clone(),
                success: true,
                message: None,
            };
            results.push(result);
        }

        results
    }
}

/// Hook结果
#[derive(Debug, Clone)]
pub struct HookResult {
    /// Hook名称
    pub hook_name: String,
    /// Hook事件
    pub hook_event: String,
    /// 是否成功
    pub success: bool,
    /// 消息
    pub message: Option<String>,
}

/// MCP认证错误处理器
pub struct McpAuthErrorHandler;

impl McpAuthErrorHandler {
    /// 处理MCP认证错误
    pub fn handle_auth_error(server_name: &str, error: &str) -> McpAuthErrorResult {
        McpAuthErrorResult {
            server_name: server_name.to_string(),
            error: error.to_string(),
            action: McpAuthAction::NeedsAuth,
        }
    }
}

/// MCP认证错误结果
#[derive(Debug, Clone)]
pub struct McpAuthErrorResult {
    pub server_name: String,
    pub error: String,
    pub action: McpAuthAction,
}

/// MCP认证动作
#[derive(Debug, Clone)]
pub enum McpAuthAction {
    /// 需要认证
    NeedsAuth,
    /// 重试
    Retry,
    /// 跳过
    Skip,
}

/// 工具决策记录管理器
pub struct ToolDecisionManager {
    /// 决策记录
    decisions: HashMap<String, ToolDecisionRecord>,
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

impl ToolDecisionManager {
    pub fn new() -> Self {
        Self {
            decisions: HashMap::new(),
        }
    }

    /// 记录决策
    pub fn record_decision(&mut self, tool_use_id: String, source: String, decision: ToolDecision) {
        self.decisions.insert(
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

    /// 获取决策
    pub fn get_decision(&self, tool_use_id: &str) -> Option<&ToolDecisionRecord> {
        self.decisions.get(tool_use_id)
    }

    /// 清理决策
    pub fn clear_decision(&mut self, tool_use_id: &str) {
        self.decisions.remove(tool_use_id);
    }

    /// 清理所有决策
    pub fn clear_all(&mut self) {
        self.decisions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_lookups() {
        let messages = vec![
            StarMessage::assistant_with_tool_calls(vec![
                StarToolCall {
                    id: "tool1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: "Read".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                StarToolCall {
                    id: "tool2".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: "Read".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
            ]),
            StarMessage::tool("tool1", "content1"),
            StarMessage::tool("tool2", "content2"),
        ];

        let lookups = MessageLookups::build(&messages);

        // 测试兄弟工具ID
        let siblings = lookups.get_sibling_tool_use_ids("tool1");
        assert!(siblings.contains("tool2"));

        // 测试工具结果
        assert!(lookups.is_tool_resolved("tool1"));
        assert!(lookups.is_tool_resolved("tool2"));

        // 测试孤立工具
        let orphaned = lookups.get_orphaned_tool_ids();
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_orphaned_tool_detection() {
        let messages = vec![
            StarMessage::assistant_with_tool_calls(vec![StarToolCall {
                id: "tool1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            // 没有tool1的结果
        ];

        let lookups = MessageLookups::build(&messages);
        let orphaned = lookups.get_orphaned_tool_ids();
        assert!(orphaned.contains("tool1"));
    }

    #[test]
    fn test_empty_lookups() {
        let lookups = get_empty_lookups();
        assert_eq!(lookups.normalized_message_count, 0);
        assert!(lookups.sibling_tool_use_ids.is_empty());
    }

    #[test]
    fn test_empty_string_set() {
        let set = get_empty_string_set();
        assert!(set.is_empty());
    }
}

/// 从Lookup获取兄弟工具ID
pub fn get_sibling_tool_use_ids_from_lookup<'a>(
    tool_use_id: &str,
    lookups: &'a MessageLookups,
) -> &'a HashSet<String> {
    lookups
        .sibling_tool_use_ids
        .get(tool_use_id)
        .unwrap_or_else(|| get_empty_string_set())
}

/// 从Lookup获取进度消息
pub fn get_progress_messages_from_lookup<'a>(
    tool_use_id: &str,
    lookups: &'a MessageLookups,
) -> &'a Vec<ProgressMessage> {
    lookups
        .progress_messages_by_tool_use_id
        .get(tool_use_id)
        .unwrap_or_else(|| {
            static EMPTY_VEC: Vec<ProgressMessage> = Vec::new();
            &EMPTY_VEC
        })
}

/// 从Lookup检查未完成的Hooks
pub fn has_unresolved_hooks_from_lookup(
    tool_use_id: &str,
    hook_event: &str,
    lookups: &MessageLookups,
) -> bool {
    let in_progress_count = lookups
        .in_progress_hook_counts
        .get(tool_use_id)
        .and_then(|m| m.get(hook_event))
        .copied()
        .unwrap_or(0);

    let resolved_count = lookups
        .resolved_hook_counts
        .get(tool_use_id)
        .and_then(|m| m.get(hook_event))
        .copied()
        .unwrap_or(0);

    in_progress_count > resolved_count
}

/// 子代理Lookups构建
pub fn build_subagent_lookups(messages: &[StarMessage]) -> (MessageLookups, HashSet<String>) {
    let mut tool_use_by_tool_use_id: HashMap<String, StarToolCall> = HashMap::new();
    let mut resolved_tool_use_ids: HashSet<String> = HashSet::new();
    let mut tool_result_by_tool_use_id: HashMap<String, usize> = HashMap::new();

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    tool_use_by_tool_use_id.insert(tc.id.clone(), tc.clone());
                }
            }
        } else if msg.role == "tool" {
            if let Some(tool_call_id) = &msg.tool_call_id {
                resolved_tool_use_ids.insert(tool_call_id.clone());
                tool_result_by_tool_use_id.insert(tool_call_id.clone(), idx);
            }
        }
    }

    let mut in_progress_tool_use_ids = HashSet::new();
    for tool_id in tool_use_by_tool_use_id.keys() {
        if !resolved_tool_use_ids.contains(tool_id) {
            in_progress_tool_use_ids.insert(tool_id.clone());
        }
    }

    let lookups = MessageLookups {
        sibling_tool_use_ids: HashMap::new(),
        progress_messages_by_tool_use_id: HashMap::new(),
        in_progress_hook_counts: HashMap::new(),
        resolved_hook_counts: HashMap::new(),
        tool_result_by_tool_use_id,
        tool_use_by_tool_use_id,
        normalized_message_count: messages.len(),
        resolved_tool_use_ids,
        errored_tool_use_ids: HashSet::new(),
    };

    (lookups, in_progress_tool_use_ids)
}

/// 附件重排序 - 将附件消息向上冒泡
pub fn reorder_attachments_for_api(messages: Vec<StarMessage>) -> Vec<StarMessage> {
    let mut result: Vec<StarMessage> = Vec::new();
    let mut pending_attachments: Vec<StarMessage> = Vec::new();

    // 从底部向上扫描
    for msg in messages.into_iter().rev() {
        if msg.role == "system" && msg.content.as_ref().map_or(false, |c| c.starts_with("[")) {
            // 附件消息，收集起来
            pending_attachments.push(msg);
        } else {
            // 检查是否是停止点
            let is_stopping_point = msg.role == "assistant" || (msg.role == "tool");

            if is_stopping_point && !pending_attachments.is_empty() {
                // 将附件放在停止点之后
                for attachment in pending_attachments.drain(..) {
                    result.push(attachment);
                }
                result.push(msg);
            } else {
                result.push(msg);
            }
        }
    }

    // 剩余的附件放到最前面
    for attachment in pending_attachments {
        result.push(attachment);
    }

    result.reverse();
    result
}

/// 不可用工具引用过滤
pub fn strip_unavailable_tool_references(
    messages: Vec<StarMessage>,
    available_tool_names: &HashSet<String>,
) -> Vec<StarMessage> {
    messages
        .into_iter()
        .map(|msg| {
            if msg.role == "user" {
                if let Some(content) = &msg.content {
                    // 检查是否包含不可用工具的引用
                    if content.contains("tool_reference") {
                        // 简单过滤：如果包含不可用工具名称，移除相关引用
                        let mut filtered_content = content.clone();
                        for tool_name in available_tool_names {
                            // 这里简化处理，实际应该解析JSON结构
                            if !available_tool_names.contains(tool_name) {
                                filtered_content = filtered_content.replace(
                                    &format!("\"tool_name\": \"{}\"", tool_name),
                                    "\"tool_name\": \"unavailable\"",
                                );
                            }
                        }
                        let mut new_msg = msg.clone();
                        new_msg.content = Some(filtered_content);
                        return new_msg;
                    }
                }
            }
            msg
        })
        .collect()
}

/// 系统初始化消息构建
pub fn build_system_init_message(
    tools: &[String],
    model: &str,
    permission_mode: &str,
) -> StarMessage {
    let tools_list = tools.join(", ");
    StarMessage::system(&format!(
        "[System Init]\nModel: {}\nPermission Mode: {}\nAvailable Tools: {}",
        model, permission_mode, tools_list
    ))
}

/// 本地命令输出消息构建
pub fn build_local_command_output_message(
    command: &str,
    output: &str,
    exit_code: i32,
) -> StarMessage {
    StarMessage::system(&format!(
        "[Local Command]\nCommand: {}\nExit Code: {}\nOutput:\n{}",
        command, exit_code, output
    ))
}

/// 压缩边界消息构建
pub fn build_compact_boundary_message(
    original_token_count: usize,
    new_token_count: usize,
    strategy: &str,
) -> StarMessage {
    StarMessage::system(&format!(
        "[Compact Boundary]\nOriginal: {} tokens → New: {} tokens\nStrategy: {}",
        original_token_count, new_token_count, strategy
    ))
}

/// 文件历史快照
pub struct FileHistorySnapshot {
    pub file_path: String,
    pub content_hash: String,
    pub timestamp: u64,
}

impl FileHistorySnapshot {
    pub fn new(file_path: &str, content: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());

        Self {
            file_path: file_path.to_string(),
            content_hash,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

/// 结构化输出跟踪器
pub struct StructuredOutputTracker {
    pub output: Option<Value>,
    pub tool_name: Option<String>,
    pub timestamp: Option<u64>,
}

impl StructuredOutputTracker {
    pub fn new() -> Self {
        Self {
            output: None,
            tool_name: None,
            timestamp: None,
        }
    }

    pub fn set_output(&mut self, output: Value, tool_name: String) {
        self.output = Some(output);
        self.tool_name = Some(tool_name);
        self.timestamp = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
    }

    pub fn get_output(&self) -> Option<&Value> {
        self.output.as_ref()
    }

    pub fn clear(&mut self) {
        self.output = None;
        self.tool_name = None;
        self.timestamp = None;
    }
}

/// 错误日志水印
pub struct ErrorLogWatermark {
    pub errors: Vec<String>,
    pub watermark_index: usize,
}

impl ErrorLogWatermark {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            watermark_index: 0,
        }
    }

    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    pub fn get_new_errors(&self) -> &[String] {
        &self.errors[self.watermark_index..]
    }

    pub fn set_watermark(&mut self) {
        self.watermark_index = self.errors.len();
    }

    pub fn has_new_errors(&self) -> bool {
        self.watermark_index < self.errors.len()
    }
}
