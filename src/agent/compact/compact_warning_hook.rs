use crate::types::StarMessage;
use super::CompactConfig;

/// 压缩警告钩子
/// 
/// 对标claude-code-main的compactWarningHook.ts
/// 在压缩即将发生时发出警告
pub struct CompactWarningHook {
    /// 是否启用警告
    enabled: bool,
    /// 警告阈值（百分比）
    warning_threshold_percent: f64,
    /// 严重警告阈值（百分比）
    critical_threshold_percent: f64,
    /// 警告状态
    state: CompactWarningState,
}

/// 压缩警告状态
#[derive(Debug, Clone)]
pub struct CompactWarningState {
    /// 最后一次警告时间
    last_warning_time: Option<std::time::Instant>,
    /// 警告次数
    warning_count: u32,
    /// 是否已发送严重警告
    critical_warning_sent: bool,
    /// 当前使用百分比
    current_usage_percent: f64,
}

impl CompactWarningHook {
    pub fn new(config: &CompactConfig) -> Self {
        Self {
            enabled: true,
            warning_threshold_percent: 80.0,
            critical_threshold_percent: 95.0,
            state: CompactWarningState {
                last_warning_time: None,
                warning_count: 0,
                critical_warning_sent: false,
                current_usage_percent: 0.0,
            },
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_COMPACT_WARNING_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let warning_threshold = std::env::var("STAR_COMPACT_WARNING_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80.0);

        let critical_threshold = std::env::var("STAR_COMPACT_CRITICAL_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(95.0);

        Self {
            enabled,
            warning_threshold_percent: warning_threshold,
            critical_threshold_percent: critical_threshold,
            state: CompactWarningState {
                last_warning_time: None,
                warning_count: 0,
                critical_warning_sent: false,
                current_usage_percent: 0.0,
            },
        }
    }

    /// 检查是否需要警告
    pub fn check_warning(&mut self, current_tokens: usize, max_tokens: usize) -> Option<CompactWarning> {
        if !self.enabled {
            return None;
        }

        let usage_percent = (current_tokens as f64 / max_tokens as f64) * 100.0;
        self.state.current_usage_percent = usage_percent;

        // 检查是否超过严重阈值
        if usage_percent >= self.critical_threshold_percent {
            if !self.state.critical_warning_sent {
                self.state.critical_warning_sent = true;
                self.state.warning_count += 1;
                self.state.last_warning_time = Some(std::time::Instant::now());

                return Some(CompactWarning {
                    level: WarningLevel::Critical,
                    usage_percent,
                    current_tokens,
                    max_tokens,
                    message: format!(
                        "⚠️ CRITICAL: Token usage at {:.1}% ({}/{}). Compaction will be triggered soon.",
                        usage_percent, current_tokens, max_tokens
                    ),
                    recommendation: "Consider ending the conversation or requesting a summary.".to_string(),
                });
            }
        }

        // 检查是否超过警告阈值
        if usage_percent >= self.warning_threshold_percent {
            // 避免频繁警告（至少间隔60秒）
            let should_warn = match self.state.last_warning_time {
                Some(last_time) => last_time.elapsed().as_secs() > 60,
                None => true,
            };

            if should_warn {
                self.state.warning_count += 1;
                self.state.last_warning_time = Some(std::time::Instant::now());

                return Some(CompactWarning {
                    level: WarningLevel::Warning,
                    usage_percent,
                    current_tokens,
                    max_tokens,
                    message: format!(
                        "Token usage at {:.1}% ({}/{}). Compaction may be needed soon.",
                        usage_percent, current_tokens, max_tokens
                    ),
                    recommendation: "Consider using more concise responses or ending the conversation.".to_string(),
                });
            }
        }

        None
    }

    /// 重置警告状态（压缩后调用）
    pub fn reset_after_compaction(&mut self) {
        self.state.critical_warning_sent = false;
        self.state.current_usage_percent = 0.0;
    }

    /// 获取当前状态
    pub fn state(&self) -> &CompactWarningState {
        &self.state
    }

    /// 获取警告次数
    pub fn warning_count(&self) -> u32 {
        self.state.warning_count
    }
}

/// 压缩警告
#[derive(Debug, Clone)]
pub struct CompactWarning {
    /// 警告级别
    pub level: WarningLevel,
    /// 使用百分比
    pub usage_percent: f64,
    /// 当前token数
    pub current_tokens: usize,
    /// 最大token数
    pub max_tokens: usize,
    /// 警告消息
    pub message: String,
    /// 建议操作
    pub recommendation: String,
}

/// 警告级别
#[derive(Debug, Clone, PartialEq)]
pub enum WarningLevel {
    /// 普通警告
    Warning,
    /// 严重警告
    Critical,
}

impl CompactWarning {
    /// 格式化警告为用户友好的消息
    pub fn format_for_user(&self) -> String {
        match self.level {
            WarningLevel::Warning => {
                format!(
                    "⚠️ {}\n💡 {}",
                    self.message, self.recommendation
                )
            }
            WarningLevel::Critical => {
                format!(
                    "🚨 {}\n💡 {}",
                    self.message, self.recommendation
                )
            }
        }
    }

    /// 格式化警告为日志消息
    pub fn format_for_log(&self) -> String {
        format!(
            "[{:?}] {} (usage: {:.1}%, tokens: {}/{})",
            self.level, self.message, self.usage_percent, self.current_tokens, self.max_tokens
        )
    }
}

/// 压缩警告管理器
/// 
/// 管理多个警告钩子，提供统一的警告接口
pub struct CompactWarningManager {
    hook: CompactWarningHook,
    /// 警告历史
    warnings: Vec<CompactWarning>,
    /// 最大历史记录数
    max_history: usize,
}

impl CompactWarningManager {
    pub fn new(config: &CompactConfig) -> Self {
        Self {
            hook: CompactWarningHook::new(config),
            warnings: Vec::new(),
            max_history: 100,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self {
            hook: CompactWarningHook::from_env(),
            warnings: Vec::new(),
            max_history: 100,
        }
    }

    /// 检查警告并记录
    pub fn check_and_record(&mut self, current_tokens: usize, max_tokens: usize) -> Option<CompactWarning> {
        let warning = self.hook.check_warning(current_tokens, max_tokens);
        
        if let Some(w) = &warning {
            self.warnings.push(w.clone());
            
            // 限制历史记录大小
            if self.warnings.len() > self.max_history {
                self.warnings.remove(0);
            }
        }

        warning
    }

    /// 压缩后重置
    pub fn reset_after_compaction(&mut self) {
        self.hook.reset_after_compaction();
    }

    /// 获取警告历史
    pub fn warnings(&self) -> &[CompactWarning] {
        &self.warnings
    }

    /// 获取警告统计
    pub fn statistics(&self) -> WarningStatistics {
        let total_warnings = self.warnings.len();
        let critical_warnings = self.warnings.iter()
            .filter(|w| w.level == WarningLevel::Critical)
            .count();
        
        let avg_usage = if total_warnings > 0 {
            self.warnings.iter().map(|w| w.usage_percent).sum::<f64>() / total_warnings as f64
        } else {
            0.0
        };

        WarningStatistics {
            total_warnings,
            critical_warnings,
            average_usage_percent: avg_usage,
        }
    }
}

/// 警告统计
#[derive(Debug)]
pub struct WarningStatistics {
    pub total_warnings: usize,
    pub critical_warnings: usize,
    pub average_usage_percent: f64,
}
