use crate::agent::tool_executor::ToolExecutor;
use crate::agent::tool_routing::{is_edit_tool_name, is_validation_tool_name};
use crate::types::{StarToolCall, ToolResult};
use std::sync::Arc;

/// 这次工具调用算不算"给出了可用结果"。
///
/// 只用于驱动 `codebase_search_attempted` 这类**一次性闸门**。区别很重要：
/// 冷启动时 `CodebaseSearch` 第一次调用几乎必然返回 `Warming`——空结果 + 一句
/// "稍后重跑"。把它算成"已搜过"的话，闸门就永久关上了，工具自己给的重跑建议
/// 被自家 flag 当场否掉，这次搜索永远补不上。
///
/// 认输（`success == false`）当然也不算——错误不代表拿到了答案。
pub(crate) fn tool_result_is_usable(result: &ToolResult) -> bool {
    if !result.success {
        return false;
    }
    let text = result.output.as_deref().unwrap_or_default();
    if text.trim().is_empty() {
        return false;
    }
    // 索引还在预热：内容是解释性文字而非检索结果。
    const COLD_MARKERS: &[&str] = &[
        "is still building",
        "is building in the background",
        "no hits this turn",
    ];
    !COLD_MARKERS.iter().any(|m| text.contains(m))
}

pub(crate) fn execute_single_tool_with_progress<'a>(
    tool_executor: Arc<ToolExecutor>,
    tool_call: StarToolCall,
    abort_signal: Option<tokio_util::sync::CancellationToken>,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<String>,
    impl std::future::Future<Output = ToolResult> + Send + 'a,
) {
    let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let update_output: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |msg| {
        let _ = progress_tx.send(msg);
    });

    let future = async move {
        let exec = tool_executor.execute_batch(vec![tool_call], Some(update_output), abort_signal);
        // 兜底超时：防止工具（尤其 CodebaseSearch/ProjectMap/Grep 等长运行工具）
        // 内部挂起导致 emit_tool_finished 永不发出、UI 圆点永远闪烁。
        // 超时后返回带超时提示的 error result，调用方仍会 emit finished 停止闪烁。
        //
        // 它必须**盖过确认窗口**，否则这道兜底会替用户把确认超时吃掉：
        // execute_batch 里含着等用户点确认的那段时间，两者都写 600s 时外层先到期，
        // 于是每一次没批准的 Bash 都报"执行超时 600s"，而真正的原因（没人点同意）
        // 被吞掉 —— 门控看上去像是坏了，实际是计时器互相踩。
        const TOOL_HARD_TIMEOUT_MARGIN_SECS: u64 = 120;
        let hard_timeout_secs =
            tool_executor.confirm_timeout_ms() / 1000 + TOOL_HARD_TIMEOUT_MARGIN_SECS;
        match tokio::time::timeout(std::time::Duration::from_secs(hard_timeout_secs), exec).await {
            Ok(results) => results.into_iter().next().unwrap_or(ToolResult {
                success: false,
                output: None,
                error: Some("tool executor returned no result".to_string()),
                data: None,
            }),
            Err(_) => ToolResult {
                success: false,
                output: None,
                error: Some(format!(
                    "Tool execution timed out after {}s",
                    hard_timeout_secs
                )),
                data: None,
            },
        }
    };

    (progress_rx, future)
}

pub(crate) fn update_verification_state(
    tool_call: &StarToolCall,
    result: &ToolResult,
    verification_required: &mut bool,
    skip_verification: bool,
) {
    if is_edit_tool_name(&tool_call.function.name) && result.success && !skip_verification {
        *verification_required = true;
    }
    if is_validation_tool_name(&tool_call.function.name) {
        *verification_required = false;
    }
}

/// 模型偶尔会把伪工具调用标签吐进 reasoning 流，这些标记不该出现在思考块里。
const REASONING_TAG_MARKERS: [&str; 6] = [
    "<function=",
    "<tool_call>",
    "<parameter=",
    "</parameter>",
    "<tool>",
    "</tool>",
];

/// reasoning 流式净化器：按行剔除伪工具调用标签，同时逐字保留空白。
///
/// 为什么必须带状态：`reasoning_content` 是分片到达的（`"The"`、`" user"`、
/// `" wants"`…）。若对每个分片各自做"按行 trim"净化，片首/片尾的空格会被
/// 吃掉，拼接后粘成 `"Theuserwants"` —— 这就是思考块里单词粘连的成因。
/// 所以净化状态必须跨分片保留，只在确认整行是标记行时才丢内容。
///
/// 判定规则（每行 trim 后做前缀匹配，与整段净化语义一致）：
/// - `<function=…>` / `<tool_call>` 开块，丢弃直到闭合标签行；
/// - `<parameter=…>`、`</parameter>` 丢弃该行；
/// - 整行恰为 `<tool>` / `</tool>` 丢弃该行；
/// - 其余行原样输出，含前导缩进。
///
/// 行首第一个非空白字符不是 `<` 时立即转入透传、不缓冲任何字符，因此正常
/// 思考文本零延迟；只有以 `<` 开头的行才攒到行尾再决定去留。
#[derive(Default)]
pub(crate) struct ReasoningSanitizer {
    state: SanitizerState,
    /// 尚未定夺去留的当前行片段（含前导空白）
    pending: String,
    /// `SkipBlock` 状态下等待的闭合标签
    close_tag: &'static str,
}

