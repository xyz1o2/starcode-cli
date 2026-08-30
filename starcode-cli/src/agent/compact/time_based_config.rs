/// 基于时间的微压缩配置
/// 
/// 对标claude-code-main的timeBasedMCConfig.ts
/// 根据时间因素动态调整微压缩策略
pub struct TimeBasedCompactConfig {
    /// 是否启用基于时间的配置
    enabled: bool,
    /// 工作时间压缩阈值（小时）
    work_hours_threshold: f64,
    /// 非工作时间压缩阈值（小时）
    off_hours_threshold: f64,
    /// 工作时间开始（小时，0-23）
    work_hours_start: u8,
    /// 工作时间结束（小时，0-23）
    work_hours_end: u8,
    /// 是否考虑时区
    consider_timezone: bool,
    /// 时区偏移（小时）
    timezone_offset: i8,
}

impl TimeBasedCompactConfig {
    pub fn new() -> Self {
        Self {
            enabled: true,
            work_hours_threshold: 2.0,  // 工作时间2小时压缩一次
            off_hours_threshold: 4.0,   // 非工作时间4小时压缩一次
            work_hours_start: 9,        // 9:00 开始工作
            work_hours_end: 18,         // 18:00 结束工作
            consider_timezone: true,
            timezone_offset: 8,         // 默认UTC+8
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_TIME_BASED_COMPACT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let work_hours_threshold = std::env::var("STAR_WORK_HOURS_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2.0);

        let off_hours_threshold = std::env::var("STAR_OFF_HOURS_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4.0);

        let work_hours_start = std::env::var("STAR_WORK_HOURS_START")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9);

        let work_hours_end = std::env::var("STAR_WORK_HOURS_END")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(18);

        let consider_timezone = std::env::var("STAR_CONSIDER_TIMEZONE")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let timezone_offset = std::env::var("STAR_TIMEZONE_OFFSET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);

        Self {
            enabled,
            work_hours_threshold,
            off_hours_threshold,
            work_hours_start,
            work_hours_end,
            consider_timezone,
            timezone_offset,
        }
    }

    /// 获取当前时间的压缩阈值
    pub fn get_current_threshold(&self) -> f64 {
        if !self.enabled {
            return self.work_hours_threshold; // 默认返回工作时间阈值
        }

        let current_hour = self.get_current_hour();
        
        if self.is_work_hours(current_hour) {
            self.work_hours_threshold
        } else {
            self.off_hours_threshold
        }
    }

    /// 获取当前小时（考虑时区）
    fn get_current_hour(&self) -> u8 {
        use chrono::{Utc, FixedOffset, DateTime, Timelike};
        
        let utc_now = Utc::now();
        
        if self.consider_timezone {
            let offset = FixedOffset::east_opt(self.timezone_offset as i32 * 3600)
                .unwrap_or(FixedOffset::east_opt(0).unwrap());
            let local_time: DateTime<FixedOffset> = utc_now.with_timezone(&offset);
            local_time.hour() as u8
        } else {
            utc_now.hour() as u8
        }
    }

    /// 检查是否是工作时间
    fn is_work_hours(&self, hour: u8) -> bool {
        if self.work_hours_start <= self.work_hours_end {
            // 正常时间范围（如9:00-18:00）
            hour >= self.work_hours_start && hour < self.work_hours_end
        } else {
            // 跨午夜时间范围（如22:00-6:00）
            hour >= self.work_hours_start || hour < self.work_hours_end
        }
    }

    /// 获取压缩建议
    pub fn get_compaction_suggestion(&self, hours_since_last_compact: f64) -> CompactionSuggestion {
        let threshold = self.get_current_threshold();
        
        if hours_since_last_compact >= threshold {
            CompactionSuggestion {
                should_compact: true,
                urgency: if hours_since_last_compact >= threshold * 1.5 {
                    CompactionUrgency::High
                } else {
                    CompactionUrgency::Normal
                },
                reason: format!(
                    "Hours since last compaction: {:.1} (threshold: {:.1})",
                    hours_since_last_compact, threshold
                ),
                suggested_strategy: self.get_suggested_strategy(),
            }
        } else {
            CompactionSuggestion {
                should_compact: false,
                urgency: CompactionUrgency::Low,
                reason: format!(
                    "Hours since last compaction: {:.1} (threshold: {:.1})",
                    hours_since_last_compact, threshold
                ),
                suggested_strategy: None,
            }
        }
    }

    /// 获取建议的压缩策略
    fn get_suggested_strategy(&self) -> Option<String> {
        let current_hour = self.get_current_hour();
        
        if self.is_work_hours(current_hour) {
            // 工作时间使用更保守的策略
            Some("micro_compact".to_string())
        } else {
            // 非工作时间可以使用更激进的策略
            Some("session_memory_compact".to_string())
        }
    }

    /// 获取配置摘要
    pub fn summary(&self) -> String {
        format!(
            "TimeBasedCompactConfig(enabled={}, work_hours={}:00-{}:00, work_threshold={}h, off_hours_threshold={}h, timezone={}{})",
            self.enabled,
            self.work_hours_start,
            self.work_hours_end,
            self.work_hours_threshold,
            self.off_hours_threshold,
            if self.timezone_offset >= 0 { "+" } else { "-" },
            self.timezone_offset.abs()
        )
    }
}

/// 压缩建议
#[derive(Debug)]
pub struct CompactionSuggestion {
    /// 是否应该压缩
    pub should_compact: bool,
    /// 紧急程度
    pub urgency: CompactionUrgency,
    /// 原因
    pub reason: String,
    /// 建议的策略
    pub suggested_strategy: Option<String>,
}

/// 压缩紧急程度
#[derive(Debug, PartialEq)]
pub enum CompactionUrgency {
    Low,
    Normal,
    High,
}

/// 时间感知的压缩管理器
/// 
/// 管理基于时间的压缩策略
pub struct TimeAwareCompactManager {
    config: TimeBasedCompactConfig,
    /// 上次压缩时间
    last_compaction_time: Option<std::time::Instant>,
    /// 压缩历史
    compaction_history: Vec<CompactionRecord>,
}

/// 压缩记录
#[derive(Debug)]
struct CompactionRecord {
    timestamp: std::time::Instant,
    tokens_before: usize,
    tokens_after: usize,
    strategy: String,
}

impl TimeAwareCompactManager {
    pub fn new() -> Self {
        Self {
            config: TimeBasedCompactConfig::new(),
            last_compaction_time: None,
            compaction_history: Vec::new(),
        }
    }

    /// 检查是否需要压缩
    pub fn should_compact(&self, current_tokens: usize, max_tokens: usize) -> CompactionSuggestion {
        let hours_since_last = self.hours_since_last_compaction();
        let token_usage_ratio = current_tokens as f64 / max_tokens as f64;

        // 结合时间和token使用率
        let suggestion = self.config.get_compaction_suggestion(hours_since_last);
        
        // 如果token使用率很高，提高紧急程度
        if token_usage_ratio > 0.9 {
            CompactionSuggestion {
                should_compact: true,
                urgency: CompactionUrgency::High,
                reason: format!(
                    "Token usage at {:.1}% and {}",
                    token_usage_ratio * 100.0,
                    suggestion.reason
                ),
                suggested_strategy: suggestion.suggested_strategy,
            }
        } else {
            suggestion
        }
    }

    /// 记录压缩事件
    pub fn record_compaction(&mut self, tokens_before: usize, tokens_after: usize, strategy: &str) {
        self.last_compaction_time = Some(std::time::Instant::now());
        self.compaction_history.push(CompactionRecord {
            timestamp: std::time::Instant::now(),
            tokens_before,
            tokens_after,
            strategy: strategy.to_string(),
        });

        // 限制历史记录大小
        if self.compaction_history.len() > 100 {
            self.compaction_history.remove(0);
        }
    }

    /// 获取上次压缩后的时间（小时）
    fn hours_since_last_compaction(&self) -> f64 {
        match self.last_compaction_time {
            Some(time) => time.elapsed().as_secs() as f64 / 3600.0,
            None => f64::MAX, // 从未压缩过
        }
    }

    /// 获取压缩统计
    pub fn get_statistics(&self) -> CompactStatistics {
        let total_compactions = self.compaction_history.len();
        let total_tokens_saved: usize = self.compaction_history.iter()
            .map(|r| r.tokens_before.saturating_sub(r.tokens_after))
            .sum();

        CompactStatistics {
            total_compactions,
            total_tokens_saved,
            average_tokens_saved: if total_compactions > 0 {
                total_tokens_saved / total_compactions
            } else {
                0
            },
        }
    }
}

/// 压缩统计
#[derive(Debug)]
pub struct CompactStatistics {
    pub total_compactions: usize,
    pub total_tokens_saved: usize,
    pub average_tokens_saved: usize,
}
