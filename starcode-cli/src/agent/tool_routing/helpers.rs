use crate::types::{StarTool, StarToolCall};
use std::collections::HashSet;

/// 检查是否是编辑工具
pub(crate) fn is_edit_tool_name(name: &str) -> bool {
    matches!(
        name,
        "Edit" | "multi_edit" | "Write" | "create_file"
    )
}

/// 检查是否是只读工具
pub(crate) fn is_read_only_tool_name(name: &str) -> bool {
    matches!(
        name,
        "Read" | "Grep" | "Glob" | "SemanticSearch" | "ProjectMap" | "get_diagnostics" | "ListDir" | "rg"
    )
}

/// 检查是否是验证工具
pub(crate) fn is_validation_tool_name(name: &str) -> bool {
    matches!(name, "get_diagnostics" | "run_tests")
}

/// 检查是否是记忆工具
pub(crate) fn is_memory_tool_name(name: &str) -> bool {
    matches!(name, "memory" | "remember" | "recall")
}

/// 获取只读轮次限制
pub(crate) fn resolved_read_only_turn_limit() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("STAR_READ_ONLY_TURN_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(5)
            .clamp(2, 20)
    })
}

/// 按字符数截断字符串（安全处理 UTF-8 多字节字符）
pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        let truncated: String = input.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

/// 检查是否有操作意图
pub(crate) fn has_action_intent(text: &str) -> bool {
    let action_keywords = [
        "create", "write", "edit", "Edit", "delete", "remove",
        "update", "change", "fix", "add", "insert", "modify",
        "implement", "refactor", "optimize", "improve",
    ];
    
    let text_lower = text.to_lowercase();
    action_keywords.iter().any(|keyword| text_lower.contains(keyword))
}

/// 获取请求复杂度标签
pub(crate) fn request_complexity_label(complexity: crate::core::routing::RequestComplexity) -> &'static str {
    match complexity {
        crate::core::routing::RequestComplexity::Simple => "simple",
        crate::core::routing::RequestComplexity::Medium => "medium",
        crate::core::routing::RequestComplexity::Complex => "complex",
    }
}

/// 构建工具循环签名
pub(crate) fn build_tool_loop_signature(tool_calls: &[StarToolCall]) -> String {
    let mut parts = tool_calls
        .iter()
        .map(|tc| {
            let compact_args = normalize_tool_arguments_signature(&tc.function.arguments);
            let compact_args = truncate_chars(&compact_args, 120);
            format!("{}({})", tc.function.name, compact_args)
        })
        .collect::<Vec<_>>();
    parts.sort_unstable();
    parts.join(" | ")
}

/// 规范化工具参数签名
fn normalize_tool_arguments_signature(arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return arguments.split_whitespace().collect::<Vec<_>>().join(" ");
    };

    canonical_json_signature(&value).to_string()
}

/// 规范化 JSON 签名
fn canonical_json_signature(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(obj) => {
            let mut sorted = std::collections::BTreeMap::new();
            for (key, value) in obj {
                let canonical = if is_path_like_tool_arg(key) {
                    value
                        .as_str()
                        .map(|path| {
                            serde_json::Value::String(
                                crate::core::utils::paths::normalize_cross_platform_path(path)
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            )
                        })
                        .unwrap_or_else(|| canonical_json_signature(value))
                } else {
                    canonical_json_signature(value)
                };
                sorted.insert(key.clone(), canonical);
            }
            serde_json::to_value(sorted).unwrap_or(serde_json::Value::Null)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(items.iter().map(canonical_json_signature).collect()),
        _ => value.clone(),
    }
}

/// 检查是否是路径类工具参数
fn is_path_like_tool_arg(key: &str) -> bool {
    matches!(
        key,
        "path" | "file_path" | "dir_path" | "directory" | "target_file" | "cwd"
    )
}

/// 检查是否应该跳过验证
pub(crate) fn should_skip_verification(user_input: &str) -> bool {
    let skip_keywords = ["quick", "fast", "simple", "minor", "small"];
    let input_lower = user_input.to_lowercase();
    skip_keywords.iter().any(|keyword| input_lower.contains(keyword))
}

