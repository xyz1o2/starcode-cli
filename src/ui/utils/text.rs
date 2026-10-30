pub fn is_status_text(s: &str) -> bool {
    let t = s.trim();
    crate::core::i18n::status_prefixes()
        .iter()
        .any(|prefix| t.starts_with(prefix))
}

// ── 选区相关的列宽换算 ────────────────────────────────────────────────────
//
// 选区要同时回答两个问题："屏幕上第 N 列是哪个字符"和"这一段字符占几列"。
// 以前这两个换算分散在三处（state/store.rs、components/chat_history.rs 各写一份），
// 而且口径不完全一致：有的按字符累加，有的按整个 span 的宽度切。
// 结果就是中文/框线字符（Ambiguous 宽度）一出现，高亮范围和复制出来的文本就对不上。
//
// 这里定一份口径：**列宽一律按 `render::char_display_width` 算**，也就是
// ratatui 分配缓冲区格子的那套规则（见 render::display_width 的注释）。

/// 按**显示宽度**截断，超出部分用 `…` 收尾。
///
/// 这是全 UI 唯一该用的截断函数。以前每个组件自己写一份 `truncate_str`，
/// 至少四种实现：
/// - 字节切片 `&s[..n]`（`collapsed_group` / `confirmation_dialog`）——
///   切点落在多字节字符中间直接 panic；
/// - 按字符数切（三个 modal + `command_suggestions`）——CJK 实际占 2 列，
///   长度会算少一倍；
/// - 按显示宽度切（`agent_task_render`）——只有这一种是对的。
///
/// 省略号也统一成 `…`（`...` 占 3 列，`…` 占 1 列）。
pub fn truncate_display(text: &str, max_width: usize) -> String {
    crate::ui::utils::render::truncate_to_display_width(text, max_width)
}

/// 一行渲染结果里的纯文本（去掉所有样式）。
pub fn line_plain_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

/// 把可视列号（鼠标坐标）换算成字符下标。
///
/// 可视列 ≠ 字符下标：CJK 汉字、emoji 占 2 列。选区复制必须走这个换算，
/// 否则中文只会复制到一半。
pub fn char_index_at_visual_col(text: &str, visual_col: usize) -> usize {
    let mut width = 0usize;
    for (i, ch) in text.chars().enumerate() {
        if width >= visual_col {
            return i;
        }
        width += crate::ui::utils::render::char_display_width(ch);
    }
    text.chars().count()
}

/// 按可视宽度把一行切成三段：前缀 / 选中段 / 后缀。
///
/// - `start_width`：前缀占多少**可视列**
/// - `end_width`：前缀 + 选中段一共占多少可视列
///
/// 切点落在宽字符中间时，整个宽字符归到后一段（宁可多给半个字，也不要把
/// 一个汉字劈成两半 —— 半个汉字进剪贴板是没法用的）。
pub fn split_by_width(
    text: &str,
    start_width: usize,
    end_width: usize,
) -> (String, String, String) {
    let mut prefix = String::new();
    let mut middle = String::new();
    let mut suffix = String::new();
    let mut current_width = 0usize;

    // 起点就在本段开头时**没有前缀**，必须直接进选中段。
    //
    // 以前这里一律 `in_prefix = true`，而 `in_middle` 只在前缀分支里被置位 ——
    // 于是 `start_width == 0` 时 `in_middle` 永远是 false，整段文字都掉进
    // suffix，选中段恒为空。选区起点落在某个 span 开头时（最常见的情形），
    // 屏幕上一点高亮都不会出现，拖选看着像"没生效"。
    let mut in_middle = start_width == 0;
    let mut in_prefix = !in_middle;

    for ch in text.chars() {
        let ch_width = crate::ui::utils::render::char_display_width(ch).max(1);

        if in_prefix && current_width < start_width {
            prefix.push(ch);
            current_width += ch_width;
            if current_width >= start_width {
                in_prefix = false;
                in_middle = true;
            }
        } else if in_middle && current_width < end_width {
            middle.push(ch);
            current_width += ch_width;
            if current_width >= end_width {
                in_middle = false;
            }
        } else {
            suffix.push(ch);
            current_width += ch_width;
        }
    }

    (prefix, middle, suffix)
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
    crate::core::i18n::t("ui.input.placeholder", "询问 Star…", "Ask Star…")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 截断不能切碎宽字符，也不能 panic。
    ///
    /// 回归：以前每个组件自己写一份 `truncate_str`，其中两处是**字节切片**
    /// （`&s[..n]`），切点落在多字节字符中间直接 panic —— 中文描述、中文路径
    /// 都会踩到。
    #[test]
    fn truncate_display_never_splits_a_multibyte_char() {
        let cjk = "研究 src/hooks 目录下的组件实现";
        for width in 0..=40 {
            let out = truncate_display(cjk, width);
            let kept = out.trim_end_matches("...");
            assert!(cjk.starts_with(kept), "宽度 {width}：{out}");
        }
    }

    /// 截断按**显示列**算，不是字符数：中文一个字占 2 列。
    #[test]
    fn truncate_display_counts_display_columns() {
        // 6 个汉字 = 12 列
        assert_eq!(crate::ui::utils::render::display_width("中文测试文本"), 12);
        let out = truncate_display("中文测试文本", 5);
        assert!(
            crate::ui::utils::render::display_width(&out) <= 6,
            "实际：{out}"
        );
    }

    /// 短文本原样返回，不加省略号。
    #[test]
    fn truncate_display_keeps_short_input_intact() {
        assert_eq!(truncate_display("abc", 10), "abc");
    }
}
