//! `tool_search`：长尾工具的发现入口。
//!
//! 对标 Claude Code 的 `SearchExtraTools`，补齐三处差距：
//! 1. **分词加权检索**。旧实现拿整条 query 去做子串匹配，"find a tool to create a
//!    git branch" 这类自然语言 query 必然零命中。现在按 token 打分，名称权重 3.0、
//!    描述权重 1.0（与参考设计的字段权重一致）。
//! 2. **返回 JSON Schema**。旧实现只给 name + description，模型发现工具后仍要猜参数。
//!    现在关键词模式为前若干命中附上完整 schema，`select:<name>` 模式返回单个工具的完整定义。
//! 3. **会话粘滞**。命中的工具名记入 [`record_discovered_tools`]，下一条用户消息的
//!    工具短名单会带上它们（见 `agent::tool_routing`）。
//!
//! 不做 `ExecuteExtraTool` 代理层：starcode 的工具执行**不受短名单门控**
//! （`turn_active_tools` 只用于自动触发与 JSON fallback），模型报得出名字就能调用，
//! 所以把 schema 交出去就够了，无需再加一层转发。

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::tools::ToolRegistry;
use serde::Deserialize;
use std::sync::{Arc, Mutex, OnceLock};

/// 关键词模式下最多为几个命中附带完整 schema。再多会显著膨胀工具结果。
const SCHEMA_DETAIL_LIMIT: usize = 3;
/// 关键词模式默认返回条数。
const DEFAULT_MAX_RESULTS: usize = 10;
/// 粘滞集合上限，超出后裁剪到 [`DISCOVERED_TRIM_TO`]（对标参考设计的 500→400）。
const DISCOVERED_CAP: usize = 500;
const DISCOVERED_TRIM_TO: usize = 400;

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSearchParams {
    pub query: String,
    /// 关键词模式返回条数上限，默认 10。
    #[serde(default)]
    pub max_results: Option<usize>,
}

// ── 会话粘滞集合 ────────────────────────────────────────────────────────
//
// 写入方只有 tool_search；读取方是 agent::tool_routing 的短名单装配。
// 之所以分 live / frozen 两份：短名单每轮都会重算，若直接读 live，模型在第 3 轮
// 发现一个工具就会让第 4 轮的 tools 数组变形，击穿 prompt 缓存前缀。
// frozen 只在每条用户消息开始时刷新，保证同一条消息内 tools 数组逐字节不变。

#[derive(Default)]
struct DiscoveredTools {
    live: Vec<String>,
    frozen: Vec<String>,
}

fn discovered_state() -> &'static Mutex<DiscoveredTools> {
    static STATE: OnceLock<Mutex<DiscoveredTools>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DiscoveredTools::default()))
}

/// 记录本次会话中被 `tool_search` 命中的工具名（按发现顺序，重复项提升到最新）。
pub fn record_discovered_tools(names: &[String]) {
    let Ok(mut state) = discovered_state().lock() else {
        return;
    };
    for name in names {
        state.live.retain(|existing| existing != name);
        state.live.push(name.clone());
    }
    if state.live.len() > DISCOVERED_CAP {
        let drop_count = state.live.len() - DISCOVERED_TRIM_TO;
        state.live.drain(..drop_count);
    }
}

/// 在每条用户消息开始时把 live 快照进 frozen。
/// 同一条消息的所有轮次因此读到同一份粘滞集合，tools 数组保持稳定。
pub fn begin_message_epoch() {
    if let Ok(mut state) = discovered_state().lock() {
        state.frozen = state.live.clone();
    }
}

/// 读取当前消息的粘滞集合快照，最近发现的排在前面。
pub fn discovered_tools_snapshot() -> Vec<String> {
    let Ok(state) = discovered_state().lock() else {
        return Vec::new();
    };
    state.frozen.iter().rev().cloned().collect()
}

/// 仅供测试使用：清空粘滞集合。
#[cfg(test)]
pub(crate) fn reset_discovered_tools() {
    if let Ok(mut state) = discovered_state().lock() {
        state.live.clear();
        state.frozen.clear();
    }
}

/// 仅供测试使用：粘滞集合是进程级全局状态，触碰它的测试必须先取这把锁串行化
/// （`agent::tool_routing` 的短名单测试也会读它）。
#[cfg(test)]
pub(crate) fn sticky_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── 分词与打分 ──────────────────────────────────────────────────────────

