/// 摘要整合管理器
/// 
/// 整合多个摘要为一个综合摘要

use super::{Summary, SummaryType, SummaryStats};

/// 整合管理器
pub struct ConsolidationManager;

impl ConsolidationManager {
    /// 创建新的整合管理器
    pub fn new() -> Self {
        Self
    }

    /// 整合多个摘要
    pub fn consolidate(&self, summaries: &[Summary]) -> Summary {
        let id = uuid::Uuid::new_v4().to_string();

        // 合并关键点
        let mut all_key_points = Vec::new();
        for summary in summaries {
            all_key_points.extend(summary.key_points.clone());
        }
        all_key_points.dedup();
        all_key_points.truncate(10); // 最多保留10个关键点

        // 合并统计信息
        let total_messages: u32 = summaries.iter().map(|s| s.stats.message_count).sum();
        let total_tool_calls: u32 = summaries.iter().map(|s| s.stats.tool_calls).sum();
        let total_duration: u64 = summaries.iter().map(|s| s.stats.duration_secs).sum();
        let total_files_modified: u32 = summaries.iter().map(|s| s.stats.files_modified).sum();

        // 生成整合摘要内容
        let content = format!(
            "Consolidated summary from {} sessions: {} messages, {} tool calls. Key topics: {}",
            summaries.len(),
            total_messages,
            total_tool_calls,
            all_key_points.join(", ")
        );

        Summary {
            id,
            summary_type: SummaryType::Session,
            title: "Consolidated Summary".to_string(),
            content,
            created_at: chrono::Utc::now().timestamp(),
            session_id: None,
            key_points: all_key_points,
            stats: SummaryStats {
                message_count: total_messages,
                tool_calls: total_tool_calls,
                duration_secs: total_duration,
                files_modified: total_files_modified,
            },
        }
    }
}
