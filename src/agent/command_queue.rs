use crate::types::StarMessage;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

/// 队列命令管理器 - 对标claude-code的队列命令系统
pub struct CommandQueue {
    /// 命令队列
    commands: VecDeque<QueuedCommand>,
    /// 命令生命周期通知
    lifecycle_notifications: HashMap<String, CommandLifecycle>,
}

/// 队列命令
#[derive(Debug, Clone)]
pub struct QueuedCommand {
    /// 命令UUID
    pub uuid: String,
    /// 命令模式
    pub mode: CommandMode,
    /// Agent ID
    pub agent_id: Option<String>,
    /// 命令内容
    pub content: String,
    /// 优先级
    pub priority: CommandPriority,
    /// 创建时间
    pub created_at: u64,
}

/// 命令模式
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMode {
    /// 用户提示
    Prompt,
    /// 任务通知
    TaskNotification,
    /// 斜杠命令
    SlashCommand,
}

/// 命令优先级
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandPriority {
    /// 下一个
    Next,
    /// 稍后
    Later,
    /// 高
    High,
}

/// 命令生命周期
#[derive(Debug, Clone)]
pub enum CommandLifecycle {
    /// 已创建
    Created,
    /// 已开始
    Started,
    /// 已完成
    Completed,
    /// 已失败
    Failed,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self {
            commands: VecDeque::new(),
            lifecycle_notifications: HashMap::new(),
        }
    }

    /// 添加命令
    pub fn enqueue(&mut self, command: QueuedCommand) {
        self.commands.push_back(command);
    }

    /// 移除命令
    pub fn remove(&mut self, uuid: &str) {
        self.commands.retain(|cmd| cmd.uuid != uuid);
    }

    /// 按优先级获取命令
    pub fn get_commands_by_max_priority(
        &self,
        max_priority: CommandPriority,
    ) -> Vec<&QueuedCommand> {
        self.commands
            .iter()
            .filter(|cmd| cmd.priority <= max_priority)
            .collect()
    }

    /// 过滤命令
    pub fn filter_commands<F>(&self, predicate: F) -> Vec<&QueuedCommand>
    where
        F: Fn(&QueuedCommand) -> bool,
    {
        self.commands.iter().filter(|cmd| predicate(cmd)).collect()
    }

    /// 检查是否是斜杠命令
    pub fn is_slash_command(command: &QueuedCommand) -> bool {
        command.mode == CommandMode::SlashCommand
    }

    /// 通知命令生命周期
    pub fn notify_lifecycle(&mut self, uuid: &str, lifecycle: CommandLifecycle) {
        self.lifecycle_notifications
            .insert(uuid.to_string(), lifecycle);
    }

    /// 消费命令
    pub fn consume(&mut self, uuid: &str) -> Option<QueuedCommand> {
        if let Some(pos) = self.commands.iter().position(|cmd| cmd.uuid == uuid) {
            self.commands.remove(pos)
        } else {
            None
        }
    }

    /// 批量消费命令
    pub fn consume_batch(&mut self, uuids: &[String]) -> Vec<QueuedCommand> {
        let mut consumed = Vec::new();
        for uuid in uuids {
            if let Some(cmd) = self.consume(uuid) {
                consumed.push(cmd);
            }
        }
        consumed
    }

    /// 清除过期命令
    pub fn clear_stale(&mut self, max_age_secs: u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.commands
            .retain(|cmd| now.saturating_sub(cmd.created_at) < max_age_secs);
    }
}

/// 附件消息管理器 - 对标claude-code的附件消息系统
pub struct AttachmentManager {
    /// 附件队列
    attachments: Vec<Attachment>,
    /// 内存预取状态
    memory_prefetch: Option<MemoryPrefetch>,
    /// 技能发现预取状态
    skill_prefetch: Option<SkillPrefetch>,
    /// 工具发现预取状态
    tool_prefetch: Option<ToolPrefetch>,
}

/// 附件
#[derive(Debug, Clone)]
pub struct Attachment {
    /// 附件类型
    pub attachment_type: AttachmentType,
    /// 附件内容
    pub content: String,
    /// 相关工具使用ID
    pub tool_use_id: Option<String>,
    /// 创建时间
    pub created_at: u64,
}

/// 附件类型
#[derive(Debug, Clone)]
pub enum AttachmentType {
    /// 编辑的文本文件
    EditedTextFile,
    /// 内存文件
    MemoryFile,
    /// 技能发现
    SkillDiscovery,
    /// 工具发现
    ToolDiscovery,
    /// 结构化输出
    StructuredOutput,
    /// Hook附件
    HookAttachment,
}

/// 内存预取
#[derive(Debug, Clone)]
pub struct MemoryPrefetch {
    /// 预取的内存文件
    pub files: Vec<MemoryFile>,
    /// 是否已解决
    pub settled: bool,
    /// 消费的迭代
    pub consumed_on_iteration: Option<usize>,
}

/// 内存文件
#[derive(Debug, Clone)]
pub struct MemoryFile {
    /// 文件路径
    pub path: String,
    /// 文件内容
    pub content: String,
    /// 相关性分数
    pub relevance_score: f64,
}

