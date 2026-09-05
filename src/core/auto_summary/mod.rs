/// 自动摘要系统
///
/// 对标claude-code-main的src/services/autoDream/和awaySummary.ts和AgentSummary/
/// 生成会话摘要、离开摘要和Agent周期性摘要
pub mod agent_summary;
pub mod consolidation;

pub use agent_summary::AgentSummaryGenerator;
pub use consolidation::ConsolidationManager;

use serde::{Deserialize, Serialize};

/// 摘要类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SummaryType {
    /// 会话摘要
    Session,
    /// 离开摘要
    Away,
    /// 任务摘要
    Task,
    /// 错误摘要
    Error,
    /// Agent周期性摘要
    AgentPeriodic,
}

/// 摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// 摘要ID
    pub id: String,
    /// 摘要类型
    pub summary_type: SummaryType,
    /// 标题
    pub title: String,
    /// 内容
    pub content: String,
    /// 创建时间
    pub created_at: i64,
    /// 相关会话ID
    pub session_id: Option<String>,
    /// 关键点
    pub key_points: Vec<String>,
    /// 摘要统计
    pub stats: SummaryStats,
}

/// 摘要统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryStats {
    /// 消息数量
    pub message_count: u32,
    /// 工具调用数量
    pub tool_calls: u32,
    /// 持续时间（秒）
    pub duration_secs: u64,
    /// 文件修改数
    pub files_modified: u32,
}

/// 摘要管理器
pub struct SummaryManager {
    summaries: Vec<Summary>,
    max_summaries: usize,
    /// Agent摘要生成器
    agent_summary_generator: AgentSummaryGenerator,
    /// 整合管理器
    consolidation_manager: ConsolidationManager,
}

impl SummaryManager {
    pub fn new(max_summaries: usize) -> Self {
        Self {
            summaries: Vec::new(),
            max_summaries,
            agent_summary_generator: AgentSummaryGenerator::new(),
            consolidation_manager: ConsolidationManager::new(),
        }
    }

    /// 生成会话摘要
    pub fn generate_session_summary(
        &mut self,
        session_id: &str,
        messages: &[crate::types::StarMessage],
    ) -> Summary {
        let id = uuid::Uuid::new_v4().to_string();

        // 统计信息
        let message_count = messages.len() as u32;
        let tool_calls = messages.iter().filter(|m| m.tool_calls.is_some()).count() as u32;

        // 提取关键点
        let key_points = self.extract_key_points(messages);

        // 生成摘要内容
        let content = format!(
            "Session with {} messages and {} tool calls. Key topics: {}",
            message_count,
            tool_calls,
            key_points.join(", ")
        );

        let summary = Summary {
            id: id.clone(),
            summary_type: SummaryType::Session,
            title: format!("Session Summary - {}", session_id),
            content,
            created_at: chrono::Utc::now().timestamp(),
            session_id: Some(session_id.to_string()),
            key_points,
            stats: SummaryStats {
                message_count,
                tool_calls,
                duration_secs: 0,
                files_modified: 0,
            },
        };

        self.summaries.push(summary.clone());

        if self.summaries.len() > self.max_summaries {
            self.summaries.remove(0);
        }

        summary
    }

    /// 生成Agent周期性摘要
    pub fn generate_agent_summary(
        &mut self,
        session_id: &str,
        messages: &[crate::types::StarMessage],
    ) -> Summary {
        self.agent_summary_generator.generate(session_id, messages)
    }

    /// 整合多个摘要
    pub fn consolidate_summaries(&mut self, summaries: &[Summary]) -> Summary {
        self.consolidation_manager.consolidate(summaries)
    }

    /// 提取关键点
    fn extract_key_points(&self, messages: &[crate::types::StarMessage]) -> Vec<String> {
        let mut key_points = Vec::new();

        for msg in messages.iter().filter(|m| m.role == "user").take(5) {
            if let Some(content) = &msg.content {
                key_points.push(crate::utils::string_utils::truncate_chars(content, 100));
            }
        }

        key_points
    }

    /// 获取所有摘要
    pub fn get_all_summaries(&self) -> &[Summary] {
        &self.summaries
    }

    /// 按类型获取摘要
    pub fn get_summaries_by_type(&self, summary_type: &SummaryType) -> Vec<&Summary> {
        self.summaries
            .iter()
            .filter(|s| {
                std::mem::discriminant(&s.summary_type) == std::mem::discriminant(summary_type)
            })
            .collect()
    }
}
