use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// 事件类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EventType {
    /// 工具调用
    ToolCall,
    /// 工具结果
    ToolResult,
    /// 用户输入
    UserInput,
    /// 模型响应
    ModelResponse,
    /// 错误事件
    Error,
    /// 性能事件
    Performance,
    /// 会话事件
    Session,
    /// 配置变更
    ConfigChange,
    /// 压缩事件
    Compaction,
    /// 认证事件
    Auth,
    /// 自定义事件
    Custom(String),
}

/// 事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// 事件ID
    pub id: String,
    /// 事件类型
    pub event_type: EventType,
    /// 时间戳（毫秒）
    pub timestamp: u64,
    /// 会话ID
    pub session_id: Option<String>,
    /// 用户ID
    pub user_id: Option<String>,
    /// 事件数据
    pub data: HashMap<String, serde_json::Value>,
    /// 事件标签
    pub tags: Vec<String>,
    /// 持续时间（毫秒，用于性能事件）
    pub duration_ms: Option<u64>,
}

impl Event {
    /// 创建新事件
    pub fn new(event_type: EventType) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            timestamp,
            session_id: None,
            user_id: None,
            data: HashMap::new(),
            tags: Vec::new(),
            duration_ms: None,
        }
    }

    /// 设置会话ID
    pub fn with_session_id(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    /// 设置用户ID
    pub fn with_user_id(mut self, user_id: &str) -> Self {
        self.user_id = Some(user_id.to_string());
        self
    }

    /// 添加数据
    pub fn with_data(mut self, key: &str, value: serde_json::Value) -> Self {
        self.data.insert(key.to_string(), value);
        self
    }

    /// 添加标签
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// 设置持续时间
    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// 获取数据字段
    pub fn get_data(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }

    /// 检查是否有标签
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(&tag.to_string())
    }
}

/// 事件日志器
///
/// 记录和管理系统事件
pub struct EventLogger {
    /// 事件缓冲区
    buffer: Vec<Event>,
    /// 最大缓冲区大小
    max_buffer_size: usize,
    /// 事件计数器
    counters: HashMap<EventType, u64>,
    /// 是否启用
    enabled: bool,
}

impl EventLogger {
    /// 创建新的事件日志器
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(max_buffer_size),
            max_buffer_size,
            counters: HashMap::new(),
            enabled: true,
        }
    }

    /// 记录事件
    pub fn log(&mut self, event: Event) {
        if !self.enabled {
            return;
        }

        // 更新计数器
        let counter = self.counters.entry(event.event_type.clone()).or_insert(0);
        *counter += 1;

        // 添加到缓冲区
        if self.buffer.len() >= self.max_buffer_size {
            // 移除最旧的事件
            self.buffer.remove(0);
        }
        self.buffer.push(event);
    }

    /// 创建并记录事件
    pub fn log_event(&mut self, event_type: EventType) -> &mut Event {
        let event = Event::new(event_type);
        self.log(event);
        self.buffer.last_mut().unwrap()
    }

    /// 获取所有事件
    pub fn events(&self) -> &[Event] {
        &self.buffer
    }

    /// 获取指定类型的事件
    pub fn events_by_type(&self, event_type: &EventType) -> Vec<&Event> {
        self.buffer
            .iter()
            .filter(|e| &e.event_type == event_type)
            .collect()
    }

    /// 获取事件计数
    pub fn count_by_type(&self, event_type: &EventType) -> u64 {
        self.counters.get(event_type).copied().unwrap_or(0)
    }

    /// 获取总事件数
    pub fn total_count(&self) -> u64 {
        self.counters.values().sum()
    }

    /// 清空缓冲区
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.counters.clear();
    }

    /// 刷新缓冲区（返回所有事件并清空）
    pub fn flush(&mut self) -> Vec<Event> {
        let events = self.buffer.drain(..).collect();
        self.counters.clear();
        events
    }

    /// 启用或禁用日志器
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 获取缓冲区使用率
    pub fn buffer_usage(&self) -> f64 {
        self.buffer.len() as f64 / self.max_buffer_size as f64
    }
}

/// 事件日志管理器
///
/// 管理多个事件日志器和事件处理
pub struct EventLogManager {
    /// 主日志器
    logger: EventLogger,
    /// 事件过滤器
    filters: Vec<EventFilter>,
    /// 事件处理器
    handlers: Vec<Box<dyn EventHandler>>,
}

/// 事件过滤器
#[derive(Debug, Clone)]
pub struct EventFilter {
    /// 过滤的事件类型
    pub event_type: Option<EventType>,
    /// 过滤的标签
    pub tag: Option<String>,
    /// 是否包含（true）或排除（false）
    pub include: bool,
}

impl EventFilter {
    /// 检查事件是否通过过滤器
    pub fn matches(&self, event: &Event) -> bool {
        let type_matches = match &self.event_type {
            Some(t) => event.event_type == *t,
            None => true,
        };

        let tag_matches = match &self.tag {
            Some(t) => event.has_tag(t),
            None => true,
        };

        let matches = type_matches && tag_matches;

        if self.include {
            matches
        } else {
            !matches
        }
    }
}

/// 事件处理器 trait
pub trait EventHandler: Send + Sync {
    /// 处理事件
    fn handle(&self, event: &Event);

    /// 处理器名称
    fn name(&self) -> &str;
}

impl EventLogManager {
    /// 创建新的事件日志管理器
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            logger: EventLogger::new(max_buffer_size),
            filters: Vec::new(),
            handlers: Vec::new(),
        }
    }

    /// 添加过滤器
    pub fn add_filter(&mut self, filter: EventFilter) {
        self.filters.push(filter);
    }

    /// 添加处理器
    pub fn add_handler(&mut self, handler: Box<dyn EventHandler>) {
        self.handlers.push(handler);
    }

    /// 记录事件（经过过滤和处理）
    pub fn log(&mut self, event: Event) {
        // 检查过滤器
        for filter in &self.filters {
            if !filter.matches(&event) {
                return;
            }
        }

        // 处理事件
        for handler in &self.handlers {
            handler.handle(&event);
        }

        // 记录事件
        self.logger.log(event);
    }

    /// 获取日志器引用
    pub fn logger(&self) -> &EventLogger {
        &self.logger
    }

    /// 获取可变日志器引用
    pub fn logger_mut(&mut self) -> &mut EventLogger {
        &mut self.logger
    }
}
