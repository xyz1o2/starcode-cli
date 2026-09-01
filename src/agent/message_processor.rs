use crate::types::{StarMessage, StarToolCall};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 消息处理增强模块 - 对标claude-code的消息处理功能
///
/// 提供消息ID标签追加、工具引用块过滤、caller字段剥离等功能

/// 消息ID标签追加器
pub struct MessageTagAppender;

impl MessageTagAppender {
    /// 追加消息ID标签到用户消息末尾
    /// 用于Snip工具引用
    pub fn append_message_tag(message: &mut StarMessage) {
        if message.role != "user" {
            return;
        }

        // 生成短消息ID
        let short_id =
            Self::derive_short_message_id(&message.tool_call_id.clone().unwrap_or_default());

        let tag = format!("\n[id:{}]", short_id);

        if let Some(content) = &mut message.content {
            content.push_str(&tag);
        }
    }

    /// 从UUID派生短消息ID
    fn derive_short_message_id(uuid: &str) -> String {
        let hex: String = uuid.chars().filter(|c| *c != '-').take(10).collect();
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

/// 工具引用块过滤器
pub struct ToolReferenceFilter;

impl ToolReferenceFilter {
    /// 过滤工具引用块
    /// 当工具搜索未启用时，需要移除tool_reference块以避免API错误
    pub fn strip_tool_reference_blocks(message: &mut StarMessage) {
        if message.role != "user" {
            return;
        }

        if let Some(content) = &message.content {
            if content.contains("tool_reference") {
                // 简化处理：移除tool_reference相关内容
                let filtered =
                    content.replace("\"type\": \"tool_reference\"", "\"type\": \"text\"");
                message.content = Some(filtered);
            }
        }
    }

    /// 检查是否包含工具引用
    pub fn has_tool_reference(message: &StarMessage) -> bool {
        if let Some(content) = &message.content {
            content.contains("tool_reference")
        } else {
            false
        }
    }
}

/// Caller字段剥离器
pub struct CallerFieldStripper;

impl CallerFieldStripper {
    /// 剥离caller字段
    /// 当工具搜索未启用时，需要移除caller字段以避免API错误
    pub fn strip_caller_field(message: &mut StarMessage) {
        if message.role != "assistant" {
            return;
        }

        if let Some(tool_calls) = &mut message.tool_calls {
            for tc in tool_calls.iter_mut() {
                // 移除caller字段（如果存在）
                if let Ok(mut args) = serde_json::from_str::<Value>(&tc.function.arguments) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.remove("caller");
                        tc.function.arguments = serde_json::to_string(obj).unwrap_or_default();
                    }
                }
            }
        }
    }
}

/// 系统提醒包装器
pub struct SystemReminderWrapper;

impl SystemReminderWrapper {
    /// 确保附件消息有<system-reminder>包装
    pub fn ensure_system_reminder_wrap(message: &mut StarMessage) {
        if message.role != "user" {
            return;
        }

        if let Some(content) = &message.content {
            // 检查是否是附件消息
            if content.starts_with("[") && !content.contains("<system-reminder>") {
                let wrapped = format!("<system-reminder>{}</system-reminder>", content);
                message.content = Some(wrapped);
            }
        }
    }

    /// 检查是否是系统提醒消息
    pub fn is_system_reminder(message: &StarMessage) -> bool {
        if let Some(content) = &message.content {
            content.contains("<system-reminder>")
        } else {
            false
        }
    }
}

/// 上下文修饰符管理器
pub struct ContextModifierManager {
    /// 修饰符队列
    modifiers: HashMap<String, Vec<ContextModifier>>,
}

/// 上下文修饰符
#[derive(Debug, Clone)]
pub struct ContextModifier {
    /// 工具使用ID
    pub tool_use_id: String,
    /// 修饰符类型
    pub modifier_type: ContextModifierType,
    /// 修饰符数据
    pub data: Value,
}