/// 技能发现预取
#[derive(Debug, Clone)]
pub struct SkillPrefetch {
    /// 发现的技能
    pub skills: Vec<DiscoveredSkill>,
    /// 是否已解决
    pub settled: bool,
    /// 消费的迭代
    pub consumed_on_iteration: Option<usize>,
}

/// 发现的技能
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// 技能名称
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 相关性分数
    pub relevance_score: f64,
}

/// 工具发现预取
#[derive(Debug, Clone)]
pub struct ToolPrefetch {
    /// 发现的工具
    pub tools: Vec<DiscoveredTool>,
    /// 是否已解决
    pub settled: bool,
    /// 消费的迭代
    pub consumed_on_iteration: Option<usize>,
}

/// 发现的工具
#[derive(Debug, Clone)]
pub struct DiscoveredTool {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 相关性分数
    pub relevance_score: f64,
}

impl AttachmentManager {
    pub fn new() -> Self {
        Self {
            attachments: Vec::new(),
            memory_prefetch: None,
            skill_prefetch: None,
            tool_prefetch: None,
        }
    }

    /// 添加附件
    pub fn add_attachment(&mut self, attachment: Attachment) {
        self.attachments.push(attachment);
    }

    /// 获取所有附件
    pub fn get_attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    /// 清除附件
    pub fn clear_attachments(&mut self) {
        self.attachments.clear();
    }

    /// 设置内存预取
    pub fn set_memory_prefetch(&mut self, prefetch: MemoryPrefetch) {
        self.memory_prefetch = Some(prefetch);
    }

    /// 消费内存预取
    pub fn consume_memory_prefetch(&mut self, iteration: usize) -> Vec<MemoryFile> {
        if let Some(ref mut prefetch) = self.memory_prefetch {
            if prefetch.settled && prefetch.consumed_on_iteration.is_none() {
                prefetch.consumed_on_iteration = Some(iteration);
                return prefetch.files.clone();
            }
        }
        Vec::new()
    }

    /// 设置技能发现预取
    pub fn set_skill_prefetch(&mut self, prefetch: SkillPrefetch) {
        self.skill_prefetch = Some(prefetch);
    }

    /// 消费技能发现预取
    pub fn consume_skill_prefetch(&mut self, iteration: usize) -> Vec<DiscoveredSkill> {
        if let Some(ref mut prefetch) = self.skill_prefetch {
            if prefetch.settled && prefetch.consumed_on_iteration.is_none() {
                prefetch.consumed_on_iteration = Some(iteration);
                return prefetch.skills.clone();
            }
        }
        Vec::new()
    }

    /// 设置工具发现预取
    pub fn set_tool_prefetch(&mut self, prefetch: ToolPrefetch) {
        self.tool_prefetch = Some(prefetch);
    }

    /// 消费工具发现预取
    pub fn consume_tool_prefetch(&mut self, iteration: usize) -> Vec<DiscoveredTool> {
        if let Some(ref mut prefetch) = self.tool_prefetch {
            if prefetch.settled && prefetch.consumed_on_iteration.is_none() {
                prefetch.consumed_on_iteration = Some(iteration);
                return prefetch.tools.clone();
            }
        }
        Vec::new()
    }

    /// 过滤重复的内存附件
    pub fn filter_duplicate_memory_attachments(
        &self,
        files: Vec<MemoryFile>,
        read_state: &HashSet<String>,
    ) -> Vec<MemoryFile> {
        files
            .into_iter()
            .filter(|f| !read_state.contains(&f.path))
            .collect()
    }
}

/// 工具刷新管理器 - 对标claude-code的工具刷新机制
pub struct ToolRefreshManager {
    /// 最后刷新时间
    last_refresh: Option<std::time::Instant>,
    /// 刷新间隔
    refresh_interval: std::time::Duration,
    /// 工具缓存
    tool_cache: HashMap<String, ToolInfo>,
}

/// 工具信息
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 是否是MCP工具
    pub is_mcp: bool,
    /// MCP服务器名称
    pub mcp_server: Option<String>,
}

impl ToolRefreshManager {
    pub fn new() -> Self {
        Self {
            last_refresh: None,
            refresh_interval: std::time::Duration::from_secs(30),
            tool_cache: HashMap::new(),
        }
    }

    /// 检查是否需要刷新
    pub fn needs_refresh(&self) -> bool {
        match self.last_refresh {
            Some(last) => last.elapsed() >= self.refresh_interval,
            None => true,
        }
    }

    /// 刷新工具
    pub fn refresh(&mut self, tools: Vec<ToolInfo>) -> bool {
        let old_count = self.tool_cache.len();
        self.tool_cache.clear();
        for tool in tools {
            self.tool_cache.insert(tool.name.clone(), tool);
        }
        self.last_refresh = Some(std::time::Instant::now());
        self.tool_cache.len() != old_count
    }

