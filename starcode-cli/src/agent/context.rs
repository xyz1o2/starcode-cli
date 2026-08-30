use crate::agent::validator::Validator;
use crate::types::{StarMessage, StarToolCall, ToolResult};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};

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
                if (trimmed.contains("src/") || trimmed.contains("path/") || trimmed.starts_with("./"))
                    && (trimmed.ends_with(".rs") || trimmed.ends_with(".ts") || trimmed.ends_with(".js")
                        || trimmed.ends_with(".py") || trimmed.ends_with(".md") || trimmed.ends_with(".toml"))
                {
                    let file_path = trimmed.split_whitespace().next().unwrap_or(trimmed);
                    if !key_files.contains(&file_path.to_string()) && key_files.len() < 20 {
                        key_files.push(file_path.to_string());
                    }
                }
                // 提取关键决策
                if (trimmed.contains("决定") || trimmed.contains("选择") || trimmed.contains("方案")
                    || trimmed.contains("问题") || trimmed.contains("修复") || trimmed.contains("修改"))
                    && trimmed.len() > 10 && trimmed.len() < 200
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
                let tool_names: Vec<&str> = m.tool_calls.as_ref()
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

pub(crate) fn inject_plan_mode_reminder_if_needed(
    approval_mode: &crate::types::ApprovalMode,
    outbound_messages: &mut Vec<StarMessage>,
) {
    if matches!(approval_mode, crate::types::ApprovalMode::Plan) {
        outbound_messages.push(StarMessage::system(
            "[PLAN_MODE] Plan mode is active. You MUST NOT make any edits, run any non-readonly tools, or otherwise make changes. You may only read/search. You may use Todo to organize the task list since it does not change code. When ready, present a concise plan as a Markdown list (use - or numbered items; nest subtasks). This list will be used to populate the task panel. Then call exit_plan_mode with {plan: ...} to ask the user to exit plan mode and start coding."
        ));
    }
}

pub(crate) fn inject_file_security_warning_if_needed(
    tool_call: &StarToolCall,
    tool_result: &ToolResult,
) -> Option<StarMessage> {
    // Security warning is now in system-prompt-security-policy.md (one-time injection)
    // No need to inject it every time a file is read
    let _ = (tool_call, tool_result);
    None
}

