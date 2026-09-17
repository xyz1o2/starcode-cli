//! Tool-related constants — merged from tool_names + tool_error
//!
//! 使用 `ToolName` 枚举管理所有工具名称，避免魔法字符串。

use std::fmt;

// ── Tool Name Enum ──────────────────────────────────────────────────
// 所有工具名称的单一事实源

/// 工具名称枚举 - 替代所有魔法字符串
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    // 文件操作
    Read,
    ReadManyFiles,
    Write,
    Edit,
    SmartEdit,
    MultiEdit,
    NotebookRead,
    NotebookEdit,

    // 搜索/导航
    Grep,
    Glob,
    ListDir,
    LSP,
    WebSearch,
    WebFetch,

    // 执行
    Bash,
    PowerShell,
    WebBrowser,
    RunTests,

    // 代理/任务
    Agent,
    Skill,
    Todo,
    ManageTasks, // 别名，映射到 Todo

    // 分析
    GetDiagnostics,
    CodebaseSearch,
    ProjectMap,
    NextEdit,

    // Git/GitHub
    GitInsight,
    GitBranch,
    GitRewind,
    GitCommitAttribution,
    GitAutofixPr,
    GitPrSubscribe,
    GhPrComments,
    GitHubApp,
    GitHubIssue,
    SuggestPr,
    SubscribePr,
    SuggestBackgroundPr,

    // 模式切换
    EnterPlanMode,
    ExitPlanMode,
    EnterWorktree,
    ExitWorktree,

    // 任务管理
    TaskGet,
    TaskList,
    TaskUpdate,
    TaskOutput,

    // 调度
    Wait,
    CronCreate,
    CronList,
    CronDelete,
    BackgroundTask,
    RemoteTrigger,
    ScheduleWakeup,

    // 其他
    AskUserQuestion,
    ToolSearch,
    McpAuth,
    Snip,
    SendMessage,
    Monitor,
    Brief,
    Workflow,
    Memory,
    LocalMemoryRecall,
    SyntheticOutput,
    Repl,

    // MCP
    McpListServers,
    McpListTools,
    McpToolInfo,
    McpSearchTools,
    McpRestartServer,
    McpRefresh,
    McpListResources,
    McpReadResource,
    McpListPrompts,
    McpGetPrompt,

    // 内部
    Compaction,
    Title,
    Summary,
}

impl ToolName {
    /// 获取工具名称的字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            // 文件操作
            ToolName::Read => "Read",
            ToolName::ReadManyFiles => "read_many_files",
            ToolName::Write => "Write",
            ToolName::Edit => "Edit",
            ToolName::SmartEdit => "smart_edit",
            ToolName::MultiEdit => "multi_edit",
            ToolName::NotebookRead => "notebook_read",
            ToolName::NotebookEdit => "notebook_edit",

            // 搜索/导航
            ToolName::Grep => "Grep",
            ToolName::Glob => "Glob",
            ToolName::ListDir => "ListDir",
            ToolName::LSP => "LSP",
            ToolName::WebSearch => "WebSearch",
            ToolName::WebFetch => "WebFetch",

            // 执行
            ToolName::Bash => "Bash",
            ToolName::PowerShell => "powershell",
            ToolName::WebBrowser => "web_browser",
            ToolName::RunTests => "run_tests",

            // 代理/任务
            ToolName::Agent => "Agent",
            ToolName::Skill => "skill",
            ToolName::Todo => "TodoWrite",
            ToolName::ManageTasks => "TodoWrite", // 别名

            // 分析
            ToolName::GetDiagnostics => "get_diagnostics",
            ToolName::CodebaseSearch => "CodebaseSearch",
            ToolName::ProjectMap => "ProjectMap",
            ToolName::NextEdit => "next_edit",