/// 构建工具选择系统消息
/// Note: This is now only used for debug logging, not injected into messages
pub(crate) fn build_tool_selection_system_message(
    tool_selection: &super::ToolSelection,
    current_turn: i32,
) -> String {
    // Return empty string - the marker is no longer injected into messages
    // Tool selection info is logged separately via debug logging
    let _ = (tool_selection, current_turn);
    String::new()
}

/// 选择工具短名单
pub(crate) fn select_tools_for_turn(
    all_tools: &[StarTool],
    user_input: &str,
    current_turn: i32,
) -> super::ToolSelection {
    select_tools_for_turn_with_limit(all_tools, user_input, current_turn, None)
}

/// 为特定客户端选择工具短名单
pub(crate) fn select_tools_for_turn_for_client(
    client: &crate::llm::client::StarClient,
    all_tools: &[StarTool],
    user_input: &str,
    current_turn: i32,
) -> super::ToolSelection {
    let limit = if client.is_kimi_code_provider() {
        Some(resolved_kimi_code_tool_shortlist_limit(current_turn))
    } else {
        None
    };
    select_tools_for_turn_with_limit(all_tools, user_input, current_turn, limit)
}

/// 核心工具集：无论评分如何，始终入选工具短名单。
/// 防止首轮因关键词评分遗漏关键工具，导致 agent "不知道能做什么"。
/// 核心工具集：无论评分如何，始终入选工具短名单。
/// 防止首轮因关键词评分遗漏关键工具，导致 agent "不知道能做什么"。
/// 注意：这里必须使用**真实注册名**（LLM 看到的是 schema 里的注册名）。
pub(crate) const CORE_TOOL_NAMES: &[&str] = &[
    "Read",
    "read_many_files",
    "Grep",
    "Grep",
    "Glob",
    "ListDir",
    "Edit",
    "multi_edit",
    "Write",
    "Bash",
    "SemanticSearch",
    "ProjectMap",
    "Todo",
    "get_diagnostics",
    "run_tests",
    // Discovery of long-tail tools (git/web/team/cron...) — always visible
    // so the agent can find any tool it doesn't know about by name.
    "tool_search",
];

/// 带限制的工具选择
fn select_tools_for_turn_with_limit(
    all_tools: &[StarTool],
    user_input: &str,
    current_turn: i32,
    limit: Option<usize>,
) -> super::ToolSelection {
    let k = limit.unwrap_or_else(|| resolved_tool_shortlist_limit(all_tools.len(), current_turn));

    // 如果工具数量小于等于限制，返回所有工具
    if all_tools.len() <= k {
        return super::ToolSelection {
            tools: all_tools.to_vec(),
            selected_names: all_tools.iter().map(|t| t.function.name.clone()).collect(),
            rationale: "all_tools_fit".to_string(),
            total_tools: all_tools.len(),
        };
    }

    // 核心工具始终入选（不依赖关键词评分），其余按评分补足到 k。
    let mut selected: Vec<StarTool> = Vec::new();
    let mut selected_names: HashSet<String> = HashSet::new();

    for tool in all_tools {
        if CORE_TOOL_NAMES.contains(&tool.function.name.as_str()) {
            if selected_names.insert(tool.function.name.clone()) {
                selected.push(tool.clone());
            }
        }
    }

    // 评分并选择剩余工具（补足到 k）
    let mut scored_tools: Vec<(f64, &StarTool)> = all_tools
        .iter()
        .filter(|tool| !selected_names.contains(&tool.function.name))
        .map(|tool| (score_tool_for_turn(tool, user_input, current_turn), tool))
        .collect();

    scored_tools.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    for (_, tool) in scored_tools.iter() {
        if selected.len() >= k {
            break;
        }
        if selected_names.insert(tool.function.name.clone()) {
            selected.push((*tool).clone());
        }
    }

    // MCP 动态工具保障：用户显式配置的 MCP 工具绕过短名单限制，始终对 LLM 可见。
    // 未入选的 MCP 动态工具按分数排序后追加，上限 8 个。
    {
        let mut unselected_mcp: Vec<(f64, &StarTool)> = scored_tools
            .iter()
            .filter(|(_, t)| is_mcp_dynamic_tool(&t.function.name) && !selected_names.contains(&t.function.name))
            .map(|(s, t)| (*s, *t))
            .collect();
        unselected_mcp.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        const MCP_DYNAMIC_TOOL_QUOTA: usize = 8;
        for (_, tool) in unselected_mcp.iter().take(MCP_DYNAMIC_TOOL_QUOTA) {
            if !selected_names.contains(&tool.function.name) {
                selected_names.insert(tool.function.name.clone());
                selected.push((*tool).clone());
            }
        }
    }

    super::ToolSelection {
        tools: selected.clone(),
        selected_names,
        rationale: format!("shortlist_k={}", k),
        total_tools: all_tools.len(),
    }
}

