// 已注册进 ToolRegistry 的工具实现。
// 注意：bash / enter_plan_mode / exit_plan_mode / todo / tool_search / web_search
// 曾在此有一份从未注册的副本，权威实现在 src/core/tools/ 下，副本已删除。
pub mod editor;
pub mod git_insight;
pub mod github_pr_comments;
pub mod lsp;
pub mod mcp_tool;
pub mod memory;
pub mod next_edit;
pub mod search;

pub use search::SearchTool;
