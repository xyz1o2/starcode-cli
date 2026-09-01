/// Hook事件系统
/// 
/// 对标claude-code-main的src/utils/hooks/hookEvents.ts
/// 提供Hook执行事件的广播和处理

pub mod events;
pub mod handler;

pub use events::{HookEvent, HookEventType, HookExecutionEvent};
pub use handler::{HookEventHandler, HookEventRegistry};

use serde::{Deserialize, Serialize};

/// Hook事件配置
#[derive(Debug, Clone)]
pub struct HookEventConfig {
    /// 是否启用事件广播
    pub enabled: bool,
    /// 最大待处理事件数
    pub max_pending_events: usize,
    /// 是否启用所有事件类型
    pub all_events_enabled: bool,
    /// 始终启用的事件类型
    pub always_emitted_events: Vec<String>,
}

impl Default for HookEventConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_pending_events: 100,
            all_events_enabled: false,
            always_emitted_events: vec![
                "SessionStart".to_string(),
                "Setup".to_string(),
            ],
        }
    }
}

impl HookEventConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_HOOK_EVENTS_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_pending_events = std::env::var("STAR_HOOK_EVENTS_MAX_PENDING")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let all_events_enabled = std::env::var("STAR_HOOK_EVENTS_ALL")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            enabled,
            max_pending_events,
            all_events_enabled,
            always_emitted_events: vec![
                "SessionStart".to_string(),
                "Setup".to_string(),
            ],
        }
    }
}

/// Hook事件管理器
pub struct HookEventManager {
    config: HookEventConfig,
    registry: HookEventRegistry,
}

impl HookEventManager {
    /// 创建新的Hook事件管理器
    pub fn new(config: HookEventConfig) -> Self {
        let registry = HookEventRegistry::new(config.max_pending_events);
        Self { config, registry }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(HookEventConfig::from_env())
    }

    /// 注册事件处理器
    pub fn register_handler(&mut self, handler: Box<dyn HookEventHandler>) {
        self.registry.register_handler(handler);
    }

    /// 发送Hook开始事件
    pub fn emit_hook_started(&mut self, hook_id: &str, hook_name: &str, hook_event: &str) {
        if !self.config.enabled {
            return;
        }

        if !self.should_emit(hook_event) {
            return;
        }

        let event = HookExecutionEvent::Started {
            hook_id: hook_id.to_string(),
            hook_name: hook_name.to_string(),
            hook_event: hook_event.to_string(),
        };

        self.registry.emit(event);
    }

    /// 发送Hook进度事件
    pub fn emit_hook_progress(
        &mut self,
        hook_id: &str,
        hook_name: &str,
        hook_event: &str,
        stdout: &str,
        stderr: &str,
        output: &str,
    ) {
        if !self.config.enabled {
            return;
        }

        if !self.should_emit(hook_event) {
            return;
        }

        let event = HookExecutionEvent::Progress {
            hook_id: hook_id.to_string(),
            hook_name: hook_name.to_string(),
            hook_event: hook_event.to_string(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            output: output.to_string(),
        };

        self.registry.emit(event);
    }

    /// 发送Hook响应事件
    pub fn emit_hook_response(
        &mut self,
        hook_id: &str,
        hook_name: &str,
        hook_event: &str,
        output: &str,
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        outcome: &str,
    ) {
        if !self.config.enabled {
            return;
        }

        // 记录到调试日志
        let output_to_log = if !stdout.is_empty() {
            stdout
        } else if !stderr.is_empty() {
            stderr
        } else {
            output
        };

        if !output_to_log.is_empty() {
            log::debug!(
                "Hook {} ({}) {}: {}",
                hook_name,
                hook_event,
                outcome,
                output_to_log
            );
        }

        if !self.should_emit(hook_event) {
            return;
        }

        let event = HookExecutionEvent::Response {
            hook_id: hook_id.to_string(),
            hook_name: hook_name.to_string(),
            hook_event: hook_event.to_string(),
            output: output.to_string(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
            outcome: outcome.to_string(),
        };

        self.registry.emit(event);
    }

    /// 检查是否应该发送事件
    fn should_emit(&self, hook_event: &str) -> bool {
        // 始终发送的事件
        if self.config.always_emitted_events.contains(&hook_event.to_string()) {
            return true;
        }

        // 如果启用了所有事件
        if self.config.all_events_enabled {
            return true;
        }

        false
    }

    /// 启用所有事件
    pub fn set_all_events_enabled(&mut self, enabled: bool) {
        self.config.all_events_enabled = enabled;
    }

    /// 清理状态
    pub fn clear(&mut self) {
        self.registry.clear();
    }
}
