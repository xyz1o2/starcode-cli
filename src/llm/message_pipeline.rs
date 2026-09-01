//! Per-provider message preprocessing pipeline.
//!
//! Each provider / API has different requirements for the message format
//! (e.g. DeepSeek needs `reasoning_content`, OpenAI rejects empty-string
//! content on tool-call messages).  Instead of scattering provider-specific
//! checks across `StarClient`, each preprocessing step lives in this module
//! and the pipeline is assembled once based on the detected provider type.

use crate::llm::{
    PROXY_TOOL_ID_PREFIX, SEGMENT_SUMMARY_ASSISTANT_MAX_CHARS, SEGMENT_SUMMARY_TOOL_MAX_CHARS,
    SUMMARY_TRUNCATE_MAX_CHARS,
};
use crate::types::StarMessage;
use std::collections::HashSet;

// ── Pipeline ───────────────────────────────────────────────────────

/// Ordered list of preprocessing steps applied to the message list before
/// it is sent to the LLM API.
#[derive(Clone)]
pub struct MessagePipeline {
    steps: &'static [PipelineStep],
}

type PipelineStep = fn(&mut Vec<StarMessage>);

impl MessagePipeline {
    /// Pipeline for generic / OpenAI-compatible providers.
    /// Includes ensure_deepseek_reasoning because many OpenAI-compatible
    /// endpoints proxy to DeepSeek, and the placeholder is harmless for
    /// other providers.
    pub const STANDARD: Self = Self {
        steps: &[
            normalize_tool_ids,
            sanitize_messages,
            ensure_deepseek_reasoning,
            remove_orphaned_tool_messages,
            normalize_tool_ids,
        ],
    };

    /// Pipeline for DeepSeek (including proxies that route to DeepSeek).
    /// Extends STANDARD with context preparation.
    pub const DEEPSEEK: Self = Self {
        steps: &[
            normalize_tool_ids,
            prepare_deepseek_context,
            sanitize_messages,
            ensure_deepseek_reasoning,
            remove_orphaned_tool_messages,
            normalize_tool_ids,
        ],
    };

    pub fn run(&self, messages: &mut Vec<StarMessage>) {
        for (i, step) in self.steps.iter().enumerate() {
            step(messages);
            crate::utils::logging::append_debug_log_line(&format!(
                "[PIPELINE] step {}/{} done, {} messages",
                i + 1,
                self.steps.len(),
                messages.len()
            ));
        }
    }
}

// ── Step: sanitize messages (all providers) ────────────────────────

fn message_has_tool_calls(message: &StarMessage) -> bool {
    message
        .tool_calls
        .as_ref()
        .map(|tc| !tc.is_empty())
        .unwrap_or(false)
}

/// Normalize proxy-specific tool-call ID prefixes so IDs are consistent
/// between assistant tool_calls and tool result messages.
/// Only strips `call_function_` which is a known proxy artifact.
/// `toolu_` / `toolu_bdrk_` are NATIVE Anthropic-format IDs — do NOT strip.
fn normalize_tool_ids(messages: &mut Vec<StarMessage>) {
    for m in messages.iter_mut() {
        if let Some(tcs) = &mut m.tool_calls {
            for tc in tcs {
                if let Some(s) = tc.id.strip_prefix(PROXY_TOOL_ID_PREFIX) {
                    tc.id = s.to_string();
                }
            }
        }
        if let Some(ref id) = m.tool_call_id {
            if let Some(s) = id.strip_prefix(PROXY_TOOL_ID_PREFIX) {
                m.tool_call_id = Some(s.to_string());
            }
        }
    }
}

pub(crate) fn sanitize_messages(messages: &mut Vec<StarMessage>) {
    for m in messages.iter_mut() {
        let has_tool_calls = message_has_tool_calls(m);
        let preserves_empty_assistant =
            m.role == "assistant" && (has_tool_calls || m.reasoning_content.is_some());
        let is_empty = m
            .content
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true);

        if is_empty {
            if preserves_empty_assistant {
                if has_tool_calls {
                    m.content = None;
                } else {
                    m.content = Some(String::new());
                }
                continue;
            }

            let placeholder = match m.role.as_str() {
                "assistant" => "...",
                "tool" => "[tool output]",
                "system" => "[system]",
                "user" => "[user message]",
                _ => "...",
            };
            m.content = Some(placeholder.to_string());
        } else if let Some(content) = m.content.as_mut() {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                *content = "...".to_string();
            } else if trimmed.len() != content.len() {
                *content = trimmed.to_string();
            }
        }
    }
}

