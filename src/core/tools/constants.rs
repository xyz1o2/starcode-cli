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
    LS,
    LSP,
    WebSearch,
    WebFetch,

    // 执行
    Bash,
    RunTests,

    // 代理/任务
    Agent,
    Skill,
    Todo,

    // 分析
    GetDiagnostics,
    CodebaseSearch,
    ProjectMap,
    CodeGraph,
    NextEdit,

    // Git/GitHub
    GitInsight,
    GitRewind,
    GitCommitAttribution,
    GitAutofixPr,
    GitPrSubscribe,
    GhPrComments,
    SuggestPr,

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
    WorkflowTool,
    Memory,
    /// 模型自调自身运行期设置的工具，见 `core::self_tuning`。
    SelfConfig,

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
}

impl ToolName {
    /// 获取工具名称的字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            // 文件操作
            ToolName::Read => "Read",
            ToolName::ReadManyFiles => "ReadManyFiles",
            ToolName::Write => "Write",
            ToolName::Edit => "Edit",
            ToolName::SmartEdit => "SmartEdit",
            ToolName::MultiEdit => "MultiEdit",
            ToolName::NotebookRead => "NotebookRead",
            ToolName::NotebookEdit => "NotebookEdit",

            // 搜索/导航
            ToolName::Grep => "Grep",
            ToolName::Glob => "Glob",
            ToolName::LS => "LS",
            ToolName::LSP => "LSP",
            ToolName::WebSearch => "WebSearch",
            ToolName::WebFetch => "WebFetch",

            // 执行
            ToolName::Bash => "Bash",
            ToolName::RunTests => "RunTests",

            // 代理/任务
            ToolName::Agent => "Agent",
            ToolName::Skill => "Skill",
            ToolName::Todo => "TodoWrite",

            // 分析
            ToolName::GetDiagnostics => "GetDiagnostics",
            ToolName::CodebaseSearch => "CodebaseSearch",
            ToolName::ProjectMap => "ProjectMap",
            ToolName::CodeGraph => "CodeGraph",
            ToolName::NextEdit => "NextEdit",

            // Git/GitHub
            ToolName::GitInsight => "GitInsight",
            ToolName::GitRewind => "GitRewind",
            ToolName::GitCommitAttribution => "GitCommitAttribution",
            ToolName::GitAutofixPr => "GitAutofixPr",
            ToolName::GitPrSubscribe => "GitPrSubscribe",
            ToolName::GhPrComments => "GhPrComments",
            ToolName::SuggestPr => "SuggestPr",

            // 模式切换
            ToolName::EnterPlanMode => "EnterPlanMode",
            ToolName::ExitPlanMode => "ExitPlanMode",
            ToolName::EnterWorktree => "EnterWorktree",
            ToolName::ExitWorktree => "ExitWorktree",

            // 任务管理
            ToolName::TaskGet => "TaskGet",
            ToolName::TaskList => "TaskList",
            ToolName::TaskUpdate => "TaskUpdate",
            ToolName::TaskOutput => "TaskOutput",

            // 调度
            ToolName::Wait => "Wait",
            ToolName::CronCreate => "CronCreate",
            ToolName::CronList => "CronList",
            ToolName::CronDelete => "CronDelete",
            ToolName::BackgroundTask => "BackgroundTask",
            ToolName::RemoteTrigger => "RemoteTrigger",
            ToolName::ScheduleWakeup => "ScheduleWakeup",

            // 其他
            ToolName::AskUserQuestion => "AskUserQuestion",
            ToolName::ToolSearch => "ToolSearch",
            ToolName::McpAuth => "McpAuth",
            ToolName::Snip => "Snip",
            ToolName::SendMessage => "SendMessage",
            ToolName::Monitor => "Monitor",
            ToolName::Brief => "Brief",
            ToolName::WorkflowTool => "WorkflowTool",
            ToolName::Memory => "Memory",
            ToolName::SelfConfig => "SelfConfig",

