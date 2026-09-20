use super::*;
use std::collections::HashSet;

#[test]
fn test_agent_mode_render() {
    let output = agent_mode::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("AGENT MODE"));
    assert!(!output.contains("<thinking>"));

    let output_thinking = agent_mode::render(true);
    assert!(output_thinking.contains("AGENT MODE"));
}

#[test]
fn test_core_identity_render() {
    let output = core_identity::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("same language as the user's input"));
    assert!(output.contains("Concise"));

    let output_thinking = core_identity::render(true);
    assert!(output_thinking.contains("Reasoning"));
}

#[test]
fn test_env_info_render() {
    let info = env_info::EnvInfo {
        today: "2023-01-01",
        platform: "linux",
        cwd: "/tmp",
        shell: "bash",
        is_git_repo: true,
        git_branch: Some("main"),
        git_status: Some("(clean)"),
        recent_commits: Some("abc1234 feat: test"),
    };
    let output = env_info::render(info);
    assert!(output.contains("linux"));
    assert!(output.contains("/tmp"));
    assert!(output.contains("bash"));
    assert!(output.contains("main"));
    assert!(output.contains("(clean)"));
}

#[test]
fn test_key_scenarios_render() {
    let output = key_scenarios::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("Key Scenarios"));
}

#[test]
fn test_main_system_render() {
    let output = main_system::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("STAR CLI - SYSTEM PROMPT"));

    let output_thinking = main_system::render(true);
    assert!(output_thinking.contains("STAR CLI"));
}

#[test]
fn test_reminders_render() {
    let output = reminders::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("Reminders"));

    let output_thinking = reminders::render(true);
    assert!(output_thinking.contains("Reminders"));
}

#[test]
fn test_security_policy_render() {
    let output = security_policy::render();
    assert!(!output.is_empty());
    assert!(output.contains("SECURITY POLICY & SAFEGUARDS"));
    assert!(output.contains("PROHIBIT"));
}

#[test]
fn test_task_agent_usage_render() {
    let output = task_agent_usage::render();
    assert!(!output.is_empty());
    assert!(output.contains("Task Delegation Protocol"));
}

#[test]
fn test_tool_catalog_render() {
    let output = tool_catalog::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("## Tools"));
    assert!(output.contains("Read"));
    assert!(output.contains("Task Delegation Protocol"));
}

#[test]
fn test_tool_catalog_always_includes_delegation_guidance() {
    // task-agent usage 段是静态文本，必须无条件纳入：它位于 system prompt 的
    // static_parts（整个 system 数组是 Anthropic 缓存前缀），若按每轮
    // active_tools 短名单条件渲染，短名单一变就击穿前缀。
    let output = tool_catalog::render(false);

    assert!(output.contains("## Tools"));
    assert!(output.contains("Task Delegation Protocol"));
}

#[test]
fn test_tool_list_render() {
    let output = tool_list::render(false);
    assert!(!output.is_empty());
    assert!(output.contains("Read"));
    assert!(output.contains("Write"));
}

#[test]
fn test_loader_embedded_fallback() {
    // 内嵌兜底：无外部目录时也能加载
    let prompt = loader::load_prompt("system-prompt.md");
    assert!(prompt.contains("STAR CLI"));
}

