use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::state::GlobalState;
use crate::core::tools::ToolRegistry;
use std::sync::Arc;

pub(super) fn register_core_runtime_tools(
    registry: &Arc<ToolRegistry>,
    selection_config: &Config,
    registry_config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
) {
    register_core_file_tools(
        registry,
        selection_config,
        registry_config,
        message_bus,
        global_state,
    );
    register_core_command_tools(registry, selection_config, global_state);
    register_core_runtime_support_tools(registry, selection_config, registry_config, message_bus);
    register_core_navigation_tools(registry, selection_config);
}

fn is_core_tool_enabled(selection_config: &Config, tool_name: &str) -> bool {
    if let Some(core_tools) = selection_config.core_tools() {
        core_tools.iter().any(|t| t == tool_name)
    } else {
        true
    }
}

fn register_core_file_tools(
    registry: &Arc<ToolRegistry>,
    selection_config: &Config,
    registry_config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
) {
    use crate::core::tools::glob::GlobMatchTool;
    use crate::core::tools::read_many::ReadManyFilesTool;
    use crate::core::tools::{EditTool, ListDirTool, ReadFileTool, WriteFileTool};

    if is_core_tool_enabled(selection_config, "Edit") {
        registry.register_tool(Arc::new(EditTool::new(
            registry_config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }

    if is_core_tool_enabled(selection_config, "Write") {
        registry.register_tool(Arc::new(WriteFileTool::new(
            registry_config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }

    // Single registration for Read — canonical name is "Read",
    // "view_file" is resolved via canonical_tool_name alias.
    if is_core_tool_enabled(selection_config, "view_file")
        || is_core_tool_enabled(selection_config, "Read")
    {
        registry.register_tool(Arc::new(ReadFileTool::new_with_name(
            registry_config.clone(),
            message_bus.clone(),
            global_state.clone(),
            "Read".to_string(),
        )));
    }

    if is_core_tool_enabled(selection_config, "read_many_files") {
        registry.register_tool(Arc::new(ReadManyFilesTool::new(registry_config.clone(), global_state.clone())));
    }

    if is_core_tool_enabled(selection_config, "Glob") {
        registry.register_tool(Arc::new(GlobMatchTool::new(
            registry_config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }

    if is_core_tool_enabled(selection_config, "ListDir") {
        registry.register_tool(Arc::new(ListDirTool::new()));
    }
}

fn register_core_command_tools(
    registry: &Arc<ToolRegistry>,
    selection_config: &Config,
    global_state: &Arc<GlobalState>,
) {
    use crate::tools::SearchTool;

    if is_core_tool_enabled(selection_config, "Grep") {
        registry.register_tool(Arc::new(SearchTool::new(global_state.clone())));
    }

    // `Bash` is registered in agent_runtime (with LLM client).
    // `todo` is now an alias for `Todo`.
    // See canonical_tool_name() in tool_names.rs.
}

fn register_core_runtime_support_tools(
    registry: &Arc<ToolRegistry>,
    selection_config: &Config,
    registry_config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
) {
    use crate::core::tools::ask_user_question::AskUserQuestionTool;
    use crate::core::tools::exit_plan_mode::ExitPlanModeTool;
    use crate::core::tools::sleep::WaitTool;

    if is_core_tool_enabled(selection_config, "exit_plan_mode") {
        registry.register_tool(Arc::new(ExitPlanModeTool::new(
            registry_config.clone(),
            message_bus.clone(),
        )));
    }

    if is_core_tool_enabled(selection_config, "ask_user_question") {
        registry.register_tool(Arc::new(AskUserQuestionTool::new(
            registry_config.clone(),
            message_bus.clone(),
        )));
    }

    if is_core_tool_enabled(selection_config, "wait") {
        registry.register_tool(Arc::new(WaitTool::new()));
    }
}

fn register_core_navigation_tools(registry: &Arc<ToolRegistry>, selection_config: &Config) {
    use crate::core::tools::web_search::WebSearchTool;
    use crate::tools::lsp::LspTool;
    use crate::tools::next_edit::NextEditTool;

    if is_core_tool_enabled(selection_config, "WebSearch") {
        registry.register_tool(Arc::new(WebSearchTool::new()));
    }

    if is_core_tool_enabled(selection_config, "LSP") {
        registry.register_tool(Arc::new(LspTool::new()));
    }

    if is_core_tool_enabled(selection_config, "next_edit") {
        registry.register_tool(Arc::new(NextEditTool::new()));
    }
}
