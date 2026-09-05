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
            let count = current_count.lock().unwrap();
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
        if let Some(ref matcher) = include_matcher {
            if !crate::utils::file_walk::glob_matches(matcher, root, entry.path()) {
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
                let mut count = current_count_clone.lock().unwrap();
                if *count >= max_results {
                    return Ok(false); // Stop search
                }

                // Add result
                let mut results_guard = results_clone.lock().unwrap();
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
        Err(arc) => arc.lock().unwrap().clone(),
    };

    Ok(results)
}
