/// 高级文件搜索模块
/// 参考 Star CLI 的实现，提供模糊匹配、智能排序、扫描缓存、路径下钻等功能
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use walkdir::WalkDir;

/// 目录扫描缓存有效期 — 避免每次按键都重新 WalkDir
const SCAN_CACHE_TTL_SECS: u64 = 5;
/// 单次目录列举上限（下钻模式）
const CHILDREN_CAP: usize = 500;

/// 文件搜索结果
#[derive(Debug, Clone)]
pub struct FileSearchResult {
    pub path: String,
    pub is_dir: bool,
    score: i32,   // 匹配分数
    depth: usize, // 目录深度
}

struct CachedEntry {
    path: String,
    is_dir: bool,
    depth: usize,
    /// 任一路径组件以 . 开头（查询时按需过滤）
    hidden: bool,
}

struct ScanCache {
    root: PathBuf,
    at: Instant,
    entries: Vec<CachedEntry>,
}

static SCAN_CACHE: Mutex<Option<ScanCache>> = Mutex::new(None);

/// 计算模糊匹配分数（类似 fzf 算法）
fn fuzzy_match_score(text: &str, pattern: &str) -> i32 {
    if pattern.is_empty() {
        return 0;
    }

    let text_lower = text.to_lowercase();
    let pattern_lower = pattern.to_lowercase();

    // 完全匹配
    if text_lower == pattern_lower {
        return 1000;
    }

    // 前缀匹配
    if text_lower.starts_with(&pattern_lower) {
        return 800 - (text.len() as i32 - pattern.len() as i32);
    }

    // 连续子串匹配
    if let Some(pos) = text_lower.find(&pattern_lower) {
        // 单词边界加分（文件名开头或 / 后）
        let word_boundary_bonus =
            if pos == 0 || text.chars().nth(pos.saturating_sub(1)) == Some('/') {
                200
            } else {
                0
            };
        return 500 - pos as i32 + word_boundary_bonus;
    }

    // 字符序列匹配（模糊匹配）
    let text_chars: Vec<char> = text_lower.chars().collect();
    let pattern_chars: Vec<char> = pattern_lower.chars().collect();

    let mut matched = 0;
    let mut last_match_idx = 0;
    let mut consecutive_bonus = 0;

    for (i, &p_char) in pattern_chars.iter().enumerate() {
        let mut found = false;
        for (j, &t_char) in text_chars.iter().enumerate().skip(last_match_idx) {
            if t_char == p_char {
                matched += 1;
                // 连续匹配加分
                if j == last_match_idx + 1 && i > 0 {
                    consecutive_bonus += 10;
                }
                last_match_idx = j + 1;
                found = true;
                break;
            }
        }
        if !found {
            return -1000; // 无法匹配所有字符
        }
    }

    if matched == pattern_chars.len() {
        // 基础分 + 连续匹配加分 - 距离惩罚
        return 300 + consecutive_bonus - (text.len() as i32 - pattern.len() as i32);
    }

    -1000
}

/// 展开 `~` / `~/` 为 home 目录
fn expand_tilde(p: &str) -> String {
    if p == "~" {
        if let Some(h) = dirs::home_dir() {
            return h.to_string_lossy().replace('\\', "/");
        }
    } else if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return format!("{}/{}", h.to_string_lossy().replace('\\', "/"), rest);
        }
    }
    p.to_string()
}

/// 全量扫描（带 5s TTL 缓存）。depth-5、跳过常见无关目录。
fn scan_entries(root: &Path) -> Vec<CachedEntry> {
    if let Ok(mut guard) = SCAN_CACHE.lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.root == root && cache.at.elapsed().as_secs() < SCAN_CACHE_TTL_SECS {
                return cache
                    .entries
                    .iter()
                    .map(|e| CachedEntry {
                        path: e.path.clone(),
                        is_dir: e.is_dir,
                        depth: e.depth,
                        hidden: e.hidden,
                    })
                    .collect();
            }
        }
    }

    let mut entries: Vec<CachedEntry> = Vec::new();
    for entry in WalkDir::new(root)
        .max_depth(5)
        .into_iter()
        .filter_entry(|e| {
            let fname = e.file_name().to_string_lossy();
            !matches!(
                fname.as_ref(),
                "node_modules" | "target" | ".git" | "dist" | "build" | "__pycache__" | ".next"
            )
        })
        .flatten()
    {
        let path = entry.path();
        if path == root {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        entries.push(CachedEntry {
            hidden: rel_str.split('/').any(|c| c.starts_with('.')),
            path: rel_str,
            is_dir: path.is_dir(),
            depth: rel.components().count(),
        });
        if entries.len() >= 5000 {
            break;
        }
    }

    if let Ok(mut guard) = SCAN_CACHE.lock() {
        *guard = Some(ScanCache {
            root: root.to_path_buf(),
            at: Instant::now(),
            entries: entries
                .iter()
                .map(|e| CachedEntry {
                    path: e.path.clone(),
                    is_dir: e.is_dir,
                    depth: e.depth,
                    hidden: e.hidden,
                })
                .collect(),
        });
    }

    entries
}

