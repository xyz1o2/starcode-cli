//! SubAgent 实时进度回流。
//!
//! 对标 Claude Code 的 `progressMessages` 机制：子 Agent 每产出一条消息，
//! 就把累计统计（tool uses / tokens）+ 最近工具摘要推回父级 UI。
//!
//! 对应参考实现：
//! - `AgentTool/UI.tsx::calculateAgentStats`   → [`AgentProgressTracker`]
//! - `AgentTool/UI.tsx::extractLastToolInfo`   → [`AgentProgressTracker::last_tool_info`]
//! - `AgentTool/UI.tsx::getSearchReadSummaryText` → [`search_read_summary_text`]
//! - `utils/agentContext.ts` 的 AsyncLocalStorage → [`ui_sink`]（进程级单活跃会话）

use crate::types::{ChatEntry, StreamingChunk};
use std::sync::{Arc, OnceLock, RwLock};
use tokio::sync::mpsc::UnboundedSender;

// ── 全局 UI sink ─────────────────────────────────────────────────────────

/// 当前活跃 UI 会话的 direct-chunk 发送端。
///
/// Claude Code 用 `AsyncLocalStorage` 把 UI 回调透传给任意深度的工具调用；
/// Rust 侧没有等价设施，且同一进程内只有一个活跃 TUI 会话，
/// 所以用一个进程级槽位承载。`StarAgent::process_user_message_stream`
/// 在建立 stream 通道时注册，AgentTool 在同步路径中取用。
static UI_SINK: OnceLock<RwLock<Option<UnboundedSender<StreamingChunk>>>> = OnceLock::new();

fn ui_sink() -> &'static RwLock<Option<UnboundedSender<StreamingChunk>>> {
    UI_SINK.get_or_init(|| RwLock::new(None))
}

/// 注册当前活跃 UI 会话的 chunk 发送端。
pub fn set_ui_sink(tx: UnboundedSender<StreamingChunk>) {
    if let Ok(mut slot) = ui_sink().write() {
        *slot = Some(tx);
    }
}

/// 跨 turn 常驻的兜底 sink（后台代理专用）。
///
/// [`UI_SINK`] 里那个发送端属于**当前 turn** 的流：turn 一结束，接收端就随流被丢弃，
/// 之后 `send` 全部失败。后台代理却会活过 turn 边界——它跑完时主循环往往正空闲，
/// 终态就此丢失，选择器只能一直显示 Running。所以再留一个由 worker 常驻持有的槽位，
/// 会话 sink 发不出去时走这里。
static BG_SINK: OnceLock<RwLock<Option<UnboundedSender<StreamingChunk>>>> = OnceLock::new();

fn bg_sink() -> &'static RwLock<Option<UnboundedSender<StreamingChunk>>> {
    BG_SINK.get_or_init(|| RwLock::new(None))
}

/// 注册常驻兜底 sink（由 `agent_worker` 在启动时调用，生命周期与进程同长）。
pub fn set_bg_progress_sink(tx: UnboundedSender<StreamingChunk>) {
    if let Ok(mut slot) = bg_sink().write() {
        *slot = Some(tx);
    }
}

/// 向 UI 推送一个 chunk。没有活跃 UI（headless / 子代理内部）时静默丢弃。
///
/// 优先走当前会话的 sink —— turn 进行中时，chunk 与文本 delta 同序抵达；
/// 会话已结束（接收端被丢弃）则退到常驻 sink，见 [`BG_SINK`]。
pub fn emit_to_ui(chunk: StreamingChunk) {
    let mut pending = chunk;
    if let Ok(slot) = ui_sink().read() {
        if let Some(tx) = slot.as_ref() {
            match tx.send(pending) {
                Ok(()) => return,
                // 接收端已随 turn 结束被丢弃，把 chunk 取回来交给兜底 sink
                Err(e) => pending = e.0,
            }
        }
    }
    if let Ok(slot) = bg_sink().read() {
        if let Some(tx) = slot.as_ref() {
            let _ = tx.send(pending);
        }
    }
}

// ── 进度事件 ─────────────────────────────────────────────────────────────

