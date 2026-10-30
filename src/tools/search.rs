use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::ripgrep::{
    display_path, format_grep_hits, plural_count, GrepHit, DEFAULT_MAX_RESULTS,
};
use crate::core::tools::tools::{FileSearchResult, SearchResult, UnifiedSearchResult};
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};
use crate::types::ToolResult;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SearchTool {
    current_directory: String,
    global_state: Arc<GlobalState>,
}

impl SearchTool {
    pub fn new(global_state: Arc<GlobalState>) -> Self {
        Self {
            current_directory: std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .to_string_lossy()
                .to_string(),
            global_state,
        }
    }

    fn base_dir(&self) -> String {
        let raw_base_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| self.current_directory.clone());
        normalize_existing_search_dir(&raw_base_dir)
    }

    pub async fn search(
        &self,
        query: &str,
        search_type: Option<String>,
        path: Option<String>,
        output_mode: Option<String>,
        include_pattern: Option<String>,
        exclude_pattern: Option<String>,
        case_sensitive: Option<bool>,
        whole_word: Option<bool>,
        regex: Option<bool>,
        max_results: Option<u32>,
        file_types: Option<Vec<String>>,
        exclude_files: Option<Vec<String>>,
        include_hidden: Option<bool>,
    ) -> Result<ToolResult, Box<dyn std::error::Error>> {
        let output_mode = output_mode.unwrap_or_else(|| "content".to_string());
        // 默认只做一件事：`@` 开头明显是在找文件，否则只搜内容。
        //
        // 以前默认 `"both"` —— 一次纯内容搜索也会顺带跑一遍文件名搜索，再把最多
        // 20 行文件路径贴进上下文（"📂 Matched Files" 那一节）。对"这个词在哪
        // 用过"这类问题，那一节基本没有信息量，纯粹烧 token。要文件名列表就显式
        // 传 `search_type` 或 `output_mode: "files_with_matches"`。
        let search_type = search_type.unwrap_or_else(|| {
            if query.starts_with('@') {
                "files".to_string()
            } else {
                "text".to_string()
            }
        });
        let base_dir = self.resolve_search_base_dir(path.as_deref());
        // If the caller supplied a file path instead of a directory, scope the search to
        // that file's parent directory and constrain include_pattern to the filename.
        let (base_dir, include_pattern) = {
            let p = Path::new(&base_dir);
            if p.is_file() {
                let parent = p
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_else(|| self.base_dir());
                let filename = p
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                (parent, Some(filename))
            } else {
                (base_dir, include_pattern)
            }
        };
        if !Path::new(&base_dir).is_dir() {
            return Ok(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Search path is not a directory: {}", base_dir)),
                data: None,
                llm_content: None,
            });
        }
        let mut results = Vec::new();
        // 模式被"窄修复"过时的说明（见下面的 try_repair_pattern）。
        let mut pattern_note: Option<String> = None;
        // dotfile 默认可见 —— 对齐 Claude Code。以前只有 `@` 开头的查询才
        // 看得到 `.github/`、`.env.example` 这类真正需要搜的文件。
        let include_hidden_default = true;
        let include_hidden_flag = include_hidden.unwrap_or_else(|| {
            std::env::var("STAR_SEARCH_INCLUDE_HIDDEN")
                .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(include_hidden_default)
        });
        let should_search_files =
            search_type == "files" || search_type == "both" || query.starts_with('@');

        if search_type == "text" || search_type == "both" {
            let file_types_ref = file_types.as_ref();
            let exclude_files_ref = exclude_files.as_ref();

            // 正则模式下先做一次**窄修复**：模型想搜字面量左括号时打成 `\(`，
            // 而 `\(` 在 JSON 里是非法转义、反斜杠会被丢掉，发过来就成了
            // `func Relay(c|relay.Relay` —— 第 12 列的 `(` 没配对，整个正则编译
            // 失败，一次搜索颗粒无收。修法只动没配对的括号，其余原样。
            let mut effective_query = query.to_string();
            if regex.unwrap_or(false) {
                if let Some(repaired) = crate::core::tools::ripgrep::try_repair_pattern(query) {
                    pattern_note = Some(format!(
                        "Unbalanced parenthesis in pattern, escaped as a literal and retried: {}",
                        repaired
                    ));
                    effective_query = repaired;
                }
            }

            // 错误必须冒出来。以前这里是 `if let Ok(...)` —— 正则编译失败
            // （`RegexMatcherBuilder::build` 对非法 pattern 返回 `Err`）被整个
            // 吞掉，然后落到下面的 `"No results found"`。于是一个写错的正则和
            // "确实没有匹配"在界面上完全一样，模型只会一遍遍换个写法重试。
            let text_results = self
                .execute_ripgrep(
                    &effective_query,
                    &base_dir,
                    include_pattern.as_deref(),
                    exclude_pattern.as_deref(),
                    case_sensitive,
                    whole_word,
                    regex,
                    max_results,
                    file_types_ref.map(|v| v),
                    exclude_files_ref.map(|v| v),
                    include_hidden_flag,
                )
                .await;

            let text_results = match text_results {
                Ok(results) => results,
                Err(error) => {
                    let message = describe_pattern_error(query, &error.to_string());
                    return Ok(ToolResult {
                        success: false,
                        output: Some(message.clone()),
                        error: Some(message),
                        data: None,
                        llm_content: None,
                    });
                }
            };

            results.extend(text_results.into_iter().map(|r| UnifiedSearchResult {
                result_type: "text".to_string(),
                file: r.file,
                line: r.line,
                column: r.column,
                text: r.text,
                match_content: r.match_content,
                score: None,
            }));
        }

        if should_search_files {
            let file_query = {
                let trimmed = query.trim_start_matches('@');
                if trimmed.is_empty() {
                    ""
                } else {
                    trimmed
                }
            };

            let file_max_depth = std::env::var("STAR_SEARCH_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(256usize);

            if let Ok(file_results) = self
                .find_files_by_pattern(
                    file_query,
                    max_results,
                    Some(include_hidden_flag),
                    exclude_pattern.as_deref(),
                    &base_dir,
                    file_max_depth,
                )
                .await
            {
                let file_results_formatted: Vec<UnifiedSearchResult> = file_results
                    .into_iter()
                    .map(|r| UnifiedSearchResult {
                        result_type: "file".to_string(),
                        file: r.path,
                        line: None,
                        column: None,
                        text: None,
                        match_content: None,
                        score: Some(r.score),
                    })
                    .collect();
                results.extend(file_results_formatted);
            }
        }

        if results.is_empty() {
            let mut output = format!("No results found for \"{}\"", query);
            if let Some(note) = &pattern_note {
                output = format!("{}\n{}", note, output);
            }
            return Ok(ToolResult {
                success: true,
                output: Some(output),
                error: None,
                data: None,
                llm_content: None,
            });
        }

        let formatted_output = match output_mode.as_str() {
            "files_with_matches" => self.format_files_with_matches(&results, query),
            "count" => self.format_counts(&results, query),
            _ => self.format_unified_results(&results, query, &search_type),
        };
        let formatted_output = match &pattern_note {
            Some(note) => format!("{}\n{}", note, formatted_output),
            None => formatted_output,
        };

        Ok(ToolResult {
            success: true,
            output: Some(formatted_output),
            error: None,
            data: Some(serde_json::to_value(&results)?),
            llm_content: None,
        })
    }

    async fn execute_ripgrep(
        &self,
        query: &str,
        base_dir: &str,
        include_pattern: Option<&str>,
        exclude_pattern: Option<&str>,
        case_sensitive: Option<bool>,
        whole_word: Option<bool>,
        regex: Option<bool>,
        max_results: Option<u32>,
        file_types: Option<&Vec<String>>,
        exclude_files: Option<&Vec<String>>,
        include_hidden: bool,
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        let mut config = crate::core::tools::ripgrep::RipgrepConfig {
            case_sensitive: case_sensitive.unwrap_or(false),
            whole_word: whole_word.unwrap_or(false),
            regex: regex.unwrap_or(false),
            // 必须有上限。以前这里直接传 `None`，而 ripgrep 把 `None` 当
            // `u32::MAX` —— 一次宽泛的 pattern 就能把上千条匹配灌进上下文。
            max_results: Some(max_results.unwrap_or(DEFAULT_MAX_RESULTS)),
            file_types: file_types.cloned(),
            exclude_patterns: None,
            include_patterns: include_pattern.map(|p| vec![p.to_string()]),
            // `include_pattern` 由 `tool_executor::normalize_search_args` 生成，
            // 而 `normalize_search_scope` 会把绝对路径剥成相对 cwd —— 所以这个
            // glob 是相对 **cwd** 写的，不是相对 `base_dir`。
            //
            // `path` 已经被 `resolve_search_base_dir` 用来把 walker 的根收窄成
            // `<cwd>/src` 了；两个基再不一致，`Grep(pattern, path="src")` 会一个
            // 文件都匹配不上（结果恒为空，且不报错）。
            include_base: Some(crate::core::utils::paths::current_dir_cached().clone()),
            include_hidden,
            max_file_size: Some(2 * 1024 * 1024),
        };

        let mut excludes = Vec::new();
        if let Some(pattern) = exclude_pattern {
            excludes.push(pattern.to_string());
        }
        if let Some(files) = exclude_files {
            excludes.extend(files.iter().cloned());
        }
        if !excludes.is_empty() {
            config.exclude_patterns = Some(excludes);
        }

        let results = crate::core::tools::ripgrep::search_with_ripgrep(query, &base_dir, config)?;

        Ok(results)
    }

    fn resolve_search_base_dir(&self, path: Option<&str>) -> String {
        let Some(raw_path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
            return self.base_dir();
        };

        let candidate = normalize_input_path(raw_path);
        if candidate.is_absolute() {
            return candidate.to_string_lossy().to_string();
        }

        Path::new(&self.base_dir())
            .join(candidate)
            .to_string_lossy()
            .to_string()
    }

    async fn find_files_by_pattern(
        &self,
        pattern: &str,
        max_results: Option<u32>,
        include_hidden: Option<bool>,
        exclude_pattern: Option<&str>,
        base_dir: &str,
        max_depth: usize,
    ) -> Result<Vec<FileSearchResult>, Box<dyn std::error::Error>> {
        let max_results = max_results.unwrap_or(300) as usize;
        let mut files = Vec::new();
        let search_pattern = pattern.to_lowercase();

        // 以前这里是 `WalkDir` + `should_skip_walk_entry`，后者拿**绝对路径**
        // 逐个组件比对硬编码黑名单：仓库放在 `/mnt/h/test/starcode` 这种路径下
        // 时，路径里的 `build`/`dist`/`target` 一命中，整个仓库就被跳过了。
        // 现在走统一 walker：`.gitignore` 生效，判断只针对 root 以下的相对路径。
        let root = Path::new(base_dir);
        let mut opts = crate::utils::file_walk::WalkOptions::new()
            .hidden(include_hidden.unwrap_or(true))
            .max_depth(max_depth);
        if let Some(exclude) = exclude_pattern.map(str::trim).filter(|p| !p.is_empty()) {
            opts = opts.exclude([exclude]);
        }

        for entry in crate::utils::file_walk::walk(root, &opts).flatten() {
            let file_path = entry.path();
            if file_path == root {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();

            let relative_path = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let relative_path = if relative_path.is_empty() {
                file_path.to_string_lossy().to_string()
            } else {
                relative_path
            };

            let _score = calculate_file_score(&file_name, &relative_path, &search_pattern);
            files.push((relative_path, file_name));

            if files.len() >= max_results {
                break;
            }
        }

        let mut file_results: Vec<FileSearchResult> = files
            .into_iter()
            .map(|(path, name)| {
                let score = calculate_file_score(&name, &path, &pattern.to_lowercase());
                FileSearchResult { path, name, score }
            })
            .collect();

        file_results.sort_by(|a, b| b.score.cmp(&a.score));
        file_results.truncate(max_results);

        Ok(file_results)
    }

    /// 文本匹配 + 命中的文件名，压成紧凑文本。
    ///
    /// 三条硬约束（都是 token 账）：
    /// 1. 路径相对 cwd，不再每个文件重复一遍绝对路径前缀；
    /// 2. 一行一条，行内容截断，每文件最多几行，其余折成 `+N more`；
    /// 3. 整体有字符预算，超了就截断并提示收窄搜索范围。
    fn format_unified_results(
        &self,
        results: &[UnifiedSearchResult],
        query: &str,
        _search_type: &str,
    ) -> String {
        if results.is_empty() {
            return format!("No results found for \"{}\"", query);
        }

        let base_dir = PathBuf::from(crate::core::utils::paths::current_dir_cached().clone());
        let mut output = String::new();

        let file_results: Vec<&UnifiedSearchResult> =
            results.iter().filter(|r| r.result_type == "file").collect();
        if !file_results.is_empty() {
            output.push_str(&format!(
                "{} matching by name:\n",
                plural_count(file_results.len(), "file")
            ));
            for result in file_results.iter().take(20) {
                output.push_str(&format!("  {}\n", display_path(&result.file, &base_dir)));
            }
            if file_results.len() > 20 {
                output.push_str(&format!("  +{} more\n", file_results.len() - 20));
            }
        }

        let text_hits: Vec<GrepHit<'_>> = results
            .iter()
            .filter(|r| r.result_type == "text")
            .map(|r| GrepHit {
                file: &r.file,
                line: r.line,
                // 注意：取 `text`（命中行的原文），不是 `match_content`
                // —— 后者存的是 pattern 本身，取错会让每一行都把正则重复一遍。
                text: r.text.as_deref(),
            })
            .collect();
        if !text_hits.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format_grep_hits(&text_hits, &base_dir, query));
        }

        output.trim_end().to_string()
    }

    /// 只回"每个文件命中几处"：判断改动面时用，最省的一种输出。
    fn format_counts(&self, results: &[UnifiedSearchResult], query: &str) -> String {
        let text_results: Vec<&UnifiedSearchResult> =
            results.iter().filter(|r| r.result_type == "text").collect();
        if text_results.is_empty() {
            return format!("No matches found for \"{}\"", query);
        }

        let base_dir = PathBuf::from(crate::core::utils::paths::current_dir_cached().clone());
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for r in &text_results {
            *counts.entry(r.file.as_str()).or_default() += 1;
        }

        let mut output = format!(
            "{} in {} for \"{}\"\n",
            plural_count(text_results.len(), "match"),
            plural_count(counts.len(), "file"),
            query
        );
        for (file, count) in counts {
            output.push_str(&format!("{}: {}\n", display_path(file, &base_dir), count));
        }
        output.trim_end().to_string()
    }

    fn format_files_with_matches(&self, results: &[UnifiedSearchResult], query: &str) -> String {
        let mut files: Vec<String> = results
            .iter()
            .filter(|r| r.result_type == "text")
            .map(|r| r.file.clone())
            .collect();
        files.sort();
        files.dedup();

        if files.is_empty() {
            return format!("No files with matches found for \"{}\"", query);
        }

        let base_dir = PathBuf::from(crate::core::utils::paths::current_dir_cached().clone());
        let mut output = format!("{} with matches:\n", plural_count(files.len(), "file"));
        for file in files {
            output.push_str(&format!("{}\n", display_path(&file, &base_dir)));
        }
        output.trim_end().to_string()
    }
}