            // Git/GitHub
            ToolName::GitInsight => "git_insight",
            ToolName::GitBranch => "git_branch",
            ToolName::GitRewind => "git_rewind",
            ToolName::GitCommitAttribution => "git_commit_attribution",
            ToolName::GitAutofixPr => "git_autofix_pr",
            ToolName::GitPrSubscribe => "git_pr_subscribe",
            ToolName::GhPrComments => "gh_pr_comments",
            ToolName::GitHubApp => "github_app",
            ToolName::GitHubIssue => "github_issue",
            ToolName::SuggestPr => "suggest_pr",
            ToolName::SubscribePr => "subscribe_pr",
            ToolName::SuggestBackgroundPr => "suggest_background_pr",

            // 模式切换
            ToolName::EnterPlanMode => "enter_plan_mode",
            ToolName::ExitPlanMode => "exit_plan_mode",
            ToolName::EnterWorktree => "enter_worktree",
            ToolName::ExitWorktree => "exit_worktree",

            // 任务管理
            ToolName::TaskGet => "task_get",
            ToolName::TaskList => "task_list",
            ToolName::TaskUpdate => "task_update",
            ToolName::TaskOutput => "task_output",

            // 调度
            ToolName::Wait => "wait",
            ToolName::CronCreate => "cron_create",
            ToolName::CronList => "cron_list",
            ToolName::CronDelete => "cron_delete",
            ToolName::BackgroundTask => "background_task",
            ToolName::RemoteTrigger => "remote_trigger",
            ToolName::ScheduleWakeup => "schedule_wakeup",

            // 其他
            ToolName::AskUserQuestion => "ask_user_question",
            ToolName::ToolSearch => "tool_search",
            ToolName::McpAuth => "mcp_auth",
            ToolName::Snip => "snip",
            ToolName::SendMessage => "send_message",
            ToolName::Monitor => "monitor",
            ToolName::Brief => "brief",
            ToolName::Workflow => "workflow",
            ToolName::Memory => "memory",
            ToolName::LocalMemoryRecall => "local_memory_recall",
            ToolName::SyntheticOutput => "synthetic_output",
            ToolName::Repl => "repl",

            // MCP
            ToolName::McpListServers => "mcp_list_servers",
            ToolName::McpListTools => "mcp_list_tools",
            ToolName::McpToolInfo => "mcp_tool_info",
            ToolName::McpSearchTools => "mcp_search_tools",
            ToolName::McpRestartServer => "mcp_restart_server",
            ToolName::McpRefresh => "mcp_refresh",
            ToolName::McpListResources => "mcp_list_resources",
            ToolName::McpReadResource => "mcp_read_resource",
            ToolName::McpListPrompts => "mcp_list_prompts",
            ToolName::McpGetPrompt => "mcp_get_prompt",

