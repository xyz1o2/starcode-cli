use ratatui::layout::HorizontalAlignment as Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use std::sync::OnceLock;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Detect language from content for syntax highlighting
fn detect_language_from_content(content: &str) -> &'static str {
    let trimmed = content.trim();

    // JSON detection
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return "json";
        }
    }

    // YAML detection (simple heuristic)
    if trimmed.contains("---") && (trimmed.contains(": ") || trimmed.contains(":\n")) {
        return "yaml";
    }

    // TOML detection
    if trimmed.contains("[[") && trimmed.contains("]]") && trimmed.contains("=") {
        return "toml";
    }

    // HTML detection
    if trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") || trimmed.contains("</") {
        return "html";
    }

    // CSS detection
    if trimmed.contains("{")
        && trimmed.contains("}")
        && trimmed.contains(":")
        && !trimmed.contains("def ")
        && !trimmed.contains("function ")
    {
        return "css";
    }

    // Python detection
    if trimmed.contains("def ") && trimmed.contains(":") && !trimmed.contains("{") {
        return "python";
    }

    // JavaScript/TypeScript detection
    if (trimmed.contains("function ")
        || trimmed.contains("=>")
        || trimmed.contains("const ")
        || trimmed.contains("let "))
        && (trimmed.contains("{") || trimmed.contains(";"))
    {
        return "javascript";
    }

    // Rust detection
    if trimmed.contains("fn ") && (trimmed.contains("let ") || trimmed.contains("->")) {
        return "rust";
    }

    // Go detection
    if trimmed.contains("func ") && trimmed.contains("package ") {
        return "go";
    }

    // Java detection
    if trimmed.contains("public class ")
        || trimmed.contains("private ") && trimmed.contains("void ")
    {
        return "java";
    }

    // Shell detection
    if trimmed.starts_with("#!/bin/bash")
        || trimmed.starts_with("#!/bin/sh")
        || trimmed.contains("#!/usr/bin/env bash")
    {
        return "shell";
    }

    // SQL detection
    if trimmed.contains("SELECT ") && trimmed.contains("FROM ") {
        return "sql";
    }

    // XML detection
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<") && trimmed.contains("xmlns:") {
        return "xml";
    }

    "unknown"
}

static ANSI_REGEX: OnceLock<Regex> = OnceLock::new();

/// Truncate a string to fit within `max_width` display cells, appending "..." if needed.
/// Uses Unicode display width, safe for CJK and other wide chars.
pub fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let width = UnicodeWidthStr::width_cjk(s);
    if width <= max_width {
        return s.to_string();
    }
    let suffix = "...";
    let suffix_w = 3; // "..." is always 3 ASCII chars = 3 cells
    let target = max_width.saturating_sub(suffix_w);
    let mut result = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = UnicodeWidthChar::width_cjk(c).unwrap_or(0);
        if w + cw > target {
            break;
        }
        result.push(c);
        w += cw;
    }
    result.push_str(suffix);
    result
}

/// Calculate CJK-aware display width of ratatui Spans.
/// ratatui's Line::width() uses non-CJK width; this matches the terminal's actual rendering.
pub fn line_spans_width_cjk(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width_cjk(s.content.as_ref()))
        .sum()
}

/// Split a span's content at a display-width boundary (CJK-aware).
/// Returns (part that fits within `max_width`, remainder).
fn split_span_at_width(content: &str, max_width: usize) -> (String, String) {
    let mut w = 0usize;
    let mut fitted = String::new();
    let mut rest = String::new();
    let mut full = false;
    for c in content.chars() {
        let cw = UnicodeWidthChar::width_cjk(c).unwrap_or(0);
        if !full && w + cw <= max_width {
            fitted.push(c);
            w += cw;
        } else {
            full = true;
            rest.push(c);
        }
    }
    (fitted, rest)
}

/// Truncate styled spans to a maximum display width (drops the overflow, caller
/// may append an ellipsis span). Equivalent of the reference implementation's
/// ANSI-aware slicing for truncation.
pub fn truncate_spans_to_width(spans: &[Span<'static>], max_width: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut remaining = max_width;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let w = UnicodeWidthStr::width_cjk(span.content.as_ref());
        if w <= remaining {
            remaining -= w;
            out.push(span.clone());
        } else {
            let (fitted, _) = split_span_at_width(&span.content, remaining);
            if !fitted.is_empty() {
                out.push(Span::styled(fitted, span.style));
            }
            break;
        }
    }
    out
}