/// 把正则编译错误说成一句**能照做**的话。
///
/// 以前只有一句 "Check the pattern's regex syntax, or set `regex: false`" ——
/// 模型看到之后多半只是把同一个 pattern 换个写法再发一次。真正有用的信息有两条：
/// **哪一列**出的问题，以及**大概率是哪种笔误**。
fn describe_pattern_error(pattern: &str, error: &str) -> String {
    let mut message = format!("Grep could not run pattern \"{}\": {}", pattern, error);

    // 大概率是"想搜字面量括号但转义丢了"——JSON 里 `\(` 是非法转义。
    let has_paren = pattern.contains('(') || pattern.contains(')');
    let has_escaped_paren = pattern.contains("\\(") || pattern.contains("\\)");

    if let Some(pos) = crate::core::tools::ripgrep::first_unbalanced_paren(pattern) {
        let ch = pattern.chars().nth(pos).unwrap_or('?');
        message.push_str(&format!("\n  Position: column {} {:?}", pos + 1, ch));
    }

    if has_paren && !has_escaped_paren {
        message.push_str(
            "\n  Hint: the pattern has an unescaped parenthesis. To match a literal one write \
             `\\(` (in JSON the backslash must be written `\\\\(`), or switch the whole pattern to \
             a literal search (`regex: false`).",
        );
    } else {
        message.push_str("\n  Fix the regex syntax, or switch to a literal search (`regex: false`).");
    }
    message
}

