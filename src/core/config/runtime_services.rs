use crate::agent::subagent::notification::NotificationQueue;
use crate::core::confirmation_bus::MessageBus;
use crate::core::state::GlobalState;
use crate::core::tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RuntimeServices {
    tool_registry: Option<Arc<ToolRegistry>>,
    message_bus: Arc<MessageBus>,
    mcp_manager: Option<Arc<crate::core::mcp::MCPManager>>,
    global_state: Arc<GlobalState>,
    /// 后台 SubAgent 完成通知队列（全局共享，供 AgentTool 写入、主 Agent 每轮消费）
    notification_queue: Option<Arc<Mutex<NotificationQueue>>>,
}

impl RuntimeServices {
    pub fn new(
        tool_registry: Option<Arc<ToolRegistry>>,
        message_bus: Arc<MessageBus>,
        mcp_manager: Option<Arc<crate::core::mcp::MCPManager>>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            tool_registry,
            message_bus,
            mcp_manager,
            global_state,
            notification_queue: None,
        }
    }

    pub(crate) fn tool_registry(&self) -> Option<Arc<ToolRegistry>> {
        self.tool_registry.clone()
    }

    pub(crate) fn message_bus(&self) -> Arc<MessageBus> {
        self.message_bus.clone()
    }

    pub(crate) fn mcp_manager(&self) -> Option<Arc<crate::core::mcp::MCPManager>> {
        self.mcp_manager.clone()
    }

    pub(crate) fn global_state(&self) -> Arc<GlobalState> {
        self.global_state.clone()
    }

    /// 设置或读取后台 SubAgent 通知队列
    pub fn with_notification_queue(mut self, queue: Arc<Mutex<NotificationQueue>>) -> Self {
        self.notification_queue = Some(queue);
        self
    }

    pub(crate) fn notification_queue(&self) -> Option<Arc<Mutex<NotificationQueue>>> {
        self.notification_queue.clone()
    }
}

pub type RuntimeHandles = RuntimeServices;
