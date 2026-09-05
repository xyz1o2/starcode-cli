mod agent_runtime;
mod core_runtime;

use crate::core::config::{Config, RuntimeServices};
use crate::core::confirmation_bus::MessageBus;
use crate::core::policy::{PolicyEngine, PolicyEngineConfig};
use crate::core::state::GlobalState;
use crate::core::tools::ToolRegistry;
use crate::llm::client::StarClient;
use std::sync::Arc;

pub struct RuntimeBootstrapArtifacts {
    pub services: RuntimeServices,
}

pub struct AgentRuntimeBootstrapArtifacts {
    pub tool_registry: Arc<ToolRegistry>,
    pub message_bus: Arc<MessageBus>,
}

#[derive(Clone, Copy)]
enum ToolRegistryAssemblyLayer {
    CoreRuntime,
    AgentRuntime,
}

pub async fn build_runtime_services(
    config: &Config,
    existing_runtime: Option<&Arc<RuntimeServices>>,
) -> Result<RuntimeBootstrapArtifacts, Box<dyn std::error::Error>> {
    let message_bus = existing_runtime
        .map(|runtime| runtime.message_bus())
        .unwrap_or_else(|| {
            // 唯一真正参与运行时的 PolicyEngine 构造点：审批模式和磁盘上的权限规则
            // 都得在这里灌进去，否则用户在 settings.json 里配的 allow/deny 形同废纸。
            Arc::new(MessageBus::new(
                PolicyEngine::with_project_rules(
                    PolicyEngineConfig {
                        approval_mode: Some(config.approval_mode().clone()),
                        ..Default::default()
                    },
                    config.project_root(),
                ),
                config.debug_mode(),
            ))
        });
    let global_state = existing_runtime
        .map(|runtime| runtime.global_state())
        .unwrap_or_else(|| Arc::new(GlobalState::new()));
    let mcp_manager = existing_runtime
        .and_then(|runtime| runtime.mcp_manager())
        .or_else(|| {
            if config.mcp_enabled() {
                Some(Arc::new(crate::core::mcp::MCPManager::new()))
            } else {
                None
            }
        });

    let mut registry_config = config.clone();
    registry_config.install_runtime_services(RuntimeServices::new(
        None,
        message_bus.clone(),
        mcp_manager.clone(),
        global_state.clone(),
    ));
    let registry_config = Arc::new(registry_config);

    let tool_registry = build_core_tool_registry(
        config,
        registry_config,
        message_bus.clone(),
        global_state.clone(),
    )
    .await?;

    // 后台 SubAgent 通知队列（全局共享，供 AgentTool 写入、主 Agent 每轮消费）
    let notification_queue = Arc::new(tokio::sync::Mutex::new(
        crate::agent::subagent::notification::NotificationQueue::new(),
    ));

    // Re-install services with the notification queue attached (retains built registry)
    let mut services = RuntimeServices::new(
        Some(tool_registry.clone()),
        message_bus,
        mcp_manager,
        global_state,
    );
    services = services.with_notification_queue(notification_queue);

    Ok(RuntimeBootstrapArtifacts { services })
}

pub async fn build_core_tool_registry(
    config: &Config,
    config_arc: Arc<Config>,
    message_bus: Arc<MessageBus>,
    global_state: Arc<GlobalState>,
) -> Result<Arc<ToolRegistry>, Box<dyn std::error::Error>> {
    let registry = Arc::new(ToolRegistry::new(config_arc.clone()));

    apply_tool_registry_layer(
        ToolRegistryAssemblyLayer::CoreRuntime,
        &registry,
        config,
        &config_arc,
        &message_bus,
        &global_state,
        None,
    );

    Ok(registry)
}

fn apply_tool_registry_layer(
    layer: ToolRegistryAssemblyLayer,
    registry: &Arc<ToolRegistry>,
    selection_config: &Config,
    registry_config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
    client: Option<&StarClient>,
) {
    match layer {
        ToolRegistryAssemblyLayer::CoreRuntime => core_runtime::register_core_runtime_tools(
            registry,
            selection_config,
            registry_config,
            message_bus,
            global_state,
        ),
        ToolRegistryAssemblyLayer::AgentRuntime => agent_runtime::register_agent_runtime_tools(
            registry,
            registry_config,
            message_bus,
            global_state,
            client.expect("StarClient is required for agent runtime tool assembly"),
        ),
    }
}

pub fn build_agent_runtime_artifacts(
    client: &StarClient,
    config: &Arc<Config>,
) -> AgentRuntimeBootstrapArtifacts {
    let tool_registry = config
        .runtime_tool_registry()
        .expect("ToolRegistry not initialized");
    let message_bus = config
        .runtime_message_bus()
        .expect("RuntimeServices not initialized");
    let global_state = config
        .runtime_global_state()
        .expect("RuntimeServices not initialized");

    apply_tool_registry_layer(
        ToolRegistryAssemblyLayer::AgentRuntime,
        &tool_registry,
        config.as_ref(),
        config,
        &message_bus,
        &global_state,
        Some(client),
    );

    AgentRuntimeBootstrapArtifacts {
        tool_registry,
        message_bus,
    }
}