#[test]
fn test_loader_external_override() {
    // 外部目录覆盖：注入目录优先于内嵌（不依赖环境变量，避免并行测试污染）
    let dir = std::env::temp_dir().join(format!("starcode-prompt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = "EXTERNAL OVERRIDE MARKER 42";
    std::fs::write(dir.join("system-prompt.md"), marker).unwrap();

    let loaded = loader::load_prompt_with_dirs("system-prompt.md", &[dir.clone()]);
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(loaded, marker);
}

#[test]
fn test_tool_description_resolution() {
    // 核心工具：schema 描述来自 frontmatter
    let desc = tool_descriptions::resolve_tool_description("Read");
    assert!(desc.is_some(), "Read 应有 .md 描述");
    let desc = desc.unwrap();
    assert!(
        desc.contains("Read file"),
        "描述应为 frontmatter 精简句: {desc}"
    );
    assert!(!desc.contains("<!--"), "描述不应含 frontmatter 注释");

    let edit_desc = tool_descriptions::resolve_tool_description("Edit");
    assert!(edit_desc.is_some());
    assert!(edit_desc.unwrap().contains("Exact string replacement"));

    // 新增工具
    let wait_desc = tool_descriptions::resolve_tool_description("wait");
    assert!(wait_desc.is_some(), "wait 应有 .md 描述");

    // 映射表与注册工具名必须一致：
    // 1. 表里不能有没注册过的"幽灵名"（对标 Claude Code：内置工具名即唯一名）
    // 2. 注册的内置工具不能漏了描述（MCP 元工具与 repl 由各自模块提供，不在此列）
    use crate::core::tools::constants::ToolName;
    let mapped: HashSet<&str> = tool_descriptions::registered_tool_keys()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let registered: HashSet<&str> = ToolName::all_builtin()
        .into_iter()
        .map(|t| t.as_str())
        .collect();
    let phantoms: Vec<&str> = mapped.difference(&registered).copied().collect();
    assert!(
        phantoms.is_empty(),
        "description map holds unregistered tool names (phantoms): {phantoms:?}"
    );
    let no_desc: Vec<&str> = registered
        .difference(&mapped)
        .copied()
        .filter(|name| !name.starts_with("mcp_"))
        .collect();
    assert!(
        no_desc.is_empty(),
        "registered tools without a description key: {no_desc:?}"
    );
}

#[test]
fn test_description_key_matches_active_tools() {
    let active = HashSet::from(["Read".to_string(), "Edit".to_string()]);
    assert!(tool_descriptions::description_key_matches_active_tools(
        "readfile", &active
    ));
    assert!(tool_descriptions::description_key_matches_active_tools(
        "edit", &active
    ));
    assert!(!tool_descriptions::description_key_matches_active_tools(
        "Bash", &active
    ));
}

/// 每个映射键都必须对应磁盘上真实存在的 `.md`。
///
/// `Grep`/`Glob`/`ListDir`/`CodebaseSearch`/`ProjectMap` 五个键原来写的是工具
/// 注册名（`"Grep" => "Grep"`），而文件叫 `tool-description-grep.md` —— 大小写
/// 不匹配，于是 schema 描述静默回退到 Rust 默认串，正文也进不了 bundle。
#[test]
fn every_mapped_description_key_has_a_file() {
    for (tool, key) in tool_descriptions::registered_tool_keys() {
        let filename = format!("tool-description-{}.md", key);
        assert!(
            loader::try_load_prompt(&filename).is_some(),
            "tool {tool:?} maps to key {key:?} but {filename} does not exist"
        );
        assert!(
            tool_descriptions::resolve_tool_description(tool).is_some(),
            "tool {tool:?} should resolve a schema description from {filename}"
        );
    }
}

/// bundle 过滤器传进来的是**小写化后的文件名**（`prompt_builder.rs:251`），
/// 所以映射值必须是小写，否则正文永远进不了系统提示词。
#[test]
fn search_tool_bodies_reach_the_prompt_bundle() {
    let active = HashSet::from([
        "Grep".to_string(),
        "Glob".to_string(),
        "ListDir".to_string(),
        "CodebaseSearch".to_string(),
        "ProjectMap".to_string(),
    ]);
    for key in ["grep", "glob", "ls", "codebase_search", "projectmap"] {
        assert!(
            tool_descriptions::description_key_matches_active_tools(key, &active),
            "{key} body should be included when its tool is active"
        );
    }
}

#[test]
fn test_scope_strategy_loaded_from_file() {
    let template = loader::load_prompt("scope-strategy.md");
    assert!(template.contains("Tight scope"));
    assert!(template.contains("Broad scope"));
}

#[test]
fn test_complexity_strategies_loaded_from_file() {
    let template = loader::load_prompt("complexity-strategies.md");
    assert!(template.contains("## COMPLEX"));
    assert!(template.contains("## MEDIUM"));
    assert!(template.contains("## SIMPLE"));
}

#[test]
fn test_token_budget_warning_template() {
    let template = loader::load_prompt("token-budget-warning.md");
    assert!(template.contains("{pct}"));
    let rendered = loader::render_template(
        &template,
        &[("pct", "90"), ("est", "72000"), ("max", "80000")],
    );
    assert!(rendered.contains("90%"));
    assert!(rendered.contains("72000"));
    assert!(!rendered.contains("{est}"));
}

/// 工具描述会被 compact_tool_definition 截断到 160 字符（默认）。
/// frontmatter 里写得再好，超了就是 "..." 结尾，等于没写。
/// 这条测试钉住 ACE 三个工具的描述长度。
#[test]
fn ace_tool_descriptions_fit_compact_limit() {
    for (tool, key) in [
        ("CodebaseSearch", "codebase_search"),
        ("ProjectMap", "projectmap"),
        ("Grep", "grep"),
    ] {
        let desc = tool_descriptions::resolve_tool_description(tool)
            .unwrap_or_else(|| panic!("no description for {tool}"));
        assert!(
            desc.chars().count() <= 160,
            "{tool} description is {} chars, exceeds the 160-char compact limit and gets truncated to \"...\": {desc:?}",
            desc.chars().count()
        );
        assert!(
            !desc.ends_with("..."),
            "{tool} description must not be pre-truncated: {desc:?}"
        );
        let _ = key;
    }
}
