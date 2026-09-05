use crate::core::confirmation_bus::MessageBus;
use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditToolParams {
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "old_string")]
    pub old_string: String,
    #[serde(rename = "new_string")]
    pub new_string: String,
    #[serde(rename = "expected_replacements")]
    pub expected_replacements: Option<usize>,
    /// 为 true 时替换文件中的所有匹配项，跳过 expected_replacements 数量校验。
    /// 对标 Claude Code 的 Edit.replace_all —— tool-description-edit.md 里承诺了该参数。
    #[serde(default, rename = "replace_all")]
    pub replace_all: Option<bool>,
    pub instruction: Option<String>,
    #[serde(rename = "modified_by_user")]
    pub modified_by_user: Option<bool>,
    #[serde(rename = "ai_proposed_content")]
    pub ai_proposed_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReplacementResult {
    pub new_content: String,
    pub occurrences: usize,
    pub final_old_string: String,
    pub final_new_string: String,
}

#[derive(Debug, Clone)]
pub struct CalculatedEdit {
    pub current_content: Option<String>,
    pub new_content: String,
    pub occurrences: usize,
    pub error: Option<EditError>,
    pub is_new_file: bool,
    pub original_line_ending: LineEnding,
}

#[derive(Debug, Clone)]
pub struct EditError {
    pub display: String,
    pub raw: String,
    pub error_type: ToolErrorType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEnding {
    CRLF,
    LF,
}

pub fn apply_replacement(
    current_content: Option<&str>,
    old_string: &str,
    new_string: &str,
    is_new_file: bool,
) -> String {
    if is_new_file {
        return new_string.to_string();
    }

    let current = current_content.unwrap_or("");

    if old_string.is_empty() && !is_new_file {
        return current.to_string();
    }

    safe_literal_replace(current, old_string, new_string)
}

pub fn safe_literal_replace(content: &str, old: &str, new: &str) -> String {
    content.replace(old, new)
}

pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn restore_trailing_newline(original: &str, modified: &str) -> String {
    let had_trailing = original.ends_with('\n');

    if had_trailing && !modified.ends_with('\n') {
        format!("{}\n", modified)
    } else if !had_trailing && modified.ends_with('\n') {
        modified.trim_end_matches('\n').to_string()
    } else {
        modified.to_string()
    }
}

pub fn detect_line_ending(content: &str) -> LineEnding {
    if content.contains("\r\n") {
        LineEnding::CRLF
    } else {
        LineEnding::LF
    }
}

pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

pub fn escape_regex(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '[' | ']' | '|' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

/// Strip line-number prefixes like "   45→" that models sometimes include
/// when copying from `Read` output. The `Read` tool returns lines
/// in `cat -n` format, and despite warnings, models occasionally include the
/// `NN→` prefix in `old_string`. This auto-strips it as a safety net.
pub fn strip_line_number_prefixes(s: &str) -> (String, bool) {
    let re = regex::Regex::new(r"(?m)^\s*\d+→\s*").unwrap();
    let stripped = re.replace_all(s, "").to_string();
    let was_stripped = stripped != s;
    (stripped, was_stripped)
}

/// 模糊匹配的归一化等级，从严到松。`calculate_replacement` 按顺序尝试，
/// 第一个命中的等级决定结果 —— 越靠前越接近原文，误匹配风险越低。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchLevel {
    /// 仅去掉每行首尾空白（缩进差异）
    Trim,
    /// 再把行内连续空白压成单个空格（`foo( a , b )` ↔ `foo(a, b)`）
    CollapseWs,
    /// 再把 Unicode 易混字符折叠成 ASCII（弯引号、NBSP、长破折号…）
    Confusable,
}

/// 把模型常写错的 Unicode 字符折叠回 ASCII。
///
/// LLM 复述代码时经常把 `'` 写成 `'`/`'`、把普通空格写成 NBSP、
/// 把 `-` 写成 `–`。这些字符在终端里和 ASCII 几乎一模一样，
/// 人眼看不出差别，exact/trim 匹配却全部失败。
fn fold_confusables(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '\u{00B4}'
            | '\u{FF07}' => out.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' | '\u{FF02}' => {
                out.push('"')
            }
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' | '\u{FF0D}' => out.push('-'),
            '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}' => out.push(' '),
            // 零宽字符：直接丢弃
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
            _ => out.push(ch),
        }
    }
    out
}

/// 行内连续空白压成单个空格，并去掉首尾空白。
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            in_ws = true;
        } else {
            if in_ws {
                out.push(' ');
            }
            in_ws = false;
            out.push(ch);
        }
    }
    out
}

/// 按归一化等级把一行变成用于比较的形式（只用于比较，不写回文件）。
fn normalize_for_match(line: &str, level: MatchLevel) -> String {
    match level {
        MatchLevel::Trim => line.trim().to_string(),
        MatchLevel::CollapseWs => collapse_ws(line),
        MatchLevel::Confusable => collapse_ws(&fold_confusables(line)),
    }
}

/// 一行的前导空白。
fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// 一组行里所有非空行共同的前导空白前缀。
fn common_indent(lines: &[&str]) -> String {
    let mut common: Option<&str> = None;
    for line in lines.iter().filter(|l| !l.trim().is_empty()) {
        let ws = leading_ws(line);
        common = Some(match common {
            None => ws,
            Some(prev) => {
                let n = prev
                    .char_indices()
                    .zip(ws.char_indices())
                    .take_while(|((_, a), (_, b))| a == b)
                    .map(|((i, c), _)| i + c.len_utf8())
                    .last()
                    .unwrap_or(0);
                &prev[..n]
            }
        });
    }
    common.unwrap_or("").to_string()
}

/// 把 `new_string` 的缩进基准换成匹配窗口的缩进基准。
///
/// 关键修复：旧实现是「窗口缩进 + 整行原样」，而 `new_string` 自己已经带了缩进，
/// 于是每次走模糊匹配的编辑都把缩进翻倍（2 空格变 4 空格）。
/// 正确做法是先脱掉 `new_string` 自身的公共缩进，再套上窗口缩进，
/// 这样块内的相对缩进保持不变，模型按文件风格写的正常情况则原样落地。
fn reindent_block(replace_lines: &[&str], window_base: &str) -> Vec<String> {
    let base = common_indent(replace_lines);
    replace_lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                let rest = line
                    .strip_prefix(base.as_str())
                    .unwrap_or(line.trim_start());
                format!("{}{}", window_base, rest)
            }
        })
        .collect()
}

