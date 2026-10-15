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
    let max_results = config.max_results.unwrap_or(u32::MAX);
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

    /// 反过来：同样的 pattern 在 `regex: false` 下是合法的字面量搜索。
    #[test]
    fn the_same_pattern_is_a_valid_literal_search_when_regex_is_off() {
        let dir = fixture();
        std::fs::write(dir.path().join("paren.rs"), "let x = needle(1);\n").unwrap();
        let hits = search_with_ripgrep(
            "needle(",
            dir.path().to_str().unwrap(),
            RipgrepConfig {
                regex: false,
                ..Default::default()
            },
        )
        .expect("literal search should run");
        assert_eq!(hits.len(), 1, "字面量搜索应当命中 paren.rs：{hits:?}");
    }
}