/// 把标识符拆成小写 token：下划线/连字符/非字母数字为分隔符，
/// 同时在 camelCase 边界断开（`SemanticSearch` → `semantic` + `search`）。
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_lower_or_digit = false;

    for ch in input.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower_or_digit && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = ch.is_lowercase() || ch.is_numeric();
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_lower_or_digit = false;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 检索噪声词：出现在自然语言 query 里但对区分工具毫无帮助。
fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "any"
            | "are"
            | "can"
            | "create"
            | "find"
            | "for"
            | "get"
            | "how"
            | "i"
            | "is"
            | "list"
            | "me"
            | "of"
            | "or"
            | "that"
            | "the"
            | "to"
            | "tool"
            | "tools"
            | "use"
            | "want"
            | "what"
            | "with"
    )
}

/// 提取有效 query token：去噪声词、去单字符。
fn query_tokens(query: &str) -> Vec<String> {
    tokenize(query)
        .into_iter()
        .filter(|t| t.len() > 1 && !is_stopword(t))
        .collect()
}

/// 给单个工具打分。字段权重对齐参考设计：名称 3.0 > 描述 1.0。
fn score_entry(name: &str, description: &str, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }

    let name_tokens = tokenize(name);
    let name_lower = name.to_lowercase();
    let desc_lower = description.to_lowercase();
    let mut score = 0.0;

    for token in tokens {
        if name_tokens.iter().any(|nt| nt == token) {
            score += 3.0;
        } else if token.len() >= 3 && name_lower.contains(token.as_str()) {
            score += 1.5;
        }
        if desc_lower.contains(token.as_str()) {
            score += 1.0;
        }
    }
    score
}

// ── 工具实现 ────────────────────────────────────────────────────────────

/// 单条命中：工具名 + 描述 + 完整参数 schema。
struct Hit {
    name: String,
    description: String,
    schema: serde_json::Value,
}

pub struct ToolSearchTool {
    registry: Arc<ToolRegistry>,
}

impl ToolSearchTool {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    /// `select:<name>` 模式：精确取一个工具（经 `get_tool` 走 canonical 别名解析）。
    fn select_tool(&self, name: &str) -> Option<Hit> {
        let tool = self.registry.get_tool(name)?;
        Some(Hit {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            schema: tool.parameter_schema(),
        })
    }

    /// 关键词模式：分词打分，按分数降序、同分按名称升序。
    fn search_ranked(&self, query: &str, max_results: usize) -> Vec<Hit> {
        rank_entries(self.registry.get_all_tool_entries(), query, max_results)
    }
}

/// 打分与排序的纯函数部分：不依赖 `ToolRegistry`，便于单元测试。
fn rank_entries(
    entries: Vec<(String, String, serde_json::Value)>,
    query: &str,
    max_results: usize,
) -> Vec<Hit> {
    let tokens = query_tokens(query);
    let mut scored: Vec<(f64, Hit)> = Vec::new();
    for (name, description, schema) in entries {
        let score = score_entry(&name, &description, &tokens);
        if score <= 0.0 {
            continue;
        }
        scored.push((
            score,
            Hit {
                name,
                description,
                schema,
            },
        ));
    }
    // 同分按名称排序：get_all_tool_entries 源自 HashMap，迭代顺序随进程随机，
    // 不排序会让同一 query 在不同进程里返回不同顺序。
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    scored.truncate(max_results.max(1));
    scored.into_iter().map(|(_, hit)| hit).collect()
}

impl Clone for ToolSearchTool {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
        }
    }
}

/// 识别 `select:<name>` 前缀（大小写不敏感），返回其后的工具名。
fn strip_select_prefix(query: &str) -> Option<&str> {
    const PREFIX: &str = "select:";
    let trimmed = query.trim();
    // `get` 而非切片：query 可能以多字节字符开头，直接切片会在非字符边界 panic。
    if !trimmed.get(..PREFIX.len())?.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    Some(trimmed[PREFIX.len()..].trim())
}

/// 把 schema 渲染成围栏 JSON 块。schema 缺失或为 null 时返回空串。
fn render_schema(schema: &serde_json::Value) -> String {
    if schema.is_null() {
        return String::new();
    }
    match serde_json::to_string_pretty(schema) {
        Ok(text) => format!("\n```json\n{}\n```", text),
        Err(_) => String::new(),
    }
}

