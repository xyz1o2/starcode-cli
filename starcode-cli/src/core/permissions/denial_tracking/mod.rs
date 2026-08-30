/// 拒绝追踪系统
/// 
/// 对标claude-code-main的src/utils/permissions/denialTracking.ts
/// 记录权限拒绝次数，用于自动模式决策

pub mod tracker;
pub mod limits;

pub use tracker::DenialTracker;
pub use limits::DenialLimits;

use serde::{Deserialize, Serialize};

/// 拒绝记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialRecord {
    /// 工具名称
    pub tool_name: String,
    /// 拒绝时间
    pub timestamp: i64,
    /// 拒绝原因
    pub reason: String,
    /// 会话ID
    pub session_id: Option<String>,
}

/// 拒绝统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenialStats {
    /// 总拒绝次数
    pub total_denials: u32,
    /// 连续拒绝次数
    pub consecutive_denials: u32,
    /// 最后拒绝时间
    pub last_denial_time: Option<i64>,
    /// 按工具统计
    pub by_tool: HashMap<String, u32>,
}

/// 拒绝追踪管理器
pub struct DenialTrackingManager {
    tracker: DenialTracker,
    limits: DenialLimits,
    stats: DenialStats,
}

impl DenialTrackingManager {
    /// 创建新的拒绝追踪管理器
    pub fn new() -> Self {
        Self {
            tracker: DenialTracker::new(),
            limits: DenialLimits::default(),
            stats: DenialStats {
                total_denials: 0,
                consecutive_denials: 0,
                last_denial_time: None,
                by_tool: HashMap::new(),
            },
        }
    }

    /// 记录拒绝
    pub fn record_denial(&mut self, tool_name: &str, reason: &str) {
        let record = DenialRecord {
            tool_name: tool_name.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            reason: reason.to_string(),
            session_id: None,
        };

        self.tracker.record(record);
        self.stats.total_denials += 1;
        self.stats.consecutive_denials += 1;
        self.stats.last_denial_time = Some(chrono::Utc::now().timestamp());
        *self.stats.by_tool.entry(tool_name.to_string()).or_insert(0) += 1;
    }

    /// 记录成功
    pub fn record_success(&mut self) {
        self.stats.consecutive_denials = 0;
    }

    /// 检查是否应该回退到提示模式
    pub fn should_fallback_to_prompting(&self) -> bool {
        self.stats.consecutive_denials >= self.limits.max_consecutive_denials
    }

    /// 获取统计信息
    pub fn stats(&self) -> &DenialStats {
        &self.stats
    }

    /// 重置统计
    pub fn reset(&mut self) {
        self.stats = DenialStats {
            total_denials: 0,
            consecutive_denials: 0,
            last_denial_time: None,
            by_tool: HashMap::new(),
        };
        self.tracker.clear();
    }
}