/// 解析工具短名单限制
fn resolved_tool_shortlist_limit(total_tools: usize, current_turn: i32) -> usize {
    if current_turn <= 1 {
        resolved_first_turn_tool_shortlist_limit()
    } else {
        resolved_general_tool_shortlist_limit()
    }
}

/// 解析通用工具短名单限制
fn resolved_general_tool_shortlist_limit() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("STAR_TOOL_SHORTLIST_K")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(24)
            .clamp(6, 64)
    })
}

/// 解析第一轮工具短名单限制
fn resolved_first_turn_tool_shortlist_limit() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("STAR_FIRST_TURN_TOOL_SHORTLIST_K")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(14)
            .clamp(6, 64)
    })
}

/// 解析 Kimi Code 工具短名单限制
fn resolved_kimi_code_tool_shortlist_limit(current_turn: i32) -> usize {
    if current_turn <= 1 {
        static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *LIMIT.get_or_init(|| {
            std::env::var("STAR_KIMI_CODE_FIRST_TURN_TOOL_SHORTLIST_K")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(4)
                .clamp(2, 20)
        })
    } else {
        static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *LIMIT.get_or_init(|| {
            std::env::var("STAR_KIMI_CODE_TOOL_SHORTLIST_K")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(10)
                .clamp(2, 20)
        })
    }
}

/// 检查是否为 MCP 动态工具（格式：mcp__<server>__<toolname>）
fn is_mcp_dynamic_tool(tool_name: &str) -> bool {
    tool_name.starts_with("mcp__") && tool_name.matches("__").count() >= 2
}

/// 为工具评分
fn score_tool_for_turn(tool: &StarTool, user_input: &str, current_turn: i32) -> f64 {
    let mut score = 0.0;
    let tool_name = &tool.function.name;
    let input_lower = user_input.to_lowercase();

    // 基础分
    score += 1.0;

    // 关键词匹配加分
    if input_lower.contains(&tool_name.to_lowercase()) {
        score += 10.0;
    }

    // 描述关键词匹配加分（对所有工具生效，MCP 工具依赖此项竞争）
    let desc_lower = tool.function.description.to_lowercase();
    for word in input_lower.split_whitespace() {
        if word.len() >= 3 && desc_lower.contains(word) {
            score += 2.5;
            break;
        }
    }

    // MCP 动态工具基础加成 — 用户显式配置的工具应该更容易被选中
    if is_mcp_dynamic_tool(tool_name) {
        score += 2.0;
    }

    // 特定工具加分
    match tool_name.as_str() {
        "Read" | "view_file" => {
            if input_lower.contains("read") || input_lower.contains("view") || input_lower.contains("show") {
                score += 5.0;
            }
        }
        "Grep" | "Glob" => {
            if input_lower.contains("Grep") || input_lower.contains("find") || input_lower.contains("Grep") {
                score += 5.0;
            }
        }
        "Edit" | "multi_edit" => {
            if input_lower.contains("edit") || input_lower.contains("change") || input_lower.contains("update") {
                score += 5.0;
            }
        }
        "Bash" => {
            if input_lower.contains("run") || input_lower.contains("execute") || input_lower.contains("command") {
                score += 5.0;
            }
        }
        _ => {}
    }

    // 第一轮降低某些工具的分数
    if current_turn <= 1 {
        if tool_name == "Bash" || tool_name == "run_tests" {
            score -= 2.0;
        }
    }

    score
}

