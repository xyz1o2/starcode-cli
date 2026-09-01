use crate::types::StarMessage;
use std::collections::HashSet;

/// Estimate total character count of all messages.
pub(crate) fn estimate_messages_chars(messages: &[StarMessage]) -> usize {
    let mut total = 0;
    for m in messages {
        total += m.content.as_deref().map(|s| s.len()).unwrap_or(0);
        if let Some(ref tool_calls) = m.tool_calls {
            for tc in tool_calls {
                total += tc.function.arguments.len();
                total += tc.function.name.len();
            }
        }
    }
    total
}

/// Find the split point in non_system_msgs to preserve `fraction` of total chars.
/// Returns the index in non_system_msgs at which to start keeping messages.
pub(crate) fn find_compress_split_point(
    non_system_msgs: &[StarMessage],
    fraction: f64,
) -> Option<usize> {
    let total = estimate_messages_chars(non_system_msgs);
    let target = (total as f64 * (1.0 - fraction)) as usize;
    let mut accumulated = 0;
    for (i, m) in non_system_msgs.iter().enumerate() {
        accumulated += m.content.as_deref().map(|s| s.len()).unwrap_or(0);
        if let Some(ref tool_calls) = m.tool_calls {
            for tc in tool_calls {
                accumulated += tc.function.arguments.len();
                accumulated += tc.function.name.len();
            }
        }
        if accumulated >= target {
            return Some(i);
        }
    }
    None
}

/// Normalize messages for LLM API - ensure empty messages have placeholders
/// and filter out system markers that might confuse the model
pub(crate) fn normalize_messages_for_llm(messages: &mut Vec<StarMessage>, supports_thinking: bool) {
    let sanitize_empty_messages = empty_message_sanitizer_enabled();
    
    // First pass: remove internal system markers from ALL messages
    // These markers are for debugging/logging only and should not be sent to the LLM
    messages.retain(|m| {
        if let Some(content) = &m.content {
            let trimmed = content.trim();
            // Remove internal markers from any role (user or system)
            if trimmed.starts_with("[COMPACTED]")
                || trimmed.starts_with("[TOOL_SELECTION]")
                || trimmed.starts_with("[TOKEN_BUDGET_WARNING]")
                || trimmed.starts_with("[LOOP_CONTEXT]")
                || trimmed.starts_with("[CONCLUSION_REQUEST]")
                || trimmed.starts_with("[EXPLORATION_LIMIT]")
                || trimmed.starts_with("[PLAN_MODE]")
                || trimmed.starts_with("[REMOVED:")
                || trimmed.starts_with("[TOOL_LOOP_GUARD]")
                || trimmed.starts_with("[REPEATED_READ]")
                || trimmed.starts_with("[AUTO_PLAN]")
                || trimmed.starts_with("[STRUCTURED_TEAM_CONTEXT]")
                || trimmed.starts_with("[END_STRUCTURED_TEAM_CONTEXT]")
            {
                return false; // Remove this message entirely
            }
        }
        true
    });

    // Second pass: strip prompt injection tags and fix empty messages
    for m in messages.iter_mut() {
        // Strip <system-reminder> tags from any message content
        if let Some(content) = &mut m.content {
            if content.contains("<system-reminder>") {
                *content = crate::core::utils::file_utils::strip_prompt_injection_tags(content);
            }
        }

        let has_tool_calls = m
            .tool_calls
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let preserves_empty_assistant_content =
            m.role == "assistant" && (has_tool_calls || m.reasoning_content.is_some());
        let is_empty = m
            .content
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true);

        if supports_thinking
            && m.role == "assistant"
            && has_tool_calls
            && m.reasoning_content.is_none()
        {
            // DeepSeek thinking + tool-call loops require reasoning_content on the
            // assistant tool-call turn; ordinary reasoning-only turns do not.
            m.reasoning_content = Some(String::new());
        }

        if !sanitize_empty_messages {
            continue;
        }

        if is_empty {
            if preserves_empty_assistant_content {
                m.content = Some(String::new());
                continue;
            }
            if has_tool_calls {
                m.content = Some("🔧 Executing tool...".to_string());
            } else if m.role == "assistant" {
                // 助理消息空内容但可能有 reasoning_content，留空让思考块渲染即可。
                // 不再显示 "💭 Thinking..." 避免歧义。
                m.content = Some(String::new());
            } else {
                // Claude Code 风格：使用随机动词而非固定的 "Processing..."
                let verb = crate::ui::components::status_line::random_spinner_verb();
                let placeholder = match m.role.as_str() {
                    "tool" => format!("⏳ {}...", verb),
                    _ => format!("{}...", verb),
                };
                m.content = Some(placeholder);
            }
        } else if let Some(content) = m.content.as_mut() {
            let replacement = {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    Some("[user interrupted]".to_string())
                } else if trimmed.len() != content.len() {
                    Some(trimmed.to_string())
                } else {
                    None
                }
            };

            if let Some(next) = replacement {
                *content = next;
            }
        }
    }
}

