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
/// 多个工具可共享同一份描述（如 Read/read_many_files → readfile）。
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
        // 键必须是**磁盘上的文件名**（`tool-description-<key>.md`），不是工具注册名。
        // 这五个原来写成了 `"Grep" => "Grep"` 这样的注册名，而文件叫
        // `tool-description-grep.md` —— `rust_embed::get()` 和 `dir.join()`
        // 都是大小写敏感的，于是这五个工具的 schema 描述一直静默回退到 Rust
        // 里的默认字符串，正文也进不了系统提示词 bundle（`prompt_builder.rs`
        // 传进来的 key 是小写化后的文件名）。
        m.insert("Grep", "grep");
        m.insert("Glob", "glob");
        m.insert("ListDir", "ls");
        m.insert("CodebaseSearch", "codebase_search");
        m.insert("ProjectMap", "projectmap");
        m.insert("tool_search", "toolsearch");
        m.insert("get_diagnostics", "getdiagnostics");
        // 执行
        m.insert("Bash", "bash");
        m.insert("run_tests", "runtests");
        m.insert("background_task", "backgroundtask");
        m.insert("monitor", "monitor");
        m.insert("wait", "wait");
        // 任务管理
        m.insert("TodoWrite", "managetasks");
        m.insert("task_get", "taskget");
        m.insert("task_list", "tasklist");
        m.insert("task_update", "taskupdate");
        m.insert("task_output", "taskoutput");
        // 代理/技能
        m.insert("Agent", "runagent");
        m.insert("skill", "skill");
        m.insert("brief", "brief");
        m.insert("workflow", "workflow");
        // 记忆
        m.insert("memory", "savememory");
        // Git/GitHub
        m.insert("git_insight", "gitinsight");
        m.insert("git_branch", "gitbranch");
        m.insert("git_rewind", "gitrewind");
        m.insert("git_commit_attribution", "gitcommitattribution");
        m.insert("git_autofix_pr", "gitautofixpr");
        m.insert("git_pr_subscribe", "gitprsubscribe");
        m.insert("gh_pr_comments", "ghprcomments");
        m.insert("suggest_pr", "suggestpr");
        // Web
        m.insert("WebSearch", "websearch");
        m.insert("WebFetch", "webfetch");
        // MCP
        m.insert("mcp_list_resources", "mcplistresources");
        m.insert("mcp_read_resource", "mcpreadresource");
        m.insert("mcp_search_tools", "mcpsearch");
        // 模式切换
        m.insert("enter_plan_mode", "enterplanmode");
        m.insert("exit_plan_mode", "exitplanmode");
        m.insert("enter_worktree", "enterworktree");
        m.insert("exit_worktree", "exitworktree");
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
        m.insert("LSP", "lsp");
        m.insert("mcp_auth", "mcp_auth");
        m.insert("notebook_read", "notebook_read");
        m.insert("remote_trigger", "remote_trigger");
        m.insert("schedule_wakeup", "schedule_wakeup");
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
    tool_description_key(tool_name).map(|key| format!("tool-description-{}.md", key))
}

/// 加载工具的 schema 描述（frontmatter `description` 字段）
///
/// 无对应 `.md` 文件或文件无 frontmatter 描述时返回 `None`，
/// 调用方应回退到 Rust 代码中的默认描述。
pub fn resolve_tool_description(tool_name: &str) -> Option<String> {
    let filename = tool_description_filename(tool_name)?;
    let content = crate::core::prompts::loader::try_load_prompt(&filename)?;
    parse_frontmatter_description(&content).or_else(|| Some(content.trim().to_string()))
}

/// 去掉 frontmatter 注释块的正文。
fn strip_frontmatter(content: &str) -> &str {
    if content.trim_start().starts_with("<!--") {
        content
            .split_once("-->")
            .map(|(_, rest)| rest)
            .unwrap_or(content)
    } else {
        content
    }
}

/// 加载工具的完整使用说明正文（`tool-description-*.md` 去掉 frontmatter）。
///
/// 供 `tool_search` 的 `select:<name>` 模式动态加载：长尾工具不常驻系统提示词，
/// 模型发现工具时随结果拿到详细用法 —— 使用说明走 tool result（消息尾部），
/// 不触碰 prompt 缓存前缀。
pub fn resolve_tool_body(tool_name: &str) -> Option<String> {
    let filename = tool_description_filename(tool_name)?;
    let content = crate::core::prompts::loader::try_load_prompt(&filename)?;
    let body = strip_frontmatter(&content).trim();
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

/// 判断某个 `tool-description-<key>.md` 文件是否与当前激活工具集匹配。
/// 供系统提示词 bundle 过滤使用，与 schema 描述共用同一映射（单一事实源）。
pub fn description_key_matches_active_tools(
    key: &str,
    active_tools: &std::collections::HashSet<String>,
) -> bool {
    tool_description_key_map()
        .iter()
        .any(|(tool_name, tool_key)| *tool_key == key && active_tools.contains(*tool_name))
}

/// 获取已注册的工具描述键数量（供测试/诊断）
pub fn registered_tool_count() -> usize {
    tool_description_key_map().len()
}

/// 所有 `(工具注册名, .md 文件键)` 对 —— 供测试校验每个键都有对应文件。
pub fn registered_tool_keys() -> Vec<(&'static str, &'static str)> {
    tool_description_key_map()
        .iter()
        .map(|(tool, key)| (*tool, *key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_body_strips_frontmatter_and_returns_prose() {
        let body = resolve_tool_body("Edit").expect("Edit 必须有使用说明");
        assert!(!body.contains("<!--"), "正文不得包含 frontmatter");
        assert!(
            body.contains("`Write`") && body.contains("`multi_edit`"),
            "正文应是工具指引原文，而非空壳"
        );
    }

    #[test]
    fn tool_body_todos_use_managetasks_file() {
        let body = resolve_tool_body("TodoWrite").expect("TodoWrite 映射到 managetasks");
        assert!(!body.is_empty());
    }

    #[test]
    fn tool_body_is_none_for_unmapped_tools() {
        assert!(resolve_tool_body("definitely_not_a_tool").is_none());
        assert!(resolve_tool_body("").is_none());
    }
}
