/// Ripgrep Search - Rust Native Implementation
///
/// Uses Rust grep crate instead of external ripgrep binary
///
/// Advantages:
/// 1. No need to download external tools, pure Rust implementation
/// 2. Performance same as ripgrep CLI (uses the same core library)
/// 3. Better error handling and result formatting
/// 4. Supports custom output format
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{sinks::UTF8, SearcherBuilder};
use std::sync::{Arc, Mutex};

use crate::core::tools::tools::SearchResult;

/// Ripgrep search configuration
pub struct RipgrepConfig {
    /// Case sensitive
    pub case_sensitive: bool,
    /// Whole word match
    pub whole_word: bool,
    /// Regular expression mode
    pub regex: bool,
    /// Maximum results
    pub max_results: Option<u32>,
    /// File type filter
    pub file_types: Option<Vec<String>>,
    /// Exclude patterns
    pub exclude_patterns: Option<Vec<String>>,
    /// Include patterns
    pub include_patterns: Option<Vec<String>>,
    /// `include_patterns` 是相对**哪个**根写的。
    ///
    /// 默认 `None` = 相对 `base_dir`（walker 的根）。
    ///
    /// # 为什么必须显式区分
    ///
    /// 调用方经常已经把搜索范围收窄过一遍了：`Grep(pattern, path="src")` 里
    /// `path` 既被 `SearchTool::resolve_search_base_dir` 用来把 walker 的根换成
    /// `<cwd>/src`，又被 `tool_executor::normalize_search_args` 折成一个
    /// `include_pattern = "src/**"`。两个 glob 基于是不同���：
    ///
    /// - walker 从 `<cwd>/src` 出发 → `entry.path()` = `<cwd>/src/foo.rs`；
    /// - 若拿 `base_dir` 当基 → `relative = "foo.rs"`；
    /// - 而 glob 是 `"src/**"` → **一个文件都匹配不上** → 结果恒为空。
    ///
    /// `normalize_search_scope` 会把绝对路径剥成相对 cwd，所以 include pattern
    /// 的基其实是 **cwd**。传进来就按它匹配，两处作用域就不再互相抵消。
    pub include_base: Option<std::path::PathBuf>,
    /// Include hidden files
    pub include_hidden: bool,
    /// Maximum file size (bytes)
    pub max_file_size: Option<u64>,
}

impl Default for RipgrepConfig {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            regex: false,
            max_results: Some(50),
            file_types: None,
            exclude_patterns: None,
            include_patterns: None,
            include_base: None,
            // dotfile 默认可见 —— 对齐 Claude Code（`GrepTool.ts` 恒传 `--hidden`）。
            include_hidden: true,
            max_file_size: Some(2 * 1024 * 1024), // 2MB
        }
    }
}

/// 调用方没给 `max_results` 时的上限。
///
/// 以前 `None` 会被当成“不限”（`u32::MAX`），一次 `Grep("func")` 能把几千条
/// 匹配整段灌进上下文 —— 这是工具最大的一处 token 浪费。
pub const DEFAULT_MAX_RESULTS: u32 = 100;

/// 单条匹配行最多保留的字符数（超出用 `…` 收尾）。
pub const MAX_LINE_CHARS: usize = 160;

/// 单个文件最多列出的匹配行数，其余折成 `+N more`。
pub const MAX_LINES_PER_FILE: usize = 10;

/// 输出总预算（字符）。超了就截断并提示收窄搜索范围，避免一次调用吃掉整个窗口。
pub const MAX_OUTPUT_CHARS: usize = 12_000;

/// 展示用路径：相对 `base_dir`，分隔符统一成 `/`。
///
/// 绝对值路径每列一个文件都要重复一遍工作目录前缀，几十个文件时这部分几乎占
/// 掉一半 token。
pub fn display_path(file: &str, base_dir: &std::path::Path) -> String {
    let rel = crate::core::utils::paths::make_relative(std::path::Path::new(file), base_dir);
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        file.replace('\\', "/")
    } else {
        rel
    }
}

/// 折叠一行源码：去首尾空白、压掉制表/换行/控制字符，超长截断。
pub fn collapse_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_LINE_CHARS));
    let mut pending_space = false;
    for c in text.trim().chars() {
        match c {
            '\t' | '\r' | '\n' => pending_space = true,
            c if c.is_control() => {}
            ' ' => {
                if !out.is_empty() {
                    pending_space = true;
                }
            }
            c => {
                if pending_space && !out.is_empty() {
                    out.push(' ');
                    pending_space = false;
                }
                out.push(c);
            }
        }
    }
    if out.chars().count() > MAX_LINE_CHARS {
        let keep: String = out.chars().take(MAX_LINE_CHARS - 1).collect();
        format!("{keep}…")
    } else {
        out
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let keep: String = s.chars().take(max - 1).collect();
        format!("{keep}…")
    } else {
        s.to_string()
    }
}

