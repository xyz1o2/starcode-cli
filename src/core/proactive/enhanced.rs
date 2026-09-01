//! Proactive 主动模式模块
//!
//! 对标 Claude Code 的 proactive.md：
//! - Tick 驱动自主代理
//! - SleepTool 控制节奏
//! - 主动建议系统

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Proactive 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveConfig {
    /// 是否启用
    pub enabled: bool,
    /// Tick 间隔（秒）
    pub tick_interval_secs: u64,
    /// 最大空闲 ticks
    pub max_idle_ticks: u32,
    /// 主动建议阈值
    pub suggestion_threshold: f32,
    /// 是否自动执行建议
    pub auto_execute: bool,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tick_interval_secs: 60,
            max_idle_ticks: 10,
            suggestion_threshold: 0.7,
            auto_execute: false,
        }
    }
}

/// Proactive 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveState {
    pub active: bool,
    pub tick_count: u32,
    pub idle_ticks: u32,
    pub suggestions_made: u32,
    pub suggestions_accepted: u32,
    pub last_tick_at: Option<u64>,
    pub current_suggestion: Option<String>,
}

impl Default for ProactiveState {
    fn default() -> Self {
        Self {
            active: false,
            tick_count: 0,
            idle_ticks: 0,
            suggestions_made: 0,
            suggestions_accepted: 0,
            last_tick_at: None,
            current_suggestion: None,
        }
    }
}

/// 主动建议
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveSuggestion {
    pub id: String,
    pub suggestion_type: SuggestionType,
    pub title: String,
    pub description: String,
    pub confidence: f32,
    pub auto_executable: bool,
    pub action: Option<SuggestedAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuggestionType {
    /// 运行测试
    RunTests,
    /// 检查错误
    CheckErrors,
    /// 代码审查
    CodeReview,
    /// 性能优化
    PerformanceOptimization,
    /// 文档更新
    DocumentationUpdate,
    /// 依赖更新
    DependencyUpdate,
    /// 安全检查
    SecurityCheck,
    /// 自定义
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub tool: String,
    pub params: Value,
    pub description: String,
}

/// Proactive 管理器
pub struct ProactiveManager {
    config: ProactiveConfig,
    state: Arc<Mutex<ProactiveState>>,
    suggestions: Vec<ProactiveSuggestion>,
    last_analysis: Option<Instant>,
}

impl ProactiveManager {
    pub fn new(config: ProactiveConfig) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(ProactiveState::default())),
            suggestions: Vec::new(),
            last_analysis: None,
        }
    }

    /// 启动 proactive 模式
    pub fn start(&self) {
        let mut state = self.state.lock().unwrap();
        state.active = true;
    }

    /// 停止 proactive 模式
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.active = false;
    }

    /// 执行一次 tick
    pub fn tick(&mut self) -> Option<ProactiveSuggestion> {
        if !self.config.enabled {
            return None;
        }

        let mut state = self.state.lock().unwrap();
        if !state.active {
            return None;
        }

        state.tick_count += 1;
        state.idle_ticks += 1;
        state.last_tick_at = Some(now_secs());

        // 检查是否超过空闲阈值
        if state.idle_ticks >= self.config.max_idle_ticks {
            state.idle_ticks = 0;
            return self.generate_suggestion(&state);
        }

        None
    }

    /// 生成建议
    fn generate_suggestion(&self, state: &ProactiveState) -> Option<ProactiveSuggestion> {
        // 基于 tick 数量和历史生成建议
        let suggestion = match state.tick_count % 7 {
            0 => ProactiveSuggestion {
                id: uuid::Uuid::new_v4().to_string(),
                suggestion_type: SuggestionType::RunTests,
                title: "Run tests".to_string(),
                description: "It's been a while since tests were run. Would you like to run them?".to_string(),
                confidence: 0.8,
                auto_executable: true,
                action: Some(SuggestedAction {
                    tool: "Bash".to_string(),
                    params: json!({"command": "cargo test"}),
                    description: "Run project tests".to_string(),
                }),
            },
            1 => ProactiveSuggestion {
                id: uuid::Uuid::new_v4().to_string(),
                suggestion_type: SuggestionType::CheckErrors,
                title: "Check for errors".to_string(),
                description: "Would you like me to check for any errors or warnings in the codebase?".to_string(),
                confidence: 0.7,
                auto_executable: true,
                action: Some(SuggestedAction {
                    tool: "Bash".to_string(),
                    params: json!({"command": "cargo check"}),
                    description: "Run cargo check".to_string(),
                }),
            },
            _ => return None,
        };

        Some(suggestion)
    }

    /// 记录用户接受了建议
    pub fn accept_suggestion(&mut self, suggestion_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.suggestions_accepted += 1;
        state.idle_ticks = 0;
    }

    /// 记录用户拒绝了建议
    pub fn reject_suggestion(&mut self, suggestion_id: &str) {
        let mut state = self.state.lock().unwrap();
        state.idle_ticks = 0;
    }

    /// 通知有活动发生
    pub fn notify_activity(&self) {
        let mut state = self.state.lock().unwrap();
        state.idle_ticks = 0;
    }

    /// 获取状态
    pub fn state(&self) -> ProactiveState {
        self.state.lock().unwrap().clone()
    }

    /// 获取当前建议
    pub fn current_suggestion(&self) -> Option<&ProactiveSuggestion> {
        self.suggestions.last()
    }

    /// 清除建议
    pub fn clear_suggestions(&mut self) {
        self.suggestions.clear();
    }
}

/// SleepTool — 控制 proactive 节奏
pub struct SleepTool;

impl SleepTool {
    pub fn new() -> Self {
        Self
    }

    /// 休眠指定时间
    pub async fn sleep(duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    /// 休眠直到条件满足
    pub async fn sleep_until(
        duration: Duration,
        check: impl Fn() -> bool,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < duration {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
