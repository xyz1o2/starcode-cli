//! Auto Mode 模块
//!
//! 对标 Claude Code 的 Auto Mode：
//! - AI 分类器驱动的自主执行模式
//! - 两阶段分类流水线（fast + thinking）
//! - 危险权限剥离与恢复
//! - Circuit Breaker 机制

pub mod classifier;
pub mod dangerous_patterns;
pub mod prompts;

pub use classifier::{AutoModeClassifier, ClassifierDecision, ClassifierResult, ClassifierStage};
pub use dangerous_patterns::{is_dangerous_pattern, DANGEROUS_BASH_PATTERNS, DANGEROUS_PERMISSION_PATTERNS};

use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};

/// Auto Mode 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoModeState {
    /// 是否处于 auto mode
    pub active: bool,
    /// Circuit breaker 是否触发
    pub circuit_broken: bool,
    /// 被剥离的危险权限规则
    pub stripped_rules: Vec<StrippedRule>,
    /// 分类器统计
    pub stats: AutoModeStats,
    /// 是否需要退出通知
    pub needs_exit_attachment: bool,
}

/// 被剥离的权限规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrippedRule {
    pub rule_type: String,
    pub pattern: String,
    pub original_action: String,
}

/// Auto Mode 统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoModeStats {
    pub total_classifications: u64,
    pub allowed: u64,
    pub denied: u64,
    pub asked: u64,
    pub fallback_count: u64,
    pub avg_latency_ms: f64,
}

impl Default for AutoModeState {
    fn default() -> Self {
        Self {
            active: false,
            circuit_broken: false,
            stripped_rules: Vec::new(),
            stats: AutoModeStats::default(),
            needs_exit_attachment: false,
        }
    }
}

impl AutoModeState {
    /// 进入 Auto Mode
    pub fn enter(&mut self) -> Result<(), String> {
        if self.circuit_broken {
            return Err("Auto Mode is disabled by circuit breaker".to_string());
        }
        self.active = true;
        self.needs_exit_attachment = false;
        Ok(())
    }

    /// 退出 Auto Mode
    pub fn exit(&mut self) {
        self.active = false;
        self.needs_exit_attachment = true;
    }

    /// 记录分类结果
    pub fn record_classification(&mut self, result: &ClassifierResult) {
        self.stats.total_classifications += 1;
        match result.decision {
            ClassifierDecision::Allow => self.stats.allowed += 1,
            ClassifierDecision::Deny => self.stats.denied += 1,
            ClassifierDecision::Ask => self.stats.asked += 1,
        }
        if result.fallback {
            self.stats.fallback_count += 1;
        }
        // 更新平均延迟
        let n = self.stats.total_classifications as f64;
        self.stats.avg_latency_ms =
            (self.stats.avg_latency_ms * (n - 1.0) + result.latency_ms) / n;
    }

    /// 剥离危险权限
    pub fn strip_dangerous_permissions(&mut self, rules: &[String]) {
        for rule in rules {
            self.stripped_rules.push(StrippedRule {
                rule_type: "permission".to_string(),
                pattern: rule.clone(),
                original_action: "allow".to_string(),
            });
        }
    }

    /// 恢复被剥离的权限
    pub fn restore_dangerous_permissions(&mut self) -> Vec<StrippedRule> {
        std::mem::take(&mut self.stripped_rules)
    }
}

/// Auto Mode 全局状态管理器
pub struct AutoModeManager {
    state: Arc<Mutex<AutoModeState>>,
    classifier: AutoModeClassifier,
}

impl AutoModeManager {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(AutoModeState::default())),
            classifier: AutoModeClassifier::new(),
        }
    }

    /// 获取状态引用
    pub fn state(&self) -> Arc<Mutex<AutoModeState>> {
        self.state.clone()
    }

    /// 是否处于 Auto Mode
    pub fn is_active(&self) -> bool {
        self.state.lock().unwrap().active
    }

    /// 进入 Auto Mode
    pub fn enter(&self) -> Result<(), String> {
        self.state.lock().unwrap().enter()
    }

    /// 退出 Auto Mode
    pub fn exit(&self) {
        self.state.lock().unwrap().exit();
    }

    /// 分类工具调用
    pub async fn classify(
        &self,
        tool_name: &str,
        tool_params: &serde_json::Value,
        transcript: &str,
    ) -> ClassifierResult {
        let result = self.classifier.classify(tool_name, tool_params, transcript).await;
        self.state.lock().unwrap().record_classification(&result);
        result
    }

    /// 触发 Circuit Breaker
    pub fn trigger_circuit_breaker(&self) {
        let mut state = self.state.lock().unwrap();
        state.circuit_broken = true;
        state.active = false;
    }

    /// 获取统计信息
    pub fn stats(&self) -> AutoModeStats {
        self.state.lock().unwrap().stats.clone()
    }
}
