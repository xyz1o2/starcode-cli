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
/// 多个工具可共享同一份描述（如 Read/ReadManyFiles → readfile）。
fn tool_description_key_map() -> &'static std::collections::HashMap<&'static str, &'static str> {
    static MAP: OnceLock<std::collections::HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        // 文件操作
        m.insert("Read", "readfile");
        m.insert("ReadManyFiles", "readfile");
        m.insert("Edit", "edit");
        m.insert("Write", "write");
        m.insert("MultiEdit", "multiedit");
        m.insert("SmartEdit", "smartedit");
        m.insert("NotebookEdit", "notebookedit");
        m.insert("NextEdit", "nextedit");
        // 搜索/导航
        // 键必须是**磁盘上的文件名**（`tool-description-<key>.md`），不是工具注册名。
        // 这五个原来写成了 `"Grep" => "Grep"` 这样的注册名，而文件叫
        // `tool-description-grep.md` —— `rust_embed::get()` 和 `dir.join()`
        // 都是大小写敏感的，于是这五个工具的 schema 描述一直静默回退到 Rust
        // 里的默认字符串，正文也进不了系统提示词 bundle（`prompt_builder.rs`
        // 传进来的 key 是小写化后的文件名）。
        m.insert("Grep", "grep");
        m.insert("Glob", "glob");
        m.insert("LS", "ls");
        m.insert("CodebaseSearch", "codebase_search");
        m.insert("ProjectMap", "projectmap");
        m.insert("CodeGraph", "code_graph");
        m.insert("ToolSearch", "toolsearch");
        m.insert("GetDiagnostics", "getdiagnostics");
        // 执行
        m.insert("Bash", "bash");
        m.insert("RunTests", "runtests");
        m.insert("BackgroundTask", "backgroundtask");
        m.insert("Monitor", "monitor");
        m.insert("Wait", "wait");
        // 任务管理
        m.insert("TodoWrite", "managetasks");
        m.insert("TaskGet", "taskget");
        m.insert("TaskList", "tasklist");
        m.insert("TaskUpdate", "taskupdate");
        m.insert("TaskOutput", "taskoutput");
        // 代理/技能
        m.insert("Agent", "runagent");
        m.insert("Skill", "skill");
        m.insert("Brief", "brief");
        m.insert("WorkflowTool", "workflow");
        // 记忆
        m.insert("Memory", "savememory");
        // Git/GitHub
        m.insert("GitInsight", "gitinsight");
        m.insert("GitRewind", "gitrewind");
        m.insert("GitCommitAttribution", "gitcommitattribution");
        m.insert("GitAutofixPr", "gitautofixpr");
        m.insert("GitPrSubscribe", "gitprsubscribe");
        m.insert("GhPrComments", "ghprcomments");
        m.insert("SuggestPr", "suggestpr");
        // Web
        m.insert("WebSearch", "websearch");
        m.insert("WebFetch", "webfetch");
        // MCP
        m.insert("McpListResources", "mcplistresources");
        m.insert("McpReadResource", "mcpreadresource");
        m.insert("McpSearchTools", "mcpsearch");
        // 模式切换
        m.insert("EnterPlanMode", "enterplanmode");
        m.insert("ExitPlanMode", "exitplanmode");
        m.insert("EnterWorktree", "enterworktree");
        m.insert("ExitWorktree", "exitworktree");
        // 其他
        m.insert("AskUserQuestion", "askuserquestion");
        m.insert("Snip", "snip");
        m.insert("SelfConfig", "selfconfig");
        m.insert("SendMessage", "sendmessage");
        m.insert("CronCreate", "cron");
        m.insert("CronList", "cron");
        m.insert("CronDelete", "cron");
        // 扩展工具集
        m.insert("Wait", "wait");
        m.insert("LSP", "lsp");
        m.insert("McpAuth", "mcp_auth");
        m.insert("NotebookRead", "notebook_read");
        m.insert("RemoteTrigger", "remote_trigger");
        m.insert("ScheduleWakeup", "schedule_wakeup");
        m
    })
}

/// 工具注册名 → 描述文件键
fn tool_description_key(tool_name: &str) -> Option<&'static str> {
    tool_description_key_map().get(tool_name).copied()
}

/// 解析 `.md` frontmatter 中的 `description` 字段（`<!-- key: value -->` 块）
fn parse_frontmatter_description(content: &str) -> Option<String> {
    parse_frontmatter_field(content, "description")
}

/// 解析 frontmatter 里的任意标量字段。
fn parse_frontmatter_field(content: &str, key: &str) -> Option<String> {
    let block = content
        .strip_prefix("<!--")
        .and_then(|rest| rest.split("-->").next())?;
    for line in block.lines() {
        let line = line.trim();
        // 必须精确匹配 `key:`：只 strip_prefix(key) 会让 `description_hint:`
        // 这类同前缀的键被误当成 `description`，而 `?` 会把整段扫描提前返回。
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.strip_prefix(':') else {
            continue;
        };
        let value = value.trim().trim_matches('\'').trim_matches('"').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// 获取工具描述文件的完整文件名（`tool-description-<key>.md`）
pub fn tool_description_filename(tool_name: &str) -> Option<String> {
    tool_description_key(tool_name).map(|key| format!("tool-description-{}.md", key))
}

/// 该工具在 frontmatter 里**主张的职责槽**（`routing:` 字段）。
///
/// 这是"这个活儿归谁"的**声明端**。权威名单在
/// [`crate::core::prompts::routing_matrix::ROUTING_MATRIX`]；这里的取值必须与之一致，
/// 由 `routing_matrix_is_the_only_authority` 等测试强制。
///
/// 没写 `routing:` 返回 `None`——绝大多数长尾工具不参与职责分工，属于正常情况。
pub fn resolve_tool_routing(tool_name: &str) -> Option<String> {
    let filename = tool_description_filename(tool_name)?;
    let content = crate::core::prompts::loader::try_load_prompt(&filename)?;
    parse_frontmatter_field(&content, "routing")
}

/// 列出**磁盘上**所有声明了 `routing:` 的描述文件：
/// `[(文件名, 槽 id, 映射到该文件的工具注册名…)]`。
///
/// 一个描述文件可以服务多个工具（如 `readfile` 同时喂 `Read` 和 `ReadManyFiles`），
/// 所以第三项是列表（已排序——`tool_description_key_map` 是 HashMap，迭代序不稳定）。
/// 拿不到任何注册名的条目由测试判为孤儿描述。
pub fn declared_routing_claims() -> Vec<(String, String, Vec<String>)> {
    let mut files: Vec<String> = super::SystemPrompts::iter()
        .filter(|f| f.starts_with("tool-description-") && f.ends_with(".md"))
        .map(|f| f.to_string())
        .collect();
    files.sort();

    files
        .into_iter()
        .filter_map(|filename| {
            let content = crate::core::prompts::loader::try_load_prompt(&filename)?;
            let routing = parse_frontmatter_field(&content, "routing")?;
            let key = filename
                .strip_prefix("tool-description-")
                .and_then(|rest| rest.strip_suffix(".md"))
                .unwrap_or_default()
                .to_string();
            let mut tools: Vec<String> = tool_description_key_map()
                .iter()
                .filter(|(_, v)| **v == key)
                .map(|(k, _)| (*k).to_string())
                .collect();
            tools.sort();
            Some((filename, routing, tools))
        })
        .collect()
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
/// 供 `ToolSearch` 的 `select:<name>` 模式动态加载：长尾工具不常驻系统提示词，
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
            body.contains("`Write`") && body.contains("`MultiEdit`"),
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
