pub fn is_status_text(s: &str) -> bool {
    let t = s.trim();
    crate::core::i18n::status_prefixes()
        .iter()
        .any(|prefix| t.starts_with(prefix))
}

pub fn strip_tool_running_prefix(s: &str) -> String {
    let t = s.trim();
    for prefix in crate::core::i18n::running_prefixes() {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    t.trim().to_string()
}

pub fn input_placeholder_text() -> String {
    crate::core::i18n::t(
        "ui.input.placeholder",
        "询问 Star…",
        "Ask Star…",
    )
}

pub fn strip_system_reminder_blocks_inplace(s: &mut String) {
    if s.is_empty() {
        return;
    }
    if !s.to_ascii_lowercase().contains("system-reminder") {
        return;
    }

    const START_TAG: &str = "<system-reminder>";
    const END_TAG: &str = "</system-reminder>";

    loop {
        let lower = s.to_ascii_lowercase();
        let Some(start) = lower.find(START_TAG) else {
            break;
        };

        if let Some(end_rel) = lower[start..].find(END_TAG) {
            let mut end = start + end_rel + END_TAG.len();
            if s[end..].starts_with('\n') {
                end += 1;
            }
            s.replace_range(start..end, "");
        } else {
            s.truncate(start);
            break;
        }
    }

    while s.contains("\n\n\n") {
        *s = s.replace("\n\n\n", "\n\n");
    }
}

pub fn sanitize_for_tui(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    // First, strip all ANSI escape sequences to prevent screen corruption
    let cleaned = crate::ui::utils::render::strip_ansi_codes(input);

    let mut out = String::with_capacity(cleaned.len());
    let mut prev_was_cr = false;
    let mut last_was_newline = false;

    for ch in cleaned.chars() {
        match ch {
            '\r' => {
                if !last_was_newline {
                    out.push('\n');
                    last_was_newline = true;
                }
                prev_was_cr = true;
            }
            '\n' => {
                if prev_was_cr {
                    prev_was_cr = false;
                    continue;
                }
                out.push('\n');
                last_was_newline = true;
            }
            '\t' => {
                out.push_str("    ");
                prev_was_cr = false;
                last_was_newline = false;
            }
            // 零宽空格和其他不可见的空白字符转换为普通空格
            // 这样 split_whitespace() 能正确分割，避免单词挤在一起
            '\u{200B}' | // Zero-width space
            '\u{200C}' | // Zero-width non-joiner
            '\u{200D}' | // Zero-width joiner
            '\u{FEFF}' | // Zero-width no-break space (BOM)
            '\u{00A0}' | // Non-breaking space
            '\u{202F}' | // Narrow no-break space
            '\u{205F}' | // Medium mathematical space
            '\u{3000}'   // Ideographic space
            => {
                out.push(' ');
                prev_was_cr = false;
                last_was_newline = false;
            }
            c if c.is_control() => {
                // Skip other control characters
                prev_was_cr = false;
            }
            c => {
                out.push(c);
                prev_was_cr = false;
                last_was_newline = false;
            }
        }
    }

    strip_system_reminder_blocks_inplace(&mut out);
    strip_dsml_blocks_inplace(&mut out);
    strip_xml_tags_inplace(&mut out);
    out
}

/// Strip XML/HTML tags that LLMs sometimes generate in thinking content.
/// This prevents tags like `<parameter=file_path>`, `<thinking>`, etc. from
/// being displayed as raw text in the UI.
fn strip_xml_tags_inplace(s: &mut String) {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut tag_start = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '<' {
            // Check if this looks like an XML tag (not a comparison operator)
            let remaining: String = chars[i..].iter().take(20).collect();
            if remaining.starts_with("</")
                || remaining.starts_with("<parameter")
                || remaining.starts_with("<thinking")
                || remaining.starts_with("</think>")
                || remaining.starts_with("<function")
                || remaining.starts_with("</tool_call>")
                || remaining.starts_with("<tool")
                || remaining.starts_with("<system")
            {
                in_tag = true;
                tag_start = i;
                // Skip to end of tag
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // Skip '>'
                }
                in_tag = false;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    *s = result;
}

pub fn sanitize_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out.chars().take(80).collect()
}

pub fn should_save_tool_output(s: &str) -> bool {
    if s.len() > 8000 {
        return true;
    }
    let mut lines = 0usize;
    for _ in s.lines() {
        lines += 1;
        if lines > 120 {
            return true;
        }
    }
    false
}

pub fn format_elapsed_for_tool(ms: u128) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        let s = ms as f64 / 1000.0;
        if s < 10.0 {
            format!("{:.1}s", s)
        } else {
            format!("{:.0}s", s)
        }
    }
}

