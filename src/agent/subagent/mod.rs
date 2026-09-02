//! SubAgent 子系统：执行器、路由决策、异步通知、Fork 机制。
//!
//! 核心文件：
//! - `runner`   — StarAgentRunner（同步迁入）+ AsyncSubagentRunner（新增异步）
//! - `router`   — AgentTool 分流决策树（纯函数）
//! - `notification` — TaskNotification 结构体 + 全局通知队列
//! - `fork`     — SubAgent fork 机制（继承父上下文、prompt cache 共享）
//! - `progress` — 子 Agent 进度回流（统计累加 + UI sink）

pub mod fork;
pub mod notification;
pub mod progress;
pub mod router;
pub mod runner;

pub use fork::{ForkConfig, ForkManager, ForkStatus, ForkedSession};
pub use notification::{
    NotificationQueue, NotificationStatus, NotificationUsage, TaskNotification,
};
pub use progress::{
    AgentProgressTracker, SubAgentProgress, SubAgentProgressSink, describe_tool_use,
    search_read_summary_text, user_facing_tool_name,
};
pub use router::{route_agent_call, AgentRoute};
pub use runner::{AsyncSubagentRunner, StarAgentRunner};