// ── Step: repair + remove orphaned tool results (all providers) ────

/// MiniMax API (and some proxies) have a known bug where tool-call IDs
/// get mutated between the assistant response and the tool result: a
/// suffix like `_1`, `_2`, or `w_1` is appended. This causes the
/// "tool id not found (2013)" error on subsequent requests.
///
/// Strategy: first try exact ID matching; fall back to fuzzy repair
/// for MiniMax-style ID mutations; remove anything still orphaned.
pub(crate) fn remove_orphaned_tool_messages(messages: &mut Vec<StarMessage>) {
    // Collect all known tool-call IDs from assistant messages
    let mut known_ids: HashSet<String> = HashSet::new();
    for m in messages.iter() {
        if m.role == "assistant" {
            if let Some(tcs) = &m.tool_calls {
                for tc in tcs {
                    known_ids.insert(tc.id.clone());
                }
            }
        }
    }

    // Pass 1: try to repair mutated tool_call_ids
    for m in messages.iter_mut() {
        if m.role != "tool" {
            continue;
        }
        let id = m.tool_call_id.as_deref().unwrap_or("");
        if id.is_empty() || known_ids.contains(id) {
            continue;
        }

        // MiniMax appends `_1`, `_2`, `w_N`, `_w_N` to the original ID.
        // Try progressively shorter suffixes.
        if let Some(repaired) = repair_minimax_id(id, &known_ids) {
            m.tool_call_id = Some(repaired);
        }
    }

    // Pass 2: remove anything still orphaned
    messages.retain(|m| {
        if m.role != "tool" {
            return true;
        }
        let id = m.tool_call_id.as_deref().unwrap_or("");
        !id.is_empty() && known_ids.contains(id)
    });
}

fn repair_minimax_id(mutated: &str, known: &HashSet<String>) -> Option<String> {
    // Exact match
    if known.contains(mutated) {
        return Some(mutated.to_string());
    }

    // MiniMax prefix mutations: `call_function_` ↔ `toolu_` ↔ bare ID
    let prefixes: &[&str] = &["call_function_", "toolu_", "toolu_bdrk_"];
    for prefix in prefixes {
        if let Some(base) = mutated.strip_prefix(prefix) {
            // Try the bare ID
            if known.contains(base) {
                return Some(base.to_string());
            }
            // Try with other prefixes
            for alt in prefixes {
                if alt != prefix {
                    let alt_id = format!("{}{}", alt, base);
                    if known.contains(&alt_id) {
                        return Some(alt_id);
                    }
                }
            }
        }
    }
    // Try adding each prefix to the whole ID
    for prefix in prefixes {
        let prefixed = format!("{}{}", prefix, mutated);
        if known.contains(&prefixed) {
            return Some(prefixed);
        }
    }

    // MiniMax suffix mutations: `_1`, `_2`, `_w_1`, `w_N`
    for suffix in &["_1", "_2", "_3", "_4", "_5", "w_1", "w_2", "_w_1", "_w_2"] {
        if let Some(base) = mutated.strip_suffix(suffix) {
            if !base.is_empty() {
                if known.contains(base) {
                    return Some(base.to_string());
                }
                // Also try base with alternate prefixes
                for prefix in prefixes {
                    if let Some(bare) = base.strip_prefix(prefix) {
                        if known.contains(bare) {
                            return Some(bare.to_string());
                        }
                    }
                }
            }
        }
    }

    // Generic: strip any trailing `_digit+`
    if let Some(pos) = mutated.rfind('_') {
        let suffix = &mutated[pos..];
        if suffix.len() >= 2 && suffix[1..].chars().all(|c| c.is_ascii_digit()) {
            let base = &mutated[..pos];
            if !base.is_empty() && known.contains(base) {
                return Some(base.to_string());
            }
        }
    }

    None
}