/// Repair tool message sequence - ensure all tool calls have corresponding tool results
pub(crate) fn repair_tool_message_sequence(messages: &mut Vec<StarMessage>) {
    if !tool_sequence_repair_needed(messages) {
        return;
    }

    fn flush_missing_tool_results(
        out: &mut Vec<StarMessage>,
        pending_ids: &[String],
        responded: &HashSet<String>,
    ) {
        if pending_ids.is_empty() {
            return;
        }
        crate::utils::logging::append_debug_log_line(&format!(
            "🔧 [工具序列管理] 检测到 {} 个未响应的工具",
            pending_ids.len()
        ));
        for id in pending_ids {
            if responded.contains(id) {
                crate::utils::logging::append_debug_log_line(&format!(
                    "  ✓ 工具 {} 已有响应，继续处理",
                    id
                ));
                continue;
            }
            crate::utils::logging::append_debug_log_line(&format!(
                "  ⚠️  Tool {} missing response, adding synthetic tool message",
                id
            ));
            out.push(StarMessage::tool(id.clone(), "[System] Tool call did not return a result. It may have been interrupted, timed out, or encountered an error. Please retry or check tool parameters."));
        }
    }

    let mut out: Vec<StarMessage> = Vec::with_capacity(messages.len());
    let mut pending_tool_call_ids: Vec<String> = Vec::new();
    let mut pending_tool_call_id_set: HashSet<String> = HashSet::new();
    let mut responded: HashSet<String> = HashSet::new();

    for msg in messages.drain(..) {
        // 进入下一段（非 tool），说明上一轮 tool 响应段结束：补齐缺失 tool
        if msg.role != "tool" {
            flush_missing_tool_results(&mut out, &pending_tool_call_ids, &responded);
            pending_tool_call_ids.clear();
            pending_tool_call_id_set.clear();
            responded.clear();
        }

        if msg.role == "assistant" {
            let has_tool_calls = msg
                .tool_calls
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false);

            out.push(msg);

            if has_tool_calls {
                if let Some(last) = out.last() {
                    let ids: Vec<String> = last
                        .tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().map(|tc| tc.id.clone()).collect())
                        .unwrap_or_default();
                    pending_tool_call_ids = ids.clone();
                    pending_tool_call_id_set = ids.into_iter().collect();
                    responded.clear();
                }
            }

            continue;
        }

        if msg.role == "tool" {
            let tcid = msg.tool_call_id.clone().unwrap_or_default();
            if tcid.is_empty() {
                // 悬空 tool：缺 tool_call_id，直接丢弃（否则必 400）
                continue;
            }
            if pending_tool_call_id_set.contains(&tcid) {
                responded.insert(tcid);
                out.push(msg);
            } else {
                // 悬空 tool：不属于最近一次 assistant(tool_calls)，丢弃（否则必 400）
                continue;
            }
            continue;
        }

        out.push(msg);
    }

    // 结束时仍有未补齐的 tool_call_id：补齐
    flush_missing_tool_results(&mut out, &pending_tool_call_ids, &responded);
    *messages = out;
}

pub(crate) fn tool_sequence_repair_needed(messages: &[StarMessage]) -> bool {
    messages.iter().any(|message| {
        message.role == "tool"
            || message
                .tool_calls
                .as_ref()
                .map(|tool_calls| !tool_calls.is_empty())
                .unwrap_or(false)
    })
}

fn empty_message_sanitizer_enabled() -> bool {
    std::env::var("STAR_ENABLE_EMPTY_MESSAGE_SANITIZER")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true)
}
 