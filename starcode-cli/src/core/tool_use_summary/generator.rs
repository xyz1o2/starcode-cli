/// 摘要生成器

use super::types::{ToolCallRecord, ToolUseSummary, ToolStats, SummaryStats};
use std::collections::HashMap;

/// 摘要生成器
pub struct SummaryGenerator;

impl SummaryGenerator {
    /// 创建新的摘要生成器
    pub fn new() -> Self {
        Self
    }

    /// 生成摘要
    pub fn generate(&self, records: &[ToolCallRecord]) -> ToolUseSummary {
        let total_calls = records.len() as u64;
        let successful_calls = records.iter().filter(|r| r.success).count() as u64;
        let failed_calls = total_calls - successful_calls;
        let total_duration_ms: u64 = records.iter().map(|r| r.duration_ms).sum();
        let average_duration_ms = if total_calls > 0 {
            total_duration_ms as f64 / total_calls as f64
        } else {
            0.0
        };

        // 计算工具统计
        let tool_stats = self.calculate_tool_stats(records);

        // 找出最常用工具
        let most_used_tool = tool_stats.iter()
            .max_by_key(|s| s.call_count)
            .map(|s| s.tool_name.clone());

        // 找出最慢工具
        let slowest_tool = tool_stats.iter()
            .max_by_key(|s| s.average_duration_ms as u64)
            .map(|s| s.tool_name.clone());

        ToolUseSummary {
            total_calls,
            successful_calls,
            failed_calls,
            total_duration_ms,
            average_duration_ms,
            tool_stats,
            most_used_tool,
            slowest_tool,
        }
    }

    /// 计算工具统计
    fn calculate_tool_stats(&self, records: &[ToolCallRecord]) -> Vec<ToolStats> {
        let mut tool_map: HashMap<String, Vec<&ToolCallRecord>> = HashMap::new();

        for record in records {
            tool_map
                .entry(record.tool_name.clone())
                .or_insert_with(Vec::new)
                .push(record);
        }

        tool_map.iter()
            .map(|(tool_name, records)| {
                let call_count = records.len() as u64;
                let success_count = records.iter().filter(|r| r.success).count() as u64;
                let failure_count = call_count - success_count;
                let total_duration_ms: u64 = records.iter().map(|r| r.duration_ms).sum();
                let average_duration_ms = if call_count > 0 {
                    total_duration_ms as f64 / call_count as f64
                } else {
                    0.0
                };

                ToolStats {
                    tool_name: tool_name.clone(),
                    call_count,
                    success_count,
                    failure_count,
                    total_duration_ms,
                    average_duration_ms,
                }
            })
            .collect()
    }

    /// 计算统计信息
    pub fn calculate_stats(&self, records: &[ToolCallRecord]) -> SummaryStats {
        let total_calls = records.len() as u64;
        let successful_calls = records.iter().filter(|r| r.success).count() as u64;
        let success_rate = if total_calls > 0 {
            successful_calls as f64 / total_calls as f64
        } else {
            0.0
        };
        let total_duration_ms: u64 = records.iter().map(|r| r.duration_ms).sum();
        let average_duration_ms = if total_calls > 0 {
            total_duration_ms as f64 / total_calls as f64
        } else {
            0.0
        };

        // 找出最常用工具
        let mut tool_counts: HashMap<String, u64> = HashMap::new();
        for record in records {
            *tool_counts.entry(record.tool_name.clone()).or_insert(0) += 1;
        }
        let most_used_tool = tool_counts.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(name, _)| name.clone());

        SummaryStats {
            total_calls,
            success_rate,
            average_duration_ms,
            most_used_tool,
        }
    }
}