/// 找出所有与 `search` 行序列匹配的窗口起点（按给定归一化等级）。
///
/// 窗口互不重叠：命中后从窗口末尾继续扫，避免自重叠块被重复计数。
fn find_line_windows(source: &[String], search: &[String], level: MatchLevel) -> Vec<usize> {
    // 空 search 或 search 比源文件还长 —— 旧实现在这里直接切片越界 panic
    if search.is_empty() || search.len() > source.len() {
        return Vec::new();
    }
    let search_norm: Vec<String> = search
        .iter()
        .map(|l| normalize_for_match(l, level))
        .collect();

    let mut hits = Vec::new();
    let mut i = 0usize;
    let last_start = source.len() - search.len();
    while i <= last_start {
        let matched =
            (0..search.len()).all(|k| normalize_for_match(&source[i + k], level) == search_norm[k]);
        if matched {
            hits.push(i);
            i += search.len();
        } else {
            i += 1;
        }
    }
    hits
}

/// 把每个命中窗口替换成重新缩进后的 `replace_lines`（从后往前splice，下标不失效）。
fn splice_windows(
    source: &[String],
    windows: &[usize],
    search_len: usize,
    replace_lines: &[&str],
) -> Vec<String> {
    let mut out: Vec<String> = source.to_vec();
    for &start in windows.iter().rev() {
        let window_base = source[start..start + search_len]
            .iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| leading_ws(l).to_string())
            .unwrap_or_default();
        let block = reindent_block(replace_lines, &window_base);
        out.splice(start..start + search_len, block);
    }
    out
}

/// 按行窗口做替换：命中几个就替换几个，`occurrences` 是真实命中数。
///
/// 真实计数很重要 —— 旧的 flexible/regex 实现命中第一个就返回 `occurrences: 1`，
/// 于是块在文件里出现两次时，工具悄悄改了第一处却声称是唯一匹配。
/// 现在多处命中会让 `get_error_replace_result` 的数量守卫拦下来（除非 replace_all）。
pub fn calculate_line_window_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
    level: MatchLevel,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let source_lines: Vec<String> = normalized_code.lines().map(|l| l.to_string()).collect();
    let search_lines: Vec<String> = normalized_search.lines().map(|l| l.to_string()).collect();
    let replace_lines: Vec<&str> = normalized_replace.lines().collect();

    let windows = find_line_windows(&source_lines, &search_lines, level);
    if windows.is_empty() {
        return None;
    }

    let new_lines = splice_windows(&source_lines, &windows, search_lines.len(), &replace_lines);
    Some(ReplacementResult {
        new_content: restore_trailing_newline(current_content, &new_lines.join("\n")),
        occurrences: windows.len(),
        final_old_string: normalized_search,
        final_new_string: normalized_replace,
    })
}

/// 首尾行锚定匹配：块中间漂了几个字符时的兜底。
///
/// 要求同时满足才动手，宁可报错也不要改错地方：
/// 行数一致、首行与末行（折叠易混字符后）匹配、命中窗口唯一、
/// 且中间行至少有一半能对上。
pub fn calculate_anchor_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let source_lines: Vec<String> = normalized_code.lines().map(|l| l.to_string()).collect();
    let search_lines: Vec<String> = normalized_search.lines().map(|l| l.to_string()).collect();
    let replace_lines: Vec<&str> = normalized_replace.lines().collect();

    // 少于 3 行时首尾锚定等于全量匹配，没有额外价值，反而更容易撞错
    if search_lines.len() < 3 || search_lines.len() > source_lines.len() {
        return None;
    }

    let lvl = MatchLevel::Confusable;
    let norm = |s: &str| normalize_for_match(s, lvl);
    let first = norm(&search_lines[0]);
    let last = norm(&search_lines[search_lines.len() - 1]);
    if first.is_empty() || last.is_empty() {
        return None;
    }
    let search_norm: Vec<String> = search_lines.iter().map(|l| norm(l)).collect();

    let mut hits = Vec::new();
    for start in 0..=(source_lines.len() - search_lines.len()) {
        let end = start + search_lines.len() - 1;
        if norm(&source_lines[start]) != first || norm(&source_lines[end]) != last {
            continue;
        }
        let inner = search_lines.len().saturating_sub(2);
        if inner > 0 {
            let same = (1..search_lines.len() - 1)
                .filter(|k| norm(&source_lines[start + k]) == search_norm[*k])
                .count();
            if same * 2 < inner {
                continue;
            }
        }
        hits.push(start);
    }

    // 唯一命中才允许落地
    if hits.len() != 1 {
        return None;
    }
    crate::utils::logging::append_debug_log_line(
        "[Edit] Matched old_string by first/last line anchors (block interior drifted)",
    );

    let new_lines = splice_windows(&source_lines, &hits, search_lines.len(), &replace_lines);
    Some(ReplacementResult {
        new_content: restore_trailing_newline(current_content, &new_lines.join("\n")),
        occurrences: 1,
        final_old_string: normalized_search,
        final_new_string: normalized_replace,
    })
}

pub fn calculate_exact_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let exact_occurrences = normalized_code
        .split(&normalized_search)
        .count()
        .saturating_sub(1);

    if exact_occurrences > 0 {
        let modified_code =
            safe_literal_replace(&normalized_code, &normalized_search, &normalized_replace);
        let modified_code = restore_trailing_newline(current_content, &modified_code);

        Some(ReplacementResult {
            new_content: modified_code,
            occurrences: exact_occurrences,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        })
    } else {
        None
    }
}

/// 忽略每行首尾空白的匹配（缩进差异）。保留公开签名，实现委托给行窗口匹配器。
pub fn calculate_flexible_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    calculate_line_window_replacement(current_content, old_string, new_string, MatchLevel::Trim)
}

