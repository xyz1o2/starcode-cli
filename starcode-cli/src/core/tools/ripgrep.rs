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
use ignore::{DirEntry, WalkBuilder};
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
            include_hidden: false,
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

    // Build Walker (file traversal)
    let mut walker_builder = WalkBuilder::new(base_dir);

    walker_builder
        .follow_links(true)
        .hidden(!config.include_hidden)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false);

    // Add exclude patterns
    if let Some(ref excludes) = config.exclude_patterns {
        for pattern in excludes {
            walker_builder.add_ignore(pattern);
        }
    }

    /*
    // Add include patterns
    if let Some(ref includes) = config.include_patterns {
        println!("DEBUG: Include patterns: {:?}", includes);
        let mut override_builder = ignore::overrides::OverrideBuilder::new(base_dir);
        for pattern in includes {
            override_builder.add(pattern)?;
        }
        walker_builder.overrides(override_builder.build()?);
    }
    */

    // General exclusions
    walker_builder.filter_entry(move |entry| !should_skip_entry(entry));

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
    for result in walker_builder.build() {
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

        // Manually check include patterns
        if let Some(ref includes) = config.include_patterns {
            let path_str = entry.path().to_string_lossy();
            let path_normalized = path_str.replace("\\", "/");
            let mut matched = false;
            for pattern in includes {
                let pattern_normalized = pattern.replace("\\", "/");
                // Simple match: if path contains the pattern (assuming pattern is part of the path)
                // This is not perfect glob matching but works for file paths passed as include_pattern
                if path_normalized.contains(&pattern_normalized) {
                    matched = true;
                    break;
                }
            }
            if !matched {
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

/// Determine whether a directory entry should be skipped
fn should_skip_entry(entry: &DirEntry) -> bool {
    let ignored_dirs = [
        "node_modules",
        ".git",
        ".svn",
        ".hg",
        "dist",
        "build",
        ".next",
        ".cache",
        "target",
        ".idea",
        ".vscode",
        "target/debug",
        "target/release",
    ];

    if let Some(file_name) = entry.file_name().to_str() {
        // Skip common build directories
        if ignored_dirs.contains(&file_name) {
            return true;
        }
    }

    false
}
 