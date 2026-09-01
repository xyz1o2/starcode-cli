//! Swarm 模块（对标 Claude Code 的 Agent Teams / Swarm）
//!
//! 实现团队协作的完整生命周期：
//! - TeamCreate/Delete: 团队创建和销毁
//! - Mailbox: 成员间异步消息传递
//! - Teammate 生命周期: 创建、运行、空闲、关闭
//! - Teams: 团队定义和管理

pub mod mailbox;
pub mod teams;

pub use mailbox::{MailboxError, MailboxManager, MailboxMessage, MessageType};
pub use teams::{SwarmManager, TeamFile, TeamInstance, TeammateDefinition, TeammateStatus};
