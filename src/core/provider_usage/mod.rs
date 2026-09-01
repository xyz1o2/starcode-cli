/// Provider Usage追踪系统
/// 
/// 对标claude-code-main的src/services/providerUsage/
/// 追踪各Provider的API使用量和余额

pub mod adapter;
pub mod balance;
pub mod store;
pub mod types;

pub use adapter::UsageAdapter;
pub use balance::BalanceTracker;
pub use store::UsageStore;
pub use types::{UsageRecord, UsageSummary, ProviderUsage};

use serde::{Deserialize, Serialize};

/// 使用量配置
#[derive(Debug, Clone)]
pub struct UsageConfig {
    /// 是否启用
    pub enabled: bool,
    /// 存储路径
    pub storage_path: Option<String>,
    /// 追踪间隔（秒）
    pub tracking_interval_secs: u64,
    /// 是否启用余额追踪
    pub balance_tracking: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            storage_path: None,
            tracking_interval_secs: 60,
            balance_tracking: false,
        }
    }
}

impl UsageConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_USAGE_TRACKING_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let storage_path = std::env::var("STAR_USAGE_STORAGE_PATH").ok();

        let tracking_interval_secs = std::env::var("STAR_USAGE_TRACKING_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let balance_tracking = std::env::var("STAR_USAGE_BALANCE_TRACKING")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            enabled,
            storage_path,
            tracking_interval_secs,
            balance_tracking,
        }
    }
}

/// Provider Usage管理器
pub struct ProviderUsageManager {
    /// 配置
    config: UsageConfig,
    /// 使用量存储
    store: UsageStore,
    /// 余额追踪器
    balance_tracker: BalanceTracker,
}

impl ProviderUsageManager {
    /// 创建新的Provider Usage管理器
    pub fn new(config: UsageConfig) -> Self {
        let store = UsageStore::new(config.storage_path.clone());
        let balance_tracker = BalanceTracker::new();

        Self {
            config,
            store,
            balance_tracker,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(UsageConfig::from_env())
    }

    /// 记录使用量
    pub fn record_usage(&mut self, record: UsageRecord) {
        if !self.config.enabled {
            return;
        }

        self.store.add_record(record);
    }

    /// 获取Provider使用量摘要
    pub fn get_summary(&self, provider: &str) -> UsageSummary {
        self.store.get_summary(provider)
    }

    /// 获取所有Provider使用量
    pub fn get_all_usage(&self) -> Vec<ProviderUsage> {
        self.store.get_all_usage()
    }

    /// 获取余额
    pub fn get_balance(&self, provider: &str) -> Option<f64> {
        self.balance_tracker.get_balance(provider)
    }

    /// 更新余额
    pub fn update_balance(&mut self, provider: &str, balance: f64) {
        self.balance_tracker.update_balance(provider, balance);
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}