/// Hard-wrap styled spans into rows of at most `wrap_width` display columns.
/// Mirrors the reference implementation: long lines are sliced at fixed width
/// boundaries while preserving ANSI-derived styles (not word-wrapped).
pub fn wrap_spans_to_width(
    spans: Vec<Span<'static>>,
    wrap_width: usize,
) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let mut rest_content = span.content.to_string();
        let style = span.style;
        while !rest_content.is_empty() {
            let space = wrap_width.saturating_sub(used);
            if space == 0 {
                rows.push(std::mem::take(&mut current));
                used = 0;
                continue;
            }
            let (fitted, rest) = split_span_at_width(&rest_content, space);
            if fitted.is_empty() {
                if used > 0 {
                    // Wide char doesn't fit the remaining columns — flush the
                    // row and retry at the start of the next row.
                    rows.push(std::mem::take(&mut current));
                    used = 0;
                    continue;
                }
                // Char is wider than the whole row width — drop it to avoid
                // an infinite loop.
                let mut chars = rest_content.chars();
                chars.next();
                rest_content = chars.as_str().to_string();
                continue;
            }
            used += UnicodeWidthStr::width_cjk(fitted.as_str());
            current.push(Span::styled(fitted, style));
            rest_content = rest;
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

/// Strip ALL ANSI escape sequences — complete, partial, and malformed.
/// Bash output often contains broken sequences (truncated pipes, partial writes).
/// We must consume everything from ESC until a safe boundary to prevent leaked chars.
pub fn strip_ansi_codes(s: &str) -> String {
    let regex = ANSI_REGEX.get_or_init(|| {
        Regex::new(concat!(
            r"\x1b\[[\d;?>=]*[a-zA-Z]", // CSI: ESC [ params* letter
            r"|\x1b\[[\d;?>=]*",        // Broken CSI: ESC [ params* (no terminator)
            r"|\x1b\].*?(\x07|\x1b\\)", // OSC: ESC ] ... (BEL or ST)
            r"|\x1b\][^\x07\x1b]*",     // Broken OSC: ESC ] ... (no terminator)
            r"|\x1b[PX^_].*?\x1b\\",    // DCS/SOS/PM/APC ... ST
            r"|\x1b.",                  // Lone ESC + one char (any)
        ))
        .unwrap()
    });
    regex.replace_all(s, "").to_string()
}

pub fn parse_ansi_text(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_style = Style::default();
    let mut current_text = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            current_text.push(c);
            continue;
        }
        // ESC found — flush accumulated text
        if !current_text.is_empty() {
            spans.push(Span::styled(current_text.clone(), current_style));
            current_text.clear();
        }

        match chars.peek().copied() {
            Some('[') => {
                chars.next(); // consume '['
                let mut params = String::new();
                // Consume CSI parameter bytes: digits, ;, ?, >, =
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_digit() || matches!(pc, ';' | '?' | '>' | '=') {
                        params.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                // Consume terminator
                if let Some(term) = chars.next() {
                    if term == 'm' {
                        // SGR: apply style
                        if params.is_empty() {
                            current_style = Style::default();
                        } else {
                            let param_list: Vec<&str> = params.split(';').collect();
                            current_style = apply_sgr_params(current_style, &param_list);
                        }
                    }
                    // Non-m: cursor/erase/etc. — silently consumed
                }
            }
            Some(']') => {
                // OSC: ESC ] ... BEL or ESC ] ... ST
                chars.next(); // consume ']'
                for pc in chars.by_ref() {
                    if pc == '\x07' {
                        break;
                    }
                    if pc == '\x1b' {
                        // ST: ESC backslash
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {
                // Unknown ESC sequence: consume one char as fallback
                chars.next();
            }
        }
    }

    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, current_style));
    }
    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    spans
}