            // 内部
            ToolName::Compaction => "compaction",
            ToolName::Title => "title",
            ToolName::Summary => "summary",
        }
    }

    /// 从字符串解析工具名称（用于向后兼容）
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            // 直接匹配
            "Read" => Some(ToolName::Read),
            "read_many_files" => Some(ToolName::ReadManyFiles),
            "Write" => Some(ToolName::Write),
            "Edit" => Some(ToolName::Edit),
            "smart_edit" => Some(ToolName::SmartEdit),
            "multi_edit" => Some(ToolName::MultiEdit),
            "notebook_read" => Some(ToolName::NotebookRead),
            "notebook_edit" => Some(ToolName::NotebookEdit),
            "Grep" => Some(ToolName::Grep),
            "Glob" => Some(ToolName::Glob),
            "ListDir" => Some(ToolName::ListDir),
            "LSP" => Some(ToolName::LSP),
            "WebSearch" => Some(ToolName::WebSearch),
            "WebFetch" => Some(ToolName::WebFetch),
            "Bash" => Some(ToolName::Bash),
            "powershell" => Some(ToolName::PowerShell),
            "web_browser" => Some(ToolName::WebBrowser),
            "run_tests" => Some(ToolName::RunTests),
            "Agent" => Some(ToolName::Agent),
            "skill" => Some(ToolName::Skill),
            "TodoWrite" => Some(ToolName::Todo),
            "get_diagnostics" => Some(ToolName::GetDiagnostics),
            "CodebaseSearch" => Some(ToolName::CodebaseSearch),
            "ProjectMap" => Some(ToolName::ProjectMap),
            "next_edit" => Some(ToolName::NextEdit),
            "git_insight" => Some(ToolName::GitInsight),
            "git_branch" => Some(ToolName::GitBranch),
            "git_rewind" => Some(ToolName::GitRewind),
            "git_commit_attribution" => Some(ToolName::GitCommitAttribution),
            "git_autofix_pr" => Some(ToolName::GitAutofixPr),
            "git_pr_subscribe" => Some(ToolName::GitPrSubscribe),
            "gh_pr_comments" => Some(ToolName::GhPrComments),
            "github_app" => Some(ToolName::GitHubApp),
            "github_issue" => Some(ToolName::GitHubIssue),
            "suggest_pr" => Some(ToolName::SuggestPr),
            "subscribe_pr" => Some(ToolName::SubscribePr),
            "suggest_background_pr" => Some(ToolName::SuggestBackgroundPr),
            "enter_plan_mode" => Some(ToolName::EnterPlanMode),
            "exit_plan_mode" => Some(ToolName::ExitPlanMode),
            "enter_worktree" => Some(ToolName::EnterWorktree),
            "exit_worktree" => Some(ToolName::ExitWorktree),
            "task_get" => Some(ToolName::TaskGet),
            "task_list" => Some(ToolName::TaskList),
            "task_update" => Some(ToolName::TaskUpdate),
            "task_output" => Some(ToolName::TaskOutput),
            "wait" => Some(ToolName::Wait),
            "cron_create" => Some(ToolName::CronCreate),
            "cron_list" => Some(ToolName::CronList),
            "cron_delete" => Some(ToolName::CronDelete),
            "background_task" => Some(ToolName::BackgroundTask),
            "remote_trigger" => Some(ToolName::RemoteTrigger),
            "schedule_wakeup" => Some(ToolName::ScheduleWakeup),
            "ask_user_question" => Some(ToolName::AskUserQuestion),
            "tool_search" => Some(ToolName::ToolSearch),
            "mcp_auth" => Some(ToolName::McpAuth),
            "snip" => Some(ToolName::Snip),
            "send_message" => Some(ToolName::SendMessage),
            "monitor" => Some(ToolName::Monitor),
            "brief" => Some(ToolName::Brief),
            "workflow" => Some(ToolName::Workflow),
            "memory" => Some(ToolName::Memory),
            "local_memory_recall" => Some(ToolName::LocalMemoryRecall),
            "synthetic_output" => Some(ToolName::SyntheticOutput),
            "repl" => Some(ToolName::Repl),
            "mcp_list_servers" => Some(ToolName::McpListServers),
            "mcp_list_tools" => Some(ToolName::McpListTools),
            "mcp_tool_info" => Some(ToolName::McpToolInfo),
            "mcp_search_tools" => Some(ToolName::McpSearchTools),
            "mcp_restart_server" => Some(ToolName::McpRestartServer),
            "mcp_refresh" => Some(ToolName::McpRefresh),
            "mcp_list_resources" => Some(ToolName::McpListResources),
            "mcp_read_resource" => Some(ToolName::McpReadResource),
            "mcp_list_prompts" => Some(ToolName::McpListPrompts),
            "mcp_get_prompt" => Some(ToolName::McpGetPrompt),
            "compaction" => Some(ToolName::Compaction),
            "title" => Some(ToolName::Title),
            "summary" => Some(ToolName::Summary),

            _ => None,
        }
    }

    /// 获取所有内置工具名称
    pub fn all_builtin() -> Vec<ToolName> {
        vec![
            // 文件工具
            ToolName::Read,
            ToolName::ReadManyFiles,
            ToolName::Write,
            ToolName::Edit,
            ToolName::SmartEdit,
            ToolName::MultiEdit,
            ToolName::NotebookRead,
            ToolName::NotebookEdit,
            // 搜索/导航
            ToolName::Grep,
            ToolName::Glob,
            ToolName::ListDir,
            ToolName::LSP,
            ToolName::WebSearch,
            ToolName::WebFetch,
            // 执行
            ToolName::Bash,
            ToolName::PowerShell,
            ToolName::WebBrowser,
            ToolName::RunTests,
            // 代理/任务
            ToolName::Agent,
            ToolName::Skill,
            ToolName::Todo,
            // 分析
            ToolName::GetDiagnostics,
            ToolName::CodebaseSearch,
            ToolName::ProjectMap,
            ToolName::NextEdit,
            // Git/GitHub
            ToolName::GitInsight,
            ToolName::GitBranch,
            ToolName::GitRewind,
            ToolName::GitCommitAttribution,
            ToolName::GitAutofixPr,
            ToolName::GitPrSubscribe,
            ToolName::GhPrComments,
            ToolName::GitHubApp,
            ToolName::GitHubIssue,
            ToolName::SuggestPr,
            // 模式切换
            ToolName::EnterPlanMode,
            ToolName::ExitPlanMode,
            ToolName::EnterWorktree,
            ToolName::ExitWorktree,
            // 任务管理
            ToolName::TaskGet,
            ToolName::TaskList,
            ToolName::TaskUpdate,
            ToolName::TaskOutput,
            // 调度
            ToolName::Wait,
            ToolName::CronCreate,
            ToolName::CronList,
            ToolName::CronDelete,
            ToolName::BackgroundTask,
            ToolName::RemoteTrigger,
            ToolName::ScheduleWakeup,
            // 其他
            ToolName::AskUserQuestion,
            ToolName::ToolSearch,
            ToolName::McpAuth,
            ToolName::Snip,
            ToolName::SendMessage,
            ToolName::Monitor,
            ToolName::Brief,
            ToolName::Workflow,
            ToolName::Memory,
            ToolName::SyntheticOutput,
            ToolName::Repl,
            // MCP
            ToolName::McpListServers,
            ToolName::McpListTools,
            ToolName::McpToolInfo,
            ToolName::McpSearchTools,
            ToolName::McpRestartServer,
            ToolName::McpRefresh,
            ToolName::McpListResources,
            ToolName::McpReadResource,
            ToolName::McpListPrompts,
            ToolName::McpGetPrompt,
        ]
    }

    /// 判断是否是只读工具
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            ToolName::Read
                | ToolName::ReadManyFiles
                | ToolName::Grep
                | ToolName::Glob
                | ToolName::ListDir
                | ToolName::CodebaseSearch
                | ToolName::ProjectMap
                | ToolName::GetDiagnostics
                | ToolName::LSP
                | ToolName::WebSearch
                | ToolName::WebFetch
                | ToolName::ToolSearch
                | ToolName::TaskGet
                | ToolName::TaskList
                | ToolName::Monitor
                | ToolName::Brief
                | ToolName::Memory
                | ToolName::LocalMemoryRecall
        )
    }

    /// 判断是否是编辑工具
    pub fn is_edit_tool(&self) -> bool {
        matches!(
            self,
            ToolName::Edit
                | ToolName::SmartEdit
                | ToolName::MultiEdit
                | ToolName::Write
                | ToolName::NotebookEdit
        )
    }

    /// 判断是否是执行工具
    pub fn is_execute_tool(&self) -> bool {
        matches!(
            self,
            ToolName::Bash | ToolName::PowerShell | ToolName::RunTests
        )
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl AsRef<str> for ToolName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// 获取所有内置工具名称
pub fn all_builtin_tool_names() -> Vec<&'static str> {
    ToolName::all_builtin().iter().map(|t| t.as_str()).collect()
}