fn normalize_input_path(raw_path: &str) -> PathBuf {
    crate::core::utils::paths::normalize_cross_platform_path(raw_path)
}

fn normalize_existing_search_dir(raw_path: &str) -> String {
    let normalized = normalize_input_path(raw_path);
    if normalized.is_dir() {
        return normalized.to_string_lossy().to_string();
    }

    raw_path.to_string()
}

fn calculate_file_score(file_name: &str, file_path: &str, pattern: &str) -> u32 {
    let lower_file_name = file_name.to_lowercase();
    let lower_file_path = file_path.to_lowercase();
    let lower_pattern = pattern.to_lowercase();

    if lower_file_name == lower_pattern {
        return 100;
    }
    if lower_file_name.contains(&lower_pattern) {
        return 80;
    }

    if lower_file_path.contains(&lower_pattern) {
        return 60;
    }

    let mut pattern_index = 0;
    for i in 0..lower_file_name.len() {
        if pattern_index >= lower_pattern.len() {
            break;
        }
        if lower_file_name.chars().nth(i) == lower_pattern.chars().nth(pattern_index) {
            pattern_index += 1;
        }
    }

    if pattern_index == lower_pattern.len() {
        let diff = (file_name.len() as i32 - pattern.len() as i32).max(0) as u32;
        return std::cmp::max(10, 40u32.saturating_sub(diff));
    }

    1
}