/// Apply a full SGR parameter list to a style, with look-ahead support for
/// 256-color (`38;5;N` / `48;5;N`) and truecolor (`38;2;R;G;B` / `48;2;R;G;B`).
/// 对标参考实现 `<Ansi>` 组件的完整色彩支持。
fn apply_sgr_params(mut style: Style, params: &[&str]) -> Style {
    let mut i = 0;
    while i < params.len() {
        match params[i] {
            "0" => style = Style::default(),
            "1" => style = style.add_modifier(Modifier::BOLD),
            "2" => style = style.add_modifier(Modifier::DIM),
            "3" => style = style.add_modifier(Modifier::ITALIC),
            "4" => style = style.add_modifier(Modifier::UNDERLINED),
            "7" => style = style.add_modifier(Modifier::REVERSED),
            "9" => style = style.add_modifier(Modifier::CROSSED_OUT),
            "22" => {
                style = style.remove_modifier(Modifier::BOLD);
                style = style.remove_modifier(Modifier::DIM);
            }
            "23" => style = style.remove_modifier(Modifier::ITALIC),
            "24" => style = style.remove_modifier(Modifier::UNDERLINED),
            "29" => style = style.remove_modifier(Modifier::CROSSED_OUT),
            "30" => style = style.fg(Color::Black),
            "31" => style = style.fg(Color::Red),
            "32" => style = style.fg(Color::Green),
            "33" => style = style.fg(Color::Yellow),
            "34" => style = style.fg(Color::Blue),
            "35" => style = style.fg(Color::Magenta),
            "36" => style = style.fg(Color::Cyan),
            "37" => style = style.fg(Color::Gray),
            "39" => style = style.fg(Color::Reset),
            "40" => style = style.bg(Color::Black),
            "41" => style = style.bg(Color::Red),
            "42" => style = style.bg(Color::Green),
            "43" => style = style.bg(Color::Yellow),
            "44" => style = style.bg(Color::Blue),
            "45" => style = style.bg(Color::Magenta),
            "46" => style = style.bg(Color::Cyan),
            "47" => style = style.bg(Color::Gray),
            "49" => style = style.bg(Color::Reset),
            "90" => style = style.fg(Color::DarkGray),
            "91" => style = style.fg(Color::LightRed),
            "92" => style = style.fg(Color::LightGreen),
            "93" => style = style.fg(Color::LightYellow),
            "94" => style = style.fg(Color::LightBlue),
            "95" => style = style.fg(Color::LightMagenta),
            "96" => style = style.fg(Color::LightCyan),
            "97" => style = style.fg(Color::White),
            "100" => style = style.bg(Color::DarkGray),
            "101" => style = style.bg(Color::LightRed),
            "102" => style = style.bg(Color::LightGreen),
            "103" => style = style.bg(Color::LightYellow),
            "104" => style = style.bg(Color::LightBlue),
            "105" => style = style.bg(Color::LightMagenta),
            "106" => style = style.bg(Color::LightCyan),
            "107" => style = style.bg(Color::White),
            "38" | "48" => {
                let is_fg = params[i] == "38";
                // 38;5;N (256-color) or 38;2;R;G;B (truecolor)
                if i + 1 < params.len() {
                    let color = match params[i + 1] {
                        "5" if i + 2 < params.len() => {
                            params[i + 2].parse::<u8>().ok().map(Color::Indexed)
                        }
                        "2" if i + 4 < params.len() => {
                            let (r, g, b) = (
                                params[i + 2].parse::<u8>().ok(),
                                params[i + 3].parse::<u8>().ok(),
                                params[i + 4].parse::<u8>().ok(),
                            );
                            match (r, g, b) {
                                (Some(r), Some(g), Some(b)) => Some(Color::Rgb(r, g, b)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = color {
                        style = if is_fg { style.fg(c) } else { style.bg(c) };
                    }
                    // Skip consumed sub-params (5;N or 2;R;G;B); unknown forms
                    // skip only the mode byte to avoid swallowing content params.
                    i += match params.get(i + 1) {
                        Some(&"5") => 2,
                        Some(&"2") => 4,
                        _ => 1,
                    };
                }
            }
            _ => {} // Ignore unsupported
        }
        i += 1;
    }
    style
}

pub fn get_spinner_anim() -> &'static str {
    // ASCII spinner is more portable across terminals/codepages and avoids glyph width issues.
    const SPINNERS: &[&str] = &["|", "/", "-", "\\"];
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as usize
        / 100
        % SPINNERS.len();
    SPINNERS[idx]
}

use crate::utils::markdown_parser::{
    parse_markdown_content_ext, render_content_blocks, render_markdown_incremental,
};

pub fn build_assistant_body_block(
    content: &str,
    is_streaming: bool,
    wrap_width: usize,
) -> Vec<Line<'static>> {
    if is_streaming {
        // During streaming, use incremental rendering: only re-parse the last paragraph
        let (stable, unstable) = render_markdown_incremental(content, Some(wrap_width));
        let mut lines = stable;
        // 拼接处补一个空行：stable 尾部空行已被裁剪，块间距保持固定一行
        if !lines.is_empty() && !unstable.is_empty() {
            lines.push(Line::from(""));
        }
        lines.extend(unstable);
        // 最终归一：连续空行折叠为一行，杜绝流式期间出现双空行
        let mut collapsed: Vec<Line<'static>> = Vec::with_capacity(lines.len());
        let mut prev_blank = false;
        for line in lines {
            let is_blank = line.spans.is_empty() || line.spans.iter().all(|s| s.content.is_empty());
            if is_blank && prev_blank {
                continue;
            }
            prev_blank = is_blank;
            collapsed.push(line);
        }
        collapsed
    } else {
        let blocks = parse_markdown_content_ext(content, false);
        render_content_blocks(&blocks, Some(wrap_width))
    }
}

/// Word-aware text wrapping — wraps at whitespace boundaries, hard-breaks long words.
/// Safe for CJK via UnicodeWidthChar. Truncates > 5000 chars to avoid OOM.
///
/// 保留原文空白：行首缩进（代码块）与词间连续空格都不会被折叠，
/// 只有折行断点处的空格被丢弃（与终端/浏览器的折行行为一致）。
pub fn wrap_text_to_width(text: &str, wrap_width: usize) -> Vec<String> {
    if wrap_width <= 2 {
        return text.lines().map(|l| l.to_string()).collect();
    }
    let chars: Vec<char> = text.chars().take(5000).collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    wrap_char_ranges(&chars, wrap_width, wrap_width)
        .into_iter()
        .map(|(s, e)| chars[s..e].iter().collect())
        .collect()
}

/// 折行核心：按显示宽度对字符流做贪心断行，返回每行在 `chars` 中的区间 `[start, end)`。
///
/// 断点规则：
/// - `'\n'` 强制换行；
/// - 空格处可断，断点处的空格被丢弃（行尾不留空格）；
/// - 宽字符（CJK / emoji）两侧可断，实现逐字折行；
/// - 单个 token 比整行还宽（长 URL、长路径）时硬断。
///
/// 与按空白重组文本的实现不同，这里只切片不重写，因此：
/// 源行的前导缩进保留（代码块靠它对齐），词间的连续空格也保留
/// （`**bold**` 前后的空格曾被 `split_whitespace` 吃掉，粘成 "withboldinside"）。
///
/// `first_width` 用于首行，`rest_width` 用于续行（悬挂缩进场景）。
pub fn wrap_char_ranges(
    chars: &[char],
    first_width: usize,
    rest_width: usize,
) -> Vec<(usize, usize)> {
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let cw = |c: char| UnicodeWidthChar::width_cjk(c).unwrap_or(0);
    // 只含空白的行归一为空行，避免行尾残留空格
    fn push_line(out: &mut Vec<(usize, usize)>, chars: &[char], s: usize, e: usize) {
        if e > s && chars[s..e].iter().all(|c| c.is_whitespace()) {
            out.push((s, s));
        } else {
            out.push((s, e.max(s)));
        }
    }

    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut avail = first_width.max(1);
    // 当前行：start = 行首下标，end = 最后一个已放入字符之后，width = 可见宽度
    let mut start: Option<usize> = None;
    let mut end = 0usize;
    let mut width = 0usize;
    // 待定空格：只有后面还跟内容时才计入行宽
    let mut pending = 0usize;
    // 本源行尚未放下第一个词 —— 此时的空格是缩进，必须保留
    let mut keep_indent = true;
    // 本输出行已有词 —— 只有这样才允许软断行（避免缩进独占一行）
    let mut has_word = false;

    let mut i = 0usize;
    while i < n {
        let c = chars[i];
        if c == '\n' {
            let s = start.unwrap_or(i);
            push_line(&mut out, chars, s, end);
            start = None;
            end = 0;
            width = 0;
            pending = 0;
            keep_indent = true;
            has_word = false;
            avail = rest_width.max(1);
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            let ws_start = i;
            let mut ws_w = 0usize;
            while i < n && chars[i].is_whitespace() && chars[i] != '\n' {
                ws_w += cw(chars[i]).max(1);
                i += 1;
            }
            match start {
                // 源行缩进：保留；折行断点处的空格：丢弃
                None if keep_indent => {
                    start = Some(ws_start);
                    end = i;
                    width = ws_w;
                }
                None => {}
                Some(_) => pending += ws_w,
            }
            continue;
        }
        // 取一个 token：单个宽字符，或一段连续窄字符
        let tok_start = i;
        if cw(c) >= 2 {
            i += 1;
        } else {
            while i < n && !chars[i].is_whitespace() && cw(chars[i]) < 2 {
                i += 1;
            }
        }
        let tok_w: usize = chars[tok_start..i].iter().map(|&ch| cw(ch)).sum();

        if has_word && width + pending + tok_w > avail {
            push_line(&mut out, chars, start.unwrap_or(tok_start), end);
            start = None;
            end = 0;
            width = 0;
            pending = 0;
            keep_indent = false;
            has_word = false;
            avail = rest_width.max(1);
        }
        match start {
            None => {
                start = Some(tok_start);
                end = i;
                width = tok_w;
            }
            Some(_) => {
                end = i;
                width += pending + tok_w;
            }
        }
        pending = 0;
        keep_indent = false;
        has_word = true;

        // token 本身超过整行宽度（长 URL / 无空格长串）→ 硬断
        while width > avail {
            let s = start.unwrap_or(tok_start);
            let mut k = s;
            let mut acc = 0usize;
            while k < end {
                let ch_w = cw(chars[k]);
                if acc + ch_w > avail && k > s {
                    break;
                }
                acc += ch_w;
                k += 1;
            }
            if k == s {
                k = s + 1;
            }
            push_line(&mut out, chars, s, k);
            start = Some(k);
            width = chars[k..end].iter().map(|&ch| cw(ch)).sum();
            avail = rest_width.max(1);
            if k >= end {
                start = None;
                width = 0;
                has_word = false;
                break;
            }
        }
    }
    if let Some(s) = start {
        push_line(&mut out, chars, s, end);
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    out
}

fn is_box_drawing(ch: char) -> bool {
    matches!(ch,
        '\u{2500}'..='\u{259F}' |  // Box Drawing + Block Elements
        '\u{2308}'..='\u{230F}' |  // Ceiling/Floor corners
        '\u{2370}'..='\u{237F}'    // APL symbols (includes ⎿ U+237F)
    )
}

/// Trim trailing lines that only contain whitespace and box-drawing characters.
/// These are visual artifacts from tool output using box-drawing formats
/// (e.g. trailing `│` lines with no actual content).
fn trim_trailing_visual_empty_lines(text: &str) -> &str {
    let mut end = text.len();
    for line in text.lines().rev() {
        let visually_empty = line.chars().all(|c| c.is_whitespace() || is_box_drawing(c));
        if visually_empty {
            end = line.as_ptr() as usize - text.as_ptr() as usize;
        } else {
            break;
        }
    }
    text[..end].trim_end()
}

/// Format a single line as pretty-printed JSON when it parses as a JSON
/// object/array (对标参考实现 tryFormatJson 的逐行处理，覆盖 ndjson/日志行)。
fn try_format_json_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.len() < 2 || (!trimmed.starts_with('{') && !trimmed.starts_with('[')) {
        return line.to_string();
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Ok(pretty) = serde_json::to_string_pretty(&val) {
            return pretty;
        }
    }
    line.to_string()
}

pub fn build_tool_body_block(
    content: &str,
    wrap_width: usize,
    expanded: bool,
) -> Vec<Line<'static>> {
    if !expanded {
        // Collapsed: show first line (display-width truncated), don't wrap.
        // Wrapping short summaries (like "Found 4 file(s)") across multiple lines
        // makes the collapsed view harder to scan.
        let total_lines = content.lines().count();
        let clean_first = content.lines().next().unwrap_or("").trim();
        let clean_first = strip_ansi_codes(clean_first);
        let preview = truncate_to_display_width(&clean_first, wrap_width.saturating_sub(3));

        let mut result = vec![Line::from(preview)];
        if total_lines > 1 {
            result.push(Line::from(vec![Span::styled(
                format!("... +{} lines", total_lines - 1),
                Style::default().fg(Color::DarkGray),
            )]));
        }
        return result;
    }

    let content = trim_trailing_visual_empty_lines(content);

    // 尝试将单行 JSON 格式化为多行（提高可读性）
    let formatted_content = if content.starts_with('{') || content.starts_with('[') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            serde_json::to_string_pretty(&val).unwrap_or_else(|_| content.to_string())
        } else {
            content.to_string()
        }
    } else {
        content.to_string()
    };

    // 逐行 JSON 美化（对标参考实现 tryJsonFormatContent，长度上限一致）
    const MAX_JSON_FORMAT_LENGTH: usize = 10_000;
    let formatted_content = if formatted_content.len() <= MAX_JSON_FORMAT_LENGTH {
        formatted_content
            .split('\n')
            .map(try_format_json_line)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        formatted_content
    };

    // Detect language for syntax highlighting
    let language = detect_language_from_content(&formatted_content);
    let use_syntax_highlight = language != "unknown";

    let mut lines: Vec<Line<'static>> = Vec::new();
    const MAX_TOOL_LINES: usize = 2000;
    let mut line_count = 0;
    let mut consecutive_empty = 0;

    for line in formatted_content.lines() {
        if line_count >= MAX_TOOL_LINES {
            lines.push(Line::from(vec![Span::styled(
                "... (output too long, truncated) ...".to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]));
            break;
        }
        line_count += 1;

        // Strip ANSI codes from each line to prevent screen corruption
        let clean_line = strip_ansi_codes(line);
        let trimmed = clean_line.trim();

        // Collapse consecutive empty lines (keep at most 1)
        if trimmed.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 1 {
                lines.push(Line::from(""));
            }
            continue;
        }
        consecutive_empty = 0;

        // 带颜色输出的行：保留 ANSI 颜色渲染（对标参考实现 <Ansi> 组件），
        // 而不是剥掉全部转义序列导致彩色输出（git/ls/测试运行器等）褪色。
        if line.contains('\x1b') {
            let spans = parse_ansi_text(line);
            let rows = if line_spans_width_cjk(&spans) > wrap_width {
                wrap_spans_to_width(spans, wrap_width)
            } else {
                vec![spans]
            };
            for row in rows {
                lines.push(Line::from(row));
            }
            continue;
        }

        // Use syntax highlighting if language is detected
        if use_syntax_highlight {
            let highlighted = crate::utils::syntax_highlight::highlight_line(&clean_line, language);
            // Manual wrapping logic (CJK-aware width check)
            if line_spans_width_cjk(&highlighted.spans) > wrap_width {
                for wrapped in wrap_text_to_width(&clean_line, wrap_width) {
                    lines.push(Line::from(wrapped));
                }
            } else {
                lines.push(highlighted);
            }
        } else {
            // Enhanced rendering for lists and key-values
            let mut spans = Vec::new();

            // Check for list items
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                spans.push(Span::styled("  • ", Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    trimmed[2..].to_string(),
                    Style::default().fg(Color::White),
                ));
            }
            // Check for JSON key-value patterns: "key": value
            else if trimmed.starts_with('"') {
                if let Some(colon_pos) = trimmed.find("\": ") {
                    let key = &trimmed[1..colon_pos];
                    let val = &trimmed[colon_pos + 3..].trim_end_matches(',');
                    spans.push(Span::styled(
                        format!("\"{}\"", key),
                        Style::default().fg(Color::Blue),
                    ));
                    spans.push(Span::styled(": ", Style::default().fg(Color::DarkGray)));
                    spans.push(Span::styled(
                        val.to_string(),
                        Style::default().fg(Color::White),
                    ));
                } else {
                    spans.push(Span::styled(clean_line.to_string(), Style::default()));
                }
            }
            // Check for Key: Value patterns (simple heuristic)
            else if let Some(idx) = clean_line.find(": ") {
                let key = &clean_line[..idx];
                let val = &clean_line[idx + 2..];
                if !key.contains(char::is_whitespace) && key.len() < 30 {
                    spans.push(Span::styled(
                        key.to_string(),
                        Style::default().fg(Color::Blue),
                    ));
                    spans.push(Span::styled(": ", Style::default().fg(Color::DarkGray)));
                    spans.push(Span::styled(
                        val.to_string(),
                        Style::default().fg(Color::White),
                    ));
                } else {
                    spans.push(Span::styled(clean_line.to_string(), Style::default()));
                }
            }
            // Check for JSON brackets/braces (standalone)
            else if trimmed == "{" || trimmed == "}" || trimmed == "[" || trimmed == "]" {
                spans.push(Span::styled(
                    clean_line.to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            // Check for file paths
            else if (clean_line.contains('/') || clean_line.contains('\\'))
                && !clean_line.contains(' ')
            {
                spans.push(Span::styled(
                    clean_line.to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                ));
            } else {
                spans.push(Span::styled(clean_line.to_string(), Style::default()));
            }

            // Manual wrapping logic (CJK-aware width check)
            if line_spans_width_cjk(&spans) > wrap_width {
                for wrapped in wrap_text_to_width(&clean_line, wrap_width) {
                    lines.push(Line::from(wrapped));
                }
            } else {
                lines.push(Line::from(spans));
            }
        }
    }
    lines
}

pub fn build_user_body_block(content: &str, wrap_width: usize) -> Vec<Line<'static>> {
    if wrap_width == 0 {
        return vec![];
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    for line in content.lines() {
        let clean = strip_ansi_codes(line);
        if line_spans_width_cjk(&[Span::raw(&clean)]) <= wrap_width {
            lines.push(Line::from(Span::styled(
                clean,
                Style::default().fg(Color::White),
            )));
        } else {
            for wrapped in wrap_text_to_width(&clean, wrap_width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(Color::White),
                )));
            }
        }
    }
    lines
}