#[derive(Default, Clone, Copy)]
enum SanitizerState {
    /// 行首：本行还没出现非空白字符
    #[default]
    LineStart,
    /// 本行以 `<` 开头，等行尾再判定
    Buffering,
    /// 本行已判定为普通文本，到行尾都透传
    PassThrough,
    /// 处在伪工具调用块内，丢弃到闭合标签行
    SkipBlock,
}

impl ReasoningSanitizer {
    /// 喂入一个 reasoning 增量分片，返回可以立即显示的净化文本。
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        let mut out = String::with_capacity(chunk.len());
        for c in chunk.chars() {
            match self.state {
                SanitizerState::LineStart => {
                    if c == '<' {
                        self.pending.push(c);
                        self.state = SanitizerState::Buffering;
                    } else if c == '\n' {
                        // 空行（可能只含缩进）：原样保留，思考块靠它分段
                        out.push_str(&self.pending);
                        self.pending.clear();
                        out.push('\n');
                    } else if c.is_whitespace() {
                        // 缩进：先攒着，本行若被丢弃要一起丢
                        self.pending.push(c);
                    } else {
                        out.push_str(&self.pending);
                        self.pending.clear();
                        out.push(c);
                        self.state = SanitizerState::PassThrough;
                    }
                }
                SanitizerState::Buffering => {
                    self.pending.push(c);
                    if c == '\n' {
                        self.resolve_buffered_line(&mut out);
                    } else if !may_start_tag_line(self.pending.trim_start()) {
                        // 前缀已排除所有标记，不必再等行尾
                        out.push_str(&self.pending);
                        self.pending.clear();
                        self.state = SanitizerState::PassThrough;
                    }
                }
                SanitizerState::PassThrough => {
                    out.push(c);
                    if c == '\n' {
                        self.state = SanitizerState::LineStart;
                    }
                }
                SanitizerState::SkipBlock => {
                    if c == '\n' {
                        let line = std::mem::take(&mut self.pending);
                        let trimmed = line.trim();
                        if trimmed == self.close_tag || trimmed.ends_with(self.close_tag) {
                            self.close_tag = "";
                            self.state = SanitizerState::LineStart;
                        }
                    } else {
                        self.pending.push(c);
                    }
                }
            }
        }
        out
    }

    /// 流结束时收尾：把仍在缓冲区里的最后一行（无换行符结尾）定夺。
    pub(crate) fn flush(&mut self) -> String {
        let line = std::mem::take(&mut self.pending);
        let keep = match self.state {
            SanitizerState::LineStart => true,
            SanitizerState::Buffering => {
                let trimmed = line.trim();
                block_close_tag(trimmed).is_none() && !is_dropped_tag_line(trimmed)
            }
            _ => false,
        };
        self.state = SanitizerState::LineStart;
        self.close_tag = "";
        if keep {
            line
        } else {
            String::new()
        }
    }

    /// `pending` 已是完整一行（含结尾换行符）：决定丢弃、开块还是原样输出。
    fn resolve_buffered_line(&mut self, out: &mut String) {
        let line = std::mem::take(&mut self.pending);
        let trimmed = line.trim();
        if let Some(close) = block_close_tag(trimmed) {
            self.close_tag = close;
            self.state = SanitizerState::SkipBlock;
        } else if is_dropped_tag_line(trimmed) {
            self.state = SanitizerState::LineStart;
        } else {
            out.push_str(&line);
            self.state = SanitizerState::LineStart;
        }
    }
}

/// 该行是否开启一个伪工具调用块，返回需要等待的闭合标签。
fn block_close_tag(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("<function=") {
        Some("</function>")
    } else if trimmed.starts_with("<tool_call>") {
        Some("</tool_call>")
    } else {
        None
    }
}

/// 该行是否是需要整行丢弃的标记行。
fn is_dropped_tag_line(trimmed: &str) -> bool {
    trimmed.starts_with("<parameter=")
        || trimmed.starts_with("</parameter>")
        || trimmed == "<tool>"
        || trimmed == "</tool>"
}

