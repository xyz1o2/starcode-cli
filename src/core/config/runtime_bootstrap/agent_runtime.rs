use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::state::GlobalState;
use crate::core::tools::ToolRegistry;
use crate::llm::client::StarClient;
use std::sync::Arc;

pub(super) fn register_agent_runtime_tools(
    tool_registry: &Arc<ToolRegistry>,
    config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
    client: &StarClient,
) {
    register_skill_tool(client, config, tool_registry);
    register_agent_editing_tools(tool_registry, config, message_bus, global_state, client);
    register_agent_observability_tools(tool_registry);
    register_agent_execution_tools(tool_registry, config, message_bus, client);
    register_tool_search(tool_registry);
}

fn register_agent_editing_tools(
    tool_registry: &Arc<ToolRegistry>,
    config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
    client: &StarClient,
) {
    use crate::tools::editor::smart_edit::SmartEditTool;
    tool_registry.register_tool(Arc::new(SmartEditTool::new(client.clone())));

    use crate::core::tools::multi_edit::MultiEditTool;
    tool_registry.register_tool(Arc::new(MultiEditTool::new(
        config.clone(),
        global_state.clone(),
    )));

    use crate::core::tools::notebook_edit::NotebookEditTool;
    tool_registry.register_tool(Arc::new(NotebookEditTool::new(
        config.clone(),
        global_state.clone(),
    )));

    use crate::core::tools::notebook_read::NotebookReadTool;
    tool_registry.register_tool(Arc::new(NotebookReadTool::new(config.clone())));

    crate::utils::logging::append_debug_log_line(
        "SmartEditTool registered (LLM fix strategy enabled).",
    );

    ensure_fallback_core_tools(config, tool_registry, message_bus, global_state);
}

fn register_agent_observability_tools(tool_registry: &Arc<ToolRegistry>) {
    use crate::core::tools::web_fetch::WebFetchTool;
    tool_registry.register_tool(Arc::new(WebFetchTool::new()));

    use crate::core::tools::web_search::WebSearchTool;
    tool_registry.register_tool(Arc::new(WebSearchTool::new()));

    use crate::tools::git_insight::GitInsightTool;
    tool_registry.register_tool(Arc::new(GitInsightTool::new()));

    use crate::tools::github_pr_comments::GhPrCommentsTool;
    tool_registry.register_tool(Arc::new(GhPrCommentsTool::new()));

    use crate::tools::memory::MemoryTool;
    tool_registry.register_tool(Arc::new(MemoryTool::new()));
}