// ── Step: prepare DeepSeek reasoning context ───────────────────────

/// DeepSeek thinking mode requires `reasoning_content` on every assistant
/// message that carries `tool_calls`.  When history has been compressed or
/// restored from a session, that field may be missing.  This step rewrites
/// invalid segments so the request isn't rejected with error 2013.
pub(crate) fn prepare_deepseek_context(messages: &mut Vec<StarMessage>) {
    // Helper: does a segment have at least one tool-call?
    let segment_has_tools = |seg: &[StarMessage]| seg.iter().any(message_has_tool_calls);

    // Helper: is reasoning *present* (not necessarily non-empty)?
    let has_reasoning = |m: &StarMessage| m.reasoning_content.is_some();

    // ── Handle orphaned tool messages before the first user message ──
    let first_user = (0..messages.len()).find(|&i| messages[i].role == "user");
    let pre_user_end = first_user.unwrap_or(messages.len());

    let mut rewritten: Vec<StarMessage> = Vec::with_capacity(messages.len());
    let mut idx = 0usize;

    if pre_user_end > 0 {
        let known_ids: HashSet<&str> = messages[..pre_user_end]
            .iter()
            .filter(|m| m.role == "assistant")
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|tc| tc.id.as_str())
            .collect();

        let has_orphan = messages[..pre_user_end].iter().any(|m| {
            m.role == "tool"
                && !m
                    .tool_call_id
                    .as_deref()
                    .map(|id| known_ids.contains(id))
                    .unwrap_or(false)
        });

        if has_orphan {
            for msg in &messages[..pre_user_end] {
                if msg.role == "tool"
                    && !msg
                        .tool_call_id
                        .as_deref()
                        .map(|id| known_ids.contains(id))
                        .unwrap_or(false)
                {
                    if let Some(summary) = summarize_tool_message(msg) {
                        rewritten.push(summary);
                    }
                } else {
                    rewritten.push(msg.clone());
                }
            }
        } else {
            rewritten.extend_from_slice(&messages[..pre_user_end]);
        }
        idx = pre_user_end;
    }

    // ── Process user-message segments ──
    while idx < messages.len() {
        rewritten.push(messages[idx].clone()); // user message
        idx += 1;

        let seg_start = idx;
        while idx < messages.len() && messages[idx].role != "user" {
            idx += 1;
        }
        let segment = &messages[seg_start..idx];

        if !segment_has_tools(segment) {
            for msg in segment {
                if let Some(s) = summarize_non_tool_message(msg) {
                    rewritten.push(s);
                }
            }
        } else {
            let missing_reasoning = segment
                .iter()
                .any(|m| m.role == "assistant" && !has_reasoning(m));

            if missing_reasoning {
                // Try to keep just the final answer
                if let Some(final_answer) = segment.iter().rev().find(|m| {
                    m.role == "assistant"
                        && !message_has_tool_calls(m)
                        && m.content
                            .as_deref()
                            .map(|c| !c.trim().is_empty())
                            .unwrap_or(false)
                }) {
                    if let Some(s) = summarize_non_tool_message(final_answer) {
                        rewritten.push(s);
                    }
                } else if let Some(s) = summarize_invalid_segment(segment) {
                    rewritten.push(s);
                }
            } else {
                rewritten.extend(segment.iter().cloned());
            }
        }
    }

    *messages = rewritten;
}

// ── Step: ensure DeepSeek reasoning_content ────────────────────────

const DEEPSEEK_REASONING_PLACEHOLDER: &str = " ";