            // MCP
            ToolName::McpListServers => "McpListServers",
            ToolName::McpListTools => "McpListTools",
            ToolName::McpToolInfo => "McpToolInfo",
            ToolName::McpSearchTools => "McpSearchTools",
            ToolName::McpRestartServer => "McpRestartServer",
            ToolName::McpRefresh => "McpRefresh",
            ToolName::McpListResources => "McpListResources",
            ToolName::McpReadResource => "McpReadResource",
            ToolName::McpListPrompts => "McpListPrompts",
            ToolName::McpGetPrompt => "McpGetPrompt",
        }
    }

    /// 从字符串解析工具名称。
    ///
    /// 对标 Claude Code：内置工具名即唯一名，`from_str` 只认注册名本身，
    /// 不收任何别名（CC 的别名也仅限于它自己改名留下来的 legacy 名）。
    /// LLM 只会发出 schema 里的注册名；用户输入的大小写漂移由
    /// `commands::permissions::normalize_tool_name` 的兜底归一处理。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            // 直接匹配
            "Read" => Some(ToolName::Read),
            "ReadManyFiles" => Some(ToolName::ReadManyFiles),
            "Write" => Some(ToolName::Write),
            "Edit" => Some(ToolName::Edit),
            "SmartEdit" => Some(ToolName::SmartEdit),
            "MultiEdit" => Some(ToolName::MultiEdit),
            "NotebookRead" => Some(ToolName::NotebookRead),
            "NotebookEdit" => Some(ToolName::NotebookEdit),
            "Grep" => Some(ToolName::Grep),
            "Glob" => Some(ToolName::Glob),
            "LS" => Some(ToolName::LS),
            "LSP" => Some(ToolName::LSP),
            "WebSearch" => Some(ToolName::WebSearch),
            "WebFetch" => Some(ToolName::WebFetch),
            "Bash" => Some(ToolName::Bash),
            "RunTests" => Some(ToolName::RunTests),
            "Agent" => Some(ToolName::Agent),
            "Skill" => Some(ToolName::Skill),
            "TodoWrite" => Some(ToolName::Todo),
            "GetDiagnostics" => Some(ToolName::GetDiagnostics),
            "CodebaseSearch" => Some(ToolName::CodebaseSearch),
            "ProjectMap" => Some(ToolName::ProjectMap),
            "CodeGraph" => Some(ToolName::CodeGraph),
            "NextEdit" => Some(ToolName::NextEdit),
            "GitInsight" => Some(ToolName::GitInsight),
            "GitRewind" => Some(ToolName::GitRewind),
            "GitCommitAttribution" => Some(ToolName::GitCommitAttribution),
            "GitAutofixPr" => Some(ToolName::GitAutofixPr),
            "GitPrSubscribe" => Some(ToolName::GitPrSubscribe),
            "GhPrComments" => Some(ToolName::GhPrComments),
            "SuggestPr" => Some(ToolName::SuggestPr),
            "EnterPlanMode" => Some(ToolName::EnterPlanMode),
            "ExitPlanMode" => Some(ToolName::ExitPlanMode),
            "EnterWorktree" => Some(ToolName::EnterWorktree),
            "ExitWorktree" => Some(ToolName::ExitWorktree),
            "TaskGet" => Some(ToolName::TaskGet),
            "TaskList" => Some(ToolName::TaskList),
            "TaskUpdate" => Some(ToolName::TaskUpdate),
            "TaskOutput" => Some(ToolName::TaskOutput),
            "Wait" => Some(ToolName::Wait),
            "CronCreate" => Some(ToolName::CronCreate),
            "CronList" => Some(ToolName::CronList),
            "CronDelete" => Some(ToolName::CronDelete),
            "BackgroundTask" => Some(ToolName::BackgroundTask),
            "RemoteTrigger" => Some(ToolName::RemoteTrigger),
            "ScheduleWakeup" => Some(ToolName::ScheduleWakeup),
            "AskUserQuestion" => Some(ToolName::AskUserQuestion),
            "ToolSearch" => Some(ToolName::ToolSearch),
            "McpAuth" => Some(ToolName::McpAuth),
            "Snip" => Some(ToolName::Snip),
            "SendMessage" => Some(ToolName::SendMessage),
            "Monitor" => Some(ToolName::Monitor),
            "Brief" => Some(ToolName::Brief),
            "WorkflowTool" => Some(ToolName::WorkflowTool),
            "Memory" => Some(ToolName::Memory),
            "SelfConfig" => Some(ToolName::SelfConfig),
            "McpListServers" => Some(ToolName::McpListServers),
            "McpListTools" => Some(ToolName::McpListTools),
            "McpToolInfo" => Some(ToolName::McpToolInfo),
            "McpSearchTools" => Some(ToolName::McpSearchTools),
            "McpRestartServer" => Some(ToolName::McpRestartServer),
            "McpRefresh" => Some(ToolName::McpRefresh),
            "McpListResources" => Some(ToolName::McpListResources),
            "McpReadResource" => Some(ToolName::McpReadResource),
            "McpListPrompts" => Some(ToolName::McpListPrompts),
            "McpGetPrompt" => Some(ToolName::McpGetPrompt),

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
            ToolName::LS,
            ToolName::LSP,
            ToolName::WebSearch,
            ToolName::WebFetch,
            // 执行
            ToolName::Bash,
            ToolName::RunTests,
            // 代理/任务
            ToolName::Agent,
            ToolName::Skill,
            ToolName::Todo,
            // 分析
            ToolName::GetDiagnostics,
            ToolName::CodebaseSearch,
            ToolName::ProjectMap,
            ToolName::CodeGraph,
            ToolName::NextEdit,
            // Git/GitHub
            ToolName::GitInsight,
            ToolName::GitRewind,
            ToolName::GitCommitAttribution,
            ToolName::GitAutofixPr,
            ToolName::GitPrSubscribe,
            ToolName::GhPrComments,
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
            ToolName::WorkflowTool,
            ToolName::Memory,
            ToolName::SelfConfig,
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
                | ToolName::LS
                | ToolName::CodebaseSearch
                | ToolName::ProjectMap
                | ToolName::CodeGraph
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
        matches!(self, ToolName::Bash | ToolName::RunTests)
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