/// 子 Agent 的一次进度快照（累计值，非增量）
#[derive(Debug, Clone, Default)]
pub struct SubAgentProgress {
    /// 累计工具调用次数（对标 `calculateAgentStats` 的 toolUseCount：数 tool_result）
    pub tool_use_count: u32,
    /// 累计 token 数（cache_creation + cache_read + input + output）
    pub tokens: u32,
    /// 最近一次工具的语义摘要（对标 `extractLastToolInfo`）
    pub last_tool_info: Option<String>,
    /// 本次新增的子条目（增量，追加到 UI 已有列表）
    pub new_entries: Vec<ChatEntry>,
}

/// 进度回调：由 AgentTool 提供，把 [`SubAgentProgress`] 转成 AgentTaskUpdate chunk
pub type SubAgentProgressSink = Arc<dyn Fn(SubAgentProgress) + Send + Sync>;

// ── 工具语义摘要（对标 extractLastToolInfo）───────────────────────────────

/// 工具类别：用于连续 search/read/repl 聚合（对标 `getSearchOrReadInfo`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKindHint {
    Search,
    Read,
    Repl,
    Other,
}

/// 判断工具属于哪一类（对标 `getSearchOrReadInfo` 的 isSearch/isRead/isRepl）
pub fn classify_tool(tool_name: &str) -> ToolKindHint {
    match tool_name {
        "Grep" | "Glob" | "grep" | "glob" | "SemanticSearch" | "semantic_search" | "Search"
        | "search" | "WebSearch" | "ProjectMap" | "project_map" => ToolKindHint::Search,
        "Read" | "read" | "ReadFile" | "read_file" | "NotebookRead" | "WebFetch" => {
            ToolKindHint::Read
        }
        "REPL" | "Repl" | "repl" => ToolKindHint::Repl,
        _ => ToolKindHint::Other,
    }
}

/// 连续 search/read/repl 操作的聚合文案（对标 `getSearchReadSummaryText`）
///
/// 例：`Searched 5 files, read 3 files…`
pub fn search_read_summary_text(
    search_count: u32,
    read_count: u32,
    is_active: bool,
    repl_count: u32,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if search_count > 0 {
        parts.push(format!(
            "Searched {} {}",
            search_count,
            if search_count == 1 { "file" } else { "files" }
        ));
    }
    if read_count > 0 {
        let verb = if parts.is_empty() { "Read" } else { "read" };
        parts.push(format!(
            "{} {} {}",
            verb,
            read_count,
            if read_count == 1 { "file" } else { "files" }
        ));
    }
    if repl_count > 0 {
        let verb = if parts.is_empty() { "Ran" } else { "ran" };
        parts.push(format!(
            "{} {} {}",
            verb,
            repl_count,
            if repl_count == 1 { "snippet" } else { "snippets" }
        ));
    }
    if parts.is_empty() {
        return String::new();
    }
    let joined = parts.join(", ");
    if is_active {
        format!("{}…", joined)
    } else {
        joined
    }
}

/// 工具的用户可见名称（对标 `tool.userFacingName()`）
pub fn user_facing_tool_name(tool_name: &str) -> String {
    match tool_name {
        "read" | "read_file" | "ReadFile" => "Read".to_string(),
        "write" | "write_file" | "WriteFile" => "Write".to_string(),
        "edit" | "edit_file" | "EditFile" => "Edit".to_string(),
        "bash" | "shell" | "run_command" => "Bash".to_string(),
        "grep" | "search_text" => "Grep".to_string(),
        "glob" | "find_files" => "Glob".to_string(),
        "semantic_search" => "SemanticSearch".to_string(),
        "project_map" => "ProjectMap".to_string(),
        "todo_write" | "TodoWrite" => "TodoWrite".to_string(),
        other => {
            // MCP 工具 mcp__server__tool → server:tool
            if let Some(rest) = other.strip_prefix("mcp__") {
                let mut it = rest.splitn(2, "__");
                if let (Some(server), Some(tool)) = (it.next(), it.next()) {
                    return format!("{}:{}", server, tool);
                }
            }
            other.to_string()
        }
    }
}

