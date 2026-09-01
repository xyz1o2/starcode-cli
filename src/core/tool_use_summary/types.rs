/// 工具使用摘要类型

use serde::{Deserialize, Serialize};

/// 工具调用记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// 记录ID
    pub id: String,
    /// 工具名称
    pub tool_name: String,
    /// 调用时间
    pub called_at: i64,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
    /// 是否成功
    pub success: bool,
    /// 输入大小（字节）
    pub input_size: u64,
    /// 输出大小（字节）
    pub output_size: u64,
    /// 错误信息
    pub error: Option<String>,
}

/// 工具使用摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseSummary {
    /// 总调用数
    pub total_calls: u64,
    /// 成功调用数
    pub successful_calls: u64,
    /// 失败调用数
    pub failed_calls: u64,
    /// 总执行时间（毫秒）
    pub total_duration_ms: u64,
    /// 平均执行时间（毫秒）
    pub average_duration_ms: f64,
    /// 工具使用统计
    pub tool_stats: Vec<ToolStats>,
    /// 最常用工具
    pub most_used_tool: Option<String>,
    /// 最慢工具
    pub slowest_tool: Option<String>,
}

/// 工具统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStats {
    /// 工具名称
    pub tool_name: String,
    /// 调用次数
    pub call_count: u64,
    /// 成功次数
    pub success_count: u64,
    /// 失败次数
    pub failure_count: u64,
    /// 总执行时间（毫秒）
    pub total_duration_ms: u64,
    /// 平均执行时间（毫秒）
    pub average_duration_ms: f64,
}

/// 摘要统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryStats {
    /// 总调用数
    pub total_calls: u64,
    /// 成功率
    pub success_rate: f64,
    /// 平均执行时间（毫秒）
    pub average_duration_ms: f64,
    /// 最常用工具
    pub most_used_tool: Option<String>,
}
