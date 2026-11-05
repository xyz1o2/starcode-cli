//! 提示词正文与工具注册名的一致性守卫（闭环）。
//!
//! 背景事故：cached bundle 里曾叫模型「把 long-running work 交给 `BackgroundTask`」，
//! 而 `BackgroundTask` 不在核心集（必须先 ToolSearch）、实现又根本不是 shell 后台
//! （只把消息入队 `.star/remote/inbox.jsonl`，由后台轮询器注回 agent）。
//! 提示词里点名的工具必须真实存在、且核心指引只能点名核心工具——
//! 这条以前只能靠人眼发现，这里把它变成测试。
//!
//! 规则：
//! 1. [`embedded_prompts_only_mention_registered_tools_or_whitelisted_words`]：
//!    所有内嵌 `system-prompts/*.md` 里反引号包住的标识符，必须是 `ToolName`
//!    注册名，或在 [`WHITELISTED`] 里；否则测试红，打印 `file:line`。
//! 2. [`core_bundle_only_mentions_declared_tools`]：
//!    进缓存 system message 的文件（静态段 + 核心工具指南 bundle）里，出现的
//!    注册工具名必须在核心集；长尾工具只有该文件同时出现 `ToolSearch` 时才放行
//!    ——也就是明确把发现通道写进指引里。
//! 3. [`routing_matrix_primaries_are_declared_core_tools`]：
//!    路由矩阵的主管工具必须在核心集——矩阵渲染进缓存前缀的表格，
//!    点名一个未声明的工具，等于让模型调用它拿不到的东西。
//! 4. [`every_core_tool_is_registered_at_runtime`]：
//!    核心集里每个工具都真的注册进了 runtime registry（防止改名/下线
//!    之后提示词还在引用它）。
//!
//! 扫描策略：跳过 ``` 代码围栏（示例参数块、长度不受正则约束），单行内的
//! 反引号配对提取；跨行配对整体丢弃。花括号式的 `<...>` 占位符因含非标识符
//! 字符天然被排除。

#![cfg(test)]

use crate::agent::tool_routing::CORE_TOOL_NAMES;
use crate::core::prompts::routing_matrix;
use crate::core::prompts::SystemPrompts;
use crate::core::tools::constants::ToolName;
use std::collections::HashSet;
use std::sync::OnceLock;

/// 与 `ToolName` 不同、但在提示词里合法存在的反引号标识符。
///
/// 收录三类：命令行程序（`gh`/`curl`/`rg`…）、参数名（`max_results`/`pattern`…）、
/// 占位符与协议词（`EOF`/`in_progress`/`mcp__exa__web_search_exa`…）。
/// **新增一个不在此列的词 → 测试红**：要么它是工具名（去枚举里注册），
/// 要么它确实该在这里并附一行说明。刻意不设「第四类」——白名单宽了这张表就白做。
const WHITELISTED: &[&str] = &[
    "EOF",
    "WF_",
    "X",
    "Y",
    "_response",
    "action",
    "activeForm",
    "analyzer",
    "args",
    "awk",
    "base_branch",
    "both",
    "budget_profile",
    "case_sensitive",
    "cat",
    "category",
    "cd",
    "code_reviewer",
    "completed",
    "content",
    "count",
    "curl",
    "delete",
    "depth",
    "dir_path",
    "directory",
    "echo",
    "editor",
    "exa",
    "exclude_pattern",
    "explorer",
    "false",
    "file_path",
    "file_paths",
    "file_types",
    "files",
    "files_with_matches",
    "filter_ext",
    "find",
    "fix_prompt",
    "focus",
    "foo",
    "force_refresh",
    "general_purpose",
    "get",
    "gh",
    "grep",
    "head",
    "in_progress",
    "include_attribution",
    "include_hidden",
    "include_pattern",
    "include_symbols",
    "inspect",
    "interval_minutes",
    "items",
    "keep_changes",
    "key",
    "kill",
    "language",
    "limit",
    "list",
    "max_depth",
    "max_entries",
    "max_files",
    "max_length",
    "max_results",
    "max_size_per_file",
    "max_symbols",
    "mcp__exa__web_search_exa",
    "message",
    "name",
    "nc",
    "notes",
    "num",
    "offset",
    "old_string",
    "output_mode",
    "params",
    "parse_config",
    "path",
    "pattern",
    "pending",
    "persist",
    "pid",
    "pop",
    "pr_number",
    "prompt",
    "query",
    "reason",
    "recursive",
    "repo",
    "reset",
    "reset_to",
    "resolve_engine",
    "respect_git_ignore",
    "respect_star_ignore",
    "retry_backoff",
    "rg",
    "save",
    "search_type",
    "sed",
    "send_message",
    "server",
    "set",
    "shutdown_request",
    "skill",
    "skip_binary",
    "sleep",
    "stash",
    "status",
    "subagent_type",
    "summary",
    "symbol",
    "tail",
    "target",
    "task_description",
    "task_id",
    "task_prompt",
    "test_command",
    "text",
    "to",
    "true",
    "truncate_lines",
    "undo_last_commit",
    "uri",
    "url",
    "value",
    "web_search_exa",
    "wget",
    "whole_word",
    "workflow",
];