impl ToolName {
    /// 面向用户的工具分组（`/tools` 的展示顺序由调用方的分组表决定）。
    ///
    /// 这个 `match` 必须穷尽：新增一个工具时编译器会逼你给它归类，
    /// 而不是让 `/tools` 静默漏掉它 —— 手写清单已经漂移过不止一次。
    pub fn category(&self) -> &'static str {
        match self {
            // 文件操作
            ToolName::Read
            | ToolName::ReadManyFiles
            | ToolName::Write
            | ToolName::Edit
            | ToolName::SmartEdit
            | ToolName::MultiEdit
            | ToolName::NotebookRead
            | ToolName::NotebookEdit => "Files",
            // 搜索/导航
            ToolName::Grep
            | ToolName::Glob
            | ToolName::LS
            | ToolName::LSP
            | ToolName::CodebaseSearch
            | ToolName::ProjectMap
            | ToolName::CodeGraph
            | ToolName::NextEdit
            | ToolName::ToolSearch
            | ToolName::Snip => "Search",
            // 执行
            ToolName::Bash | ToolName::RunTests | ToolName::GetDiagnostics => "Execution",
            // 代理/任务
            ToolName::Agent
            | ToolName::Skill
            | ToolName::Todo
            | ToolName::TaskGet
            | ToolName::TaskList
            | ToolName::TaskUpdate
            | ToolName::TaskOutput
            | ToolName::SendMessage
            | ToolName::Brief => "Agents & tasks",
            // 模式切换
            ToolName::EnterPlanMode
            | ToolName::ExitPlanMode
            | ToolName::EnterWorktree
            | ToolName::ExitWorktree => "Plan & worktree",
            // Web
            ToolName::WebFetch | ToolName::WebSearch => "Web",
            // Git/GitHub
            ToolName::GitInsight
            | ToolName::GitRewind
            | ToolName::GitCommitAttribution
            | ToolName::GitAutofixPr
            | ToolName::GitPrSubscribe
            | ToolName::GhPrComments
            | ToolName::SuggestPr => "Git",
            // 调度
            ToolName::Wait
            | ToolName::CronCreate
            | ToolName::CronList
            | ToolName::CronDelete
            | ToolName::BackgroundTask
            | ToolName::RemoteTrigger
            | ToolName::ScheduleWakeup
            | ToolName::Monitor => "Scheduling",
            // MCP
            ToolName::McpAuth
            | ToolName::McpListServers
            | ToolName::McpListTools
            | ToolName::McpToolInfo
            | ToolName::McpSearchTools
            | ToolName::McpRestartServer
            | ToolName::McpRefresh
            | ToolName::McpListResources
            | ToolName::McpReadResource
            | ToolName::McpListPrompts
            | ToolName::McpGetPrompt => "MCP",
            // 其他
            ToolName::AskUserQuestion
            | ToolName::WorkflowTool
            | ToolName::Memory
            | ToolName::SelfConfig => "Other",
        }
    }
}

