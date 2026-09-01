/// Agent周期性摘要生成器
///
/// 对标claude-code-main的src/services/AgentSummary/
/// 每30秒为子Agent生成摘要
use super::{Summary, SummaryStats, SummaryType};

/// Agent摘要生成器
pub struct AgentSummaryGenerator {
    /// 上次生成时间
    last_generated: Option<i64>,
    /// 生成间隔（秒）
    interval_secs: i64,
}

impl AgentSummaryGenerator {
    /// 创建新的Agent摘要生成器
    pub fn new() -> Self {
        Self {
            last_generated: None,
            interval_secs: 30, // 30秒
        }
    }

    /// 检查是否应该生成摘要
    pub fn should_generate(&self) -> bool {
        match self.last_generated {
            Some(last) => {
                let now = chrono::Utc::now().timestamp();
                now - last >= self.interval_secs
            }
            None => true,
        }
    }

    /// 生成摘要
    pub fn generate(
        &mut self,
        session_id: &str,
        messages: &[crate::types::StarMessage],
    ) -> Summary {
        let id = uuid::Uuid::new_v4().to_string();
        self.last_generated = Some(chrono::Utc::now().timestamp());

        // 统计信息
        let message_count = messages.len() as u32;
        let tool_calls = messages.iter().filter(|m| m.tool_calls.is_some()).count() as u32;

        // 提取最近的活动
        let recent_activity = self.extract_recent_activity(messages);

        // 生成摘要内容
        let content = format!(
            "Agent periodic summary: {} messages, {} tool calls. Recent: {}",
            message_count, tool_calls, recent_activity
        );

        Summary {
            id,
            summary_type: SummaryType::AgentPeriodic,
            title: format!("Agent Summary - {}", session_id),
            content,
            created_at: chrono::Utc::now().timestamp(),
            session_id: Some(session_id.to_string()),
            key_points: vec![recent_activity],
            stats: SummaryStats {
                message_count,
                tool_calls,
                duration_secs: 0,
                files_modified: 0,
            },
        }
    }

    /// 提取最近活动
    fn extract_recent_activity(&self, messages: &[crate::types::StarMessage]) -> String {
        // 获取最近5条消息
        let recent: Vec<String> = messages
            .iter()
            .rev()
            .take(5)
            .filter_map(|m| m.content.clone())
            .map(|c| {
                if c.len() > 50 {
                    format!("{}...", &c[..50])
                } else {
                    c
                }
            })
            .collect();

        if recent.is_empty() {
            "No recent activity".to_string()
        } else {
            recent.join("; ")
        }
    }
}
