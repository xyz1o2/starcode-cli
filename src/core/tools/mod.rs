// 工具实现模块。
// 只登记真正被 runtime_bootstrap（或 agent_core）注册进 ToolRegistry 的工具，
// 以及被其它模块复用的辅助模块；从未注册、模型碰不到的工具模块已移除。
pub mod agent_tool;
pub mod ask_user_question;
pub mod background_task;
pub mod brief;
pub mod constants;
pub mod cron;
pub mod cross_agent;
pub mod diagnostics;
pub mod diff_options;
pub mod edit;
pub mod enhanced_tool;
pub mod enter_plan_mode;
pub mod enter_worktree;
pub mod exit_plan_mode;
pub mod exit_worktree;
pub mod git_autofix_pr;
pub mod git_commit_attribution;
pub mod git_pr_subscribe;
pub mod git_rewind;
mod git_utils;
pub mod glob;
/// GrepTool：ensure_fallback_core_tools 的兜底实现（主实现是 crate::tools::search::SearchTool）。
pub mod grep;
pub mod ls;
pub mod mcp_auth;
pub mod mcp_resources;
/// MemoryTool 结构体已废弃未注册（实现见 crate::tools::memory），
/// 但 get_global_memory_file_path 等路径辅助函数仍被 agent::context 使用。
pub mod memory_tool;
pub mod modifiable_tool;
pub mod monitor;
pub mod multi_edit;
pub mod notebook_edit;
pub mod notebook_read;
pub mod project_map;
pub mod read_file;
pub mod read_many;
pub mod remote_trigger;
pub mod result_budget;
pub mod ripgrep;
pub mod rtk;
pub mod run_tests;
pub mod schedule_wakeup;
pub mod semantic_search;
pub mod shell;
pub mod skill;
pub mod sleep;
pub mod snip;
pub mod suggest_pr;
pub mod task_management;
pub mod tasks;
pub mod tool_registry;
pub mod tool_search;
pub mod tools;
pub mod verify_edit;
pub mod web_fetch;
pub mod web_search;
pub mod workflow;
pub mod write_file;

pub use edit::*;
pub use ls::*;
pub use read_file::*;
pub use tool_registry::*;
pub use tools::*;
pub use write_file::*;
