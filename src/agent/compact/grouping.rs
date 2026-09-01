use crate::types::StarMessage;

/// 消息分组策略
/// 
/// 对标claude-code-main的grouping.ts
/// 将消息按逻辑分组，便于压缩和管理
pub struct MessageGroupingStrategy {
    /// 最大组大小
    max_group_size: usize,
    /// 是否保留组边界
    preserve_group_boundaries: bool,
}

impl MessageGroupingStrategy {
    pub fn new() -> Self {
        Self {
            max_group_size: 10,
            preserve_group_boundaries: true,
        }
    }

    /// 将消息分组
    pub fn group_messages(&self, messages: &[StarMessage]) -> Vec<MessageGroup> {
        let mut groups = Vec::new();
        let mut current_group = MessageGroup::new();

        for (i, msg) in messages.iter().enumerate() {
            // 检查是否应该开始新组
            if self.should_start_new_group(msg, &current_group, i) {
                if !current_group.is_empty() {
                    groups.push(current_group);
                }
                current_group = MessageGroup::new();
            }

            current_group.add_message(msg.clone(), i);
        }

        // 添加最后一个组
        if !current_group.is_empty() {
            groups.push(current_group);
        }

        groups
    }

    /// 检查是否应该开始新组
    fn should_start_new_group(&self, msg: &StarMessage, current_group: &MessageGroup, index: usize) -> bool {
        // 如果当前组为空，不需要开始新组
        if current_group.is_empty() {
            return false;
        }

        // 如果当前组已满，开始新组
        if current_group.len() >= self.max_group_size {
            return true;
        }

        // 用户消息通常开始新组
        if msg.role == "user" && !current_group.messages.is_empty() {
            return true;
        }

        // 系统消息通常开始新组
        if msg.role == "system" && !current_group.messages.is_empty() {
            return true;
        }

        // 工具调用结果通常与对应的助手消息在同一组
        if msg.role == "tool" {
            // 查找对应的工具调用
            if let Some(tool_call_id) = &msg.tool_call_id {
                // 检查当前组中是否有对应的工具调用
                let has_matching_call = current_group.messages.iter().any(|m| {
                    m.tool_calls.as_ref().map_or(false, |tc| {
                        tc.iter().any(|tc| tc.id == *tool_call_id)
                    })
                });

                if !has_matching_call {
                    // 没有对应的工具调用，开始新组
                    return true;
                }
            }
        }

        false
    }

    /// 压缩组
    pub fn compress_group(&self, group: &MessageGroup, target_tokens: usize) -> Vec<StarMessage> {
        // 如果组很小，直接返回
        if group.len() <= 2 {
            return group.messages.clone();
        }

        // 计算当前token数
        let current_tokens = self.estimate_group_tokens(group);
        
        if current_tokens <= target_tokens {
            return group.messages.clone();
        }

        // 需要压缩
        let compression_ratio = target_tokens as f64 / current_tokens as f64;
        let target_messages = (group.len() as f64 * compression_ratio).ceil() as usize;
        let target_messages = target_messages.max(2); // 至少保留2条消息

        // 保留第一条和最后几条消息
        let mut result = Vec::new();
        
        // 保留第一条消息（通常是用户输入）
        if let Some(first) = group.messages.first() {
            result.push(first.clone());
        }

        // 保留最后几条消息
        let skip_count = group.len() - target_messages + 1;
        for msg in group.messages.iter().skip(skip_count) {
            result.push(msg.clone());
        }

        // 添加摘要消息
        if skip_count > 0 {
            let summary = format!(
                "[{} messages summarized]",
                skip_count
            );
            result.insert(1, StarMessage::system(&summary));
        }

        result
    }

    /// 估算组的token数
    fn estimate_group_tokens(&self, group: &MessageGroup) -> usize {
        group.messages.iter().map(|msg| {
            let content_len = msg.content.as_ref().map_or(0, |c| c.len());
            let tool_calls_len = msg.tool_calls.as_ref().map_or(0, |tc| {
                tc.iter().map(|tc| tc.function.arguments.len()).sum::<usize>()
            });
            // 粗略估算：1 token ≈ 4 字符
            (content_len + tool_calls_len) / 4
        }).sum()
    }
}

/// 消息组
#[derive(Debug, Clone)]
pub struct MessageGroup {
    /// 组内的消息
    pub messages: Vec<StarMessage>,
    /// 消息在原始列表中的索引
    pub indices: Vec<usize>,
    /// 组类型
    pub group_type: GroupType,
}

impl MessageGroup {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            indices: Vec::new(),
            group_type: GroupType::Mixed,
        }
    }

    /// 添加消息到组
    pub fn add_message(&mut self, message: StarMessage, index: usize) {
        // 更新组类型
        self.group_type = self.determine_group_type(&message);
        
        self.messages.push(message);
        self.indices.push(index);
    }

    /// 确定组类型
    fn determine_group_type(&self, new_message: &StarMessage) -> GroupType {
        if self.messages.is_empty() {
            return match new_message.role.as_str() {
                "user" => GroupType::UserInput,
                "assistant" => GroupType::AssistantResponse,
                "tool" => GroupType::ToolExecution,
                "system" => GroupType::SystemMessage,
                _ => GroupType::Mixed,
            };
        }

        // 如果类型不一致，标记为混合
        let first_type = self.messages[0].role.clone();
        if new_message.role != first_type {
            return GroupType::Mixed;
        }

        self.group_type.clone()
    }

    /// 检查组是否为空
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 获取组大小
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// 获取组的起始索引
    pub fn start_index(&self) -> usize {
        self.indices.first().copied().unwrap_or(0)
    }

    /// 获取组的结束索引
    pub fn end_index(&self) -> usize {
        self.indices.last().copied().unwrap_or(0)
    }
}

/// 组类型
#[derive(Debug, Clone, PartialEq)]
pub enum GroupType {
    /// 用户输入
    UserInput,
    /// 助手响应
    AssistantResponse,
    /// 工具执行
    ToolExecution,
    /// 系统消息
    SystemMessage,
    /// 混合类型
    Mixed,
}

/// 分组压缩管理器
/// 
/// 管理消息分组和压缩
pub struct GroupingCompactManager {
    strategy: MessageGroupingStrategy,
    /// 是否启用分组压缩
    enabled: bool,
}

impl GroupingCompactManager {
    pub fn new() -> Self {
        Self {
            strategy: MessageGroupingStrategy::new(),
            enabled: true,
        }
    }

    /// 执行分组压缩
    pub fn compact_by_grouping(
        &self,
        messages: &[StarMessage],
        target_tokens: usize,
    ) -> Vec<StarMessage> {
        if !self.enabled {
            return messages.to_vec();
        }

        let groups = self.strategy.group_messages(messages);
        let group_count = groups.len();
        let mut result = Vec::new();

        for group in &groups {
            let compressed = self.strategy.compress_group(group, target_tokens / group_count);
            result.extend(compressed);
        }

        result
    }

    /// 启用或禁用分组压缩
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}