pub fn build_diff_block(diff_content: &str, wrap_width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // 1. Detect language from diff header
    let mut language = "text";
    for line in diff_content.lines().take(10) {
        if line.starts_with("diff --git") {
            // diff --git a/path/to/file.rs b/path/to/file.rs
            if let Some(last) = line.split_whitespace().last() {
                if let Some(ext) = last.split('.').last() {
                    language = match ext {
                        "rs" => "rust",
                        "ts" | "tsx" | "js" | "jsx" => "javascript",
                        "py" => "python",
                        "go" => "go",
                        "html" => "html",
                        "css" => "css",
                        "json" => "json",
                        "md" => "markdown",
                        "toml" => "toml",
                        _ => "text",
                    };
                }
            }
            if language != "text" {
                break;
            }
        } else if line.starts_with("+++ ") || line.starts_with("--- ") {
            if let Some(path) = line.split_whitespace().last() {
                if let Some(ext) = path.split('.').last() {
                    let detected = match ext {
                        "rs" => "rust",
                        "ts" | "tsx" | "js" | "jsx" => "javascript",
                        "py" => "python",
                        "go" => "go",
                        "html" => "html",
                        "css" => "css",
                        "json" => "json",
                        "md" => "markdown",
                        "toml" => "toml",
                        _ => "text",
                    };
                    if detected != "text" {
                        language = detected;
                    }
                }
            }
            if language != "text" {
                break;
            }
        }
    }

    // 2. Parse lines
    struct DiffLine {
        line_type: u8, // 0: other, 1: hunk, 2: add, 3: del, 4: context
        old_line: Option<usize>,
        new_line: Option<usize>,
        content: String,
    }

    let mut parsed_lines: Vec<DiffLine> = Vec::new();
    let mut current_old_line = 0;
    let mut current_new_line = 0;
    let mut in_hunk = false;

    // Safety limit for parsing large diffs to prevent UI freeze
    const MAX_PARSE_LINES: usize = 3000;

    for (line_idx, raw_line) in diff_content.lines().enumerate() {
        if line_idx >= MAX_PARSE_LINES {
            break;
        }

        let line_string = if raw_line.contains('\x1b') {
            strip_ansi_codes(raw_line)
        } else {
            raw_line.to_string()
        };
        let line = line_string.as_str();

        if line.starts_with("@@") {
            // @@ -192,4 +192,9 @@
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let new_part = parts[2].trim_start_matches('+');
                current_new_line = new_part
                    .split(',')
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);

                let old_part = parts[1].trim_start_matches('-');
                current_old_line = old_part
                    .split(',')
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);

                // Adjust because hunk header is 1-based start
                if current_new_line > 0 {
                    current_new_line -= 1;
                }
                if current_old_line > 0 {
                    current_old_line -= 1;
                }
            }
            in_hunk = true;
            parsed_lines.push(DiffLine {
                line_type: 1,
                old_line: None,
                new_line: None,
                content: line.to_string(),
            });
            continue;
        }

        if !in_hunk {
            if line.starts_with("---") || line.starts_with("+++") {
                continue;
            }
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            current_new_line += 1;
            parsed_lines.push(DiffLine {
                line_type: 2,
                old_line: None,
                new_line: Some(current_new_line),
                content: line[1..].to_string(),
            });
        } else if line.starts_with('-') && !line.starts_with("---") {
            current_old_line += 1;
            parsed_lines.push(DiffLine {
                line_type: 3,
                old_line: Some(current_old_line),
                new_line: None,
                content: line[1..].to_string(),
            });
        } else if line.starts_with(' ') {
            current_old_line += 1;
            current_new_line += 1;
            parsed_lines.push(DiffLine {
                line_type: 4,
                old_line: Some(current_old_line),
                new_line: Some(current_new_line),
                content: line[1..].to_string(),
            });
        } else if in_hunk && line.trim().is_empty() {
            // Empty lines in hunk are context
            current_old_line += 1;
            current_new_line += 1;
            parsed_lines.push(DiffLine {
                line_type: 4,
                old_line: Some(current_old_line),
                new_line: Some(current_new_line),
                content: String::new(),
            });
        } else {
            // Other lines (e.g. diff header)
            parsed_lines.push(DiffLine {
                line_type: 0,
                old_line: None,
                new_line: None,
                content: line.to_string(),
            });
        }
    }

    // 3. Calculate base indentation (Smart Indentation)
    let mut base_indentation = usize::MAX;
    for line in &parsed_lines {
        if line.line_type == 2 || line.line_type == 3 || line.line_type == 4 {
            // add, del, context
            if !line.content.trim().is_empty() {
                let indent = line.content.len() - line.content.trim_start().len();
                if indent < base_indentation {
                    base_indentation = indent;
                }
            }
        }
    }
    if base_indentation == usize::MAX {
        base_indentation = 0;
    }

    // 4. Render
    let max_line_num = parsed_lines
        .iter()
        .map(|l| l.new_line.unwrap_or(0).max(l.old_line.unwrap_or(0)))
        .max()
        .unwrap_or(0);
    let gutter_width = max_line_num.to_string().len().max(3);

    // Gap detection constants
    let mut last_line_num: Option<usize> = None;
    let _max_context_lines_without_gap = 5;

    // Filter out hunks/others for display
    let displayable_lines: Vec<&DiffLine> = parsed_lines
        .iter()
        .filter(|l| l.line_type == 2 || l.line_type == 3 || l.line_type == 4)
        .collect();

    if displayable_lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No changes detected.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    use crate::utils::markdown_parser::highlight_code_line;

    // Limit rendered lines to avoid UI freeze on large diffs
    const MAX_RENDER_LINES: usize = 2000;
    let total_lines = displayable_lines.len();
    let render_limit = total_lines.min(MAX_RENDER_LINES);

    for (_idx, line) in displayable_lines.iter().enumerate().take(render_limit) {
        // Gap Indicator
        let relevant_line_num = if line.line_type == 3 {
            line.old_line
        } else {
            line.new_line
        };

        if let Some(last) = last_line_num {
            if let Some(curr) = relevant_line_num {
                // If jump is too big, show gap
                if curr > last + 1 {
                    let sep = "═".repeat(wrap_width.min(100));
                    lines.push(Line::from(Span::styled(
                        sep,
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
        }

        // Update last line num
        if line.line_type == 2 || line.line_type == 4 {
            // Add or Context updates new_line
            last_line_num = line.new_line;
        } else if line.line_type == 3 {
            // Del updates old_line
            last_line_num = line.old_line;
        }

        // Content processing
        let display_content = if line.content.len() >= base_indentation {
            // Safety: We must operate on char boundaries, not bytes, to avoid panic on wide chars
            let mut char_indices = line.content.char_indices();
            if let Some((byte_pos, _)) = char_indices.nth(base_indentation) {
                &line.content[byte_pos..]
            } else {
                // If base_indentation is beyond string length (unlikely given the check above, but safe fallback)
                // Note: base_indentation was calculated based on byte len diff in step 3, which is correct for space indentation
                // But if indentation mixes tabs/spaces/chars, it's tricky.
                // However, since we used len() - trim_start().len() before, that was byte count.
                // So actually, line.content[base_indentation..] IS technically correct IF base_indentation is byte offset.
                // Let's verify step 3.
                // Step 3: let indent = line.content.len() - line.content.trim_start().len(); -> This IS byte offset.
                // So simple slicing IS safe IF base_indentation is exactly at a char boundary.
                // Since trim_start() only trims whitespace (which are 1-byte chars usually), it should be safe.
                // BUT, to be absolutely robust against mixed weirdness:
                if base_indentation <= line.content.len()
                    && line.content.is_char_boundary(base_indentation)
                {
                    &line.content[base_indentation..]
                } else {
                    &line.content // Fallback: don't dedent if unsafe
                }
            }
        } else {
            &line.content
        };

        // Advanced Diff Rendering with Single Line Number Column for Alignment
        // Format: " Num │ M │ Content "

        let (num_str, marker, _style, bg_color) = match line.line_type {
            2 => (
                // Add: Show new line num
                line.new_line
                    .map(|n| format!("{:width$}", n, width = gutter_width))
                    .unwrap_or_else(|| " ".repeat(gutter_width)),
                "+",
                Style::default().fg(Color::Green),
                Some(Color::Indexed(22)),
            ),
            3 => (
                // Del: Show old line num
                line.old_line
                    .map(|n| format!("{:width$}", n, width = gutter_width))
                    .unwrap_or_else(|| " ".repeat(gutter_width)),
                "-",
                Style::default().fg(Color::Red),
                Some(Color::Indexed(52)),
            ),
            4 => (
                // Context: Show new line num (or old if new is missing, but context usually has both)
                line.new_line
                    .or(line.old_line)
                    .map(|n| format!("{:width$}", n, width = gutter_width))
                    .unwrap_or_else(|| " ".repeat(gutter_width)),
                " ",
                Style::default().fg(Color::DarkGray),
                None,
            ),
            _ => (String::new(), " ", Style::default(), None),
        };

        // Render syntax highlighted content
        let highlighted_line = highlight_code_line(display_content, language);

        let mut spans = Vec::new();
        let gutter_style = Style::default().fg(Color::DarkGray);

        // Line Num
        let num_display = if num_str.is_empty() {
            " ".repeat(gutter_width)
        } else {
            num_str
        };
        spans.push(Span::styled(
            format!(" {} ", num_display),
            if let Some(bg) = bg_color {
                gutter_style.bg(bg)
            } else {
                gutter_style
            },
        ));

        // Separator
        spans.push(Span::styled(
            "│",
            if let Some(bg) = bg_color {
                gutter_style.bg(bg)
            } else {
                gutter_style
            },
        ));

        // Marker
        let marker_style = match line.line_type {
            2 => Style::default().fg(Color::Green),
            3 => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };
        spans.push(Span::styled(
            format!(" {} ", marker),
            if let Some(bg) = bg_color {
                marker_style.bg(bg)
            } else {
                marker_style
            },
        ));

        // Content (Highlighted)
        // Optimize: Merge spans during construction to reduce rendering overhead
        let mut last_span: Option<Span> = None;

        for span in highlighted_line.spans {
            let mut s = span.style;
            if let Some(bg) = bg_color {
                s = s.bg(bg);
            }

            if let Some(mut last) = last_span.take() {
                if last.style == s {
                    last.content = format!("{}{}", last.content, span.content).into();
                    last_span = Some(last);
                } else {
                    spans.push(last);
                    last_span = Some(Span::styled(span.content, s));
                }
            } else {
                last_span = Some(Span::styled(span.content, s));
            }
        }
        if let Some(last) = last_span {
            spans.push(last);
        }

        lines.push(Line::from(spans));
    }

    if total_lines > MAX_RENDER_LINES {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "... (diff too long, omitted {} remaining lines) ...",
                total_lines - MAX_RENDER_LINES
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    lines
}

/// Diff 统计信息
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
    pub files_changed: usize,
}

/// 计算 Diff 统计信息
pub fn calculate_diff_stats(diff_content: &str) -> DiffStats {
    let mut additions = 0;
    let mut deletions = 0;
    let mut files_changed = 0;
    let mut current_file_added = false;
    let mut current_file_deleted = false;

    for line in diff_content.lines() {
        if line.starts_with("diff --git") {
            // 新文件开始
            if current_file_added || current_file_deleted {
                files_changed += 1;
            }
            current_file_added = false;
            current_file_deleted = false;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
            current_file_added = true;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
            current_file_deleted = true;
        }
    }

    // 计算最后一个文件
    if current_file_added || current_file_deleted {
        files_changed += 1;
    }

    DiffStats {
        additions,
        deletions,
        files_changed,
    }
}

/// 渲染 Diff 统计摘要
pub fn render_diff_stats_summary(stats: &DiffStats) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("+{}", stats.additions),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(
            format!("-{}", stats.deletions),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" in {} file(s)", stats.files_changed),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

/// 渲染折叠的 Diff 摘要
pub fn render_collapsed_diff_summary(diff_content: &str, file_path: &str) -> Vec<Line<'static>> {
    let stats = calculate_diff_stats(diff_content);
    let mut lines = Vec::new();

    // 文件路径
    lines.push(Line::from(vec![
        Span::styled("📄 ", Style::default().fg(Color::Blue)),
        Span::styled(
            file_path.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // 统计信息
    lines.push(render_diff_stats_summary(&stats));

    lines
}

/// 渲染展开的 Diff 内容
pub fn render_expanded_diff(
    diff_content: &str,
    file_path: &str,
    wrap_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // 文件路径标题
    lines.push(Line::from(vec![
        Span::styled("📄 ", Style::default().fg(Color::Blue)),
        Span::styled(
            file_path.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" (Tab to collapse)", Style::default().fg(Color::DarkGray)),
    ]));

    // 分隔线
    lines.push(Line::from(Span::styled(
        "─".repeat(wrap_width.min(100)),
        Style::default().fg(Color::DarkGray),
    )));

    // Diff 内容
    let diff_lines = build_diff_block(diff_content, wrap_width);
    lines.extend(diff_lines);

    lines
}

/// 增强的 Diff 块构建函数，支持折叠/展开
pub fn build_diff_block_with_collapse(
    diff_content: &str,
    file_path: &str,
    wrap_width: usize,
    expanded: bool,
) -> Vec<Line<'static>> {
    if expanded {
        render_expanded_diff(diff_content, file_path, wrap_width)
    } else {
        render_collapsed_diff_summary(diff_content, file_path)
    }
}

pub fn apply_alignment(lines: &mut Vec<Line<'static>>, align: Alignment) {
    match align {
        Alignment::Left => {}
        Alignment::Center => {
            let max_width = lines.iter().map(|l| l.width()).max().unwrap_or(0);
            for line in lines.iter_mut() {
                let line_width = line.width();
                if line_width < max_width {
                    let padding = " ".repeat((max_width - line_width) / 2);
                    line.spans.insert(0, Span::raw(padding));
                }
            }
        }
        Alignment::Right => {
            let max_width = lines.iter().map(|l| l.width()).max().unwrap_or(0);
            for line in lines.iter_mut() {
                let line_width = line.width();
                if line_width < max_width {
                    let padding = " ".repeat(max_width - line_width);
                    line.spans.insert(0, Span::raw(padding));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_256_and_truecolor_are_preserved() {
        let spans = parse_ansi_text("\x1b[38;5;196mred\x1b[0m plain");
        assert_eq!(spans[0].style.fg, Some(Color::Indexed(196)));
        assert_eq!(spans[0].content, "red");

        let spans = parse_ansi_text("\x1b[38;2;12;34;56mtrue\x1b[0m");
        assert_eq!(spans[0].style.fg, Some(Color::Rgb(12, 34, 56)));

        let spans = parse_ansi_text("\x1b[48;5;22mgreen-bg\x1b[0m");
        assert_eq!(spans[0].style.bg, Some(Color::Indexed(22)));

        // dim + bold combos and reset
        let spans = parse_ansi_text("\x1b[1;31mbold-red\x1b[22m plain-red\x1b[0m x");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[0].style.fg, Some(Color::Red));
        assert!(!spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn ansi_colors_survive_in_tool_body() {
        let lines = build_tool_body_block("\x1b[32m✔ done\x1b[0m\nplain", 80, true);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
        assert_eq!(lines[1].spans[0].content, "plain");
    }

    #[test]
    fn ansi_lines_are_hard_wrapped_by_display_width() {
        let long = format!("\x1b[36m{}\x1b[0m", "x".repeat(50));
        let lines = build_tool_body_block(&long, 20, true);
        assert_eq!(lines.len(), 3);
        for line in &lines {
            assert!(line_spans_width_cjk(&line.spans) <= 20);
        }
    }

    #[test]
    fn truncate_spans_keeps_style_and_width() {
        let spans = vec![
            Span::styled("abcdef", Style::default().fg(Color::Red)),
            Span::styled("ghij", Style::default().fg(Color::Blue)),
        ];
        let cut = truncate_spans_to_width(&spans, 4);
        assert_eq!(line_spans_width_cjk(&cut), 4);
        assert_eq!(cut[0].content, "abcd");
        assert_eq!(cut.len(), 1);
    }

    #[test]
    fn wide_chars_are_never_split() {
        let spans = vec![Span::raw("中文中文")];
        // width 4 = exactly two CJK chars per row
        let rows = wrap_spans_to_width(spans.clone(), 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "中文");
        assert_eq!(rows[1][0].content, "中文");
        // width 3: one char per row, nothing dropped or split
        let rows = wrap_spans_to_width(spans, 3);
        assert_eq!(rows.len(), 4);
        let joined: String = rows
            .iter()
            .flat_map(|r| r.iter().map(|s| s.content.to_string()))
            .collect();
        assert_eq!(joined, "中文中文");
        for row in &rows {
            assert!(line_spans_width_cjk(row) <= 3);
        }
    }

    #[test]
    fn json_lines_are_pretty_printed_per_line() {
        let out = try_format_json_line(r#"{"a":1,"b":[2,3]}"#);
        assert!(out.contains("\n"));
        assert_eq!(try_format_json_line("not json {"), "not json {");
        assert_eq!(try_format_json_line("text"), "text");
    }

    #[test]
    fn cjk_truncate_uses_display_width() {
        // 7 cells: 3 for "..." leaves 4 → two CJK chars (4 cells) fit
        let s = truncate_to_display_width("中文中文中文", 7);
        assert_eq!(s, "中文...");
        assert!(UnicodeWidthStr::width_cjk(s.as_str()) <= 7);
        // ASCII unaffected
        assert_eq!(truncate_to_display_width("abcdef", 10), "abcdef");
    }

    #[test]
    fn wrap_preserves_leading_indentation() {
        // 回归：split_whitespace 重组会吃掉行首缩进，代码块整体左移
        let out = wrap_text_to_width("fn main() {\n    let x = 42;\n}", 40);
        assert_eq!(out, vec!["fn main() {", "    let x = 42;", "}"]);
    }

    #[test]
    fn wrap_preserves_inner_spacing() {
        // 回归："with **bold** inside" 各片段分别折行时被粘成 "withboldinside"
        let src = "with  bold inside and code here";
        assert_eq!(wrap_text_to_width(src, 80), vec![src.to_string()]);
    }

    #[test]
    fn wrap_breaks_cjk_per_char_without_loss() {
        let out = wrap_text_to_width("中文中文中文", 5);
        assert_eq!(out.len(), 3);
        assert_eq!(out.concat(), "中文中文中文");
        for line in &out {
            assert!(UnicodeWidthStr::width_cjk(line.as_str()) <= 5);
        }
    }

    #[test]
    fn wrap_hard_breaks_overlong_token() {
        let url = format!("https://example.com/{}", "a".repeat(80));
        let out = wrap_text_to_width(&url, 20);
        assert_eq!(out.concat(), url);
        for line in &out {
            assert!(
                UnicodeWidthStr::width_cjk(line.as_str()) <= 20,
                "长 token 未硬断: {:?}",
                line
            );
        }
    }

    #[test]
    fn wrap_never_exceeds_width_for_mixed_content() {
        let src = "混合 content with 中文 and a very-long-hyphenated-identifier-token plus  \
                   trailing words to force several breaks";
        for w in [12usize, 20, 33, 50] {
            for line in wrap_text_to_width(src, w) {
                assert!(
                    UnicodeWidthStr::width_cjk(line.as_str()) <= w,
                    "width {} 超宽: {:?}",
                    w,
                    line
                );
            }
        }
    }

    #[test]
    fn wrap_char_ranges_supports_hanging_indent() {
        let chars: Vec<char> = "aaa bbb ccc ddd".chars().collect();
        // 首行 7 列、续行 3 列 → "aaa bbb" / "ccc" / "ddd"
        let rows: Vec<String> = wrap_char_ranges(&chars, 7, 3)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect();
        assert_eq!(rows, vec!["aaa bbb", "ccc", "ddd"]);
    }
}
