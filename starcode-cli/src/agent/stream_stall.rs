use std::time::{Duration, Instant};

/// 流式停滞检测配置
#[derive(Debug, Clone)]
pub struct StreamStallConfig {
    /// 是否启用停滞检测
    pub enabled: bool,
    /// 停滞阈值（秒）- 超过此时间无事件视为停滞
    pub stall_threshold_secs: u64,
    /// 空闲超时（秒）- 完全无响应则终止
    pub idle_timeout_secs: u64,
    /// 最大停滞次数
    pub max_stall_count: u32,
}

impl Default for StreamStallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stall_threshold_secs: 10,
            idle_timeout_secs: 30,
            max_stall_count: 3,
        }
    }
}

impl StreamStallConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_STREAM_STALL_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let stall_threshold_secs = std::env::var("STAR_STREAM_STALL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let idle_timeout_secs = std::env::var("STAR_STREAM_IDLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let max_stall_count = std::env::var("STAR_STREAM_MAX_STALLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        Self {
            enabled,
            stall_threshold_secs,
            idle_timeout_secs,
            max_stall_count,
        }
    }
}

/// 流式停滞检测器
pub struct StreamStallDetector {
    config: StreamStallConfig,
    /// 最后一次收到事件的时间
    last_event_at: Instant,
    /// 停滞次数
    stall_count: u32,
    /// 总停滞时间（毫秒）
    total_stall_ms: u64,
    /// 流式开始时间
    stream_start_at: Instant,
}

impl StreamStallDetector {
    pub fn new() -> Self {
        let config = StreamStallConfig::from_env();
        let now = Instant::now();
        Self {
            config,
            last_event_at: now,
            stall_count: 0,
            total_stall_ms: 0,
            stream_start_at: now,
        }
    }

    /// 记录事件发生
    pub fn record_event(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_event_at);

        // 如果之前是停滞状态，记录停滞时间
        if elapsed > Duration::from_secs(self.config.stall_threshold_secs) {
            self.stall_count += 1;
            self.total_stall_ms += elapsed.as_millis() as u64;
        }

        self.last_event_at = now;
    }

    /// 检查是否停滞
    pub fn is_stalled(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        let elapsed = self.last_event_at.elapsed();
        elapsed > Duration::from_secs(self.config.stall_threshold_secs)
    }

    /// 检查是否应该终止
    pub fn should_abort(&self) -> StallDecision {
        if !self.config.enabled {
            return StallDecision::Continue;
        }

        let elapsed = self.last_event_at.elapsed();
        let total_elapsed = self.stream_start_at.elapsed();

        // 检查空闲超时
        if elapsed > Duration::from_secs(self.config.idle_timeout_secs) {
            return StallDecision::Abort {
                reason: format!(
                    "Stream idle timeout after {:.1}s (last event {:.1}s ago)",
                    total_elapsed.as_secs_f64(),
                    elapsed.as_secs_f64()
                ),
                stall_count: self.stall_count,
                total_stall_ms: self.total_stall_ms,
            };
        }

        // 检查停滞次数
        if self.stall_count >= self.config.max_stall_count {
            return StallDecision::Abort {
                reason: format!(
                    "Max stall count reached ({}/{})",
                    self.stall_count, self.config.max_stall_count
                ),
                stall_count: self.stall_count,
                total_stall_ms: self.total_stall_ms,
            };
        }

        // 检查当前是否停滞
        if self.is_stalled() {
            return StallDecision::Stalled {
                elapsed_secs: elapsed.as_secs_f64(),
                stall_count: self.stall_count,
            };
        }

        StallDecision::Continue
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> StallStats {
        let elapsed = self.last_event_at.elapsed();
        let total_elapsed = self.stream_start_at.elapsed();

        StallStats {
            elapsed_since_last_event_ms: elapsed.as_millis() as u64,
            total_elapsed_ms: total_elapsed.as_millis() as u64,
            stall_count: self.stall_count,
            total_stall_ms: self.total_stall_ms,
            is_stalled: self.is_stalled(),
        }
    }
}

/// 停滞决策
#[derive(Debug, Clone)]
pub enum StallDecision {
    /// 继续执行
    Continue,
    /// 当前停滞
    Stalled {
        elapsed_secs: f64,
        stall_count: u32,
    },
    /// 应该终止
    Abort {
        reason: String,
        stall_count: u32,
        total_stall_ms: u64,
    },
}

/// 停滞统计信息
#[derive(Debug, Clone)]
pub struct StallStats {
    pub elapsed_since_last_event_ms: u64,
    pub total_elapsed_ms: u64,
    pub stall_count: u32,
    pub total_stall_ms: u64,
    pub is_stalled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stall_detection() {
        let config = StreamStallConfig {
            enabled: true,
            stall_threshold_secs: 1,
            idle_timeout_secs: 5,
            max_stall_count: 3,
        };

        let mut detector = StreamStallDetector {
            config,
            last_event_at: Instant::now(),
            stall_count: 0,
            total_stall_ms: 0,
            stream_start_at: Instant::now(),
        };

        // 初始状态不应该是停滞
        assert!(!detector.is_stalled());

        // 模拟事件
        detector.record_event();
        assert!(!detector.is_stalled());
    }
}
