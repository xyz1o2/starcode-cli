//! Messaging 路由模块
//!
//! 对标 Claude Code 的 SendMessageTool：
//! - 多路路由（mailbox / agentId / name）
//! - 消息类型分类
//! - 异步投递

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 消息目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageTarget {
    /// 投递到 mailbox
    Mailbox { team: String, name: String },
    /// 按 agent ID 路由
    AgentId(String),
    /// 按名称路由
    AgentName(String),
    /// 广播
    Broadcast,
}

/// 消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    /// 任务分配
    TaskAssignment,
    /// 状态更新
    StatusUpdate,
    /// 结果返回
    Result,
    /// 计划审批请求
    PlanApprovalRequest,
    /// 计划审批响应
    PlanApprovalResponse,
    /// 错误报告
    Error,
    /// 自定义
    Custom(String),
}

/// 路由消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutedMessage {
    pub id: String,
    pub from: String,
    pub to: MessageTarget,
    pub message_type: MessageType,
    pub content: Value,
    pub timestamp: u64,
    pub requires_response: bool,
    pub correlation_id: Option<String>,
}

/// 路由结果
#[derive(Debug, Clone)]
pub enum RouteResult {
    /// 投递成功
    Delivered { message_id: String },
    /// 目标不存在
    TargetNotFound { target: String },
    /// 目标忙碌
    TargetBusy { target: String, queue_size: usize },
    /// 投递失败
    Failed { reason: String },
}

/// 消息路由管理器
pub struct MessageRouter {
    /// mailbox 存储
    mailboxes: HashMap<String, Vec<RoutedMessage>>,
    /// agent 注册表
    agents: HashMap<String, AgentRegistration>,
    /// 消息历史
    history: Vec<RoutedMessage>,
    /// 最大历史记录
    max_history: usize,
}

/// Agent 注册信息
#[derive(Debug, Clone)]
pub struct AgentRegistration {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub status: AgentStatus,
    pub inbox: Vec<RoutedMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Busy,
    Offline,
}

impl MessageRouter {
    pub fn new(max_history: usize) -> Self {
        Self {
            mailboxes: HashMap::new(),
            agents: HashMap::new(),
            history: Vec::new(),
            max_history,
        }
    }

    /// 注册 agent
    pub fn register_agent(&mut self, registration: AgentRegistration) {
        self.agents.insert(registration.id.clone(), registration);
    }

    /// 注销 agent
    pub fn unregister_agent(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    /// 路由消息
    pub fn route(&mut self, message: RoutedMessage) -> RouteResult {
        let result = match &message.to {
            MessageTarget::Mailbox { team, name } => {
                self.route_to_mailbox(team, name, message.clone())
            }
            MessageTarget::AgentId(id) => self.route_to_agent_id(id, message.clone()),
            MessageTarget::AgentName(name) => self.route_to_agent_name(name, message.clone()),
            MessageTarget::Broadcast => self.route_broadcast(message.clone()),
        };

        // 记录历史
        self.history.push(message);
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }

        result
    }

    fn route_to_mailbox(&mut self, team: &str, name: &str, message: RoutedMessage) -> RouteResult {
        let key = format!("{}/{}", team, name);
        let mailbox = self.mailboxes.entry(key.clone()).or_insert_with(Vec::new);
        let queue_size = mailbox.len();
        mailbox.push(message);

        RouteResult::Delivered {
            message_id: format!("msg_{}", uuid::Uuid::new_v4()),
        }
    }

    fn route_to_agent_id(&mut self, agent_id: &str, message: RoutedMessage) -> RouteResult {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            if agent.status == AgentStatus::Offline {
                return RouteResult::TargetNotFound {
                    target: agent_id.to_string(),
                };
            }
            agent.inbox.push(message);
            RouteResult::Delivered {
                message_id: format!("msg_{}", uuid::Uuid::new_v4()),
            }
        } else {
            RouteResult::TargetNotFound {
                target: agent_id.to_string(),
            }
        }
    }

    fn route_to_agent_name(&mut self, name: &str, message: RoutedMessage) -> RouteResult {
        let agent_id = self
            .agents
            .values()
            .find(|a| a.name == name)
            .map(|a| a.id.clone());

        if let Some(id) = agent_id {
            self.route_to_agent_id(&id, message)
        } else {
            RouteResult::TargetNotFound {
                target: name.to_string(),
            }
        }
    }

    fn route_broadcast(&mut self, message: RoutedMessage) -> RouteResult {
        let mut delivered = 0;
        for agent in self.agents.values_mut() {
            if agent.status != AgentStatus::Offline {
                agent.inbox.push(message.clone());
                delivered += 1;
            }
        }

        if delivered > 0 {
            RouteResult::Delivered {
                message_id: format!("broadcast_{}", uuid::Uuid::new_v4()),
            }
        } else {
            RouteResult::Failed {
                reason: "No active agents to broadcast to".to_string(),
            }
        }
    }

    /// 从 agent 收取消息
    pub fn poll_messages(&mut self, agent_id: &str) -> Vec<RoutedMessage> {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            std::mem::take(&mut agent.inbox)
        } else {
            Vec::new()
        }
    }

    /// 从 mailbox 收取消息
    pub fn poll_mailbox(&mut self, team: &str, name: &str) -> Vec<RoutedMessage> {
        let key = format!("{}/{}", team, name);
        self.mailboxes
            .get_mut(&key)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// 获取路由统计
    pub fn stats(&self) -> RouterStats {
        RouterStats {
            registered_agents: self.agents.len(),
            total_mailboxes: self.mailboxes.len(),
            messages_routed: self.history.len(),
            agents_by_status: self.count_by_status(),
        }
    }

    fn count_by_status(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for agent in self.agents.values() {
            let status = format!("{:?}", agent.status);
            *counts.entry(status).or_insert(0) += 1;
        }
        counts
    }
}

/// 路由统计
#[derive(Debug, Serialize)]
pub struct RouterStats {
    pub registered_agents: usize,
    pub total_mailboxes: usize,
    pub messages_routed: usize,
    pub agents_by_status: HashMap<String, usize>,
}