pub fn inject_elapsed_into_tool_first_line(content: String, elapsed: Option<String>) -> String {
    let Some(e) = elapsed else {
        return content;
    };
    if let Some(pos) = content.find('\n') {
        let (first, rest) = content.split_at(pos);
        // Star CLI 风格：✓ tool_name 或 × tool_name
        let updated = if first.starts_with("✓ ") {
            first.replacen("✓ ", &format!("✓({}) ", e), 1)
        } else if first.starts_with("× ") {
            first.replacen("× ", &format!("×({}) ", e), 1)
        } else {
            first.to_string()
        };
        let mut out = updated;
        out.push_str(rest);
        out
    } else if content.starts_with("✓ ") {
        content.replacen("✓ ", &format!("✓({}) ", e), 1)
    } else if content.starts_with("× ") {
        content.replacen("× ", &format!("×({}) ", e), 1)
    } else {
        content
    }
}

pub fn is_action_chain_line(line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() {
        return false;
    }
    // 经验规则：这种中间态通常很短，并包含 input/action/do 以及分隔符
    if l.len() > 240 {
        return false;
    }
    let lower = l.to_ascii_lowercase();
    (lower.contains("input") && lower.contains("action") && lower.contains("do"))
        && (lower.contains('>') || lower.contains('→'))
}

pub fn collapse_action_chain_lines_inplace(s: &mut String) {
    if s.is_empty() {
        return;
    }
    // 只在出现明显动作链痕迹时才做处理，避免影响正常内容
    if !(s.contains('>') || s.contains('→')) {
        return;
    }
    if !(s.contains("input") || s.contains("Input")) {
        return;
    }

    let had_trailing_newline = s.ends_with('\n');
    let mut kept: Vec<String> = Vec::new();
    let mut last_chain: Option<String> = None;
    for line in s.split('\n') {
        if is_action_chain_line(line) {
            last_chain = Some(line.trim().to_string());
        } else {
            kept.push(line.to_string());
        }
    }
    if let Some(chain) = last_chain {
        kept.push(chain);
    }
    let mut rebuilt = kept.join("\n");
    if had_trailing_newline && !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }
    *s = rebuilt;
}

pub fn strip_dsml_blocks_inplace(s: &mut String) {
    if s.is_empty() {
        return;
    }
    if !s.contains("DSML") {
        return;
    }

    // 有些上游会输出全角竖线格式，例如：<｜DSML｜invoke ...>
    // 这种情况下往往没有统一的 "<| DSML ... </| DSML" 包裹。
    // 先按行剔除明显的 DSML 行，避免其在 TUI/终端中透出。
    if s.contains("<｜DSML｜") || s.contains("</｜DSML｜") {
        let had_trailing_newline = s.ends_with('\n');
        let kept: Vec<&str> = s
            .lines()
            .filter(|line| {
                !(line.contains("DSML")
                    && (line.contains("<｜DSML｜")
                        || line.contains("</｜DSML｜")
                        || line.contains("function_calls")
                        || line.contains("invoke name=")))
            })
            .collect();
        let mut rebuilt = kept.join("\n");
        if had_trailing_newline && !rebuilt.ends_with('\n') {
            rebuilt.push('\n');
        }
        *s = rebuilt;
    }

    // 兼容旧的包裹格式：<| DSML ... </| DSML
    if !(s.contains("<|") || s.contains("<｜")) {
        return;
    }

    loop {
        let start = s
            .find("<| DSML")
            .or_else(|| s.find("<|DSML"))
            .or_else(|| s.find("<｜DSML"))
            .or_else(|| s.find("<｜ DSML"));
        let Some(start) = start else {
            break;
        };

        let end_candidates = ["</| DSML", "</|DSML", "</｜DSML", "</｜ DSML"];
        let mut end: Option<usize> = None;
        for m in end_candidates {
            if let Some(pos) = s[start..].find(m).map(|i| start + i) {
                end = match end {
                    Some(prev) => Some(prev.min(pos)),
                    None => Some(pos),
                };
            }
        }

        match end {
            Some(end) => {
                // 删除到结束标记所在行末（含换行），避免留下残片
                let after_end = s[end..]
                    .find('\n')
                    .map(|i| end + i + 1)
                    .unwrap_or_else(|| s.len());
                s.replace_range(start..after_end, "");
            }
            None => {
                // 只出现起始但还没等到结束：直接截断后半段
                s.truncate(start);
                break;
            }
        }
    }
}

pub fn stream_split_threshold_chars() -> usize {
    std::env::var("STAR_STREAM_SPLIT_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8000)
}

pub fn find_last_split_point(s: &str, max_chars: usize) -> usize {
    let mut last_boundary = 0usize;
    let mut count = 0usize;
    for (i, ch) in s.char_indices() {
        if count >= max_chars {
            break;
        }
        count += 1;
        if ch == '\n' {
            last_boundary = i + ch.len_utf8();
        }
    }
    if last_boundary == 0 {
        let mut idx = 0usize;
        let mut c = 0usize;
        for (i, _) in s.char_indices() {
            if c >= max_chars {
                break;
            }
            idx = i;
            c += 1;
        }
        if idx == 0 {
            0
        } else {
            s[..=idx].len()
        }
    } else {
        last_boundary
    }
}