/// 把匹配结果压成上下文友好文本时的几条硬约束。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::state::GlobalState;

    fn text_hit(file: &str, line: u32, text: &str) -> UnifiedSearchResult {
        UnifiedSearchResult {
            result_type: "text".to_string(),
            file: file.to_string(),
            line: Some(line),
            column: None,
            text: Some(text.to_string()),
            // ripgrep 把 pattern 本身塞在 `match_content` 里 —— 格式化时必须用
            // `text`，否则每行都在重复正则。
            match_content: Some("model\\.DB".to_string()),
            score: None,
        }
    }

    fn tool() -> SearchTool {
        SearchTool::new(Arc::new(GlobalState::new()))
    }

    /// 回归：每行显示的是**命中行的原文**，不是 pattern。
    ///
    /// 事故实况：`format_unified_results` 取的是 `match_content`，于是
    /// `Grep("model\.DB|DB\.Where|...")` 的每一行输出都是那个正则本身 —— 5 条
    /// 结果把同一串 40 字符的 pattern 抄了 5 遍，还一个字都没告诉模型。
    #[test]
    fn a_match_line_shows_the_line_not_the_pattern() {
        let results = vec![text_hit(
            "src/models/gorm.go",
            36,
            "model.DB, err = gorm.Open(dsn)",
        )];
        let out = tool().format_unified_results(&results, "model\\.DB", "text");

        assert!(
            out.contains("model.DB, err = gorm.Open(dsn)"),
            "实际输出：{out}"
        );
        assert!(
            !out.contains("36: model\\.DB"),
            "行内容里出现了 pattern 本身，说明又取错了字段：{out}"
        );
    }

    /// 路径相对 cwd：绝对路径前缀在每个文件上重复一遍，文件一多就是纯浪费。
    #[test]
    fn paths_are_relative_to_the_working_directory() {
        let cwd = crate::core::utils::paths::current_dir_cached().clone();
        let abs = cwd.join("src").join("models").join("gorm.go");
        let results = vec![text_hit(&abs.to_string_lossy(), 36, "model.DB = nil")];
        let out = tool().format_unified_results(&results, "model\\.DB", "text");

        assert!(
            out.contains("src/models/gorm.go"),
            "应当输出相对路径，实际：{out}"
        );
        assert!(
            !out.contains(&cwd.to_string_lossy().to_string()),
            "实际：{out}"
        );
    }

    /// 同文件超过 MAX_LINES_PER_FILE 条时折叠成 `+N more`，而不是全量铺开。
    #[test]
    fn a_file_with_many_hits_is_folded() {
        let results: Vec<_> = (1..=25)
            .map(|i| text_hit("src/a.rs", i, &format!("let x{i} = 1;")))
            .collect();
        let out = tool().format_unified_results(&results, "let x", "text");

        assert!(out.contains("+15 more"), "应当折叠多余命中，实际：{out}");
        assert!(out.contains("25 matches"), "总数仍要可见，实际：{out}");
    }

    /// `count` 模式只回"每个文件几处"，给判断改动面用。
    #[test]
    fn count_mode_returns_matches_per_file_only() {
        let results = vec![
            text_hit("src/a.rs", 1, "let x = 1;"),
            text_hit("src/a.rs", 2, "let y = 2;"),
            text_hit("src/b.rs", 9, "let z = 3;"),
        ];
        let out = tool().format_counts(&results, "let ");

        assert!(out.contains("3 matches in 2 files"), "实际：{out}");
        assert!(out.contains("src/a.rs: 2"), "实际：{out}");
        assert!(!out.contains("let x"), "count 模式不该带行内容：{out}");
    }
}