/// 从字符串获取规范化的工具名称（别名归一到 `ToolName`）
pub fn canonical_tool_name(name: &str) -> String {
    ToolName::from_str(name)
        .map(|t| t.as_str().to_string())
        .unwrap_or_else(|| name.to_string())
}

// ── Tool error types ─────────────────────────────────────────────────

/// Canonical marker string for edit_file_not_read errors.
/// Used both in tool error messages and agent-side detection.
pub const EDIT_FILE_NOT_READ_MARKER: &str = "[edit_file_not_read]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolErrorType {
    InvalidToolParams,
    Unknown,
    UnhandledException,
    ToolNotRegistered,
    ExecutionFailed,
    FileNotFound,
    FileWriteFailure,
    ReadContentFailure,
    AttemptToCreateExistingFile,
    FileTooLarge,
    PermissionDenied,
    NoSpaceLeft,
    TargetIsDirectory,
    PathNotInWorkspace,
    SearchPathNotFound,
    SearchPathNotADirectory,
    EditPreparationFailure,
    EditNoOccurrenceFound,
    EditExpectedOccurrenceMismatch,
    EditNoChange,
    EditNoChangeLlmJudgement,
    EditFileNotRead,
    EditFileModified,
    FullFileRewriteBlocked,
    GlobExecutionError,
    GrepExecutionError,
    LsExecutionError,
    PathIsNotADirectory,
    McpToolError,
    MemoryToolExecutionError,
    ReadManyFilesSearchError,
    ShellExecuteError,
    DiscoveredToolExecutionError,
    WebFetchNoUrlInPrompt,
    WebFetchFallbackFailed,
    WebFetchProcessingError,
    WebSearchFailed,
    StopExecution,
}