/// `1 file` / `3 files` / `2 matches` —— 单复数只管观感，但 "2 matchs" 一看就是
/// 没测过：以 s/ch/sh/x/z 结尾的词加 `es`。
pub fn plural_word(n: usize, word: &str) -> String {
    if n == 1 {
        return word.to_string();
    }
    let needs_es = word.ends_with('s')
        || word.ends_with('x')
        || word.ends_with('z')
        || word.ends_with("ch")
        || word.ends_with("sh");
    format!("{}{}", word, if needs_es { "es" } else { "s" })
}

/// `3 files` —— 带数字的复数形式。
pub fn plural_count(n: usize, word: &str) -> String {
    format!("{} {}", n, plural_word(n, word))
}

/// 一次 Grep 结果输入给格式化器的最小集（两个 Grep 实现共用）。
pub struct GrepHit<'a> {
    pub file: &'a str,
    pub line: Option<u32>,
    pub text: Option<&'a str>,
}

/// 把匹配结果压成紧凑文本：
///
/// ```text
/// 12 matches in 3 files for "model\.DB"
/// src/models/gorm.go
///   36: func InitDB(cfg *Config) error {
///   42: model.DB, err = gorm.Open(mysql.Open(dsn), &gorm.Config{})
///   +8 more
/// ```
///
/// 关键点：一行一条、路径相对、行长有上限、每文件有上限、整体有预算。
pub fn format_grep_hits(hits: &[GrepHit<'_>], base_dir: &std::path::Path, pattern: &str) -> String {
    if hits.is_empty() {
        return format!(
            "No matches found for pattern \"{}\"",
            truncate_str(pattern, 80)
        );
    }

    let mut by_file: std::collections::BTreeMap<&str, Vec<&GrepHit<'_>>> =
        std::collections::BTreeMap::new();
    for hit in hits {
        by_file.entry(hit.file).or_default().push(hit);
    }

    let mut out = format!(
        "{} in {} for \"{}\"\n",
        plural_count(hits.len(), "match"),
        plural_count(by_file.len(), "file"),
        truncate_str(pattern, 80)
    );

    let mut truncated = false;
    for (file, mut file_hits) in by_file {
        if out.len() >= MAX_OUTPUT_CHARS {
            truncated = true;
            break;
        }
        file_hits.sort_by_key(|h| h.line.unwrap_or(0));

        out.push_str(&display_path(file, base_dir));
        out.push('\n');
        for hit in file_hits.iter().take(MAX_LINES_PER_FILE) {
            let text = hit.text.map(collapse_line).unwrap_or_default();
            match hit.line {
                Some(line) => out.push_str(&format!("  {}: {}\n", line, text)),
                None => out.push_str(&format!("  {}\n", text)),
            }
        }
        if file_hits.len() > MAX_LINES_PER_FILE {
            out.push_str(&format!(
                "  +{} more\n",
                file_hits.len() - MAX_LINES_PER_FILE
            ));
        }
    }

    if truncated {
        out.push_str(
            "\n[truncated] 结果过多已截断：换更精确的 pattern、加 include_pattern 或缩小 path 再搜一次。\n",
        );
    }

    out.trim_end().to_string()
}

/// Search using Rust grep crate
pub fn search_with_ripgrep(
    query: &str,
    base_dir: &str,
    config: RipgrepConfig,
) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
    // Build Matcher
    let mut matcher_builder = RegexMatcherBuilder::new();

    if !config.case_sensitive {
        matcher_builder.case_insensitive(true);
    }

    if config.whole_word {
        matcher_builder.word(true);
    }

    let matcher = if config.regex {
        matcher_builder.build(query)?
    } else {
        // Fixed string search, escape special characters
        let escaped = regex::escape(query);
        matcher_builder.build(&escaped)?
    };

    // Build Walker —— 走 `utils::file_walk` 的统一口径（dotfile 默认可见、
    // 只剪 6 个 VCS 目录、`.gitignore`/`.starignore` 生效）。
    let root = std::path::Path::new(base_dir);
    let mut walk_opts = crate::utils::file_walk::WalkOptions::new()
        .hidden(config.include_hidden)
        .case_sensitive(config.case_sensitive)
        .follow_links(true);
    if let Some(ref excludes) = config.exclude_patterns {
        // 以前这些 pattern 被喂给 `add_ignore()` —— 那个参数是"忽略文件的
        // 路径"，不是 pattern，所以整段是静默 no-op。
        walk_opts = walk_opts.exclude(excludes.clone());
    }
    if let Some(ref includes) = config.include_patterns {
        walk_opts = walk_opts.include(includes.clone());
    }
    let include_matcher = crate::utils::file_walk::include_matcher(&walk_opts);
    let include_root = config
        .include_base
        .clone()
        .unwrap_or_else(|| root.to_path_buf());
    let walker = crate::utils::file_walk::walk(root, &walk_opts);

    // Build Searcher
    let mut searcher_builder = SearcherBuilder::new();

    // Note: grep-searcher doesn't have a built-in max_filesize method
    // We manually check file size before searching

    searcher_builder
        .line_number(true)
        .binary_detection(grep_searcher::BinaryDetection::quit(b'\x00'));

    let mut searcher = searcher_builder.build();
    let max_file_size = config.max_file_size;

    // Result collector (thread-safe)
    let results = Arc::new(Mutex::new(Vec::new()));
    let max_results = config.max_results.unwrap_or(DEFAULT_MAX_RESULTS);
    let current_count = Arc::new(Mutex::new(0u32));

    // Traverse files and search
    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Check if max results exceeded
        {
            let count = current_count.lock().unwrap_or_else(|e| e.into_inner());
            if *count >= max_results {
                break;
            }
        }

        // Only search files, skip directories
        if !entry.path().is_file() {
            continue;
        }

        // Check file size (manual max_filesize implementation)
        if let Some(max_size) = max_file_size {
            if let Ok(metadata) = entry.path().metadata() {
                if metadata.len() > max_size {
                    continue; // Skip files that are too large
                }
            }
        }

        // File type filter
        if let Some(ref types) = config.file_types {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if !types.iter().any(|t| t.to_lowercase() == ext_str) {
                    continue;
                }
            } else {
                // Files without extension, skip if type filter exists
                continue;
            }
        }

        // include pattern：真正的 glob 匹配。以前是 `path.contains(pattern)`
        // 的子串比较 —— `*.rs` 这种正常写法永远匹配不到任何东西。
        //
        // 匹配基必须和"这个 glob 是相对谁写的"一致：`include_base`（调用方给出，
        // 通常是 cwd）优先，缺省才退回 walker 的根。两者不一致时一个文件都匹配
        // 不上 —— 见 `include_base` 的文档注释。
        if let Some(ref matcher) = include_matcher {
            if !crate::utils::file_walk::glob_matches(matcher, &include_root, entry.path()) {
                continue;
            }
        }

        // Execute search
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();

        let results_clone = Arc::clone(&results);
        let current_count_clone = Arc::clone(&current_count);

        let sink_result = searcher.search_path(
            &matcher,
            path,
            UTF8(|line_num, line_content| {
                // Check if max results reached
                let mut count = current_count_clone
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if *count >= max_results {
                    return Ok(false); // Stop search
                }

                // Add result
                let mut results_guard = results_clone.lock().unwrap_or_else(|e| e.into_inner());
                results_guard.push(SearchResult {
                    file: path_str.clone(),
                    line: Some(line_num as u32),
                    column: None, // grep-searcher doesn't directly provide column numbers
                    text: Some(line_content.trim().to_string()),
                    match_content: Some(query.to_string()),
                });

                *count += 1;

                Ok(true) // Continue search
            }),
        );

        // Ignore binary file errors
        if let Err(err) = sink_result {
            if !err.to_string().contains("binary") {
                crate::utils::logging::append_agent_log_line(&format!(
                    "Search file failed {}: {}",
                    path_str, err
                ));
            }
        }
    }

    // Get results
    let results = match Arc::try_unwrap(results) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    };

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一棵小树：`<root>/src/lib.rs` 里有一处 `needle`。
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("lib.rs"), "fn a() { needle }\n").unwrap();
        std::fs::write(dir.path().join("top.rs"), "needle at top\n").unwrap();
        dir
    }

    /// 回归：`include_patterns` 必须相对**它被写出来的那个根**匹配。
    ///
    /// 真实事故：`Grep(pattern="needle", path="src")` 恒返回 0 条。
    /// `path` 被用了两次 —— `SearchTool::resolve_search_base_dir` 把 walker 的根
    /// 收窄成 `<cwd>/src`，`tool_executor` 又把它折成 `include_pattern="src/**"`。
    /// 拿 walker 的根当 glob 基，`relative` 就只剩 `lib.rs`，`src/**` 谁也匹配
    /// 不上，每个文件都被 `continue` 掉 —— 而且**不报错**，界面上就是
    /// "No results found"。
    #[test]
    fn an_include_pattern_written_against_cwd_matches_when_the_walk_root_is_narrower() {
        let dir = fixture();
        let root = dir.path();
        let scoped_root = root.join("src");

        // 旧行为：glob 基 = walker 根。`src/**` vs `lib.rs` → 匹配不上。
        let old = search_with_ripgrep(
            "needle",
            scoped_root.to_str().unwrap(),
            RipgrepConfig {
                include_patterns: Some(vec!["src/**".to_string()]),
                ..Default::default()
            },
        )
        .expect("search should run");
        assert!(
            old.is_empty(),
            "这条是旧行为的见证：glob 基取错时结果为空（如果哪天不再为空，说明 \
             测试的前提变了）"
        );

        // 新行为：glob 基 = pattern 被写出来的根（cwd）。
        let fixed = search_with_ripgrep(
            "needle",
            scoped_root.to_str().unwrap(),
            RipgrepConfig {
                include_patterns: Some(vec!["src/**".to_string()]),
                include_base: Some(root.to_path_buf()),
                ..Default::default()
            },
        )
        .expect("search should run");
        assert_eq!(
            fixed.len(),
            1,
            "`path=src` + `include=src/**` 必须真的搜到 src/lib.rs，实际：{fixed:?}"
        );
        assert!(fixed[0].file.ends_with("lib.rs"));
    }

    /// 不给 `include_base` 时保持旧语义：相对 walker 根。这样 `GrepTool` 那些
    /// 直接传 `include` 的调用点不受影响。
    #[test]
    fn without_an_include_base_the_glob_is_relative_to_the_walk_root() {
        let dir = fixture();
        let root = dir.path();

        let hits = search_with_ripgrep(
            "needle",
            root.to_str().unwrap(),
            RipgrepConfig {
                include_patterns: Some(vec!["src/*.rs".to_string()]),
                ..Default::default()
            },
        )
        .expect("search should run");
        assert_eq!(hits.len(), 1, "默认应当相对 walker 根匹配：{hits:?}");
        assert!(hits[0].file.ends_with("lib.rs"));
    }

    /// 非法正则必须冒泡成 `Err`，不能变成 0 条结果。
    ///
    /// `SearchTool::search` 以前用 `if let Ok(..)`，编译失败被整个吞掉，然后回
    /// "No results found" —— 写错的 pattern 和"确实没匹配"在界面上无法区分。
    #[test]
    fn an_invalid_regex_is_an_error_not_an_empty_result() {
        let dir = fixture();
        let result = search_with_ripgrep(
            "needle(",
            dir.path().to_str().unwrap(),
            RipgrepConfig {
                regex: true,
                ..Default::default()
            },
        );
        assert!(result.is_err(), "非法正则必须返回 Err，实际：{result:?}");
    }

    /// 超长行（压缩过的 js、base64 blob）必须被截断 —— 一条 5KB 的行能顶掉
    /// 三十条正常结果。
    #[test]
    fn a_very_long_line_is_truncated() {
        let long = "x".repeat(5_000);
        let collapsed = collapse_line(&long);
        assert_eq!(collapsed.chars().count(), MAX_LINE_CHARS);
        assert!(collapsed.ends_with('…'));
    }

    /// 制表符/连续空白压成一个空格：缩进对模型没信息量，但很占字符。
    #[test]
    fn whitespace_runs_collapse_to_one_space() {
        assert_eq!(collapse_line("\t\t  let\t\t x = 1;  "), "let x = 1;");
    }

    /// 输出形状：一行计数 + 分组文件路径 + 缩进 `行号: 内容`。
    #[test]
    fn hits_are_grouped_by_file_with_relative_paths() {
        let base = std::path::Path::new("/proj");
        let hits = vec![
            GrepHit {
                file: "/proj/src/a.rs",
                line: Some(3),
                text: Some("fn a() {}"),
            },
            GrepHit {
                file: "/proj/src/b.rs",
                line: Some(1),
                text: Some("fn b() {}"),
            },
        ];
        let out = format_grep_hits(&hits, base, "fn ");

        assert_eq!(
            out,
            "2 matches in 2 files for \"fn \"\nsrc/a.rs\n  3: fn a() {}\nsrc/b.rs\n  1: fn b() {}"
        );
    }

    /// 空结果也要给一句能看懂的话，而不是空字符串。
    #[test]
    fn empty_hits_say_so() {
        let out = format_grep_hits(&[], std::path::Path::new("/proj"), "nope");
        assert!(out.starts_with("No matches found"), "实际：{out}");
    }
}