impl BaseDeclarativeTool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn display_name(&self) -> &str {
        "Tool Search"
    }

    fn description(&self) -> &str {
        "Discover tools that are not in the current tool list (built-in long-tail tools and MCP \
         tools). Two query modes: plain keywords (\"rename a git branch\") return a ranked list \
         with JSON schemas for the top matches; `select:<tool_name>` returns the full parameter \
         schema for one tool. Any tool returned here can be called directly by name."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords describing the capability you need, or \
                                    `select:<tool_name>` to fetch one tool's full schema."
                },
                "max_results": {
                    "type": "number",
                    "description": "Maximum number of matches to return in keyword mode \
                                    (default 10)."
                }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ToolSearchParams = serde_json::from_value(params)?;
        Ok(Box::new(ToolSearchInvocation {
            tool: self.clone(),
            params,
        }))
    }
}

pub struct ToolSearchInvocation {
    tool: ToolSearchTool,
    params: ToolSearchParams,
}

impl ToolSearchInvocation {
    /// 组装 `select:<name>` 模式的结果。
    fn render_select(hit: Option<Hit>, wanted: &str) -> ToolResult {
        let Some(hit) = hit else {
            let message = format!(
                "No tool named `{}` is registered. Retry with plain keywords to search by \
                 capability.",
                wanted
            );
            return ToolResult {
                llm_content: Some(message.clone()),
                return_display: Some(format!("No such tool: {}", wanted)),
                output: message,
                error: None,
                data: Some(serde_json::json!({
                    "mode": "select",
                    "matched_tools": [],
                    "total": 0,
                    "query": wanted,
                })),
            };
        };

        record_discovered_tools(std::slice::from_ref(&hit.name));
        let output = format!(
            "**{}** — {}{}\n\nCall it directly by name; no further discovery step is needed.",
            hit.name,
            hit.description,
            render_schema(&hit.schema)
        );
        ToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(format!("Tool schema: {}", hit.name)),
            output,
            error: None,
            data: Some(serde_json::json!({
                "mode": "select",
                "matched_tools": [{
                    "name": hit.name,
                    "description": hit.description,
                    "parameter_schema": hit.schema,
                }],
                "total": 1,
                "query": wanted,
            })),
        }
    }

    /// 组装关键词模式的结果：前 [`SCHEMA_DETAIL_LIMIT`] 条附完整 schema，其余只给一行摘要。
    fn render_keyword(hits: Vec<Hit>, query: &str) -> ToolResult {
        if hits.is_empty() {
            let message = format!(
                "No tools matched '{}'. Try fewer or more specific keywords, or use \
                 `select:<tool_name>` if you already know the name.",
                query
            );
            return ToolResult {
                llm_content: Some(message.clone()),
                return_display: Some(format!("No tools matched '{}'", query)),
                output: message,
                error: None,
                data: Some(serde_json::json!({
                    "mode": "keyword",
                    "matched_tools": [],
                    "total": 0,
                    "query": query,
                })),
            };
        }

        let names: Vec<String> = hits.iter().map(|hit| hit.name.clone()).collect();
        record_discovered_tools(&names);

        let mut lines = vec![format!(
            "Found {} tool(s) matching '{}':",
            hits.len(),
            query
        )];
        for (index, hit) in hits.iter().enumerate() {
            if index < SCHEMA_DETAIL_LIMIT {
                lines.push(format!(
                    "\n{}. **{}** — {}{}",
                    index + 1,
                    hit.name,
                    hit.description,
                    render_schema(&hit.schema)
                ));
            } else {
                lines.push(format!(
                    "{}. **{}** — {}",
                    index + 1,
                    hit.name,
                    hit.description
                ));
            }
        }
        if hits.len() > SCHEMA_DETAIL_LIMIT {
            lines.push(format!(
                "\nSchemas shown for the first {} match(es). Query `select:<tool_name>` for the \
                 rest.",
                SCHEMA_DETAIL_LIMIT
            ));
        }
        lines.push("Every tool listed above can be called directly by name.".to_string());

        let output = lines.join("\n");
        ToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(format!("Found {} tool(s) for '{}'", hits.len(), query)),
            output,
            error: None,
            data: Some(serde_json::json!({
                "mode": "keyword",
                "matched_tools": hits
                    .iter()
                    .enumerate()
                    .map(|(index, hit)| {
                        let mut entry = serde_json::json!({
                            "name": hit.name,
                            "description": hit.description,
                        });
                        if index < SCHEMA_DETAIL_LIMIT {
                            entry["parameter_schema"] = hit.schema.clone();
                        }
                        entry
                    })
                    .collect::<Vec<_>>(),
                "total": hits.len(),
                "query": query,
            })),
        }
    }
}