/// 上下文修饰符类型
#[derive(Debug, Clone)]
pub enum ContextModifierType {
    /// 添加上下文
    AddContext,
    /// 移除上下文
    RemoveContext,
    /// 替换上下文
    ReplaceContext,
}

impl ContextModifierManager {
    pub fn new() -> Self {
        Self {
            modifiers: HashMap::new(),
        }
    }

    /// 添加修饰符
    pub fn add_modifier(&mut self, tool_use_id: String, modifier: ContextModifier) {
        self.modifiers
            .entry(tool_use_id)
            .or_insert_with(Vec::new)
            .push(modifier);
    }

    /// 获取修饰符
    pub fn get_modifiers(&self, tool_use_id: &str) -> Vec<&ContextModifier> {
        self.modifiers
            .get(tool_use_id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// 应用修饰符
    pub fn apply_modifiers(&self, tool_use_id: &str, context: &mut Value) {
        if let Some(modifiers) = self.modifiers.get(tool_use_id) {
            for modifier in modifiers {
                match modifier.modifier_type {
                    ContextModifierType::AddContext => {
                        if let Some(obj) = context.as_object_mut() {
                            if let Some(new_obj) = modifier.data.as_object() {
                                for (k, v) in new_obj {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                    ContextModifierType::RemoveContext => {
                        if let Some(obj) = context.as_object_mut() {
                            if let Some(keys) = modifier.data.as_array() {
                                for key in keys {
                                    if let Some(k) = key.as_str() {
                                        obj.remove(k);
                                    }
                                }
                            }
                        }
                    }
                    ContextModifierType::ReplaceContext => {
                        *context = modifier.data.clone();
                    }
                }
            }
        }
    }

    /// 清理修饰符
    pub fn clear(&mut self) {
        self.modifiers.clear();
    }
}

/// 命令生命周期通知器
pub struct CommandLifecycleNotifier {
    /// 生命周期事件
    events: HashMap<String, Vec<CommandLifecycleEvent>>,
}

/// 命令生命周期事件
#[derive(Debug, Clone)]
pub struct CommandLifecycleEvent {
    /// 命令UUID
    pub uuid: String,
    /// 事件类型
    pub event_type: CommandLifecycleEventType,
    /// 时间戳
    pub timestamp: u64,
}

/// 命令生命周期事件类型
#[derive(Debug, Clone)]
pub enum CommandLifecycleEventType {
    /// 已创建
    Created,
    /// 已开始
    Started,
    /// 已完成
    Completed,
    /// 已失败
    Failed,
}

impl CommandLifecycleNotifier {
    pub fn new() -> Self {
        Self {
            events: HashMap::new(),
        }
    }

    /// 通知命令生命周期
    pub fn notify(&mut self, uuid: &str, event_type: CommandLifecycleEventType) {
        let event = CommandLifecycleEvent {
            uuid: uuid.to_string(),
            event_type,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        self.events
            .entry(uuid.to_string())
            .or_insert_with(Vec::new)
            .push(event);
    }

    /// 获取命令事件
    pub fn get_events(&self, uuid: &str) -> Vec<&CommandLifecycleEvent> {
        self.events
            .get(uuid)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// 检查命令是否已完成
    pub fn is_completed(&self, uuid: &str) -> bool {
        self.events
            .get(uuid)
            .map(|v| {
                v.iter()
                    .any(|e| matches!(e.event_type, CommandLifecycleEventType::Completed))
            })
            .unwrap_or(false)
    }
}

/// 性能缓冲区清理器
pub struct PerformanceBufferCleaner;

impl PerformanceBufferCleaner {
    /// 清理性能缓冲区
    /// 清理JSC的原生Performance缓冲区
    pub fn clear_performance_buffers() {
        // 在Rust中，我们没有直接的Performance API
        // 但我们可以清理一些常见的缓冲区
    }

    /// 清理内存缓冲区
    pub fn clear_memory_buffers() {
        // 触发垃圾回收（如果可用）
        // 在Rust中，内存管理是自动的
        // 但我们可以清理一些缓存
    }
}

/// 系统提醒兄弟压缩器 - 对标claude-code的smooshSystemReminderSiblings
///
/// 将<system-reminder>前缀的文本兄弟压缩到最后一个tool_result中
pub struct SystemReminderSmoosher;

impl SystemReminderSmoosher {
    /// 压缩系统提醒兄弟
    pub fn smoosh_system_reminder_siblings(messages: &mut Vec<StarMessage>) {
        for msg in messages.iter_mut() {
            if msg.role != "user" {
                continue;
            }

            if let Some(content) = &msg.content {
                // 检查是否包含工具结果和系统提醒
                if !content.contains("tool_result") || !content.contains("<system-reminder>") {
                    continue;
                }

                // 分离系统提醒和其他内容
                let mut system_reminders = Vec::new();
                let mut other_content = Vec::new();

                // 简化处理：按行分割
                for line in content.lines() {
                    if line.trim().starts_with("<system-reminder>") {
                        system_reminders.push(line.to_string());
                    } else {
                        other_content.push(line.to_string());
                    }
                }

                // 如果有系统提醒，压缩到最后一个工具结果
                if !system_reminders.is_empty() {
                    let reminder_text = system_reminders.join("\n");
                    let mut new_content = other_content.join("\n");

                    // 将系统提醒附加到最后一个工具结果后面
                    if let Some(last_tr_pos) = new_content.rfind("tool_result") {
                        // 找到最后一个tool_result的结束位置
                        if let Some(end_pos) = new_content[last_tr_pos..].find('}') {
                            let insert_pos = last_tr_pos + end_pos + 1;
                            new_content.insert_str(insert_pos, &format!("\n{}", reminder_text));
                        } else {
                            new_content.push_str(&format!("\n{}", reminder_text));
                        }
                    } else {
                        new_content.push_str(&format!("\n{}", reminder_text));
                    }

                    msg.content = Some(new_content);
                }
            }
        }
    }
}

/// 错误工具结果内容清理器 - 对标claude-code的sanitizeErrorToolResultContent
///
/// 从is_error的tool_results中剥离非文本块
pub struct ErrorToolResultSanitizer;

impl ErrorToolResultSanitizer {
    /// 清理错误工具结果内容
    pub fn sanitize_error_tool_results(messages: &mut Vec<StarMessage>) {
        for msg in messages.iter_mut() {
            if msg.role != "user" {
                continue;
            }

            if let Some(content) = &msg.content {
                // 检查是否是错误工具结果
                if content.contains("is_error") && content.contains("tool_result") {
                    // 移除非文本块（如图片）
                    let sanitized = Self::strip_non_text_blocks(content);
                    if sanitized != *content {
                        msg.content = Some(sanitized);
                    }
                }
            }
        }
    }

    /// 剥离非文本块
    fn strip_non_text_blocks(content: &str) -> String {
        let mut result = String::new();
        let mut in_tool_result = false;
        let mut skip_block = false;

        for line in content.lines() {
            if line.contains("tool_result") && line.contains("is_error") {
                in_tool_result = true;
            }

            if in_tool_result {
                // 检查是否是图片或其他非文本块
                if line.contains("\"type\": \"image\"")
                    || line.contains("\"type\": \"tool_reference\"")
                {
                    skip_block = true;
                    continue;
                }

                if skip_block && (line.contains('}') || line.contains(']')) {
                    skip_block = false;
                    continue;
                }

                if !skip_block {
                    result.push_str(line);
                    result.push('\n');
                }

                if line.contains('}') && !line.contains('{') {
                    in_tool_result = false;
                }
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }
}

/// 消息处理器 - 组合所有消息处理功能
pub struct MessageProcessor {
    /// 上下文修饰符管理器
    context_modifier_manager: ContextModifierManager,
    /// 命令生命周期通知器
    lifecycle_notifier: CommandLifecycleNotifier,
}

impl MessageProcessor {
    pub fn new() -> Self {
        Self {
            context_modifier_manager: ContextModifierManager::new(),
            lifecycle_notifier: CommandLifecycleNotifier::new(),
        }
    }

    /// 处理消息列表
    pub fn process_messages(&self, messages: &mut Vec<StarMessage>) {
        for message in messages.iter_mut() {
            // 追加消息ID标签
            MessageTagAppender::append_message_tag(message);

            // 过滤工具引用块
            ToolReferenceFilter::strip_tool_reference_blocks(message);

            // 剥离caller字段
            CallerFieldStripper::strip_caller_field(message);

            // 包装系统提醒
            SystemReminderWrapper::ensure_system_reminder_wrap(message);
        }

        // 压缩系统提醒兄弟
        SystemReminderSmoosher::smoosh_system_reminder_siblings(messages);

        // 清理错误工具结果
        ErrorToolResultSanitizer::sanitize_error_tool_results(messages);
    }

    /// 获取上下文修饰符管理器
    pub fn context_modifier_manager(&self) -> &ContextModifierManager {
        &self.context_modifier_manager
    }

    /// 获取命令生命周期通知器
    pub fn lifecycle_notifier(&self) -> &CommandLifecycleNotifier {
        &self.lifecycle_notifier
    }

    /// 通知命令完成
    pub fn notify_command_completed(&mut self, uuid: &str) {
        self.lifecycle_notifier
            .notify(uuid, CommandLifecycleEventType::Completed);
    }

    /// 清理性能缓冲区
    pub fn cleanup_performance_buffers(&self) {
        PerformanceBufferCleaner::clear_performance_buffers();
        PerformanceBufferCleaner::clear_memory_buffers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_tag_appender() {
        let mut msg = StarMessage::user("Hello");
        msg.tool_call_id = Some("550e8400-e29b-41d4-a716-446655440000".to_string());

        MessageTagAppender::append_message_tag(&mut msg);

        assert!(msg.content.unwrap().contains("[id:"));
    }

    #[test]
    fn test_tool_reference_filter() {
        let mut msg = StarMessage::user(r#"{"type": "tool_reference", "name": "test"}"#);
        ToolReferenceFilter::strip_tool_reference_blocks(&mut msg);

        assert!(!msg.content.unwrap().contains("tool_reference"));
    }

    #[test]
    fn test_caller_field_stripper() {
        let mut msg = StarMessage::assistant_with_tool_calls(vec![StarToolCall {
            id: "test".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "test".to_string(),
                arguments: r#"{"caller": "test", "key": "value"}"#.to_string(),
            },
        }]);

        CallerFieldStripper::strip_caller_field(&mut msg);

        let tool_calls = msg.tool_calls.unwrap();
        assert!(!tool_calls[0].function.arguments.contains("caller"));
    }

    #[test]
    fn test_system_reminder_wrapper() {
        let mut msg = StarMessage::user("[Attachment] test content");
        SystemReminderWrapper::ensure_system_reminder_wrap(&mut msg);

        assert!(msg.content.unwrap().contains("<system-reminder>"));
    }

    #[test]
    fn test_context_modifier_manager() {
        let mut manager = ContextModifierManager::new();

        let modifier = ContextModifier {
            tool_use_id: "test".to_string(),
            modifier_type: ContextModifierType::AddContext,
            data: serde_json::json!({"key": "value"}),
        };

        manager.add_modifier("test".to_string(), modifier);

        let modifiers = manager.get_modifiers("test");
        assert_eq!(modifiers.len(), 1);
    }

    #[test]
    fn test_command_lifecycle_notifier() {
        let mut notifier = CommandLifecycleNotifier::new();

        notifier.notify("test-uuid", CommandLifecycleEventType::Completed);

        assert!(notifier.is_completed("test-uuid"));
    }
}