/// DeepSeek V3.2+ requires `reasoning_content` to be present (not None)
/// on every assistant message that contains `tool_calls`.
pub(crate) fn ensure_deepseek_reasoning(messages: &mut Vec<StarMessage>) {
    let mut injected = 0usize;
    for m in messages.iter_mut() {
        if m.role == "assistant" && message_has_tool_calls(m) {
            match &m.reasoning_content {
                None => {
                    m.reasoning_content = Some(DEEPSEEK_REASONING_PLACEHOLDER.to_string());
                    injected += 1;
                }
                Some(s) if s.trim().is_empty() => {
                    m.reasoning_content = Some(DEEPSEEK_REASONING_PLACEHOLDER.to_string());
                    injected += 1;
                }
                _ => {}
            }
        }
    }
    if injected > 0 {
        crate::utils::logging::append_debug_log_line(&format!(
            "[PIPELINE] ensure_deepseek_reasoning: injected reasoning_content into {} assistant tool-call messages",
            injected
        ));
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn truncate_str(input: &str, max_chars: usize) -> String {
    let compact: String = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out: String = compact.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

fn summarize_tool_message(msg: &StarMessage) -> Option<StarMessage> {
    let content = msg.content.as_deref().filter(|c| !c.trim().is_empty())?;
    Some(StarMessage::system(format!(
        "Previous tool result summary: {}",
        truncate_str(content, SUMMARY_TRUNCATE_MAX_CHARS)
    )))
}

fn summarize_non_tool_message(msg: &StarMessage) -> Option<StarMessage> {
    match msg.role.as_str() {
        "assistant" => msg
            .content
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .map(|c| {
                StarMessage::system(format!(
                    "Previous assistant response summary: {}",
                    truncate_str(c, SUMMARY_TRUNCATE_MAX_CHARS)
                ))
            }),
        "tool" => msg
            .content
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .map(|c| {
                StarMessage::system(format!(
                    "Previous tool result summary: {}",
                    truncate_str(c, SUMMARY_TRUNCATE_MAX_CHARS)
                ))
            }),
        _ => Some(msg.clone()),
    }
}

fn summarize_invalid_segment(segment: &[StarMessage]) -> Option<StarMessage> {
    if segment.is_empty() {
        return None;
    }
    let mut lines = vec![
        "Previous tool interaction was summarized because DeepSeek thinking mode requires reasoning_content on tool-call turns.".to_string(),
    ];
    for msg in segment {
        match msg.role.as_str() {
            "assistant" if message_has_tool_calls(msg) => {
                let names: Vec<&str> = msg
                    .tool_calls
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .map(|tc| tc.function.name.as_str())
                    .filter(|n| !n.trim().is_empty())
                    .collect();
                if !names.is_empty() {
                    lines.push(format!("- tool_calls: {}", names.join(", ")));
                }
            }
            "assistant" => {
                if let Some(c) = msg.content.as_deref().filter(|c| !c.trim().is_empty()) {
                    lines.push(format!(
                        "- assistant: {}",
                        truncate_str(c, SEGMENT_SUMMARY_ASSISTANT_MAX_CHARS)
                    ));
                }
            }
            "tool" => {
                if let Some(c) = msg.content.as_deref().filter(|c| !c.trim().is_empty()) {
                    let label = msg
                        .tool_call_id
                        .as_deref()
                        .filter(|id| !id.trim().is_empty())
                        .map(|id| format!("tool_result `{}`", id))
                        .unwrap_or_else(|| "tool_result".to_string());
                    lines.push(format!(
                        "- {}: {}",
                        label,
                        truncate_str(c, SEGMENT_SUMMARY_TOOL_MAX_CHARS)
                    ));
                }
            }
            _ => {}
        }
    }
    if lines.len() == 1 {
        lines.push("- details: unavailable".to_string());
    }
    Some(StarMessage::system(lines.join("\n")))
}

// ── Provider detection ─────────────────────────────────────────────

/// Select the pipeline for a given provider.
pub fn pipeline_for(
    provider_env_id: Option<&str>,
    model: &str,
    thinking_detected: bool,
) -> MessagePipeline {
    if provider_env_id == Some(crate::llm::PROVIDER_ENV_ID_DEEPSEEK) {
        return MessagePipeline::DEEPSEEK;
    }
    if model.to_ascii_lowercase().contains("deepseek") {
        return MessagePipeline::DEEPSEEK;
    }
    if thinking_detected {
        return MessagePipeline::DEEPSEEK;
    }
    MessagePipeline::STANDARD
}