impl BaseDeclarativeTool for SearchTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn display_name(&self) -> &str {
        "Search Tool"
    }

    fn description(&self) -> &str {
        "Search file contents (or file names, when the query starts with '@') with ripgrep. \
         output_mode: 'content' (default, matching lines), 'files_with_matches' (paths only, cheapest), \
         'count' (matches per file). Output is bounded: relative paths, at most 100 matches, \
         at most 10 lines per file, long lines truncated — raise `max_results` or narrow \
         `path`/`include_pattern` if you need more."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query (text or file pattern). Use either query or pattern. Treated as a regular expression; unbalanced parentheses are repaired automatically and searched literally (to search for a string as-is, set regex: false)." },
                "pattern": { "type": "string", "description": "Alias for query, accepted for grep-compatible calls." },
                "path": { "type": "string", "description": "Directory to search. Supports relative paths, absolute paths, and Windows drive paths such as G:\\\\project when running under WSL." },
                "dir_path": { "type": "string", "description": "Alias for path, accepted for compatibility with older grep tools." },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Output mode. 'content' returns matching lines (bounded: max_results matches, 10 per file, long lines truncated); 'files_with_matches' returns unique file paths only; 'count' returns the number of matches per file." },
                "search_type": { "type": "string", "enum": ["text", "files", "both"], "description": "Type of search: 'text' for content, 'files' for filenames, 'both' for combined" },
                "include_pattern": { "type": "string", "description": "Glob pattern to include files" },
                "exclude_pattern": { "type": "string", "description": "Glob pattern to exclude files" },
                "case_sensitive": { "type": "boolean", "description": "Case sensitive search" },
                "whole_word": { "type": "boolean", "description": "Match whole words only" },
                "regex": { "type": "boolean", "description": "Treat query as regex" },
                "max_results": { "type": "integer", "description": "Maximum number of matches to return (default 100). Raise it only when you really need more — every extra match costs context." },
                "file_types": { "type": "array", "items": { "type": "string" }, "description": "Filter by file types (e.g., ['rs', 'toml'])" },
                "exclude_files": { "type": "array", "items": { "type": "string" }, "description": "List of files to exclude" },
                "include_hidden": { "type": "boolean", "description": "Include hidden files" }
            },
            "required": []
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let query = params
            .get("query")
            .or_else(|| params.get("pattern"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| Box::<dyn std::error::Error + Send + Sync>::from("Missing query"))?
            .to_string();
        Ok(Box::new(SearchToolInvocation {
            tool: self.clone(),
            query,
            params,
            global_state: self.global_state.clone(),
        }))
    }
}

