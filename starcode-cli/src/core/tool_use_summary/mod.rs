/// 工具使用摘要系统
/// 
/// 对标claude-code-main的src/services/toolUseSummary/
/// 生成和管理工具使用摘要

pub mod generator;
pub mod storage;
pub mod types;

pub use generator::SummaryGenerator;
pub use storage::SummaryStorage;
pub use types::{ToolUseSummary, ToolCallRecord, SummaryStats};

use serde::{Deserialize, Serialize};

/// 摘要配置
#[derive(Debug, Clone)]
pub struct SummaryConfig {
    /// 是否启用
    pub enabled: bool,
    /// 最大记录数
    pub max_records: usize,
    /// 是否启用自动摘要
    pub auto_summary: bool,
    /// 摘要间隔（秒）
    pub summary_interval_secs: u64,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_records: 1000,
            auto_summary: false,
            summary_interval_secs: 300,
        }
    }
}

impl SummaryConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_TOOL_SUMMARY_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let max_records = std::env::var("STAR_TOOL_SUMMARY_MAX_RECORDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);

        Self {
            enabled,
            max_records,
            auto_summary: false,
            summary_interval_secs: 300,
        }
    }
}

/// 工具使用摘要管理器
pub struct ToolUseSummaryManager {
    /// 配置
    config: SummaryConfig,
    /// 存储
    storage: SummaryStorage,
    /// 生成器
    generator: SummaryGenerator,
}

impl ToolUseSummaryManager {
    /// 创建新的工具使用摘要管理器
    pub fn new(config: SummaryConfig) -> Self {
        Self {
            config,
            storage: SummaryStorage::new(),
            generator: SummaryGenerator::new(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(SummaryConfig::from_env())
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, record: ToolCallRecord) {
        if !self.config.enabled {
            return;
        }

        self.storage.add_record(record);
    }

    /// 生成摘要
    pub fn generate_summary(&self) -> ToolUseSummary {
        let records = self.storage.get_all_records();
        self.generator.generate(&records)
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> SummaryStats {
        let records = self.storage.get_all_records();
        self.generator.calculate_stats(&records)
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}