impl ToolInvocation for ToolSearchInvocation {
    fn get_description(&self) -> String {
        format!("Tool search: {}", self.params.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let tool = self.tool.clone();
        let query = self.params.query.clone();
        let max_results = self.params.max_results.unwrap_or(DEFAULT_MAX_RESULTS);

        Box::pin(async move {
            // `select:<name>` 走精确查找，其余一律当关键词处理。
            // 前缀大小写不敏感：模型偶尔会写成 `Select:`。
            if let Some(wanted) = strip_select_prefix(&query) {
                return Ok(ToolSearchInvocation::render_select(
                    tool.select_tool(wanted),
                    wanted,
                ));
            }
            let hits = tool.search_ranked(&query, max_results);
            Ok(ToolSearchInvocation::render_keyword(hits, &query))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 粘滞集合是进程级全局状态，触碰它的测试必须串行。
    fn sticky_guard() -> std::sync::MutexGuard<'static, ()> {
        super::sticky_test_guard()
    }

    fn entry(name: &str, description: &str) -> (String, String, serde_json::Value) {
        (
            name.to_string(),
            description.to_string(),
            serde_json::json!({
                "type": "object",
                "properties": { "target": { "type": "string" } },
                "required": ["target"]
            }),
        )
    }

    fn sample_entries() -> Vec<(String, String, serde_json::Value)> {
        vec![
            entry("git_branch", "Create, rename or delete a git branch."),
            entry(
                "git_rewind",
                "Restore the working tree to an earlier checkpoint.",
            ),
            entry(
                "SemanticSearch",
                "Find code by meaning rather than exact text.",
            ),
            entry("Grep", "Search file contents with a regular expression."),
            entry("cron", "Schedule a recurring prompt."),
        ]
    }

    #[test]
    fn tokenize_splits_snake_and_camel_case() {
        assert_eq!(tokenize("git_branch"), vec!["git", "branch"]);
        assert_eq!(tokenize("SemanticSearch"), vec!["semantic", "search"]);
        assert_eq!(
            tokenize("mcp__server__do_thing"),
            vec!["mcp", "server", "do", "thing"]
        );
    }

    #[test]
    fn stopwords_are_dropped_from_the_query() {
        assert_eq!(
            query_tokens("find a tool to create a git branch"),
            vec!["git", "branch"]
        );
    }

    /// 回归：旧实现拿整条 query 做子串匹配，多词自然语言 query 必然零命中。
    #[test]
    fn natural_language_query_matches_by_token() {
        let hits = rank_entries(sample_entries(), "find a tool to create a git branch", 10);
        assert_eq!(hits.first().map(|h| h.name.as_str()), Some("git_branch"));
    }

    #[test]
    fn name_match_outranks_description_only_match() {
        // "search" 命中 SemanticSearch 的名称（3.0），只命中 Grep 的描述（1.0）。
        let hits = rank_entries(sample_entries(), "search", 10);
        assert_eq!(
            hits.first().map(|h| h.name.as_str()),
            Some("SemanticSearch")
        );
        assert!(hits.iter().any(|h| h.name == "Grep"));
    }

    #[test]
    fn zero_score_entries_are_excluded() {
        let hits = rank_entries(sample_entries(), "kubernetes", 10);
        assert!(hits.is_empty());
    }

    #[test]
    fn max_results_caps_the_list() {
        let hits = rank_entries(sample_entries(), "git", 1);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn ranking_is_stable_regardless_of_entry_order() {
        let mut reversed = sample_entries();
        reversed.reverse();
        let forward: Vec<String> = rank_entries(sample_entries(), "git branch", 10)
            .into_iter()
            .map(|h| h.name)
            .collect();
        let backward: Vec<String> = rank_entries(reversed, "git branch", 10)
            .into_iter()
            .map(|h| h.name)
            .collect();
        assert_eq!(forward, backward);
    }

    #[test]
    fn keyword_result_carries_schema_for_top_hits() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        let hits = rank_entries(sample_entries(), "git search cron", 10);
        assert!(
            hits.len() > SCHEMA_DETAIL_LIMIT,
            "need more hits than the schema limit"
        );
        let result = ToolSearchInvocation::render_keyword(hits, "git search cron");

        // 前 SCHEMA_DETAIL_LIMIT 条附 schema，其余只有一行摘要。
        assert_eq!(
            result.output.matches("```json").count(),
            SCHEMA_DETAIL_LIMIT
        );
        let data = result.data.expect("data present");
        assert_eq!(data["mode"], "keyword");
        let matched = data["matched_tools"]
            .as_array()
            .expect("matched_tools array");
        assert!(matched[0].get("parameter_schema").is_some());
        assert!(matched[SCHEMA_DETAIL_LIMIT]
            .get("parameter_schema")
            .is_none());
    }

    #[test]
    fn empty_keyword_result_is_not_an_error() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        let result = ToolSearchInvocation::render_keyword(Vec::new(), "kubernetes");
        assert!(result.error.is_none());
        assert!(result.output.contains("No tools matched"));
        // 零命中不应污染粘滞集合。
        begin_message_epoch();
        assert!(discovered_tools_snapshot().is_empty());
    }

    #[test]
    fn select_mode_returns_the_full_schema() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        let (name, description, schema) = entry("git_branch", "Rename a git branch.");
        let hit = Hit {
            name,
            description,
            schema,
        };
        let result = ToolSearchInvocation::render_select(Some(hit), "git_branch");
        assert!(result.output.contains("```json"));
        assert!(result.output.contains("\"target\""));
        assert_eq!(result.data.expect("data present")["mode"], "select");
    }

    #[test]
    fn select_prefix_is_case_insensitive_and_utf8_safe() {
        assert_eq!(strip_select_prefix("select:git_branch"), Some("git_branch"));
        assert_eq!(
            strip_select_prefix("  Select: git_branch "),
            Some("git_branch")
        );
        assert_eq!(strip_select_prefix("keyword search"), None);
        // 多字节开头不得 panic。
        assert_eq!(strip_select_prefix("查找分支工具"), None);
        assert_eq!(strip_select_prefix("sel"), None);
    }

    #[test]
    fn select_mode_reports_an_unknown_tool_without_erroring() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        let result = ToolSearchInvocation::render_select(None, "no_such_tool");
        assert!(result.error.is_none());
        assert!(result.output.contains("no_such_tool"));
        assert_eq!(result.data.expect("data present")["total"], 0);
    }