/// 最后的兜底：把 old_string 按分隔符切成 token，用 `\s*` 连接成正则。
///
/// 只在前面所有等级都没命中时才用 —— 它对空白完全不敏感，误匹配风险最高。
/// 三处修复：正则编译失败不再静默吞掉（记日志）；命中数按真实数量统计而不是恒为 1；
/// 替换块的缩进按窗口基准重排，不再叠加。
pub fn calculate_regex_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let delimiters = ['(', ')', ':', '[', ']', '{', '}', '>', '<', '='];

    let mut processed_string = normalized_search.clone();
    for delim in delimiters {
        processed_string = processed_string.replace(delim, &format!(" {} ", delim));
    }

    let tokens: Vec<&str> = processed_string.split_whitespace().collect();

    if tokens.is_empty() {
        return None;
    }

    let escaped_tokens: Vec<String> = tokens.iter().map(|t| escape_regex(t)).collect();
    let pattern = escaped_tokens.join(r"\s*");
    // (?m) enables multi-line mode so ^ matches start of each line, not just start of string
    let final_pattern = format!(r"(?m)^([ \t]*){}", pattern);

    // 大块 old_string 生成的模式会超过 regex 默认体积上限，旧实现在这里静默返回 None，
    // 表现成"三种匹配都试过了"却其实没试
    let regex = match regex::RegexBuilder::new(&final_pattern)
        .size_limit(32 * 1024 * 1024)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Edit] regex fallback unavailable ({} tokens): {}",
                tokens.len(),
                e
            ));
            return None;
        }
    };

    let replace_lines: Vec<&str> = normalized_replace.lines().collect();
    let matches: Vec<(usize, usize, String)> = regex
        .captures_iter(&normalized_code)
        .filter_map(|c| {
            let whole = c.get(0)?;
            let indent = c.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            Some((whole.start(), whole.end(), indent))
        })
        .collect();

    if matches.is_empty() {
        return None;
    }

    let mut modified_code = normalized_code.clone();
    for (start, end, indent) in matches.iter().rev() {
        let block = reindent_block(&replace_lines, indent).join("\n");
        modified_code.replace_range(*start..*end, &block);
    }

    Some(ReplacementResult {
        new_content: restore_trailing_newline(current_content, &modified_code),
        occurrences: matches.len(),
        final_old_string: normalized_search,
        final_new_string: normalized_replace,
    })
}

/// 依次尝试各匹配等级，第一个命中的等级决定结果。
///
/// 顺序即安全性排序：越靠后越宽松，所以只有前面全部落空才会往下走。
/// 除 exact 之外的等级都按行窗口替换并统计真实命中数，多处命中时由
/// `get_error_replace_result` 的数量守卫拦下（除非显式 replace_all）。
fn try_all_levels(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    if let Some(result) = calculate_exact_replacement(current_content, old_string, new_string) {
        return Some(result);
    }
    for level in [
        MatchLevel::Trim,
        MatchLevel::CollapseWs,
        MatchLevel::Confusable,
    ] {
        if let Some(result) =
            calculate_line_window_replacement(current_content, old_string, new_string, level)
        {
            if level != MatchLevel::Trim {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Edit] Matched old_string at fuzzy level {:?}",
                    level
                ));
            }
            return Some(result);
        }
    }
    if let Some(result) = calculate_anchor_replacement(current_content, old_string, new_string) {
        return Some(result);
    }
    calculate_regex_replacement(current_content, old_string, new_string)
}

pub fn calculate_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> ReplacementResult {
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    if normalized_search.is_empty() {
        return ReplacementResult {
            new_content: current_content.to_string(),
            occurrences: 0,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        };
    }

    if let Some(result) = try_all_levels(current_content, old_string, new_string) {
        return result;
    }

    // Auto-strip line-number prefixes (e.g. "   45→") that models sometimes
    // accidentally include when copying from read_file output.
    let (stripped_old, was_stripped) = strip_line_number_prefixes(old_string);
    if was_stripped {
        crate::utils::logging::append_debug_log_line(
            "[Edit] Auto-stripped line-number prefix from old_string",
        );
        if let Some(result) = try_all_levels(current_content, &stripped_old, new_string) {
            return result;
        }
    }

    ReplacementResult {
        new_content: current_content.to_string(),
        occurrences: 0,
        final_old_string: normalized_search,
        final_new_string: normalized_replace,
    }
}

pub fn get_error_replace_result(
    params: &EditToolParams,
    occurrences: usize,
    expected_replacements: usize,
    final_old_string: &str,
    final_new_string: &str,
) -> Option<EditError> {
    if occurrences == 0 {
        // Enhanced diagnosis: try to find similar content in the file
        let diagnosis = diagnose_replace_failure(&params.file_path, &params.old_string);
        Some(EditError {
            display: "Failed to edit, could not find the string to replace.".to_string(),
            raw: format!(
                "Failed to edit, 0 occurrences found for old_string in {}. \
                 The string to replace was not found in the file. \
                 All matching strategies were tried and failed: exact, ignore-indentation, \
                 collapse-inner-whitespace, unicode-fold (curly quotes / NBSP / dashes), \
                 first-and-last-line anchors, and whitespace-insensitive regex. \
                 \n\n{}\n\n\
                 Next step: re-read the file with Read to get the current content, then retry with the exact text.",
                params.file_path, diagnosis
            ),
            error_type: ToolErrorType::EditNoOccurrenceFound,
        })
    } else if occurrences != expected_replacements && params.replace_all != Some(true) {
        // replace_all=true 时不校验数量：替换逻辑本身已是全量替换，只需放行计数守卫。
        let occurrence_term = if expected_replacements == 1 {
            "occurrence"
        } else {
            "occurrences"
        };

        Some(EditError {
            display: format!(
                "Failed to edit, expected {} {} but found {}.",
                expected_replacements, occurrence_term, occurrences
            ),
            raw: format!(
                "Failed to edit, Expected {} {} but found {} for old_string in file: {}. \
                 Next step: extend old_string with a few surrounding lines so it identifies exactly \
                 one location, or pass replace_all=true to change every occurrence.",
                expected_replacements, occurrence_term, occurrences, params.file_path
            ),
            error_type: ToolErrorType::EditExpectedOccurrenceMismatch,
        })
    } else if final_old_string == final_new_string {
        let detail = if params.old_string != params.new_string {
            " (they differ only in line endings: old_string uses CRLF, new_string uses LF)"
                .to_string()
        } else {
            String::new()
        };
        Some(EditError {
            display: format!(
                "No changes to apply: old_string and new_string are identical after normalization.{}",
                detail
            ),
            raw: format!(
                "No changes to apply. The old_string ({:?}) and new_string ({:?}) are identical \
                 after line-ending normalization in file: {}. \
                 This means the replacement would not change the file. \
                 Check that old_string and new_string have genuinely different content.",
                params.old_string, params.new_string, params.file_path
            ),
            error_type: ToolErrorType::EditNoChange,
        })
    } else {
        None
    }
}