/// 从工具参数里提取一行摘要（对标 `tool.getToolUseSummary()`）
///
/// 返回 `None` 表示该工具没有可展示的摘要，只显示工具名。
pub fn tool_use_summary(tool_name: &str, arguments: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let obj = parsed.as_object()?;

    // 按优先级挑选最有信息量的字段
    const KEYS: [&str; 10] = [
        "file_path",
        "path",
        "pattern",
        "query",
        "command",
        "url",
        "description",
        "prompt",
        "name",
        "notebook_path",
    ];
    for key in KEYS {
        if let Some(value) = obj.get(key).and_then(|v| v.as_str()) {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            // 路径类只保留末段，避免占满一行
            let display = if matches!(key, "file_path" | "path" | "notebook_path") {
                shorten_path(trimmed)
            } else {
                first_line(trimmed)
            };
            return Some(display);
        }
    }
    None
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn shorten_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 2 {
        return normalized;
    }
    segments[segments.len() - 2..].join("/")
}

/// 组合出 `ToolName: summary` 形式的一行状态（对标 `extractLastToolInfo` 尾部逻辑）
pub fn describe_tool_use(tool_name: &str, arguments: &str) -> String {
    let display_name = user_facing_tool_name(tool_name);
    match tool_use_summary(tool_name, arguments) {
        Some(summary) if !summary.is_empty() => format!("{}: {}", display_name, summary),
        _ => display_name,
    }
}

// ── 进度累加器 ───────────────────────────────────────────────────────────

/// 累计子 Agent 的统计信息，并在每次更新后回调 sink。
///
/// 对标 `calculateAgentStats` + `extractLastToolInfo` 的组合：
/// - `tool_use_count` 只在工具**结果**到达时递增（与参考实现数 tool_result 一致，
///   避免 tool_use 与 tool_result 双计）
/// - `tokens` 取最近一次 usage 的总和（参考实现取最后一条 assistant 的 usage）
/// - 尾部连续 search/read ≥2 次时，`last_tool_info` 折叠为聚合文案
pub struct AgentProgressTracker {
    sink: Option<SubAgentProgressSink>,
    tool_use_count: u32,
    tokens: u32,
    /// 尾部连续 search 计数（被非 search/read 工具打断则清零）
    trailing_search: u32,
    trailing_read: u32,
    trailing_repl: u32,
    /// 最近一次工具调用的描述（未被聚合时使用）
    last_tool_desc: Option<String>,
    /// tool_call_id → (tool_name, arguments)，用于结果到达时还原摘要
    pending: std::collections::HashMap<String, (String, String)>,
    /// 全部子条目（用于最终 SubAgentResult.entries）
    entries: Vec<ChatEntry>,
}

impl AgentProgressTracker {
    pub fn new(sink: Option<SubAgentProgressSink>) -> Self {
        Self {
            sink,
            tool_use_count: 0,
            tokens: 0,
            trailing_search: 0,
            trailing_read: 0,
            trailing_repl: 0,
            last_tool_desc: None,
            pending: std::collections::HashMap::new(),
            entries: Vec::new(),
        }
    }

    pub fn tool_use_count(&self) -> u32 {
        self.tool_use_count
    }

    pub fn tokens(&self) -> u32 {
        self.tokens
    }

    pub fn into_entries(self) -> Vec<ChatEntry> {
        self.entries
    }

    /// 最近状态文案：尾部连续 search/read 达到 2 次以上时折叠为聚合文案
    pub fn last_tool_info(&self) -> Option<String> {
        let grouped = self.trailing_search + self.trailing_read + self.trailing_repl;
        if grouped >= 2 {
            let text = search_read_summary_text(
                self.trailing_search,
                self.trailing_read,
                true,
                self.trailing_repl,
            );
            if !text.is_empty() {
                return Some(text);
            }
        }
        self.last_tool_desc.clone()
    }

    /// 记录一次工具调用开始
    pub fn on_tool_started(&mut self, tool_call: &crate::types::StarToolCall) {
        let name = tool_call.function.name.clone();
        let args = tool_call.function.arguments.clone();
        self.last_tool_desc = Some(describe_tool_use(&name, &args));
        self.pending.insert(tool_call.id.clone(), (name, args));

        let entry = ChatEntry::tool_call(String::new(), tool_call.clone());
        self.entries.push(entry.clone());
        self.flush(vec![entry]);
    }