    /// 获取工具
    pub fn get_tool(&self, name: &str) -> Option<&ToolInfo> {
        self.tool_cache.get(name)
    }

    /// 获取所有工具
    pub fn get_all_tools(&self) -> Vec<&ToolInfo> {
        self.tool_cache.values().collect()
    }
}

/// 周期性任务摘要管理器 - 对标claude-code的任务摘要系统
pub struct TaskSummaryManager {
    /// 最后生成时间
    last_generated: Option<std::time::Instant>,
    /// 生成间隔
    generation_interval: std::time::Duration,
    /// 当前摘要
    current_summary: Option<String>,
}

impl TaskSummaryManager {
    pub fn new() -> Self {
        Self {
            last_generated: None,
            generation_interval: std::time::Duration::from_secs(300), // 5分钟
            current_summary: None,
        }
    }

    /// 检查是否应该生成摘要
    pub fn should_generate(&self) -> bool {
        match self.last_generated {
            Some(last) => last.elapsed() >= self.generation_interval,
            None => true,
        }
    }

    /// 生成摘要
    pub fn generate_summary(&mut self, messages: &[StarMessage]) -> Option<String> {
        if !self.should_generate() {
            return self.current_summary.clone();
        }

        // 简单的摘要生成逻辑
        let mut summary = String::new();
        let recent_messages: Vec<_> = messages.iter().rev().take(10).collect();

        for msg in recent_messages.iter().rev() {
            if let Some(content) = &msg.content {
                let preview: String = content.chars().take(100).collect();
                summary.push_str(&format!("{}: {}\n", msg.role, preview));
            }
        }

        self.current_summary = Some(summary.clone());
        self.last_generated = Some(std::time::Instant::now());
        Some(summary)
    }

    /// 获取当前摘要
    pub fn get_current_summary(&self) -> Option<&str> {
        self.current_summary.as_deref()
    }
}

/// 附件消息创建器
pub struct AttachmentMessageCreator;

impl AttachmentMessageCreator {
    /// 创建编辑的文本文件附件消息
    pub fn create_edited_text_file_attachment(
        file_path: &str,
        old_content: &str,
        new_content: &str,
    ) -> StarMessage {
        let content = format!(
            "[Edited Text File]\nPath: {}\nOld content length: {}\nNew content length: {}",
            file_path,
            old_content.len(),
            new_content.len()
        );
        StarMessage::system(&content)
    }

    /// 创建内存文件附件消息
    pub fn create_memory_file_attachment(file_path: &str, content: &str) -> StarMessage {
        let msg_content = format!(
            "[Memory File]\nPath: {}\nContent length: {}",
            file_path,
            content.len()
        );
        StarMessage::system(&msg_content)
    }

    /// 创建技能发现附件消息
    pub fn create_skill_discovery_attachment(skills: &[DiscoveredSkill]) -> StarMessage {
        let mut content = String::from("[Skill Discovery]\n");
        for skill in skills {
            content.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        StarMessage::system(&content)
    }

    /// 创建工具发现附件消息
    pub fn create_tool_discovery_attachment(tools: &[DiscoveredTool]) -> StarMessage {
        let mut content = String::from("[Tool Discovery]\n");
        for tool in tools {
            content.push_str(&format!("- {}: {}\n", tool.name, tool.description));
        }
        StarMessage::system(&content)
    }

    /// 创建最大轮次达到附件消息
    pub fn create_max_turns_reached_attachment(
        max_turns: usize,
        current_turn: usize,
    ) -> StarMessage {
        StarMessage::system(&format!(
            "[Max Turns Reached]\nReached maximum turns limit: {}/{}",
            current_turn, max_turns
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_queue() {
        let mut queue = CommandQueue::new();

        let cmd = QueuedCommand {
            uuid: "test-uuid".to_string(),
            mode: CommandMode::Prompt,
            agent_id: None,
            content: "test content".to_string(),
            priority: CommandPriority::Next,
            created_at: 0,
        };

        queue.enqueue(cmd);
        assert_eq!(queue.commands.len(), 1);

        let consumed = queue.consume("test-uuid");
        assert!(consumed.is_some());
        assert_eq!(queue.commands.len(), 0);
    }

    #[test]
    fn test_attachment_manager() {
        let mut manager = AttachmentManager::new();

        let attachment = Attachment {
            attachment_type: AttachmentType::EditedTextFile,
            content: "test content".to_string(),
            tool_use_id: None,
            created_at: 0,
        };

        manager.add_attachment(attachment);
        assert_eq!(manager.get_attachments().len(), 1);

        manager.clear_attachments();
        assert_eq!(manager.get_attachments().len(), 0);
    }

    #[test]
    fn test_tool_refresh_manager() {
        let mut manager = ToolRefreshManager::new();

        assert!(manager.needs_refresh());

        let tools = vec![ToolInfo {
            name: "Read".to_string(),
            description: "Read file".to_string(),
            is_mcp: false,
            mcp_server: None,
        }];

        manager.refresh(tools);
        assert!(!manager.needs_refresh());
        assert!(manager.get_tool("Read").is_some());
    }
}