/// 下钻模式：pattern 形如 "dir/sub/" 时列出该目录的直接子项
fn search_children(pattern: &str, current_dir: &Path) -> Option<Vec<String>> {
    let normalized = expand_tilde(pattern);
    let (dir_part, name_query) = match normalized.rfind('/') {
        Some(i) => (&normalized[..=i], &normalized[i + 1..]),
        None => return None,
    };

    let base: PathBuf = if Path::new(dir_part).is_absolute() {
        PathBuf::from(dir_part)
    } else {
        current_dir.join(dir_part)
    };
    if !base.is_dir() {
        return None;
    }

    let show_hidden = name_query.starts_with('.');
    let mut results: Vec<FileSearchResult> = Vec::new();
    let prefix = dir_part.replace('\\', "/");
    for entry in std::fs::read_dir(&base).ok()?.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && fname.starts_with('.') {
            continue;
        }
        let is_dir = entry.path().is_dir();
        let child_path = format!("{}{}", prefix, fname);
        let score = if name_query.is_empty() {
            if is_dir {
                1000
            } else {
                500
            }
        } else {
            let s = fuzzy_match_score(&fname, name_query);
            if s < 0 {
                continue;
            }
            s + if is_dir { 10 } else { 0 }
        };
        results.push(FileSearchResult {
            path: child_path,
            is_dir,
            score,
            depth: prefix.matches('/').count() + 1,
        });
        if results.len() >= CHILDREN_CAP {
            break;
        }
    }

    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.path.cmp(&b.path),
            })
    });

    let hints: Vec<String> = results
        .into_iter()
        .take(30)
        .map(|r| {
            if r.is_dir {
                format!("{}/", r.path)
            } else {
                r.path
            }
        })
        .collect();
    Some(hints)
}

/// 改进的文件搜索：支持模糊匹配、智能排序、路径下钻、~ 展开
pub fn search_files(pattern: &str) -> Vec<String> {
    let current_dir = crate::core::utils::paths::current_dir_cached().clone();

    // 路径下钻模式：带 / 的 pattern 且目录存在 → 列子项
    if pattern.contains('/') {
        if let Some(hints) = search_children(pattern, &current_dir) {
            return hints;
        }
    }

    let entries = scan_entries(&current_dir);
    let show_hidden = pattern.starts_with('.');
    let mut results: Vec<FileSearchResult> = Vec::new();

    for e in &entries {
        if !show_hidden && e.hidden {
            continue;
        }
        let is_dir = e.is_dir;
        let depth = e.depth;
        let path_str = e.path.clone();

        let score = if pattern.is_empty() {
            // 空输入：只显示顶层，目录优先
            if depth == 1 {
                if is_dir {
                    1000
                } else {
                    500
                }
            } else {
                continue;
            }
        } else {
            let file_name = path_str.rsplit('/').next().unwrap_or(&path_str).to_string();
            let name_score = fuzzy_match_score(&file_name, pattern);
            let path_score = fuzzy_match_score(&path_str, pattern);
            // 文件名匹配优先于纯路径匹配（+100），目录小幅加成（+10）
            if name_score < 0 && path_score < 0 {
                continue;
            }
            let best_score = if name_score >= 0 {
                name_score + 100
            } else {
                path_score
            };
            best_score + if is_dir { 10 } else { 0 }
        };

        results.push(FileSearchResult {
            path: path_str,
            is_dir,
            score,
            depth,
        });

        if results.len() >= 200 {
            break;
        }
    }

    // 排序
    results.sort_by(|a, b| match b.score.cmp(&a.score) {
        std::cmp::Ordering::Equal => match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => match a.depth.cmp(&b.depth) {
                std::cmp::Ordering::Equal => a.path.cmp(&b.path),
                other => other,
            },
        },
        other => other,
    });

    // 格式对标 Claude Code：仅路径本身，目录带尾 `/`；无匹配返回空列表
    results
        .into_iter()
        .take(30)
        .map(|r| {
            if r.is_dir {
                format!("{}/", r.path)
            } else {
                r.path
            }
        })
        .collect()
}

#[cfg(test)]
mod hint_format_tests {
    use super::*;

    #[test]
    fn hints_are_bare_paths() {
        let r = search_files("");
        assert!(!r.is_empty());
        // 无大小/类型标签；目录带尾 /
        for h in &r {
            assert!(!h.contains('['), "hint should be bare path, got: {}", h);
        }
        assert!(
            r.iter().any(|h| h.ends_with('/')),
            "top-level listing should contain dirs"
        );
        assert!(
            r.iter().any(|h| !h.ends_with('/')),
            "top-level listing should contain files"
        );
    }

    #[test]
    fn filename_match_beats_path_match() {
        let r = search_files("cargo.toml");
        assert!(r.iter().any(|h| h == "Cargo.toml"), "got: {:?}", r);
    }
}
