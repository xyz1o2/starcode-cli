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
    /// lead 的 Agent 与 SkillTool 共用；子代理使用独立模型切换槽。
    model_switch_cell: Option<crate::agent::model_switch::ModelSwitchCell>,
    /// Agent 间消息路由 hub（进程级单例，见 `agent::messaging::hub`）。
    agent_hub: Option<crate::agent::messaging::SharedAgentRouter>,
    subagent_tasks: crate::agent::subagent::tasks::SharedSubagentTasks,
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
            model_switch_cell: None,
            agent_hub: None,
            subagent_tasks: Arc::new(Mutex::new(Default::default())),
        }
    }

    pub(crate) fn tool_registry(&self) -> Option<Arc<ToolRegistry>> {
        self.tool_registry.clone()
    }

    /// 仅替换工具映射，保留同一会话的审批、任务与消息句柄。
    pub(crate) fn with_tool_registry(mut self, registry: Option<Arc<ToolRegistry>>) -> Self {
        self.tool_registry = registry;
        self
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

    pub(crate) fn subagent_tasks(&self) -> crate::agent::subagent::tasks::SharedSubagentTasks {
        self.subagent_tasks.clone()
    }

    pub(crate) fn with_subagent_tasks(
        mut self,
        tasks: crate::agent::subagent::tasks::SharedSubagentTasks,
    ) -> Self {
        self.subagent_tasks = tasks;
        self
    }

    /// 设置或读取后台 SubAgent 通知队列
    pub fn with_notification_queue(mut self, queue: Arc<Mutex<NotificationQueue>>) -> Self {
        self.notification_queue = Some(queue);
        self
    }

    pub(crate) fn notification_queue(&self) -> Option<Arc<Mutex<NotificationQueue>>> {
        self.notification_queue.clone()
    }

    /// 设置 lead 的模型切换槽；子代理在 Agent 构造时各自建槽。
    pub fn with_model_switch_cell(
        mut self,
        cell: crate::agent::model_switch::ModelSwitchCell,
    ) -> Self {
        self.model_switch_cell = Some(cell);
        self
    }

    pub(crate) fn model_switch_cell(&self) -> Option<crate::agent::model_switch::ModelSwitchCell> {
        self.model_switch_cell.clone()
    }

    /// 设置或读取 Agent 间消息路由 hub（进程级单例）。
    pub fn with_agent_hub(mut self, hub: crate::agent::messaging::SharedAgentRouter) -> Self {
        self.agent_hub = Some(hub);
        self
    }

    pub(crate) fn agent_hub(&self) -> Option<crate::agent::messaging::SharedAgentRouter> {
        self.agent_hub.clone()
    }
}

pub type RuntimeHandles = RuntimeServices;