    /// 记录一次工具执行完成
    pub fn on_tool_finished(
        &mut self,
        tool_call: &crate::types::StarToolCall,
        result: &crate::types::ToolResult,
    ) {
        // 与参考实现一致：统计口径是 tool_result
        self.tool_use_count = self.tool_use_count.saturating_add(1);

        let (name, args) = self
            .pending
            .remove(&tool_call.id)
            .unwrap_or_else(|| (tool_call.function.name.clone(), tool_call.function.arguments.clone()));

        match classify_tool(&name) {
            ToolKindHint::Search => self.trailing_search = self.trailing_search.saturating_add(1),
            ToolKindHint::Read => self.trailing_read = self.trailing_read.saturating_add(1),
            ToolKindHint::Repl => self.trailing_repl = self.trailing_repl.saturating_add(1),
            ToolKindHint::Other => {
                // 被非 search/read 工具打断 → 清空尾部聚合窗口
                self.trailing_search = 0;
                self.trailing_read = 0;
                self.trailing_repl = 0;
            }
        }
        self.last_tool_desc = Some(describe_tool_use(&name, &args));

        let content = if result.success {
            result.output.clone().unwrap_or_default()
        } else {
            result.error.clone().unwrap_or_default()
        };
        let entry = ChatEntry::tool_result(content, tool_call.clone(), result.clone());
        self.entries.push(entry.clone());
        self.flush(vec![entry]);
    }

    /// 记录 token 用量更新
    pub fn on_tokens(&mut self, tokens: u32) {
        if tokens == 0 {
            return;
        }
        self.tokens = tokens;
        self.flush(Vec::new());
    }

    /// 记录一条 assistant 文本
    pub fn on_assistant_text(&mut self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let entry = ChatEntry::assistant(text);
        self.entries.push(entry.clone());
        self.flush(vec![entry]);
    }

    fn flush(&self, new_entries: Vec<ChatEntry>) {
        if let Some(sink) = &self.sink {
            sink(SubAgentProgress {
                tool_use_count: self.tool_use_count,
                tokens: self.tokens,
                last_tool_info: self.last_tool_info(),
                new_entries,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_text_matches_reference_phrasing() {
        assert_eq!(
            search_read_summary_text(5, 3, true, 0),
            "Searched 5 files, read 3 files…"
        );
        assert_eq!(search_read_summary_text(1, 0, false, 0), "Searched 1 file");
        assert_eq!(search_read_summary_text(0, 2, false, 0), "Read 2 files");
        assert_eq!(search_read_summary_text(0, 0, true, 0), "");
    }

    #[test]
    fn tool_summary_prefers_path_tail() {
        let s = tool_use_summary("Read", r#"{"file_path":"/a/b/c/d.rs"}"#);
        assert_eq!(s.as_deref(), Some("c/d.rs"));
    }

    #[test]
    fn describe_falls_back_to_name_without_args() {
        assert_eq!(describe_tool_use("Bash", "not json"), "Bash");
    }

    #[test]
    fn mcp_tool_name_is_shortened() {
        assert_eq!(user_facing_tool_name("mcp__exa__web_search"), "exa:web_search");
    }

    #[test]
    fn trailing_search_reads_collapse_into_summary() {
        let mut t = AgentProgressTracker::new(None);
        let call = |id: &str, name: &str| crate::types::StarToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        };
        let ok = crate::types::ToolResult {
            success: true,
            output: Some("ok".to_string()),
            error: None,
            data: None,
        };
        t.on_tool_finished(&call("1", "Grep"), &ok);
        t.on_tool_finished(&call("2", "Read"), &ok);
        assert_eq!(
            t.last_tool_info().as_deref(),
            Some("Searched 1 file, read 1 file…")
        );

        // 非 search/read 工具打断聚合窗口
        t.on_tool_finished(&call("3", "Bash"), &ok);
        assert_eq!(t.last_tool_info().as_deref(), Some("Bash"));
        assert_eq!(t.tool_use_count(), 3);
    }
}