fn register_agent_execution_tools(
    tool_registry: &Arc<ToolRegistry>,
    config: &Arc<Config>,
    message_bus: &Arc<MessageBus>,
    client: &StarClient,
) {
    // 后台 SubAgent 异步执行器 + 全局通知队列
    let subagent_runner = crate::agent::StarAgentRunner::shared(client.clone(), config.clone());
    let async_runner = config.runtime_notification_queue().map(|queue| {
        Arc::new(crate::agent::subagent::runner::AsyncSubagentRunner::new(
            subagent_runner.clone(),
            queue,
        ))
    });

    use crate::core::tools::agent_tool::AgentTool;
    let mut agent_tool = AgentTool::new(subagent_runner.clone());
    if let Some(ref runner) = async_runner {
        crate::utils::logging::append_debug_log_line(
            "[AgentTool] AsyncSubagentRunner installed (background subagents enabled).",
        );
        agent_tool = agent_tool.with_async_runner(runner.clone());
    }
    tool_registry.register_tool(Arc::new(agent_tool));

    use crate::core::tools::tasks::TaskTool;
    tool_registry.register_tool(Arc::new(TaskTool::new(subagent_runner, config.clone())));

    use crate::core::tools::diagnostics::GetDiagnosticsTool;
    tool_registry.register_tool(Arc::new(GetDiagnosticsTool::new(config.clone())));

    use crate::core::tools::enter_plan_mode::EnterPlanModeTool;
    tool_registry.register_tool(Arc::new(EnterPlanModeTool::new(
        config.clone(),
        message_bus.clone(),
    )));

    use crate::core::tools::exit_plan_mode::ExitPlanModeTool;
    tool_registry.register_tool(Arc::new(ExitPlanModeTool::new(
        config.clone(),
        message_bus.clone(),
    )));

    use crate::core::tools::enter_worktree::EnterWorktreeTool;
    tool_registry.register_tool(Arc::new(EnterWorktreeTool::new(
        config.clone(),
        message_bus.clone(),
    )));

    use crate::core::tools::exit_worktree::ExitWorktreeTool;
    tool_registry.register_tool(Arc::new(ExitWorktreeTool::new(
        config.clone(),
        message_bus.clone(),
    )));

    use crate::core::tools::project_map::ProjectMapTool;
    tool_registry.register_tool(Arc::new(ProjectMapTool::new(config.clone())));

    use crate::core::tools::run_tests::RunTestsTool;
    tool_registry.register_tool(Arc::new(RunTestsTool::new(config.clone())));

    use crate::core::tools::semantic_search::SemanticSearchTool;
    tool_registry.register_tool(Arc::new(SemanticSearchTool::new(config.clone())));

    use crate::core::tools::shell::ShellTool;
    tool_registry.register_tool(Arc::new(ShellTool::new(
        config.clone(),
        Some(client.clone()),
    )));

    // ── Scheduling & Background ──────────────────────────────
    use crate::core::tools::background_task::BackgroundTaskTool;
    tool_registry.register_tool(Arc::new(BackgroundTaskTool::new(config.clone())));

    use crate::core::tools::cron::{CronCreateTool, CronDeleteTool, CronListTool};
    tool_registry.register_tool(Arc::new(CronCreateTool::new(config.clone())));
    tool_registry.register_tool(Arc::new(CronListTool::new(config.clone())));
    tool_registry.register_tool(Arc::new(CronDeleteTool::new(config.clone())));

    // ── Remote Trigger ───────────────────────────────────────
    use crate::core::tools::remote_trigger::RemoteTriggerTool;
    tool_registry.register_tool(Arc::new(RemoteTriggerTool::new()));

    // ── Snippets ─────────────────────────────────────────────
    use crate::core::tools::snip::SnipTool;
    tool_registry.register_tool(Arc::new(SnipTool::new(config.clone())));

    // ── PR Suggestions ───────────────────────────────────────
    use crate::core::tools::suggest_pr::SuggestBackgroundPRTool;
    tool_registry.register_tool(Arc::new(SuggestBackgroundPRTool::new(config.clone())));

    // ── Schedule Wakeup ──────────────────────────────────────
    use crate::core::tools::schedule_wakeup::ScheduleWakeupTool;
    tool_registry.register_tool(Arc::new(ScheduleWakeupTool::new()));

    // ── MCP Auth ─────────────────────────────────────────────
    use crate::core::tools::mcp_auth::McpAuthTool;
    tool_registry.register_tool(Arc::new(McpAuthTool::new()));

    // ── Cross-Agent Messaging ───────────────────────────────
    use crate::core::tools::cross_agent::SendMessageTool;
    let mut send_msg_tool = SendMessageTool::new(config.clone());
    if let Some(ref runner) = async_runner {
        send_msg_tool = send_msg_tool.with_agent_registry(runner.clone());
    }
    tool_registry.register_tool(Arc::new(send_msg_tool));

    // ── Task Management Extensions ──────────────────────────
    use crate::core::tools::task_management::{
        TaskGetTool, TaskListTool, TaskOutputTool, TaskUpdateTool,
    };
    tool_registry.register_tool(Arc::new(TaskGetTool::new(config.clone())));
    tool_registry.register_tool(Arc::new(TaskListTool::new(config.clone())));
    tool_registry.register_tool(Arc::new(TaskUpdateTool::new(config.clone())));
    tool_registry.register_tool(Arc::new(TaskOutputTool::new(config.clone())));

    // ── Process Monitor ─────────────────────────────────────
    use crate::core::tools::monitor::MonitorTool;
    tool_registry.register_tool(Arc::new(MonitorTool::new()));

    // ── Brief/Summary ───────────────────────────────────────
    use crate::core::tools::brief::BriefTool;
    tool_registry.register_tool(Arc::new(BriefTool::new()));

    // ── Workflow Execution ──────────────────────────────────
    use crate::core::tools::workflow::WorkflowTool;
    tool_registry.register_tool(Arc::new(WorkflowTool::new(config.clone())));

    // ── Git Advanced Features ───────────────────────────────
    use crate::core::tools::git_pr_subscribe::GitPrSubscribeTool;
    tool_registry.register_tool(Arc::new(GitPrSubscribeTool::new(config.clone())));

    use crate::core::tools::git_rewind::GitRewindTool;
    tool_registry.register_tool(Arc::new(GitRewindTool::new(config.clone())));

    use crate::core::tools::git_commit_attribution::GitCommitAttributionTool;
    tool_registry.register_tool(Arc::new(GitCommitAttributionTool::new(config.clone())));

    use crate::core::tools::git_autofix_pr::GitAutofixPrTool;
    tool_registry.register_tool(Arc::new(GitAutofixPrTool::new(config.clone())));

    // ── MCP Resource Tools ──────────────────────────────────
    use crate::core::tools::mcp_resources::McpListResourcesTool;
    tool_registry.register_tool(Arc::new(McpListResourcesTool::new()));

    use crate::core::tools::mcp_resources::McpReadResourceTool;
    tool_registry.register_tool(Arc::new(McpReadResourceTool::new()));
}

