/// Hook事件处理器

use super::events::HookExecutionEvent;

/// Hook事件处理器trait
pub trait HookEventHandler: Send + Sync {
    /// 处理事件
    fn handle(&self, event: &HookExecutionEvent);
    
    /// 处理器名称
    fn name(&self) -> &str;
}

/// Hook事件注册表
pub struct HookEventRegistry {
    /// 事件处理器
    handlers: Vec<Box<dyn HookEventHandler>>,
    /// 待处理事件
    pending_events: Vec<HookExecutionEvent>,
    /// 最大待处理事件数
    max_pending_events: usize,
}

impl HookEventRegistry {
    /// 创建新的事件注册表
    pub fn new(max_pending_events: usize) -> Self {
        Self {
            handlers: Vec::new(),
            pending_events: Vec::new(),
            max_pending_events,
        }
    }

    /// 注册事件处理器
    pub fn register_handler(&mut self, handler: Box<dyn HookEventHandler>) {
        self.handlers.push(handler);
        
        // 处理待处理的事件
        for event in self.pending_events.drain(..) {
            for handler in &self.handlers {
                handler.handle(&event);
            }
        }
    }

    /// 发送事件
    pub fn emit(&mut self, event: HookExecutionEvent) {
        if self.handlers.is_empty() {
            // 没有处理器，加入待处理队列
            self.pending_events.push(event);
            
            // 限制待处理事件数量
            if self.pending_events.len() > self.max_pending_events {
                self.pending_events.remove(0);
            }
        } else {
            // 有处理器，直接发送
            for handler in &self.handlers {
                handler.handle(&event);
            }
        }
    }

    /// 清理状态
    pub fn clear(&mut self) {
        self.pending_events.clear();
    }
}

/// 日志Hook事件处理器
pub struct LoggingHookEventHandler;

impl HookEventHandler for LoggingHookEventHandler {
    fn handle(&self, event: &HookExecutionEvent) {
        match event {
            HookExecutionEvent::Started { hook_id, hook_name, hook_event } => {
                log::info!("Hook started: {} ({}) - {}", hook_name, hook_id, hook_event);
            }
            HookExecutionEvent::Progress { hook_id, hook_name, hook_event, output, .. } => {
                log::debug!("Hook progress: {} ({}) - {} - {}", hook_name, hook_id, hook_event, output);
            }
            HookExecutionEvent::Response { hook_id, hook_name, hook_event, outcome, .. } => {
                log::info!("Hook response: {} ({}) - {} - {}", hook_name, hook_id, hook_event, outcome);
            }
        }
    }

    fn name(&self) -> &str {
        "logging"
    }
}
