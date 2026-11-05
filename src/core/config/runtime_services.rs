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
    /// 运行中切模型的共享槽。
    ///
    /// 放在这里（而不是 `Agent::new` 里新建）是因为**两个消费者**：
    /// - `Agent`：每轮 drain，把用户 `/model` 的选择换上去
    /// - `SkillTool`：skill 的 `model` frontmatter 字段也要换模型
    ///
    /// 两边必须是**同一个槽** —— 否则 skill 的模型切换会被用户的 `/model`
    /// 覆盖，或者反过来。
    model_switch_cell: Option<crate::agent::model_switch::ModelSwitchCell>,
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

    /// 设置模型切换槽（`Agent::new` 与 `SkillTool` 共用同一个）。
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
}

pub type RuntimeHandles = RuntimeServices;