struct SearchToolInvocation {
    tool: SearchTool,
    query: String,
    params: serde_json::Value,
    global_state: Arc<GlobalState>,
}

impl ToolInvocation for SearchToolInvocation {
    fn get_description(&self) -> String {
        format!("Search: {}", self.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: std::path::PathBuf::from("."),
            location_type: crate::core::tools::tools::LocationType::Read,
        }]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(None) })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let tool = self.tool.clone();
        let query = self.query.clone();
        let params = self.params.clone();
        let global_state = self.global_state.clone();

        Box::pin(async move {
            let search_type = params["search_type"].as_str().map(|s| s.to_string());
            let path = params
                .get("path")
                .or_else(|| params.get("dir_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let output_mode = params["output_mode"].as_str().map(|s| s.to_string());
            let include_pattern = params["include_pattern"].as_str().map(|s| s.to_string());
            let exclude_pattern = params["exclude_pattern"].as_str().map(|s| s.to_string());
            let case_sensitive = params["case_sensitive"].as_bool();
            let whole_word = params["whole_word"].as_bool();
            let regex = params["regex"].as_bool();
            let max_results = params["max_results"].as_u64().map(|n| n as u32);
            let file_types = params["file_types"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
            let exclude_files = params["exclude_files"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });
            let include_hidden = params["include_hidden"].as_bool();

            // 搜索根：结果路径以它为基准，下面补全相对结果时要用同一个根。
            let search_base_dir = tool.resolve_search_base_dir(path.as_deref());

            let result = tool
                .search(
                    &query,
                    search_type,
                    path,
                    output_mode,
                    include_pattern,
                    exclude_pattern,
                    case_sensitive,
                    whole_word,
                    regex,
                    max_results,
                    file_types,
                    exclude_files,
                    include_hidden,
                )
                .await?;

            // ── 更新 read_file_state：从搜索结果中提取文件路径 ──
            if let Some(ref data) = result.data {
                if let Ok(results) =
                    serde_json::from_value::<Vec<UnifiedSearchResult>>(data.clone())
                {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let mut state = global_state.read_file_state.write().await;
                    let base_dir = Path::new(&search_base_dir);
                    for r in &results {
                        // 键必须和 Edit 查表时一致（read_state_key），否则搜索
                        // 命中过的文件仍会被 [edit_file_not_read] 拦下。
                        let resolved =
                            crate::core::utils::paths::resolve_tool_path(base_dir, &r.file);
                        let key = crate::core::utils::paths::read_state_key(&resolved);
                        state.entry(key).or_insert(ReadFileState {
                            content: String::new(),
                            timestamp: now,
                            file_system_timestamp: now,
                        });
                    }
                }
            }

            Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: result.output.unwrap_or_default(),
                error: result.error.map(|e| ToolError {
                    error_type: "execution_error".to_string(),
                    message: e,
                }),
                data: None,
            })
        })
    }
}
