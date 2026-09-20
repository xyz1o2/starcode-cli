use crate::types::StarMessage;
use std::path::Path;

pub(crate) fn truncate_chars_for_injection(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

pub(crate) fn auto_compact_enabled() -> bool {
    std::env::var("STAR_ENABLE_AUTO_COMPACT")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true)
}

pub(crate) fn auto_compact_log_enabled() -> bool {
    std::env::var("STAR_ENABLE_AUTO_COMPACT_LOG")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true)
}

pub(crate) fn append_auto_compact_log(
    summary: &str,
    removed_count: usize,
    before_chars: usize,
    after_chars: usize,
) {
    if !auto_compact_log_enabled() {
        return;
    }
    let path = std::env::var("STAR_AUTO_COMPACT_LOG_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| ".star/auto_compact.jsonl".to_string());

    let p = std::path::PathBuf::from(&path);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let line = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "type": "auto_compact",
        "summary": summary,
        "removed_count": removed_count,
        "before_chars": before_chars,
        "after_chars": after_chars,
    });

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        use std::io::Write;
        let _ = writeln!(f, "{}", line);
    }
}

pub(crate) fn build_compaction_summary(removed: &[StarMessage]) -> Option<String> {
    if removed.is_empty() {
        return None;
    }

    let max_msgs = std::env::var("STAR_AUTO_COMPACT_MAX_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80);
    let max_chars = std::env::var("STAR_AUTO_COMPACT_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8000);

    // 提取关键信息
    let mut key_files: Vec<String> = Vec::new();
    let mut key_decisions: Vec<String> = Vec::new();
    let mut tool_calls_summary: Vec<String> = Vec::new();

    for m in removed.iter() {
        if let Some(c) = m.content.as_deref() {
            // 提取文件路径（src/、path/、./等开头的路径）
            for line in c.lines() {
                let trimmed = line.trim();
                if (trimmed.contains("src/")
                    || trimmed.contains("path/")
                    || trimmed.starts_with("./"))
                    && (trimmed.ends_with(".rs")
                        || trimmed.ends_with(".ts")
                        || trimmed.ends_with(".js")
                        || trimmed.ends_with(".py")
                        || trimmed.ends_with(".md")
                        || trimmed.ends_with(".toml"))
                {
                    let file_path = trimmed.split_whitespace().next().unwrap_or(trimmed);
                    if !key_files.contains(&file_path.to_string()) && key_files.len() < 20 {
                        key_files.push(file_path.to_string());
                    }
                }
                // 提取关键决策
                if (trimmed.contains("决定")
                    || trimmed.contains("选择")
                    || trimmed.contains("方案")
                    || trimmed.contains("问题")
                    || trimmed.contains("修复")
                    || trimmed.contains("修改"))
                    && trimmed.len() > 10
                    && trimmed.len() < 200
                {
                    if !key_decisions.contains(&trimmed.to_string()) && key_decisions.len() < 10 {
                        key_decisions.push(trimmed.to_string());
                    }
                }
            }
        }
        // 统计工具调用
        if m.role.as_str() == "assistant" {
            if let Some(tcs) = &m.tool_calls {
                for tc in tcs {
                    let tool_name = &tc.function.name;
                    if !tool_calls_summary.contains(tool_name) {
                        tool_calls_summary.push(tool_name.clone());
                    }
                }
            }
        }
    }

    let mut lines: Vec<String> = Vec::new();

    // 添加关键信息摘要
    if !key_files.is_empty() {
        lines.push("## 涉及文件".to_string());
        for f in key_files.iter().take(15) {
            lines.push(format!("- {}", f));
        }
        lines.push("".to_string());
    }

    if !key_decisions.is_empty() {
        lines.push("## 关键决策/操作".to_string());
        for d in key_decisions.iter().take(8) {
            lines.push(format!("- {}", d));
        }
        lines.push("".to_string());
    }

    if !tool_calls_summary.is_empty() {
        lines.push(format!("## 使用工具: {}", tool_calls_summary.join(", ")));
        lines.push("".to_string());
    }

    // 添加消息摘要
    lines.push("## 对话摘要".to_string());
    for m in removed.iter().take(max_msgs) {
        let role = m.role.as_str();
        let mut body = String::new();
        if let Some(c) = m.content.as_deref() {
            // 取前3行或前300字符
            let content_lines: Vec<&str> = c.lines().take(3).collect();
            let content = content_lines.join(" ");
            if !content.trim().is_empty() {
                body = truncate_chars_for_injection(content.trim(), 300);
            }
        }
        if body.is_empty() {
            if m.tool_calls
                .as_ref()
                .map(|v| !v.is_empty())
                .unwrap_or(false)
            {
                let tool_names: Vec<&str> = m
                    .tool_calls
                    .as_ref()
                    .map(|tcs| tcs.iter().map(|tc| tc.function.name.as_str()).collect())
                    .unwrap_or_default();
                body = format!("[tool_calls: {}]", tool_names.join(", "));
            } else {
                continue; // 跳过空消息
            }
        }
        lines.push(format!("{}: {}", role, body));
    }

    let merged = lines.join("\n");
    let clipped = truncate_chars_for_injection(&merged, max_chars);
    if clipped.trim().is_empty() {
        return None;
    }
    Some(clipped)
}

/// Plan mode 只读护栏的提醒文本。
///
/// 返回 `Some` 时由调用方包进当轮 user 消息的 `<system-reminder>` 里
/// （见 `agent_run` 的 turn_extras 组装），**不**另起 system 消息：system
/// 数组整体是缓存前缀，且 system 消息曾以 `[PLAN_MODE]` 开头，会被
/// `normalize_messages_for_llm` 连体整条删掉，护栏从未真正送达模型。
/// 对标 Claude Code 的 system-reminder-plan-mode-is-active.md。
pub(crate) fn plan_mode_reminder_if_needed(
    approval_mode: &crate::types::ApprovalMode,
) -> Option<String> {
    if matches!(approval_mode, crate::types::ApprovalMode::Plan) {
        Some(
            "Plan mode is active. You MUST NOT make any edits, run any non-readonly tools, or otherwise make changes. You may only read/search. You may use Todo to organize the task list since it does not change code. When ready, present a concise plan as a Markdown list (use - or numbered items; nest subtasks). This list will be used to populate the task panel. Then call exit_plan_mode with {plan: ...} to ask the user to exit plan mode and start coding."
                .to_string(),
        )
    } else {
        None
    }
}

pub(crate) fn trim_context_if_needed(messages: &mut Vec<StarMessage>) -> Option<(String, bool)> {
    // ============ 智能化改进 6: 上下文智能管理 ============
    let enabled = std::env::var("STAR_ENABLE_CONTEXT_MONITOR")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true);
    if !enabled {
        return None;
    }

    let max_chars = std::env::var("STAR_CONTEXT_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(180_000);

    let keep_last = std::env::var("STAR_CONTEXT_KEEP_LAST")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80);

    let preserve_fraction = std::env::var("STAR_CONTEXT_PRESERVE_FRACTION")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0 && *v < 1.0);

    let before_chars = crate::agent::message_processing::estimate_messages_chars(messages);
    if before_chars <= max_chars {
        return None;
    }

    // 计算非 system 消息的索引
    let mut non_system_indices: Vec<usize> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if m.role != "system" {
            non_system_indices.push(i);
        }
    }

    let keep_set: std::collections::HashSet<usize> = if let Some(frac) = preserve_fraction {
        let non_system_msgs: Vec<StarMessage> = non_system_indices
            .iter()
            .filter_map(|&i| messages.get(i).cloned())
            .collect();
        let split_in_non_system =
            crate::agent::message_processing::find_compress_split_point(&non_system_msgs, frac)
                .unwrap_or(0);
        non_system_indices
            .iter()
            .enumerate()
            .filter_map(|(pos, &idx)| {
                if pos >= split_in_non_system {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    } else {
        if non_system_indices.len() <= keep_last {
            return Some((
                format!(
                    "⚠️ [Context Warning] Estimated context too large (~{} chars) but insufficient room to trim. Consider increasing STAR_CONTEXT_MAX_CHARS or reducing keep_last.",
                    before_chars
                ),
                false,
            ));
        }
        non_system_indices
            .iter()
            .rev()
            .take(keep_last)
            .copied()
            .collect()
    };

    // ============ 工具调用完整性管理 ============
    let mut expanded_keep_set = keep_set.clone();
    let mut changed = true;

    while changed {
        changed = false;
        let before_size = expanded_keep_set.len();

        // 扩展规则 1：如果保留了 assistant(tool_calls)，必须保留所有对应的 tool 消息
        for i in 0..messages.len() {
            if !expanded_keep_set.contains(&i) {
                continue;
            }

            let msg = &messages[i];
            if msg.role == "assistant"
                && msg
                    .tool_calls
                    .as_ref()
                    .map(|tc| !tc.is_empty())
                    .unwrap_or(false)
            {
                let tool_call_ids: std::collections::HashSet<String> = msg
                    .tool_calls
                    .as_ref()
                    .map(|tcs| tcs.iter().map(|tc| tc.id.clone()).collect())
                    .unwrap_or_default();

                for j in (i + 1)..messages.len() {
                    let next_msg = &messages[j];
                    if next_msg.role == "tool" {
                        if let Some(tcid) = &next_msg.tool_call_id {
                            if tool_call_ids.contains(tcid) {
                                expanded_keep_set.insert(j);
                            }
                        }
                    } else if next_msg.role == "assistant" {
                        break;
                    }
                }
            }
        }

        // 扩展规则 2：如果保留了 tool 消息，必须保留对应的 assistant(tool_calls)
        for i in 0..messages.len() {
            if !expanded_keep_set.contains(&i) {
                continue;
            }

            let msg = &messages[i];
            if msg.role == "tool" {
                if let Some(tcid) = &msg.tool_call_id {
                    for j in (0..i).rev() {
                        let prev_msg = &messages[j];
                        if prev_msg.role == "assistant" {
                            if let Some(tcs) = &prev_msg.tool_calls {
                                if tcs.iter().any(|tc| &tc.id == tcid) {
                                    expanded_keep_set.insert(j);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        if expanded_keep_set.len() > before_size {
            changed = true;
        }
    }

    let keep_set = expanded_keep_set;
    let before_len = messages.len();

    // 收集需要压缩的消息
    let mut removed: Vec<StarMessage> = Vec::new();
    if auto_compact_enabled() {
        for (i, m) in messages.iter().enumerate() {
            if m.role == "system" {
                continue;
            }
            if keep_set.contains(&i) {
                continue;
            }
            removed.push(m.clone());
        }
    }

    // 生成智能摘要
    let compact_summary = if auto_compact_enabled() {
        build_compaction_summary(&removed)
    } else {
        None
    };

    let compact_summary_for_log = compact_summary.clone();

    // 重构消息列表
    let mut new_messages: Vec<StarMessage> = Vec::with_capacity(messages.len());

    // 1. 先保留所有 system 消息
    for m in messages.iter() {
        if m.role == "system" {
            new_messages.push(m.clone());
        }
    }

    // 2. 插入压缩摘要
    if let Some(summary) = compact_summary {
        new_messages.push(StarMessage::system(format!(
            "📦 [History Summary] (Compressed {} old messages, keeping key information)\n{}",
            removed.len(),
            summary
        )));
    }

    // 3. 再保留最近 keep_last 的非 system 消息
    for (i, m) in messages.iter().enumerate() {
        if m.role == "system" {
            continue;
        }
        if keep_set.contains(&i) {
            new_messages.push(m.clone());
        }
    }

    let after_chars = crate::agent::message_processing::estimate_messages_chars(&new_messages);
    if after_chars >= before_chars {
        return Some((
            format!(
                "⚠️ [Context Compression Skipped] Compressed size not smaller: {} → {} chars, keeping original context.",
                before_chars, after_chars
            ),
            false,
        ));
    }

    *messages = new_messages;

    let removed_count = before_len.saturating_sub(messages.len());

    // 记录压缩日志
    if let Some(summary) = compact_summary_for_log.as_deref() {
        if !summary.trim().is_empty() {
            append_auto_compact_log(summary, removed_count, before_chars, after_chars);
        }
    }

    Some((
        format!(
            "✅ [上下文已压缩] 压缩 {} 条旧消息为摘要 | 估算大小: {} → {} 字符 (减少 {:.1}%) | 保留最近 {} 条",
            removed_count,
            before_chars,
            after_chars,
            (before_chars.saturating_sub(after_chars) as f64 / before_chars as f64 * 100.0),
            keep_last
        ),
        true,
    ))
}