/// 行首前缀（已 trim 左侧）是否仍可能长成标记行：既包含"还没打完"的前缀，
/// 也包含已经匹配上、但要等行尾才能定夺的情况（`<tool>` 要求整行相等）。
fn may_start_tag_line(prefix: &str) -> bool {
    REASONING_TAG_MARKERS
        .iter()
        .any(|marker| marker.starts_with(prefix) || prefix.starts_with(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟流式：逐片喂入并拼接输出，最后 flush。
    fn sanitize_stream(chunks: &[&str]) -> String {
        let mut sanitizer = ReasoningSanitizer::default();
        let mut out = String::new();
        for chunk in chunks {
            out.push_str(&sanitizer.push(chunk));
        }
        out.push_str(&sanitizer.flush());
        out
    }

    #[test]
    fn reasoning_deltas_keep_word_boundaries() {
        // 回归：逐片 trim 会粘成 "Theuserwantsafix"
        let out = sanitize_stream(&["The", " user", " wants", " a", " fix", "."]);
        assert_eq!(out, "The user wants a fix.");
    }

    #[test]
    fn whitespace_only_delta_survives() {
        assert_eq!(sanitize_stream(&["word", " ", "next"]), "word next");
    }

    #[test]
    fn blank_lines_and_indentation_survive() {
        assert_eq!(
            sanitize_stream(&["first\n", "\n", "  second", " line\n"]),
            "first\n\n  second line\n"
        );
    }

    #[test]
    fn plain_text_is_not_buffered_until_newline() {
        // 打字机效果：普通文本必须即刻回显，不能攒到行尾
        let mut sanitizer = ReasoningSanitizer::default();
        assert_eq!(sanitizer.push("Let me "), "Let me ");
        assert_eq!(sanitizer.push("check"), "check");
    }

    #[test]
    fn pseudo_tool_call_block_dropped_across_chunks() {
        let out = sanitize_stream(&[
            "I should read it.\n<tool",
            "_call>\n{\"path\"",
            ": \"a.rs\"}\n</tool_call>\n",
            "Then edit it.",
        ]);
        assert_eq!(out, "I should read it.\nThen edit it.");
    }

    #[test]
    fn function_block_and_parameter_lines_dropped() {
        let out = sanitize_stream(&[
            "before\n",
            "  <function=Read>\n",
            "  <parameter=file_path>a.rs</parameter>\n",
            "  </function>\n",
            "after",
        ]);
        assert_eq!(out, "before\nafter");
    }

    #[test]
    fn parameter_line_dropped_with_its_indentation() {
        let out = sanitize_stream(&["keep\n", "   <parameter=x>1</parameter>\n", "keep2"]);
        assert_eq!(out, "keep\nkeep2");
    }

    #[test]
    fn bare_tool_tag_line_dropped_but_similar_text_kept() {
        assert_eq!(sanitize_stream(&["a\n<tool>\nb\n"]), "a\nb\n");
        assert_eq!(
            sanitize_stream(&["<tools> are listed\n"]),
            "<tools> are listed\n"
        );
        assert_eq!(sanitize_stream(&["<div> is html"]), "<div> is html");
    }

    #[test]
    fn flush_resolves_unterminated_angle_line() {
        // 行以 '<' 开头且流在行中结束：不能静默丢掉
        let mut sanitizer = ReasoningSanitizer::default();
        assert_eq!(sanitizer.push("<tool"), "");
        assert_eq!(sanitizer.flush(), "<tool");

        let mut sanitizer = ReasoningSanitizer::default();
        assert_eq!(sanitizer.push("<parameter=x>"), "");
        assert_eq!(sanitizer.flush(), "");
    }

    #[test]
    fn cjk_reasoning_passes_through_unchanged() {
        let out = sanitize_stream(&["用户", "想要", "修复", "这个 bug"]);
        assert_eq!(out, "用户想要修复这个 bug");
    }

    // ── tool_result_is_usable ──────────────────────────────────────────────

    fn result(success: bool, output: &str) -> ToolResult {
        ToolResult {
            success,
            output: Some(output.to_string()),
            error: None,
            data: None,
        }
    }

    /// 回归：`codebase_search_attempted` 曾是"调用过"语义，于是冷启动那次
    /// Warming 空结果把闸门永久关上，工具自己给的"稍后重跑"被自家 flag 否掉。
    #[test]
    fn warming_result_is_not_usable() {
        let warming = result(
            true,
            "The cross-file search index for '/repo' is still building in the background \
             (first build; typically finishes in seconds).\nNo hits this turn — this is a cold \
             index, not an empty result. Re-run `CodebaseSearch` with the same keywords once \
             before giving up on it.",
        );
        assert!(
            !tool_result_is_usable(&warming),
            "预热空结果不能让闸门关闭，否则那次重跑永远补不上"
        );
    }

    #[test]
    fn real_hits_are_usable() {
        assert!(tool_result_is_usable(&result(
            true,
            "## src/core/tools/codebase_search.rs (score 8.2)\n..."
        )));
    }

    #[test]
    fn failures_and_empties_are_not_usable() {
        assert!(!tool_result_is_usable(&result(false, "boom")));
        assert!(!tool_result_is_usable(&result(true, "   ")));
    }
}