/// 按字符（而不是字节）截断，避免在多字节字符中间切开导致 panic。
fn clip(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// 分析 replace 失败的原因，给出能直接指导下一步动作的诊断。
///
/// 检查顺序即信息量从高到低：先排除整体性偏差（大小写、空白），
/// 再定位到具体是哪一行对不上 —— 旧实现只会说"首行部分匹配"，
/// 而首行几乎总能匹配上，模型除了把整块重猜一遍别无选择。
pub(crate) fn diagnose_replace_failure(file_path: &str, old_string: &str) -> String {
    let mut diagnosis = String::new();

    // Try to read the file
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            diagnosis.push_str(&format!("Could not read file for diagnosis: {}", e));
            return diagnosis;
        }
    };

    let source_lines: Vec<&str> = content.lines().collect();
    let search_lines: Vec<&str> = old_string.lines().collect();
    let norm = |s: &str| normalize_for_match(s, MatchLevel::Confusable);

    // Check 1: 只差大小写
    if !old_string.trim().is_empty() && content.to_lowercase().contains(&old_string.to_lowercase())
    {
        diagnosis.push_str(
            "DIAGNOSIS: Found a case-insensitive match — old_string differs from the file only in letter case. \
             Suggestion: copy the exact case from the file."
        );
        return diagnosis;
    }

    // Check 2: 空白差异大到连模糊匹配也桥不过去（例如整块换了行）
    let strip_ws = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let old_bare = strip_ws(old_string);
    if !old_bare.is_empty() && strip_ws(&content).contains(&old_bare) {
        diagnosis.push_str(
            "DIAGNOSIS: Found a match after ignoring all whitespace — old_string has different \
             indentation or line breaks than the file. \
             Suggestion: Read the file and copy the block verbatim, including spaces and tabs.",
        );
        return diagnosis;
    }

    // Check 3: 定位第一条在文件里完全找不到的行 —— 这就是真正对不上的那行。
    // 一行都对不上时不走这条：那不是"某行漂了"，而是整段都不在这个文件里。
    let file_norm: std::collections::HashSet<String> =
        source_lines.iter().map(|l| norm(l)).collect();
    let mut culprit: Option<(usize, &str)> = None;
    let (mut found, mut missing) = (0usize, 0usize);
    for (idx, line) in search_lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if file_norm.contains(&norm(line)) {
            found += 1;
        } else {
            missing += 1;
            culprit = culprit.or(Some((idx, line)));
        }
    }

    if let Some((idx, line)) = culprit.filter(|_| found > 0) {
        diagnosis.push_str(&format!(
            "DIAGNOSIS: old_string line {} does not appear anywhere in the file: '{}'. \
             {} other line(s) of old_string were found, {} were not — so the block matches the file \
             at first and then diverges at this line, which was probably paraphrased or came from a \
             stale read. Suggestion: Read the file around the target and copy the block verbatim.",
            idx + 1,
            clip(line.trim(), 80),
            found,
            missing
        ));
        return diagnosis;
    }

    // Check 4: every line exists individually, but not as a contiguous block
    if !search_lines.is_empty() {
        let first = search_lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .map(|l| norm(l));
        if let Some(first) = first {
            let anchors: Vec<usize> = source_lines
                .iter()
                .enumerate()
                .filter(|(_, l)| norm(l) == first)
                .map(|(i, _)| i + 1)
                .collect();
            if !anchors.is_empty() {
                diagnosis.push_str(&format!(
                    "DIAGNOSIS: all lines of old_string exist in the file but not as one contiguous block \
                     (first line matches at line {}). Lines were probably reordered, or a line was dropped \
                     or inserted in the middle. Suggestion: Read that region and copy the block verbatim.",
                    anchors
                        .iter()
                        .take(3)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                return diagnosis;
            }
        }
    }

    // Default diagnosis
    diagnosis.push_str(
        "DIAGNOSIS: No similar content found in the file. \
         Common causes: (1) typo in old_string, (2) file was already modified, \
         (3) wrong file path. Suggestion: Read to get current content.",
    );

    diagnosis
}

pub struct EditTool {
    pub config: Arc<crate::core::config::Config>,
    pub message_bus: Arc<MessageBus>,
    pub global_state: Arc<GlobalState>,
}

impl EditTool {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            message_bus,
            global_state,
        }
    }

    pub fn name(&self) -> &str {
        "Edit"
    }

    pub fn display_name(&self) -> &str {
        "EditFile"
    }

    pub fn description(&self) -> &str {
        "Replaces a string in a file with a new string. This tool requires that the file has been read first to ensure you have the correct context."
    }

    pub fn kind(&self) -> Kind {
        Kind::Edit
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        Self::parameter_schema_json()
    }

    /// 不依赖实例的 schema，便于单元测试与其它调用方复用。
    pub fn parameter_schema_json() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "expected_replacements": { "type": "number" },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence of old_string (default false)."
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }
}

pub struct EditToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: EditToolParams,
    message_bus: Arc<MessageBus>,
    global_state: Arc<GlobalState>,
}