/// 获取所有内置工具名称
pub fn all_builtin_tool_names() -> Vec<&'static str> {
    ToolName::all_builtin().iter().map(|t| t.as_str()).collect()
}

/// 把工具名折叠成「忽略大小写」的比较键。
///
/// 注册名是 PascalCase，模型和用户偶尔会写成 `todowrite` 这种全小写。
/// 折叠大小写后这些拼法就统一了。
///
/// 刻意**不**折叠下划线：`smart_edit` 是改名前的旧注册名，本次改名是硬切换，
/// 它应该落回「未知工具名」而不是被悄悄纠正 —— 否则用户改了名、
/// 旧配置却继续生效，排查起来最难。
fn fold_tool_name(name: &str) -> String {
    name.to_lowercase()
}

/// 从字符串获取规范化的工具名称（归一到注册名）。
///
/// 只认注册名本身 —— 旧的下划线写法不是别名，而是拼错的注册名，
/// 折叠匹配会把它纠正回来（见 [`fold_tool_name`]）。
pub fn canonical_tool_name(name: &str) -> String {
    ToolName::from_str(name)
        .map(|t| t.as_str().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// 按「忽略大小写」的规则查找注册名。
///
/// 供用户手输工具名的场合兜底（`/permissions add`、hook matcher 等）。
/// 找不到时返回 `None` —— 调用方应保留原样，而不是猜一个。
pub fn find_builtin_tool_name(name: &str) -> Option<&'static str> {
    let wanted = fold_tool_name(name);
    all_builtin_tool_names()
        .into_iter()
        .find(|candidate| fold_tool_name(candidate) == wanted)
}

/// 判断工具名是否属于编辑类工具（只认注册名）
pub fn is_edit_tool_name(name: &str) -> bool {
    ToolName::from_str(name).is_some_and(|t| t.is_edit_tool())
}

/// 判断工具名是否属于只读工具（只认注册名）
pub fn is_read_only_tool_name(name: &str) -> bool {
    ToolName::from_str(name).is_some_and(|t| t.is_read_only())
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn registered_names_are_their_own_canonical_form() {
        // 对标 CC：内置工具名即唯一名，canonical_tool_name 对注册名是恒等变换
        assert_eq!(canonical_tool_name("Read"), "Read");
        assert_eq!(canonical_tool_name("Write"), "Write");
        assert_eq!(canonical_tool_name("Edit"), "Edit");
        assert_eq!(canonical_tool_name("Grep"), "Grep");
        assert_eq!(canonical_tool_name("Glob"), "Glob");
        assert_eq!(canonical_tool_name("Bash"), "Bash");
        assert_eq!(canonical_tool_name("Agent"), "Agent");
        assert_eq!(canonical_tool_name("CodebaseSearch"), "CodebaseSearch");
        assert_eq!(canonical_tool_name("SmartEdit"), "SmartEdit");
        assert_eq!(canonical_tool_name("TodoWrite"), "TodoWrite");
    }

    #[test]
    fn unknown_names_pass_through_unchanged() {
        // 没有别名表：未注册的名字原样返回，调用方拿不到对应工具，
        // 符合"唯一名"语义
        assert_eq!(canonical_tool_name("view_file"), "view_file");
        assert_eq!(canonical_tool_name("create_file"), "create_file");
        assert_eq!(canonical_tool_name("read"), "read");
        assert_eq!(canonical_tool_name("task"), "task");
        // MCP 工具名 mcp__server__tool 不应被改写
        assert_eq!(
            canonical_tool_name("mcp__github__create_issue"),
            "mcp__github__create_issue"
        );
        assert_eq!(canonical_tool_name("some_future_tool"), "some_future_tool");
    }

    #[test]
    fn edit_tool_detection_covers_all_registered_edit_tools() {
        // 注册名
        assert!(is_edit_tool_name("Edit"));
        assert!(is_edit_tool_name("SmartEdit"));
        assert!(is_edit_tool_name("MultiEdit"));
        assert!(is_edit_tool_name("Write"));
        assert!(is_edit_tool_name("NotebookEdit"));
        // 非编辑工具
        assert!(!is_edit_tool_name("Read"));
        assert!(!is_edit_tool_name("Grep"));
    }

    #[test]
    fn read_only_detection_covers_navigation_tools() {
        assert!(is_read_only_tool_name("Read"));
        assert!(is_read_only_tool_name("Grep"));
        assert!(is_read_only_tool_name("CodebaseSearch"));
        assert!(is_read_only_tool_name("ProjectMap"));
        assert!(is_read_only_tool_name("CodeGraph"));
        assert!(is_read_only_tool_name("GetDiagnostics"));
        assert!(is_read_only_tool_name("LS"));
        // 编辑工具不是只读
        assert!(!is_read_only_tool_name("Edit"));
        assert!(!is_read_only_tool_name("Write"));
    }

    /// 工具名统一用 PascalCase（对标 Claude Code）。
    ///
    /// 这条不变量不是为了好看：LLM 靠工具名识别工具，snake_case 与
    /// PascalCase 混排会让模型在 `multi_edit` / `MultiEdit` 之间摇摆，
    /// 而错的那个名字在注册表里根本不存在 —— 调用会直接失败。
    /// `LS` / `LSP` 是例外：缩写本身就是 PascalCase 的写法。
    #[test]
    fn every_registered_name_is_pascal_case() {
        for tool in ToolName::all_builtin() {
            let name = tool.as_str();
            assert!(
                name.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
                "工具名 {name} 必须以大写字母开头"
            );
            assert!(
                !name.contains('_'),
                "工具名 {name} 不该含下划线（应写 {name} 的 PascalCase 形式）"
            );
        }
    }

    /// 旧的下划线写法不再是别名 —— 改名是硬切换。
    ///
    /// 保留旧名兼容看起来友好，实际更糟：用户改了名而旧配置继续生效，
    /// 出问题时排查方向完全相反。所以旧名必须原样落回「未知工具名」。
    #[test]
    fn legacy_snake_case_names_are_not_aliases() {
        for legacy in [
            "smart_edit",
            "multi_edit",
            "notebook_read",
            "read_many_files",
            "get_diagnostics",
            "task_get",
            "tool_search",
            "ask_user_question",
            "enter_plan_mode",
            "mcp_list_servers",
        ] {
            assert!(
                ToolName::from_str(legacy).is_none(),
                "{legacy} 是改名前的旧名，不该再被认作注册名"
            );
        }
    }

    /// 大小写漂移能被纠正 —— 模型偶尔会写成全小写。
    #[test]
    fn lowercase_spellings_resolve_to_the_registered_name() {
        assert_eq!(find_builtin_tool_name("todowrite"), Some("TodoWrite"));
        assert_eq!(find_builtin_tool_name("read"), Some("Read"));
        assert_eq!(find_builtin_tool_name("GREP"), Some("Grep"));
        assert_eq!(find_builtin_tool_name("multiedit"), Some("MultiEdit"));
        // 下划线差异不算大小写漂移
        assert_eq!(find_builtin_tool_name("multi_edit"), None);
        // 未注册的名字原样落回
        assert_eq!(find_builtin_tool_name("mcp__github"), None);
        assert_eq!(find_builtin_tool_name("view_file"), None);
    }
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