impl std::fmt::Display for ToolErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl ToolErrorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolErrorType::InvalidToolParams => "invalid_tool_params",
            ToolErrorType::Unknown => "unknown",
            ToolErrorType::UnhandledException => "unhandled_exception",
            ToolErrorType::ToolNotRegistered => "tool_not_registered",
            ToolErrorType::ExecutionFailed => "execution_failed",
            ToolErrorType::FileNotFound => "file_not_found",
            ToolErrorType::FileWriteFailure => "file_write_failure",
            ToolErrorType::ReadContentFailure => "read_content_failure",
            ToolErrorType::AttemptToCreateExistingFile => "attempt_to_create_existing_file",
            ToolErrorType::FileTooLarge => "file_too_large",
            ToolErrorType::PermissionDenied => "permission_denied",
            ToolErrorType::NoSpaceLeft => "no_space_left",
            ToolErrorType::TargetIsDirectory => "target_is_directory",
            ToolErrorType::PathNotInWorkspace => "path_not_in_workspace",
            ToolErrorType::SearchPathNotFound => "search_path_not_found",
            ToolErrorType::SearchPathNotADirectory => "search_path_not_a_directory",
            ToolErrorType::EditPreparationFailure => "edit_preparation_failure",
            ToolErrorType::EditNoOccurrenceFound => "edit_no_occurrence_found",
            ToolErrorType::EditExpectedOccurrenceMismatch => "edit_expected_occurrence_mismatch",
            ToolErrorType::EditNoChange => "edit_no_change",
            ToolErrorType::EditNoChangeLlmJudgement => "edit_no_change_llm_judgement",
            ToolErrorType::EditFileNotRead => EDIT_FILE_NOT_READ_MARKER,
            ToolErrorType::EditFileModified => "edit_file_modified",
            ToolErrorType::FullFileRewriteBlocked => "full_file_rewrite_blocked",
            ToolErrorType::GlobExecutionError => "glob_execution_error",
            ToolErrorType::GrepExecutionError => "grep_execution_error",
            ToolErrorType::LsExecutionError => "ls_execution_error",
            ToolErrorType::PathIsNotADirectory => "path_is_not_a_directory",
            ToolErrorType::McpToolError => "mcp_tool_error",
            ToolErrorType::MemoryToolExecutionError => "memory_tool_execution_error",
            ToolErrorType::ReadManyFilesSearchError => "read_many_files_search_error",
            ToolErrorType::ShellExecuteError => "shell_execute_error",
            ToolErrorType::DiscoveredToolExecutionError => "discovered_tool_execution_error",
            ToolErrorType::WebFetchNoUrlInPrompt => "web_fetch_no_url_in_prompt",
            ToolErrorType::WebFetchFallbackFailed => "web_fetch_fallback_failed",
            ToolErrorType::WebFetchProcessingError => "web_fetch_processing_error",
            ToolErrorType::WebSearchFailed => "web_search_failed",
            ToolErrorType::StopExecution => "stop_execution",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "invalid_tool_params" => Some(ToolErrorType::InvalidToolParams),
            "unknown" => Some(ToolErrorType::Unknown),
            "unhandled_exception" => Some(ToolErrorType::UnhandledException),
            "tool_not_registered" => Some(ToolErrorType::ToolNotRegistered),
            "execution_failed" => Some(ToolErrorType::ExecutionFailed),
            "file_not_found" => Some(ToolErrorType::FileNotFound),
            "file_write_failure" => Some(ToolErrorType::FileWriteFailure),
            "read_content_failure" => Some(ToolErrorType::ReadContentFailure),
            "attempt_to_create_existing_file" => Some(ToolErrorType::AttemptToCreateExistingFile),
            "file_too_large" => Some(ToolErrorType::FileTooLarge),
            "permission_denied" => Some(ToolErrorType::PermissionDenied),
            "no_space_left" => Some(ToolErrorType::NoSpaceLeft),
            "target_is_directory" => Some(ToolErrorType::TargetIsDirectory),
            "path_not_in_workspace" => Some(ToolErrorType::PathNotInWorkspace),
            "search_path_not_found" => Some(ToolErrorType::SearchPathNotFound),
            "search_path_not_a_directory" => Some(ToolErrorType::SearchPathNotADirectory),
            "edit_preparation_failure" => Some(ToolErrorType::EditPreparationFailure),
            "edit_no_occurrence_found" => Some(ToolErrorType::EditNoOccurrenceFound),
            "edit_expected_occurrence_mismatch" => {
                Some(ToolErrorType::EditExpectedOccurrenceMismatch)
            }
            "edit_no_change" => Some(ToolErrorType::EditNoChange),
            "edit_no_change_llm_judgement" => Some(ToolErrorType::EditNoChangeLlmJudgement),
            s if s == EDIT_FILE_NOT_READ_MARKER => Some(ToolErrorType::EditFileNotRead),
            "edit_file_modified" => Some(ToolErrorType::EditFileModified),
            "full_file_rewrite_blocked" => Some(ToolErrorType::FullFileRewriteBlocked),
            "glob_execution_error" => Some(ToolErrorType::GlobExecutionError),
            "grep_execution_error" => Some(ToolErrorType::GrepExecutionError),
            "ls_execution_error" => Some(ToolErrorType::LsExecutionError),
            "path_is_not_a_directory" => Some(ToolErrorType::PathIsNotADirectory),
            "mcp_tool_error" => Some(ToolErrorType::McpToolError),
            "memory_tool_execution_error" => Some(ToolErrorType::MemoryToolExecutionError),
            "read_many_files_search_error" => Some(ToolErrorType::ReadManyFilesSearchError),
            "shell_execute_error" => Some(ToolErrorType::ShellExecuteError),
            "discovered_tool_execution_error" => Some(ToolErrorType::DiscoveredToolExecutionError),
            "web_fetch_no_url_in_prompt" => Some(ToolErrorType::WebFetchNoUrlInPrompt),
            "web_fetch_fallback_failed" => Some(ToolErrorType::WebFetchFallbackFailed),
            "web_fetch_processing_error" => Some(ToolErrorType::WebFetchProcessingError),
            "web_search_failed" => Some(ToolErrorType::WebSearchFailed),
            "stop_execution" => Some(ToolErrorType::StopExecution),
            _ => None,
        }
    }
}

pub fn is_fatal_tool_error(error_type: Option<&str>) -> bool {
    match error_type {
        Some("no_space_left") => true,
        _ => false,
    }
}