/// 工具提示
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolHints {
    pub(crate) explicit_skill: bool,
    pub(crate) explicit_plan: bool,
    pub(crate) explicit_git: bool,
    pub(crate) explicit_web: bool,
    pub(crate) explicit_diag: bool,
    pub(crate) explicit_task: bool,
    pub(crate) explicit_new_file: bool,
    pub(crate) explicit_code_nav: bool,
    pub(crate) explicit_memory: bool,
}

impl ToolHints {
    pub(crate) fn active_tags(&self) -> Vec<&'static str> {
        let mut tags = Vec::new();
        if self.explicit_skill { tags.push("skill"); }
        if self.explicit_plan { tags.push("plan"); }
        if self.explicit_git { tags.push("git"); }
        if self.explicit_web { tags.push("web"); }
        if self.explicit_diag { tags.push("diag"); }
        if self.explicit_task { tags.push("task"); }
        if self.explicit_new_file { tags.push("new_file"); }
        if self.explicit_code_nav { tags.push("code_nav"); }
        if self.explicit_memory { tags.push("memory"); }
        tags
    }
}

/// 推断工具提示
pub(crate) fn infer_tool_hints(user_input: &str) -> ToolHints {
    let input_lower = user_input.to_lowercase();
    let mut hints = ToolHints::default();
    
    // 技能提示
    if input_lower.contains("skill") || input_lower.contains("能力") {
        hints.explicit_skill = true;
    }
    
    // 计划提示
    if input_lower.contains("plan") || input_lower.contains("计划") {
        hints.explicit_plan = true;
    }
    
    // Git 提示
    if input_lower.contains("git") || input_lower.contains("commit") || input_lower.contains("branch") {
        hints.explicit_git = true;
    }
    
    // Web 提示
    if input_lower.contains("web") || input_lower.contains("http") || input_lower.contains("url") {
        hints.explicit_web = true;
    }
    
    // 诊断提示
    if input_lower.contains("diagnostic") || input_lower.contains("error") || input_lower.contains("lint") {
        hints.explicit_diag = true;
    }
    
    // 任务提示
    if input_lower.contains("task") || input_lower.contains("任务") {
        hints.explicit_task = true;
    }
    
    // 新文件提示
    if input_lower.contains("new file") || input_lower.contains("create file") || input_lower.contains("新建文件") {
        hints.explicit_new_file = true;
    }
    
    // 代码导航提示
    if input_lower.contains("navigate") || input_lower.contains("go to") || input_lower.contains("跳转") {
        hints.explicit_code_nav = true;
    }
    
    // 记忆提示
    if input_lower.contains("remember") || input_lower.contains("memory") || input_lower.contains("记忆") {
        hints.explicit_memory = true;
    }
    
    hints
}

/// 工具角色
#[derive(Debug, Clone)]
pub(crate) struct ToolRoles {
    pub(crate) is_read: bool,
    pub(crate) is_search: bool,
    pub(crate) is_edit: bool,
    pub(crate) is_execute: bool,
    pub(crate) is_project_map: bool,
    pub(crate) is_semantic_search: bool,
}

/// 计算工具角色
pub(crate) fn compute_tool_roles(tool: &StarTool) -> ToolRoles {
    let name = &tool.function.name;
    ToolRoles {
        is_read: is_read_tool(tool),
        is_search: is_search_tool(tool),
        is_edit: is_edit_tool_name(name),
        is_execute: name == "Bash" || name == "run_tests",
        is_project_map: is_project_map_tool(tool),
        is_semantic_search: is_semantic_search_tool(tool),
    }
}

/// 检查是否是读取工具
fn is_read_tool(tool: &StarTool) -> bool {
    matches!(tool.function.name.as_str(), "Read" | "view_file" | "ListDir")
}

/// 检查是否是搜索工具
fn is_search_tool(tool: &StarTool) -> bool {
    matches!(tool.function.name.as_str(), "Grep" | "Glob" | "rg" | "SemanticSearch")
}

/// 检查是否是项目地图工具
fn is_project_map_tool(tool: &StarTool) -> bool {
    tool.function.name == "ProjectMap"
}

/// 检查是否是语义搜索工具
fn is_semantic_search_tool(tool: &StarTool) -> bool {
    tool.function.name == "SemanticSearch"
}

/// 检查是否匹配模式
fn tool_matches_patterns(tool: &StarTool, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| tool.function.name.contains(pattern))
}