pub(crate) fn inject_directory_context_if_needed(
    tool_call: &StarToolCall,
    tool_result: &ToolResult,
    injected_dir_context_hashes: &Arc<Mutex<HashSet<u64>>>,
) -> Option<StarMessage> {
    if !tool_result.success {
        return None;
    }

    if tool_call.function.name != "view_file" {
        return None;
    }

    let enabled = std::env::var("STAR_ENABLE_DIR_CONTEXT")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true);
    if !enabled {
        return None;
    }

    let max_chars = std::env::var("STAR_DIR_CTX_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(6000);
    let file_list = std::env::var("STAR_DIR_CTX_FILES")
        .ok()
        .unwrap_or_else(|| "README.md,AGENTS.md".to_string());
    let file_names: Vec<String> = file_list
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if file_names.is_empty() {
        return None;
    }

    let args = match Validator::parse_args(&tool_call.function.arguments) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let path = match Validator::require_str(&args, "path") {
        Ok(v) => v,
        Err(_) => return None,
    };
    let resolved = match Path::new(&path).canonicalize() {
        Ok(p) => p,
        Err(_) => return None,
    };
    let mut dir = if resolved.is_dir() {
        resolved
    } else {
        match resolved.parent() {
            Some(p) => p.to_path_buf(),
            None => return None,
        }
    };

    let root = crate::core::utils::paths::current_dir_cached();

    let mut inject_blocks: Vec<String> = Vec::new();

    loop {
        if !dir.starts_with(&root) {
            break;
        }

        for name in &file_names {
            let p = dir.join(name);
            if !p.is_file() {
                continue;
            }
            let content = match std::fs::read_to_string(&p) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let trimmed = content.trim();
            if trimmed.is_empty() {
                continue;
            }
            let clipped = truncate_chars_for_injection(trimmed, max_chars);

            let mut hasher = DefaultHasher::new();
            p.to_string_lossy().hash(&mut hasher);
            clipped.hash(&mut hasher);
            let h = hasher.finish();

            let already = {
                let cache = injected_dir_context_hashes.lock().unwrap();
                cache.contains(&h)
            };
            if already {
                continue;
            }
            {
                let mut cache = injected_dir_context_hashes.lock().unwrap();
                cache.insert(h);
            }

            inject_blocks.push(format!(
                "[Directory Context: {}]\n{}",
                p.to_string_lossy(),
                clipped
            ));
        }

        if dir == *root {
            break;
        }
        let Some(parent) = dir.parent() else {
            break;
        };
        dir = parent.to_path_buf();
    }

    if inject_blocks.is_empty() {
        return None;
    }

    let merged = inject_blocks.join("\n\n");
    Some(StarMessage::system(format!(
        "目录上下文（自动注入，仅供参考）：\n{}",
        merged
    )))
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

pub(crate) fn find_rules_file(start_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cur: Option<&std::path::Path> = Some(start_dir);
    while let Some(p) = cur {
        let cand1 = p.join(".star").join("rules.txt");
        if cand1.is_file() {
            return Some(cand1);
        }
        let cand2 = p.join(".cursorrules");
        if cand2.is_file() {
            return Some(cand2);
        }
        cur = p.parent();
    }
    None
}

pub(crate) fn inject_project_rules_if_needed(
    messages: &mut Vec<StarMessage>,
    injected_rules_hash: &Arc<Mutex<Option<u64>>>,
) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let enabled = std::env::var("STAR_ENABLE_RULES")
        .ok()
        .map(|v| {
            let v = v.to_lowercase();
            !(v == "0" || v == "false" || v == "off")
        })
        .unwrap_or(true);
    if !enabled {
        return;
    }

    let cwd = crate::core::utils::paths::current_dir_cached();

    let Some(path) = find_rules_file(cwd) else {
        return;
    };

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let content = content.trim();
    if content.is_empty() {
        return;
    }

    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    content.hash(&mut hasher);
    let hash = hasher.finish();

    {
        let mut last = injected_rules_hash.lock().unwrap();
        if last.as_ref() == Some(&hash) {
            return;
        }
        *last = Some(hash);
    }

    messages.push(StarMessage::system(format!(
        "项目规则（请严格遵守；来源：{}）：\n{}",
        path.to_string_lossy(),
        content
    )));
}

pub(crate) fn inject_project_memory_if_needed(
    messages: &mut Vec<StarMessage>,
    injected_memory_hash: &Arc<Mutex<Option<u64>>>,
) {
    use crate::core::tools::memory_tool::get_global_memory_file_path;
    use crate::core::utils::paths::current_project_star_dir;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;

    let mut sections: Vec<String> = Vec::new();
    let mut hasher = DefaultHasher::new();

    let mut add_section = |label: &str, path: &PathBuf, content: &str| {
        sections.push(format!(
            "{} (from {}):\n{}",
            label,
            path.to_string_lossy(),
            content
        ));
        path.to_string_lossy().hash(&mut hasher);
        content.hash(&mut hasher);
    };

    let global_path = get_global_memory_file_path();
    if global_path.exists() {
        let content = std::fs::read_to_string(&global_path).unwrap_or_default();
        let content = content.trim();
        if !content.is_empty() {
            add_section("Memory Context", &global_path, content);
        }
    }

    let project_memory_path = current_project_star_dir().join("memory.md");
    if project_memory_path.exists() {
        let content = std::fs::read_to_string(&project_memory_path).unwrap_or_default();
        let content = content.trim();
        if !content.is_empty() {
            add_section("Project Memory", &project_memory_path, content);
        }
    }

    if sections.is_empty() {
        return;
    }

    let hash = hasher.finish();
    {
        let mut last = injected_memory_hash.lock().unwrap();
        if last.as_ref() == Some(&hash) {
            return;
        }
        *last = Some(hash);
    }

    messages.push(StarMessage::system(format!(
        "Memory Context:\n{}",
        sections.join("\n\n")
    )));
}
