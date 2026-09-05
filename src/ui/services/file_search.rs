//! `@` 文件选择器 —— 模糊匹配、智能排序、扫描缓存、路径下钻。
//!
//! 候选来源对标 Claude Code 的 `fileSuggestions.ts`：先问 git
//! （`ls-files --cached` + `ls-files --others --exclude-standard`），
//! 不是 git 仓库才退回 `utils::file_walk` 的统一遍历。
//!
//! 之前这里是 `WalkDir::max_depth(5)` + 7 个硬编码目录名 + 5000 条上限，
//! 完全不认 `.gitignore`。实测本仓库：走过 7198 个条目，其中 6228 个
//! （87%）是被 `.gitignore` 忽略的；5000 条的名额在
//! `study_or_copy_projects/` 里就被吃掉 4067 个，真正想选的源文件反而
//! 因为 depth-5 和名额耗尽进不来。
//!
//! dotfile 现在正常参与模糊匹配（`.github/workflows/ci.yml` 是要选的），
//! 只在同分时轻微降权，不再要求查询必须以 `.` 开头。
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::utils::file_walk::{walk, WalkOptions, VCS_DIRS};

/// 目录扫描缓存有效期 — 避免每次按键都重新扫描
const SCAN_CACHE_TTL_SECS: u64 = 5;
/// 单次目录列举上限（下钻模式）
const CHILDREN_CAP: usize = 500;
/// 扫描条目上限。纯内存保护 —— 尊重 `.gitignore` 之后正常仓库远达不到
/// （本仓库 826 个 tracked 文件 ≈ 1.1K 条含目录）。
const SCAN_CAP: usize = 50_000;
/// 单次查询最多收集多少条命中再排序。
const MATCH_CAP: usize = 2000;
/// dotfile 的同分降权。不再整条过滤，只是排在普通文件后面。
const HIDDEN_PENALTY: i32 = 5;

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

/// 全量扫描（带 5s TTL 缓存）。git 仓库走 `git ls-files`，否则走统一 walker。
fn scan_entries(root: &Path) -> Vec<CachedEntry> {
    if let Ok(guard) = SCAN_CACHE.lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.root == root && cache.at.elapsed().as_secs() < SCAN_CACHE_TTL_SECS {
                return clone_entries(&cache.entries);
            }
        }
    }

    let entries = match git_ls_files(root) {
        Some(paths) => entries_from_paths(paths),
        None => walk_entries(root),
    };

    if let Ok(mut guard) = SCAN_CACHE.lock() {
        *guard = Some(ScanCache {
            root: root.to_path_buf(),
            at: Instant::now(),
            entries: clone_entries(&entries),
        });
    }

    entries
}

fn clone_entries(entries: &[CachedEntry]) -> Vec<CachedEntry> {
    entries
        .iter()
        .map(|e| CachedEntry {
            path: e.path.clone(),
            is_dir: e.is_dir,
            depth: e.depth,
            hidden: e.hidden,
        })
        .collect()
}

/// tracked + untracked-but-not-ignored 的文件清单。
///
/// `-z` 是必须的：默认输出会把非 ASCII 文件名转义成 `"\344\275\240"`，
/// 直接当路径用就找不到文件了。非 git 仓库（或没装 git）返回 `None`。
fn git_ls_files(root: &Path) -> Option<Vec<String>> {
    const ARGS: [&[&str]; 2] = [
        &["ls-files", "-z", "--cached"],
        &["ls-files", "-z", "--others", "--exclude-standard"],
    ];
    let mut paths = Vec::new();
    for args in ARGS {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        paths.extend(
            output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|chunk| !chunk.is_empty())
                .map(|chunk| String::from_utf8_lossy(chunk).replace('\\', "/")),
        );
    }
    Some(paths)
}

/// 把文件清单展开成"文件 + 各级祖先目录"—— git 只列文件，选择器要能下钻。
fn entries_from_paths(paths: Vec<String>) -> Vec<CachedEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries: Vec<CachedEntry> = Vec::new();
    for path in paths {
        let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        let mut accumulated = String::new();
        let mut hidden = false;
        for (index, component) in components.iter().enumerate() {
            if !accumulated.is_empty() {
                accumulated.push('/');
            }
            accumulated.push_str(component);
            hidden |= component.starts_with('.');
            if !seen.insert(accumulated.clone()) {
                continue;
            }
            entries.push(CachedEntry {
                path: accumulated.clone(),
                is_dir: index + 1 < components.len(),
                depth: index + 1,
                hidden,
            });
        }
        if entries.len() >= SCAN_CAP {
            break;
        }
    }
    entries
}

/// 非 git 仓库的退路：统一 walker，口径和 Grep / Glob 一致。
fn walk_entries(root: &Path) -> Vec<CachedEntry> {
    let mut entries: Vec<CachedEntry> = Vec::new();
    for entry in walk(root, &WalkOptions::new()).flatten() {
        let path = entry.path();
        if path == root {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative_str = relative.to_string_lossy().replace('\\', "/");
        entries.push(CachedEntry {
            hidden: relative_str.split('/').any(|c| c.starts_with('.')),
            path: relative_str,
            is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
            depth: relative.components().count(),
        });
        if entries.len() >= SCAN_CAP {
            break;
        }
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

    // 下钻也走统一 walker：`.gitignore` 生效、dotfile 可见、VCS 目录剪掉。
    // 之前是裸 `read_dir`，`target/` 和 `node_modules/` 的内容一览无余。
    let mut results: Vec<FileSearchResult> = Vec::new();
    let prefix = dir_part.replace('\\', "/");
    for entry in walk(&base, &WalkOptions::new().max_depth(1)).flatten() {
        if entry.path() == base {
            continue;
        }
        let fname = entry.file_name().to_string_lossy().to_string();
        if VCS_DIRS.contains(&fname.as_str()) {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
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
            score: score
                - if fname.starts_with('.') {
                    HIDDEN_PENALTY
                } else {
                    0
                },
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
    let mut results: Vec<FileSearchResult> = Vec::new();

    for e in &entries {
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
            // dotfile 不再被整条过滤掉，只在同分时排后面。
            score: score - if e.hidden { HIDDEN_PENALTY } else { 0 },
            depth,
        });

        if results.len() >= MATCH_CAP {
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