fn register_tool_search(tool_registry: &Arc<ToolRegistry>) {
    use crate::core::tools::tool_search::ToolSearchTool;
    tool_registry.register_tool(Arc::new(ToolSearchTool::new(tool_registry.clone())));
}

fn register_skill_tool(
    client: &StarClient,
    config: &Arc<Config>,
    tool_registry: &Arc<ToolRegistry>,
) {
    use crate::agent::tools::SkillTool;

    let env_enable_skill = std::env::var("STAR_ENABLE_SKILL_TOOL")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(false);
    let env_disable_skill = std::env::var("STAR_DISABLE_SKILL_TOOL")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(false);
    let enable_skill_tool = (config.skills_support() || env_enable_skill) && !env_disable_skill;

    if config.recursion_depth == 0 && enable_skill_tool {
        tool_registry.register_tool(Arc::new(SkillTool::new(client.clone(), config.clone())));
    } else if config.recursion_depth > 0 {
        crate::utils::logging::append_debug_log_line(
            "[SkillTool] Skipped registration for sub-agent to prevent recursion.",
        );
    } else {
        crate::utils::logging::append_debug_log_line(
            "[SkillTool] Disabled (set STAR_ENABLE_SKILL_TOOL=1 to enable).",
        );
    }
}

fn ensure_fallback_core_tools(
    config: &Arc<Config>,
    tool_registry: &Arc<ToolRegistry>,
    message_bus: &Arc<MessageBus>,
    global_state: &Arc<GlobalState>,
) {
    let registered_tools = tool_registry.get_function_declarations();
    let tool_names: Vec<String> = registered_tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    crate::utils::logging::append_debug_log_line(&format!(
        "Registered tool list: {:?}",
        tool_names
    ));

    // Critical file tools - must always be available
    if !tool_names
        .iter()
        .any(|name| name == "Read" || name == "view_file")
    {
        crate::utils::logging::append_debug_log_line(
            "Read/view_file missing, attempting manual registration.",
        );
        use crate::core::tools::ReadFileTool;
        tool_registry.register_tool(Arc::new(ReadFileTool::new(
            config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }
    if !tool_names.iter().any(|name| name == "Write") {
        crate::utils::logging::append_debug_log_line(
            "Write missing, attempting manual registration.",
        );
        use crate::core::tools::WriteFileTool;
        tool_registry.register_tool(Arc::new(WriteFileTool::new(
            config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }
    if !tool_names
        .iter()
        .any(|name| name == "Edit" || name == "edit")
    {
        crate::utils::logging::append_debug_log_line(
            "Edit/edit missing, attempting manual registration.",
        );
        use crate::core::tools::EditTool;
        tool_registry.register_tool(Arc::new(EditTool::new(
            config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }

    // Critical search tools
    if !tool_names.iter().any(|name| name == "Grep") {
        crate::utils::logging::append_debug_log_line(
            "Grep missing, attempting manual registration.",
        );
        use crate::core::tools::grep::GrepTool;
        tool_registry.register_tool(Arc::new(GrepTool::new(config.clone(), message_bus.clone())));
    }

    // Other important tools
    if !tool_names.iter().any(|name| name == "read_many_files") {
        crate::utils::logging::append_debug_log_line(
            "read_many_files missing, attempting manual registration.",
        );
        use crate::core::tools::read_many::ReadManyFilesTool;
        tool_registry.register_tool(Arc::new(ReadManyFilesTool::new(
            config.clone(),
            global_state.clone(),
        )));
    }
    if !tool_names.iter().any(|name| name == "ListDir") {
        crate::utils::logging::append_debug_log_line(
            "ListDir (ListDirTool) missing, attempting manual registration.",
        );
        use crate::core::tools::ls::ListDirTool;
        tool_registry.register_tool(Arc::new(ListDirTool::new()));
    }
    if !tool_names.iter().any(|name| name == "Glob") {
        crate::utils::logging::append_debug_log_line(
            "Glob (GlobMatchTool) missing, attempting manual registration.",
        );
        use crate::core::tools::glob::GlobMatchTool;
        tool_registry.register_tool(Arc::new(GlobMatchTool::new(
            config.clone(),
            message_bus.clone(),
            global_state.clone(),
        )));
    }
}
