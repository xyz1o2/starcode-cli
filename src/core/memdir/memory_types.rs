/// 记忆类型

use serde::{Deserialize, Serialize};

/// 记忆类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MemoryType {
    /// 代码偏好
    CodePreference,
    /// 项目知识
    ProjectKnowledge,
    /// 用户习惯
    UserHabit,
    /// 错误解决方案
    ErrorSolution,
    /// 工作流程
    Workflow,
    /// 架构决策
    ArchitectureDecision,
    /// 会话摘要
    SessionSummary,
    /// 团队记忆
    TeamMemory,
    /// 自定义
    Custom(String),
}