impl EditToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: EditToolParams,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            message_bus,
            global_state,
        }
    }
}

impl crate::core::tools::tools::ToolInvocation for EditToolInvocation {
    fn get_description(&self) -> String {
        format!("Edit file: {}", self.params.file_path)
    }

    fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
        vec![crate::core::tools::tools::ToolLocation {
            path: std::path::PathBuf::from(&self.params.file_path),
            location_type: crate::core::tools::tools::LocationType::Write,
        }]
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
        let config = self.config.clone();
        let path_str = self.params.file_path.clone();
        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let path = std::path::Path::new(&path_str);
                    if !tf.is_path_trusted(path).unwrap_or(false) {
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Edit".to_string(),
                             prompt: format!("Security: Editing file in untrusted path {:?} is blocked. Do you want to proceed?", path),
                             on_confirm: std::sync::Arc::new(move |_outcome| {
                                 // Placeholder for trust logic
                             }),
                         }));
                    }
                }
            }
            Ok(None)
        })
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
        let config = self.config.clone();
        let params = self.params.clone();
        let global_state = self.global_state.clone();

        Box::pin(async move {
            // Re-use logic from original EditTool::execute
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let path = std::path::Path::new(&params.file_path);
                    if !tf.is_path_trusted(path).unwrap_or(false) {
                        let msg = format!("Security: Path {:?} is not in a trusted folder.", path);
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: "SecurityError".to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                }
            }

            // Resolve path consistently with read_file (join with target_dir)
            let resolved_path = config.target_dir().join(&params.file_path);
            let abs_path = resolved_path
                .canonicalize()
                .unwrap_or_else(|_| resolved_path.clone())
                .to_string_lossy()
                .to_string();

            // Disable strict read check if STAR_DISABLE_READ_CHECK is true
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true); // Default to true (strict check enabled)

            if strict_read_check {
                let read_state = global_state.read_file_state.read().await;
                if let Some(file_state) = read_state.get(&abs_path) {
                    // If recorded timestamp was a fallback (0), skip strict modified check
                    if file_state.file_system_timestamp > 0 {
                        if let Ok(metadata) = tokio::fs::metadata(&resolved_path).await {
                            if let Ok(modified) = metadata.modified() {
                                let current_mtime = modified
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis();

                                // Use 2000ms buffer to account for filesystem timestamp granularity
                                if current_mtime > file_state.file_system_timestamp + 2000 {
                                    // Mtime changed — but the content may still be the same (e.g.
                                    // cargo check metadata writes, filesystem timestamp quirks,
                                    // touch, or assistant's own prior edits via bash/write_file).
                                    // Read current content and compare before blocking.
                                    let content_changed = match crate::core::utils::file_utils::read_file_with_encoding_async(&resolved_path).await {
                                        Ok(current_content) => current_content != file_state.content,
                                        Err(_) => true, // can't read — assume changed
                                    };
                                    if content_changed {
                                        let msg = format!("File '{}' has been modified since you last read it. Please read the file again to ensure you are editing the latest version.", params.file_path);
                                        return Ok(ToolResult {
                                            llm_content: Some(msg.clone()),
                                            return_display: Some(format!("Error: {}", msg)),
                                            output: msg.clone(),
                                            error: Some(ToolError {
                                                error_type: ToolErrorType::EditFileModified
                                                    .to_string(),
                                                message: msg,
                                            }),
                                            data: None,
                                        });
                                    }
                                    // Content unchanged — update the recorded timestamp to suppress
                                    // future false positives, then proceed with the edit.
                                    let mut state = global_state.read_file_state.write().await;
                                    if let Some(fs) = state.get_mut(&abs_path) {
                                        fs.file_system_timestamp = current_mtime;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Not in state
                    // Allow if file doesn't exist (creating new file)
                    if std::path::Path::new(&params.file_path).exists() {
                        let msg = format!(
                            "Edit blocked [edit_file_not_read]: file '{}' must be read with `Read` before using `replace`. \
                             REQUIRED NEXT STEP: call `Read` with file_path='{}' first, then retry. \
                             Do NOT retry without reading the file first.",
                            params.file_path, params.file_path
                        );
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: ToolErrorType::EditFileNotRead.to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                }
            }

            // Early check: if old_string and new_string are identical after line-ending normalization,
            // there's nothing to do. This catches LLM mistakes before unnecessary file I/O.
            {
                let normalized_old = normalize_line_endings(&params.old_string);
                let normalized_new = normalize_line_endings(&params.new_string);
                if normalized_old == normalized_new {
                    let msg = if params.old_string != params.new_string {
                        "No changes to apply: old_string and new_string differ only in line endings (CRLF vs LF). \
                         After normalizing \\r\\n→\\n they are identical. \
                         Ensure old_string and new_string have different content, not just different line endings.".to_string()
                    } else {
                        "No changes to apply: old_string and new_string are identical. \
                         You must provide different old_string (text to find) and new_string (replacement text).".to_string()
                    };
                    return Ok(ToolResult {
                        llm_content: Some(msg.clone()),
                        return_display: Some(msg.clone()),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::EditNoChange.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }
            }

            // File-history checkpoint: snapshot the file BEFORE we edit it.
            // track_edit is async (awaits IO), so it must run BEFORE
            // spawn_blocking captures `params`. Failures are best-effort —
            // they must never block the edit.
            {
                let msg_id = global_state.current_message_id().await;
                let edit_file_path = std::path::Path::new(&params.file_path).to_path_buf();
                if let Err(e) = crate::utils::checkpoint_manager::track_edit(
                    &edit_file_path,
                    msg_id,
                    Some("edit"),
                    None, // session_id: per-cwd fallback, matches /undo and /rewind
                )
                .await
                {
                    log::warn!(
                        "FileHistory: track_edit failed for {}: {}",
                        params.file_path,
                        e
                    );
                }
            }

            let result = tokio::task::spawn_blocking(move || {
                let expected_replacements = params.expected_replacements.unwrap_or(1);

                let mut current_content: Option<String> = None;
                let mut _file_exists = false;
                let mut original_line_ending = LineEnding::LF;

                match crate::core::utils::file_utils::read_file_with_encoding_io(
                    std::path::Path::new(&params.file_path),
                ) {
                    Ok(content) => {
                        original_line_ending = detect_line_ending(&content);
                        current_content = Some(normalize_line_endings(&content));
                        _file_exists = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        _file_exists = false;
                    }
                    Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                }

                let is_new_file = params.old_string.is_empty() && !_file_exists;

                if is_new_file {
                    let line_count = params.new_string.lines().count();
                    let msg = format!("Wrote {} lines to {}", line_count, params.file_path);
                    // Create parent directory if needed
                    if let Some(parent) = Path::new(&params.file_path).parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    }
                    // Atomic write: write to temp then rename
                    let tmp_path = format!("{}.star_tmp", &params.file_path);
                    std::fs::write(&tmp_path, &params.new_string)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    std::fs::rename(&tmp_path, &params.file_path)
                        .or_else(|_| {
                            std::fs::copy(&tmp_path, &params.file_path)
                                .map(|_| ())
                                .and_then(|_| std::fs::remove_file(&tmp_path))
                        })
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                    let line_count = params.new_string.lines().count();
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Wrote {} lines to {}",
                            line_count, params.file_path
                        )),
                        return_display: Some(format!(
                            "Wrote {} lines to {}",
                            line_count, params.file_path
                        )),
                        output: msg.clone(),
                        error: None,
                        data: None,
                    });
                }

                if !_file_exists {
                    let msg = format!("File not found: {}", params.file_path);
                    return Ok(ToolResult {
                        llm_content: Some(format!("File not found: {}", params.file_path)),
                        return_display: Some(
                            "Error: File not found. Cannot apply edit.".to_string(),
                        ),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::FileNotFound.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let current = current_content.as_ref().unwrap();

                if params.old_string.is_empty() {
                    let msg = format!("File already exists, cannot create: {}", params.file_path);
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "File already exists, cannot create: {}",
                            params.file_path
                        )),
                        return_display: Some("Error: File already exists.".to_string()),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::AttemptToCreateExistingFile.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let replacement_result =
                    calculate_replacement(current, &params.old_string, &params.new_string);

                if let Some(error) = get_error_replace_result(
                    &params,
                    replacement_result.occurrences,
                    expected_replacements,
                    &replacement_result.final_old_string,
                    &replacement_result.final_new_string,
                ) {
                    let msg = error.raw.to_string();
                    return Ok(ToolResult {
                        llm_content: Some(msg.clone()),
                        return_display: Some(format!("Error: {}", error.display)),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: error.error_type.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let mut final_content = replacement_result.new_content.clone();

                if !is_new_file && original_line_ending == LineEnding::CRLF {
                    final_content = final_content.replace('\n', "\r\n");
                }

                if let Some(parent) = Path::new(&params.file_path).parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                }

                // Atomic write: write to temp then rename
                let tmp_path = format!("{}.star_tmp", &params.file_path);
                std::fs::write(&tmp_path, &final_content)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                std::fs::rename(&tmp_path, &params.file_path)
                    .or_else(|_| {
                        std::fs::copy(&tmp_path, &params.file_path)
                            .map(|_| ())
                            .and_then(|_| std::fs::remove_file(&tmp_path))
                    })
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                // Generate Diff
                let diff = TextDiff::from_lines(current, &final_content);
                let diff_output = format!(
                    "{}",
                    diff.unified_diff()
                        .header(&params.file_path, &params.file_path)
                );

                let added = params.new_string.lines().count();
                let removed = params.old_string.lines().count();
                let msg = if replacement_result.occurrences == 1 {
                    format!(
                        "Updated {} (+{} -{})",
                        params.file_path,
                        added.saturating_sub(removed),
                        removed.saturating_sub(added)
                    )
                } else {
                    format!(
                        "Updated {} ({} replacements, +{} -{})",
                        params.file_path,
                        replacement_result.occurrences,
                        added.saturating_sub(removed),
                        removed.saturating_sub(added)
                    )
                };
                Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some(format!(
                        "Modified {} ({} replacements)",
                        params.file_path, replacement_result.occurrences
                    )),
                    output: msg,
                    error: None,
                    data: Some(serde_json::json!({
                        "diff": diff_output
                    })),
                })
            })
            .await;

            match result {
                Ok(inner_result) => {
                    // Update ReadFileState after successful edit so subsequent edits don't see stale mtime
                    if inner_result.is_ok()
                        && inner_result
                            .as_ref()
                            .map(|r| r.error.is_none())
                            .unwrap_or(false)
                    {
                        let file_system_timestamp = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let timestamp = file_system_timestamp;

                        // Read current content for the state update
                        if let Ok(content) =
                            crate::core::utils::file_utils::read_file_with_encoding_async(
                                &resolved_path,
                            )
                            .await
                        {
                            let mut state = global_state.read_file_state.write().await;
                            state.insert(
                                abs_path.clone(),
                                ReadFileState {
                                    content,
                                    timestamp,
                                    file_system_timestamp,
                                },
                            );
                        }
                    }

                    inner_result.map_err(|e| e as Box<dyn std::error::Error>)
                }
                Err(e) => Err(Box::new(e)),
            }
        })
    }
}

impl BaseDeclarativeTool for EditTool {
    fn name(&self) -> &str {
        EditTool::name(self)
    }

    fn display_name(&self) -> &str {
        EditTool::display_name(self)
    }

    fn description(&self) -> &str {
        EditTool::description(self)
    }

    fn kind(&self) -> Kind {
        EditTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        EditTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<
        Box<dyn crate::core::tools::tools::ToolInvocation>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let edit_params: EditToolParams = serde_json::from_value(params)?;
        Ok(Box::new(EditToolInvocation::new(
            self.config.clone(),
            edit_params,
            self.message_bus.clone(),
            self.global_state.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(replace_all: Option<bool>) -> EditToolParams {
        EditToolParams {
            file_path: "/tmp/does-not-need-to-exist.rs".to_string(),
            old_string: "oldName".to_string(),
            new_string: "newName".to_string(),
            expected_replacements: None,
            replace_all,
            instruction: None,
            modified_by_user: None,
            ai_proposed_content: None,
        }
    }

    #[test]
    fn count_mismatch_is_an_error_without_replace_all() {
        let err = get_error_replace_result(&params(None), 3, 1, "oldName", "newName")
            .expect("3 occurrences vs expected 1 必须报错");
        assert_eq!(
            err.error_type,
            ToolErrorType::EditExpectedOccurrenceMismatch
        );
    }

    #[test]
    fn replace_all_suppresses_count_mismatch() {
        assert!(
            get_error_replace_result(&params(Some(true)), 3, 1, "oldName", "newName").is_none(),
            "replace_all=true 时不应再校验替换数量"
        );
    }

    #[test]
    fn replace_all_still_reports_zero_occurrences() {
        let err = get_error_replace_result(&params(Some(true)), 0, 1, "oldName", "newName")
            .expect("0 occurrences 必须报错，即使 replace_all=true");
        assert_eq!(err.error_type, ToolErrorType::EditNoOccurrenceFound);
    }

    #[test]
    fn replace_all_is_advertised_in_the_schema() {
        let schema = EditTool::parameter_schema_json();
        assert!(
            schema["properties"]["replace_all"].is_object(),
            "tool-description-edit.md 承诺了 replace_all，schema 必须暴露它"
        );
    }

    #[test]
    fn replace_all_deserializes_from_llm_arguments() {
        let parsed: EditToolParams = serde_json::from_str(
            r#"{"file_path":"a.rs","old_string":"x","new_string":"y","replace_all":true}"#,
        )
        .expect("参数解析失败");
        assert_eq!(parsed.replace_all, Some(true));
    }

    // ---- 匹配级联的回归测试 ----------------------------------------------
    // 这些用例来自真实失败：模型复述代码时改了空白或引号，
    // 旧实现要么报 0 occurrences，要么悄悄把缩进翻倍。

    /// 2 空格缩进的样板文件，覆盖大部分回归场景
    const FILE: &str = "class Api {\n  async testAlert(): Promise<Alert> {\n    return this.get('/alert');\n  }\n}\n";

    fn expect_edit(file: &str, old: &str, new: &str) -> ReplacementResult {
        let result = calculate_replacement(file, old, new);
        assert!(
            result.occurrences > 0,
            "匹配失败（occurrences=0），old_string=\n{:?}",
            old
        );
        result
    }

    #[test]
    fn exact_match_is_preferred_and_untouched() {
        let r = expect_edit(
            FILE,
            "    return this.get('/alert');",
            "    return this.post('/alert');",
        );
        assert_eq!(r.occurrences, 1);
        assert_eq!(
            r.new_content,
            FILE.replace("this.get('/alert');", "this.post('/alert');")
        );
    }

    #[test]
    fn fuzzy_match_does_not_double_indent() {
        // 回归：旧实现是「窗口缩进 + 整行原样」，new_string 自带缩进时每次编辑都把缩进翻倍。
        // 这里用弯引号迫使走模糊匹配，new_string 按文件风格带了 2 空格缩进。
        let old = "  async testAlert(): Promise<Alert> {\n    return this.get(\u{2018}/alert\u{2019});\n  }";
        let new = "  async testAlert(kind: string): Promise<Alert> {\n    return this.get('/alert/' + kind);\n  }";
        let r = expect_edit(FILE, old, new);
        assert_eq!(r.occurrences, 1);
        assert_eq!(
            r.new_content,
            "class Api {\n  async testAlert(kind: string): Promise<Alert> {\n    return this.get('/alert/' + kind);\n  }\n}\n",
            "缩进被翻倍或丢失"
        );
    }
    #[test]
    fn dedented_old_string_keeps_relative_indent() {
        // 模型把块整体贴平（丢了外层缩进），替换块也是贴平的 —— 内部相对缩进必须保住
        let old = "async testAlert(): Promise<Alert> {\nreturn this.get('/alert');\n}";
        let new = "async testAlert(): Promise<Alert> {\n  return this.post('/alert');\n}";
        let r = expect_edit(FILE, old, new);
        assert_eq!(
            r.new_content,
            "class Api {\n  async testAlert(): Promise<Alert> {\n    return this.post('/alert');\n  }\n}\n"
        );
    }

    #[test]
    fn curly_quotes_still_match() {
        // 回归：exact/trim 三个 pass 全灭，因为 ' 被写成 U+2019
        let old = "return this.get(\u{2018}/alert\u{2019});";
        let r = expect_edit(FILE, old, "return this.get('/v2/alert');");
        assert!(r.new_content.contains("this.get('/v2/alert');"));
    }

    #[test]
    fn nbsp_and_zero_width_still_match() {
        // NBSP / 表意空格 / 零宽空格在终端里和普通空格一模一样，模型分不清
        let old = "return\u{00A0}this.get('/alert');\u{200B}";
        let r = expect_edit(FILE, old, "return this.get('/v3/alert');");
        assert!(r.new_content.contains("this.get('/v3/alert');"));
        assert!(!r.new_content.contains('\u{00A0}'), "NBSP 被写回了文件");
    }

    #[test]
    fn collapsed_inner_whitespace_still_matches() {
        let old = "return  this.get('/alert');"; // 行内多了一个空格
        let r = expect_edit(FILE, old, "return this.get('/v4/alert');");
        assert!(r.new_content.contains("this.get('/v4/alert');"));
    }

    #[test]
    fn tab_indentation_still_matches_space_indentation() {
        let old = "\tasync testAlert(): Promise<Alert> {\n\t\treturn this.get('/alert');\n\t}";
        let new = "async testAlert(): Promise<Alert> {\n  return this.head('/alert');\n}";
        let r = expect_edit(FILE, old, new);
        assert!(r
            .new_content
            .contains("\n    return this.head('/alert');\n"));
        assert!(!r.new_content.contains('\t'), "制表符被写回了文件");
    }
    #[test]
    fn anchor_match_recovers_drifted_interior() {
        // 首尾行对得上、中间某行漂了 —— 兜底靠首尾锚定，但要求过半中间行匹配
        let file = "class Api {\n  async testAlert(): Promise<Alert> {\n    const url = '/api/alert';\n    return this.get(url);\n  }\n}\n";
        let old = "  async testAlert(): Promise<Alert> {\n    const url = '/api/alerts';\n    return this.get(url);\n  }";
        let new = "  async testAlert(): Promise<Alert> {\n    return this.get('/api/alert');\n  }";
        let r = expect_edit(file, old, new);
        assert_eq!(r.occurrences, 1);
        assert_eq!(
            r.new_content,
            "class Api {\n  async testAlert(): Promise<Alert> {\n    return this.get('/api/alert');\n  }\n}\n"
        );
    }

    #[test]
    fn anchor_match_refuses_when_interior_is_mostly_different() {
        // 中间行全不一样时宁可报错也不能猜 —— 首尾行是 `{` / `}` 这种到处都有的字符
        let file = "fn a() {\n    one();\n    two();\n}\n";
        let old = "fn a() {\n    nine();\n    ten();\n}";
        let r = calculate_replacement(file, old, "fn a() {\n    zero();\n}");
        assert_eq!(r.occurrences, 0, "中间行大面积不匹配时不应落地");
    }

    #[test]
    fn ambiguous_fuzzy_match_reports_true_occurrence_count() {
        // 回归：旧实现命中第一处就返回 occurrences=1，悄悄改了两处中的一处
        let file = "class A {\n  run() {\n    work();\n  }\n}\nclass B {\n    run() {\n      work();\n    }\n}\n";
        let old = "run() {\nwork();\n}";
        let r = calculate_replacement(file, old, "run() {\n  work(1);\n}");
        assert_eq!(r.occurrences, 2, "两处同形块必须如实上报");
        let err = get_error_replace_result(
            &params(None),
            r.occurrences,
            1,
            &r.final_old_string,
            &r.final_new_string,
        )
        .expect("多处命中且未开 replace_all 必须报错");
        assert_eq!(
            err.error_type,
            ToolErrorType::EditExpectedOccurrenceMismatch
        );
    }

    #[test]
    fn old_string_longer_than_file_does_not_panic() {
        // 回归：source_lines[i..i + search_len] 越界 panic
        // （"range end index 30 out of range for slice of length 17"）
        let old: String = (0..30).map(|i| format!("line {}\n", i)).collect();
        let r = calculate_replacement("a\nb\nc\n", &old, "x\n");
        assert_eq!(r.occurrences, 0);
    }
    #[test]
    fn empty_old_string_is_reported_as_no_match() {
        let r = calculate_replacement(FILE, "", "x");
        assert_eq!(r.occurrences, 0);
        assert_eq!(r.new_content, FILE);
    }

    #[test]
    fn read_style_line_number_prefixes_are_stripped() {
        // 模型经常把 Read 的输出（"  12→code"）整段粘进 old_string
        let old = "  2→  async testAlert(): Promise<Alert> {\n  3→    return this.get('/alert');\n  4→  }";
        let new = "  async testAlert(): Promise<Alert> {\n    return this.head('/alert');\n  }";
        let r = expect_edit(FILE, old, new);
        assert!(r
            .new_content
            .contains("\n    return this.head('/alert');\n"));
    }

    // ---- 失败诊断 ---------------------------------------------------------

    fn temp_file(tag: &str, content: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("starcode-edit-{}-{}.txt", std::process::id(), tag));
        std::fs::write(&path, content).expect("写入临时文件失败");
        path
    }

    #[test]
    fn clip_never_splits_a_multibyte_char() {
        assert_eq!(clip("这是一段中文", 3), "这是一…");
        assert_eq!(clip("这是一段中文", 99), "这是一段中文");
    }

    #[test]
    fn cjk_old_string_does_not_panic_in_diagnosis() {
        // 回归：旧实现按字节切半（&old_trimmed[..len/2]），CJK 必然切在字符中间 panic。
        // 这里是 25 个汉字 = 75 字节，字节中点 37 不是字符边界。
        let path = temp_file("cjk", "fn main() {\n    println!(\"hi\");\n}\n");
        let old = "这是一段完全不存在于文件里的中文注释内容需要被替换";
        let d = diagnose_replace_failure(path.to_str().unwrap(), old);
        assert!(d.contains("DIAGNOSIS"), "诊断为空: {}", d);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diagnosis_names_the_divergent_line() {
        // 旧诊断只说"首行部分匹配"，而首行本来就是对的 —— 错的是第 2 行
        let path = temp_file("divergent", FILE);
        let old = "class Api {\n  async testAlarm(): Promise<Alarm> {\n";
        let d = diagnose_replace_failure(path.to_str().unwrap(), old);
        assert!(d.contains("line 2"), "诊断必须点名对不上的行: {}", d);
        assert!(d.contains("testAlarm"), "诊断必须回显那一行的原文: {}", d);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn diagnosis_flags_a_non_contiguous_block() {
        let path = temp_file("noncontig", "let a = 1;\nlet b = 2;\nlet c = 3;\n");
        let d = diagnose_replace_failure(path.to_str().unwrap(), "let a = 1;\nlet c = 3;\n");
        assert!(d.contains("contiguous"), "应指出块不连续: {}", d);
        assert!(d.contains("line 1"), "应给出首行位置: {}", d);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diagnosis_flags_case_only_difference() {
        let path = temp_file("case", "const Timeout = 30;\n");
        let d = diagnose_replace_failure(path.to_str().unwrap(), "const timeout = 30;");
        assert!(d.contains("case-insensitive"), "{}", d);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diagnosis_falls_back_when_nothing_is_similar() {
        let path = temp_file("nothing", "fn main() {}\n");
        let d = diagnose_replace_failure(path.to_str().unwrap(), "impl Display for Widget {}");
        assert!(d.contains("No similar content"), "{}", d);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn diagnosis_reports_unreadable_file_instead_of_panicking() {
        let d = diagnose_replace_failure("/nonexistent/starcode/does-not-exist.rs", "anything");
        assert!(d.contains("Could not read file"), "{}", d);
    }
}
