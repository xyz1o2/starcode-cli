/// 空闲超时管理器
/// 
/// 对标claude-code-main的src/utils/idleTimeout.ts
/// SDK模式下的空闲超时管理

use std::time::{Duration, Instant};

/// 空闲超时配置
#[derive(Debug, Clone)]
pub struct IdleTimeoutConfig {
    /// 空闲超时时间（秒）
    pub idle_timeout_secs: u64,
    /// 是否启用
    pub enabled: bool,
}

impl Default for IdleTimeoutConfig {
    fn default() -> Self {
        Self {
            idle_timeout_secs: 300, // 5分钟
            enabled: false,
        }
    }
}

impl IdleTimeoutConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let idle_timeout_secs = std::env::var("STAR_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        let enabled = std::env::var("STAR_IDLE_TIMEOUT_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            idle_timeout_secs,
            enabled,
        }
    }
}

/// 空闲超时管理器
pub struct IdleTimeoutManager {
    config: IdleTimeoutConfig,
    /// 最后活动时间
    last_activity: Instant,
    /// 是否空闲
    is_idle: bool,
}

impl IdleTimeoutManager {
    /// 创建新的空闲超时管理器
    pub fn new(config: IdleTimeoutConfig) -> Self {
        Self {
            config,
            last_activity: Instant::now(),
            is_idle: false,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(IdleTimeoutConfig::from_env())
    }

    /// 更新活动时间
    pub fn update_activity(&mut self) {
        self.last_activity = Instant::now();
        self.is_idle = false;
    }

    /// 检查是否超时
    pub fn is_timeout(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        self.last_activity.elapsed().as_secs() >= self.config.idle_timeout_secs
    }

    /// 获取空闲时间（秒）
    pub fn idle_time_secs(&self) -> u64 {
        self.last_activity.elapsed().as_secs()
    }

    /// 标记为空闲
    pub fn mark_idle(&mut self) {
        self.is_idle = true;
    }

    /// 检查是否空闲
    pub fn is_idle(&self) -> bool {
        self.is_idle
    }

    /// 获取配置
    pub fn config(&self) -> &IdleTimeoutConfig {
        &self.config
    }
}
