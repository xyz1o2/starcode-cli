/// Provider Usage类型定义

use serde::{Deserialize, Serialize};

/// 使用量记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// 记录ID
    pub id: String,
    /// Provider名称
    pub provider: String,
    /// 模型名称
    pub model: String,
    /// 提示token数
    pub prompt_tokens: u32,
    /// 完成token数
    pub completion_tokens: u32,
    /// 总token数
    pub total_tokens: u32,
    /// 成本（美元）
    pub cost: Option<f64>,
    /// 时间戳
    pub timestamp: i64,
    /// 会话ID
    pub session_id: Option<String>,
    /// 请求ID
    pub request_id: Option<String>,
}

/// 使用量摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Provider名称
    pub provider: String,
    /// 总请求数
    pub total_requests: u64,
    /// 总提示token数
    pub total_prompt_tokens: u64,
    /// 总完成token数
    pub total_completion_tokens: u64,
    /// 总token数
    pub total_tokens: u64,
    /// 总成本
    pub total_cost: f64,
    /// 平均延迟（毫秒）
    pub average_latency_ms: f64,
    /// 成功率
    pub success_rate: f64,
    /// 开始时间
    pub start_time: i64,
    /// 结束时间
    pub end_time: i64,
}

/// Provider使用量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    /// Provider名称
    pub provider: String,
    /// 模型使用量
    pub models: Vec<ModelUsage>,
    /// 总使用量
    pub total: UsageSummary,
}

/// 模型使用量
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    /// 模型名称
    pub model: String,
    /// 请求数
    pub requests: u64,
    /// 总token数
    pub total_tokens: u64,
    /// 成本
    pub cost: f64,
}

/// 余额信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceInfo {
    /// Provider名称
    pub provider: String,
    /// 余额
    pub balance: f64,
    /// 货币
    pub currency: String,
    /// 更新时间
    pub updated_at: i64,
}