    /// 缓存稳定性的核心不变量：消息进行中发现的工具不会立刻改变短名单，
    /// 必须等到下一条用户消息调用 begin_message_epoch 才可见。
    #[test]
    fn sticky_set_is_frozen_until_the_next_message_epoch() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        record_discovered_tools(&["git_branch".to_string()]);
        assert!(discovered_tools_snapshot().is_empty());

        begin_message_epoch();
        assert_eq!(discovered_tools_snapshot(), vec!["git_branch".to_string()]);

        // 同一条消息内再发现工具，快照保持不变。
        record_discovered_tools(&["cron".to_string()]);
        assert_eq!(discovered_tools_snapshot(), vec!["git_branch".to_string()]);

        begin_message_epoch();
        assert_eq!(
            discovered_tools_snapshot(),
            vec!["cron".to_string(), "git_branch".to_string()]
        );
    }

    #[test]
    fn sticky_set_dedups_to_newest_and_stays_bounded() {
        let _guard = sticky_guard();
        reset_discovered_tools();

        record_discovered_tools(&["a".to_string(), "b".to_string(), "a".to_string()]);
        begin_message_epoch();
        // 快照最新优先，重复项只留一份。
        assert_eq!(
            discovered_tools_snapshot(),
            vec!["a".to_string(), "b".to_string()]
        );

        let bulk: Vec<String> = (0..DISCOVERED_CAP + 50)
            .map(|i| format!("t{}", i))
            .collect();
        record_discovered_tools(&bulk);
        begin_message_epoch();
        assert_eq!(discovered_tools_snapshot().len(), DISCOVERED_TRIM_TO);
    }
}
