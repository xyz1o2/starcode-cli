//! 工具描述加载器
//!
//! 工具 schema 的 `description` 统一从 `tool-description-*.md` 文件加载，
//! 作为工具描述的单一事实源（与系统提示词 bundle 同源，保证提示词与工具配合一致）。
//!
//! 文件格式约定（与现有 tool-description-*.md 一致）：
//!
//! ```md
//! <!--
//! name: 'Tool Description: Edit'
//! description: Exact string replacement in files
//! -->
//! (正文：详细的使用指引，注入系统提示词 bundle)
//! ```
//!
//! - LLM 收到的工具 schema 描述 = frontmatter 中的 `description` 字段（精简句）
//! - 系统提示词 bundle = 文件全文（详细指引）
//!
//! 加载顺序：外部目录（loader）→ 编译期内嵌。无对应 `.md` 时返回 `None`，
//! 由调用方回退到 Rust 代码中的默认描述。

use std::sync::OnceLock;

/// 工具注册名 → `tool-description-*.md` 文件名键 的映射。
/// 多个工具可共享同一份描述（如 Read/view_file/read_many_files → readfile）。
fn tool_description_key_map() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static MAP: OnceLock<std::collections::HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        // 文件操作
        m.insert("Read", "readfile");
        m.insert("read_many_files", "readfile");
        m.insert("Edit", "edit");
        m.insert("Write", "write");
        m.insert("multi_edit", "multiedit");
        m.insert("smart_edit", "smartedit");
        m.insert("notebook_edit", "notebookedit");
        m.insert("next_edit", "nextedit");
        // 搜索/导航
        m.insert("Grep", "Grep");
        m.insert("Glob", "Glob");
        m.insert("ListDir", "ListDir");
        m.insert("SemanticSearch", "SemanticSearch");
        m.insert("ProjectMap", "ProjectMap");
        m.insert("tool_search", "toolsearch");
        m.insert("get_diagnostics", "getdiagnostics");
        // 执行
        m.insert("Bash", "bash");
        m.insert("powershell", "powershell");
        m.insert("run_tests", "runtests");
        m.insert("background_task", "backgroundtask");
        m.insert("monitor", "monitor");
        m.insert("wait", "wait");
        // 任务管理
        m.insert("Todo", "managetasks");
        m.insert("task_get", "taskget");
        m.insert("task_list", "tasklist");
        m.insert("task_update", "taskupdate");
        m.insert("task_output", "taskoutput");
        // 代理/技能
        m.insert("Agent", "runagent");
        m.insert("skill", "skill");
        m.insert("discover_skills", "skill");
        m.insert("brief", "brief");
        m.insert("workflow", "workflow");
        // 记忆
        m.insert("memory", "savememory");
        m.insert("local_memory_recall", "savememory");
        // Git/GitHub
        m.insert("git_insight", "gitinsight");
        m.insert("git_branch", "gitbranch");
        m.insert("git_rewind", "gitrewind");
        m.insert("git_commit_attribution", "gitcommitattribution");
        m.insert("git_autofix_pr", "gitautofixpr");
        m.insert("git_pr_subscribe", "gitprsubscribe");
        m.insert("gh_pr_comments", "ghprcomments");
        m.insert("github_app", "githubapp");
        m.insert("github_issue", "githubissue");
        m.insert("suggest_pr", "suggestpr");
        m.insert("subscribe_pr", "suggestpr");
        m.insert("suggest_background_pr", "suggestpr");
        // Web
        m.insert("WebSearch", "websearch");
        m.insert("WebFetch", "webfetch");
        m.insert("web_browser", "webfetch");
        // MCP
        m.insert("mcp_list_resources", "mcplistresources");
        m.insert("mcp_read_resource", "mcpreadresource");
        m.insert("mcp_search_tools", "mcpsearch");
        // 模式切换
        m.insert("enter_plan_mode", "enterplanmode");
        m.insert("exit_plan_mode", "exitplanmode");
        m.insert("enter_worktree", "enterworktree");
        m.insert("exit_worktree", "exitworktree");
        // 计划/验证
        m.insert("verify_plan_execution", "enterplanmode");
        // 其他
        m.insert("ask_user_question", "askuserquestion");
        m.insert("synthetic_output", "syntheticoutput");
        m.insert("snip", "snip");
        m.insert("send_message", "sendmessage");
        m.insert("cron_create", "cron");
        m.insert("cron_list", "cron");
        m.insert("cron_delete", "cron");
        // 扩展工具集
        m.insert("wait", "wait");
        m.insert("config", "config");
        m.insert("ctx_inspect", "ctx_inspect");
        m.insert("search_extra_tools", "extra_tools");
        m.insert("execute_extra_tool", "extra_tools");
        m.insert("goal", "goal");
        m.insert("team_create", "team");
        m.insert("team_delete", "team");
        m.insert("list_peers", "team");
        m.insert("LSP", "lsp");
        m.insert("mcp_auth", "mcp_auth");
        m.insert("notebook_read", "notebook_read");
        m.insert("push_notification", "cross_agent");
        m.insert("send_user_file", "cross_agent");
        m.insert("remote_trigger", "remote_trigger");
        m.insert("review_artifact", "review_artifact");
        m.insert("schedule_wakeup", "schedule_wakeup");
        m.insert("terminal_capture", "terminal_capture");
        m.insert("vault_http_fetch", "vault_http_fetch");
        m
    })
}

/// 工具注册名 → 描述文件键
fn tool_description_key(tool_name: &str) -> Option<&'static str> {
    tool_description_key_map().get(tool_name).copied()
}

/// 解析 `.md` frontmatter 中的 `description` 字段（`<!-- key: value -->` 块）
fn parse_frontmatter_description(content: &str) -> Option<String> {
    let block = content
        .strip_prefix("<!--")
        .and_then(|rest| rest.split("-->").next())?;
    for line in block.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim().trim_matches('\'').trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 获取工具描述文件的完整文件名（`tool-description-<key>.md`）
pub fn tool_description_filename(tool_name: &str) -> Option<String> {
    tool_description_key(tool_name)
        .map(|key| format!("tool-description-{}.md", key))
}

/// 加载工具的 schema 描述（frontmatter `description` 字段）
///
/// 无对应 `.md` 文件或文件无 frontmatter 描述时返回 `None`，
/// 调用方应回退到 Rust 代码中的默认描述。
pub fn resolve_tool_description(tool_name: &str) -> Option<String> {
    let filename = tool_description_filename(tool_name)?;
    let content = crate::core::prompts::loader::try_load_prompt(&filename)?;
    parse_frontmatter_description(&content)
        .or_else(|| Some(content.trim().to_string()))
}

/// 判断某个 `tool-description-<key>.md` 文件是否与当前激活工具集匹配。
/// 供系统提示词 bundle 过滤使用，与 schema 描述共用同一映射（单一事实源）。
pub fn description_key_matches_active_tools(
    key: &str,
    active_tools: &std::collections::HashSet<String>,
) -> bool {
    tool_description_key_map()
        .iter()
        .any(|(tool_name, tool_key)| {
            *tool_key == key && active_tools.contains(*tool_name)
        })
}

/// 获取已注册的工具描述键数量（供测试/诊断）
pub fn registered_tool_count() -> usize {
    tool_description_key_map().len()
}