fn is_identifier(token: &str) -> bool {
    let mut chars = token.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 提取反引号标识符：跳过 ``` 代码围栏，单行内配对，跨行整体丢弃。
/// 返回 `(行号, 标识符)`。
fn backticked_identifiers(content: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut line_no = 0usize;

    for raw in content.lines() {
        line_no += 1;
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut rest = raw;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            match after.find('`') {
                Some(end) => {
                    let token = after[..end].trim();
                    if is_identifier(token) {
                        out.push((line_no, token.to_string()));
                    }
                    rest = &after[end + 1..];
                }
                None => break,
            }
        }
    }
    out
}

fn embedded_markdown_files() -> Vec<String> {
    let mut files: Vec<String> = SystemPrompts::iter()
        .map(|f| f.to_string())
        .filter(|f| f.ends_with(".md"))
        .collect();
    files.sort();
    files
}

/// bundle 的**输入**（静态段 + 核心工具指南），文件清单以
/// `prompt_builder::BUNDLE_STATIC_FILE_NAMES` 为唯一事实源。
fn bundle_input_files() -> Vec<String> {
    let mut files: Vec<String> = crate::agent::prompt_builder::BUNDLE_STATIC_FILE_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
    files.extend(crate::agent::prompt_builder::PromptBuilder::core_tool_description_filenames());
    files.sort();
    files.dedup();
    files
}

fn core_set() -> HashSet<&'static str> {
    CORE_TOOL_NAMES.iter().copied().collect()
}

/// 每个文件都必须真正被扫到——否则测试会像被架空的守卫一样空转。
fn scanned_file_count() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| embedded_markdown_files().len())
}

#[test]
fn embedded_prompts_only_mention_registered_tools_or_whitelisted_words() {
    assert!(
        scanned_file_count() >= 60,
        "嵌入式提示词文件数量异常（{}），扫描逻辑坏了",
        scanned_file_count()
    );

    let mut violations = Vec::new();
    let mut scanned_tokens = 0usize;
    for name in embedded_markdown_files() {
        let Some(content) = crate::core::prompts::loader::try_load_prompt(&name) else {
            violations.push(format!("{name}: 无法加载（内嵌清单里有却读不出内容）"));
            continue;
        };
        for (line, token) in backticked_identifiers(&content) {
            scanned_tokens += 1;
            if ToolName::from_str(&token).is_some() {
                continue;
            }
            if WHITELISTED.contains(&token.as_str()) {
                continue;
            }
            violations.push(format!("{name}:{line}: `{token}`"));
        }
    }
    assert!(
        scanned_tokens >= 100,
        "只扫到 {scanned_tokens} 个标识符，提取逻辑坏了（会空转通过）"
    );
    assert!(
        violations.is_empty(),
        "提示词里出现了既不是注册工具名、也不在白名单里的反引号标识符。\n\
         要么它是工具名（先注册），要么把它加进 reference_guards::WHITELISTED 并注明用途：\n{}",
        violations.join("\n")
    );
}

#[test]
fn core_bundle_only_mentions_declared_tools() {
    let core = core_set();
    let mut violations = Vec::new();
    let mut checked = 0usize;

    for name in bundle_input_files() {
        let Some(content) = crate::core::prompts::loader::try_load_prompt(&name) else {
            violations.push(format!("{name}: bundle 输入文件无法加载"));
            continue;
        };
        checked += 1;
        // 文件若同时写了 ToolSearch，说明它明确把长尾工具交给发现通道，放行。
        let has_discovery = content.contains("ToolSearch");
        for (line, token) in backticked_identifiers(&content) {
            if ToolName::from_str(&token).is_none() {
                continue;
            }
            if core.contains(token.as_str()) || has_discovery {
                continue;
            }
            violations.push(format!(
                "{name}:{line}: `{token}`（不在核心集，且未写发现通道）"
            ));
        }
    }

    assert!(
        checked >= 10,
        "bundle 输入文件数量异常（{checked}），选择逻辑坏了"
    );
    assert!(
        violations.is_empty(),
        "cached system message 里出现了核心集中不到、又没写发现通道的工具——\n\
         模型拿着指引却拿不到这个工具，每次都要先 ToolSearch 一圈（或直接调不起）。\n\
         要么把它放进核心集，要么在该文件里写明用 ToolSearch 发现：\n{}",
        violations.join("\n")
    );
}

#[test]
fn routing_matrix_primaries_are_declared_core_tools() {
    let core = core_set();
    let missing: Vec<&str> = routing_matrix::ROUTING_MATRIX
        .iter()
        .map(|s| s.primary)
        .filter(|p| !core.contains(p))
        .collect();
    assert!(
        missing.is_empty(),
        "路由矩阵的主管工具不在核心集——矩阵渲染进缓存前缀的表格，\
         等于让模型调用它没有的工具：{missing:?}"
    );
}

#[tokio::test]
async fn every_core_tool_is_registered_at_runtime() {
    let registry =
        crate::core::config::runtime_bootstrap::build_two_layer_registry_for_tests().await;
    let missing: Vec<&str> = CORE_TOOL_NAMES
        .iter()
        .copied()
        .filter(|name| registry.get_tool(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "核心集工具没有注册进 runtime registry——提示词还在引用它们：{missing:?}"
    );
}
