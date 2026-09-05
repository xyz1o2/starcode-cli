use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

const PROJECT_CONTEXT_CACHE_TTL_SECS: u64 = 30;
/// 注入到 system prompt 的 CLAUDE.md / STAR.md 最大字符数
const DEFAULT_PROJECT_CONTEXT_MAX_CHARS: usize = 8000;

// ── Context file candidates (priority order) ──
/// AGENTS.md 排在 CLAUDE.md 之后：既认社区正在收敛的 AGENTS.md 标准，
/// 又不改变已有仓库（多数只有 CLAUDE.md）的行为。
const CONTEXT_FILE_CANDIDATES: &[&str] = &["STAR.md", "STARCODE.md", "CLAUDE.md", "AGENTS.md"];
const CONTEXT_FILE_LEGACY: &str = ".star/STAR.md";
/// 覆盖文件：某个目录里存在它时，**只用它**，忽略同目录其它候选文件。
/// 用途是在不动团队共享的 CLAUDE.md 的前提下本地改写指令（对标 pi 的 override 语义）。
/// 建议加进 .gitignore。
const CONTEXT_FILE_OVERRIDE: &str = "AGENTS.override.md";

#[derive(Clone)]
struct ProjectContextCacheEntry {
    source_path: Option<PathBuf>,
    modified_at_unix_ms: Option<u128>,
    file_len: u64,
    checked_at: Instant,
    content: Option<String>,
}

/// 多级上下文缓存条目：跟踪多个文件的指纹
#[derive(Clone)]
struct MultiLevelCacheEntry {
    /// (path, modified_at_ms, file_len) 对每个级别
    file_fingerprints: Vec<(PathBuf, Option<u128>, u64)>,
    checked_at: Instant,
    merged_content: Option<String>,
}

fn project_context_max_chars() -> usize {
    std::env::var("STAR_PROJECT_CONTEXT_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PROJECT_CONTEXT_MAX_CHARS)
}

fn project_context_cache() -> &'static Mutex<HashMap<PathBuf, ProjectContextCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ProjectContextCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn multi_level_cache() -> &'static Mutex<HashMap<PathBuf, MultiLevelCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, MultiLevelCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn project_context_cache_ttl() -> Duration {
    Duration::from_secs(
        std::env::var("STAR_PROJECT_CONTEXT_CACHE_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(PROJECT_CONTEXT_CACHE_TTL_SECS),
    )
}

fn file_fingerprint(path: &Path) -> Option<(Option<u128>, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified_at_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis());
    Some((modified_at_unix_ms, metadata.len()))
}

/// 在目录中查找第一个匹配的上下文文件
///
/// `AGENTS.override.md` 优先于全部候选：存在即独占。
fn find_context_file_in_dir(dir: &Path) -> Option<PathBuf> {
    let override_path = dir.join(CONTEXT_FILE_OVERRIDE);
    if override_path.exists() {
        return Some(override_path);
    }
    for filename in CONTEXT_FILE_CANDIDATES {
        let p = dir.join(filename);
        if p.exists() {
            return Some(p);
        }
    }
    // 检查 legacy 路径
    let legacy = dir.join(CONTEXT_FILE_LEGACY);
    if legacy.exists() {
        return Some(legacy);
    }
    None
}

/// 递归向上查找项目上下文文件（单文件模式，保持向后兼容）
/// 优先顺序: AGENTS.override.md > STAR.md > STARCODE.md > CLAUDE.md > AGENTS.md > .star/STAR.md
pub fn find_project_context_file(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir;
    loop {
        if let Some(found) = find_context_file_in_dir(current) {
            return Some(found);
        }
        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }
    None
}

/// 查找项目根目录（包含 .git 或第一个匹配的上下文文件所在的目录）
fn find_project_root(start_dir: &Path) -> Option<PathBuf> {
    let mut current = start_dir;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        // 如果目录有上下文文件且父目录没有，也视为项目根
        if find_context_file_in_dir(current).is_some() {
            let parent = current.parent();
            let parent_has_context = parent.and_then(|p| find_context_file_in_dir(p)).is_some();
            if !parent_has_context {
                return Some(current.to_path_buf());
            }
        }
        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }
    None
}

/// 收集多级上下文文件路径（对标 Claude Code 的 getClaudeMds）
///
/// 层级（从低到高优先级，合并时后面的覆盖前面的）：
/// 1. 用户全局: ~/.star/STAR.md 或 ~/.star/CLAUDE.md
/// 2. 从 CWD 向上遍历到项目根，每级目录的 STAR.md/CLAUDE.md
///
/// 返回的 Vec 按层级从高（用户全局）到低（CWD 附近）排列，
/// 合并时后面的条目优先级更高。
fn collect_context_files(start_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // 1. 用户全局上下文
    if let Some(home) = dirs::home_dir() {
        let user_star_dir = home.join(".star");
        if let Some(found) = find_context_file_in_dir(&user_star_dir) {
            files.push(found);
        }
        // 也检查 ~/.CLAUDE.md 等直接放 home 下的
        for candidate in CONTEXT_FILE_CANDIDATES {
            let p = home.join(candidate);
            if p.exists() {
                files.push(p);
                break;
            }
        }
    }

    // 2. 从 CWD 向上遍历，收集每一级的上下文文件
    // 但只收集到项目根（.git 所在目录）为止
    let project_root = find_project_root(start_dir);

    let mut current = start_dir.to_path_buf();
    let mut collected_dirs: Vec<PathBuf> = Vec::new();

    loop {
        if let Some(found) = find_context_file_in_dir(&current) {
            // 避免重复（同一路径只收集一次）
            if !files.iter().any(|f| f == &found) {
                collected_dirs.push(current.clone());
                files.push(found);
            }
        }

        // 到达项目根就停止向上遍历
        if let Some(ref root) = project_root {
            if current == *root {
                break;
            }
        }

        match current.parent() {
            Some(p) => {
                // 安全检查：防止无限循环
                if p == current {
                    break;
                }
                current = p.to_path_buf();
            }
            None => break,
        }
    }

    files
}

/// 读取并合并多级上下文文件内容
///
/// 合并策略（对标 Claude Code）：
/// - 每个级别的内容用分隔符标注来源
/// - 后面的级别（更靠近 CWD）优先级更高
/// - 总字符数受 STAR_PROJECT_CONTEXT_MAX_CHARS 限制
fn load_and_merge_context_files(files: &[PathBuf]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let max_chars = project_context_max_chars();
    let mut merged = String::new();
    let mut total_chars = 0usize;
    let separator = "\n\n---\n\n";

    for (i, path) in files.iter().enumerate() {
        let raw = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };

        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 计算来源标签
        let source_label = if let Some(home) = dirs::home_dir() {
            if path.starts_with(&home) {
                format!(
                    "User Global ({})",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            } else {
                format!(
                    "Project ({})",
                    path.strip_prefix(
                        find_project_root(path.parent().unwrap_or(path))
                            .unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf())
                    )
                    .unwrap_or(path)
                    .display()
                )
            }
        } else {
            path.display().to_string()
        };

        let section = if i == 0 && files.len() > 1 {
            // 第一个（全局）用标签标注
            format!("[{}]\n{}", source_label, trimmed)
        } else if files.len() > 1 {
            format!("[{}]\n{}", source_label, trimmed)
        } else {
            trimmed.to_string()
        };

        let added_len = if merged.is_empty() {
            section.len()
        } else {
            separator.len() + section.len()
        };

        if total_chars + added_len > max_chars {
            // 截断当前 section 以适应限制
            let remaining = max_chars.saturating_sub(total_chars);
            if remaining > 100 {
                // 至少需要 100 字符才值得添加
                if !merged.is_empty() {
                    merged.push_str(separator);
                    total_chars += separator.len();
                }
                let safe_end = section
                    .char_indices()
                    .nth(remaining)
                    .map(|(i, _)| i)
                    .unwrap_or(section.len());
                merged.push_str(&section[..safe_end]);
                merged.push_str(&format!(
                    "\n\n... [Truncated at {} chars. Set STAR_PROJECT_CONTEXT_MAX_CHARS to adjust.]",
                    max_chars
                ));
            }
            break;
        }

        if !merged.is_empty() {
            merged.push_str(separator);
        }
        merged.push_str(&section);
        total_chars += added_len;
    }

    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

/// 截断过长的项目上下文，保留前 N 字符
fn truncate_project_context(content: String) -> String {
    let max_chars = project_context_max_chars();
    if content.len() <= max_chars {
        return content;
    }
    let safe_end = content
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    format!(
        "{}\n\n... [Project context truncated to {} chars (total: {}). Set STAR_PROJECT_CONTEXT_MAX_CHARS to adjust.]",
        &content[..safe_end],
        max_chars,
        content.len()
    )
}

/// 读取项目上下文文件内容（单文件模式，保持向后兼容）
pub fn load_project_context(start_dir: &Path) -> Option<String> {
    let cache_key = start_dir.to_path_buf();
    let ttl = project_context_cache_ttl();

    if let Ok(cache) = project_context_cache().lock() {
        if let Some(entry) = cache.get(&cache_key) {
            if entry.checked_at.elapsed() < ttl {
                match &entry.source_path {
                    Some(source_path) => {
                        if let Some((modified_at_unix_ms, file_len)) = file_fingerprint(source_path)
                        {
                            if entry.modified_at_unix_ms == modified_at_unix_ms
                                && entry.file_len == file_len
                            {
                                return entry.content.clone();
                            }
                        }
                    }
                    None => return None,
                }
            }
        }
    }

    let source_path = find_project_context_file(start_dir);
    let mut modified_at_unix_ms = None;
    let mut file_len = 0;
    let mut content = None;

    if let Some(path) = source_path.as_ref() {
        if let Some((fingerprint_modified_at_unix_ms, fingerprint_file_len)) =
            file_fingerprint(path)
        {
            modified_at_unix_ms = fingerprint_modified_at_unix_ms;
            file_len = fingerprint_file_len;
        }

        if let Ok(raw) = std::fs::read_to_string(path) {
            if !raw.trim().is_empty() {
                content = Some(truncate_project_context(raw));
            }
        }
    }

    if let Ok(mut cache) = project_context_cache().lock() {
        cache.insert(
            cache_key,
            ProjectContextCacheEntry {
                source_path,
                modified_at_unix_ms,
                file_len,
                checked_at: Instant::now(),
                content: content.clone(),
            },
        );
    }

    content
}

/// 加载多级合并的项目上下文（对标 Claude Code 的 getClaudeMds）
///
/// 合并顺序：
/// 1. ~/.star/STAR.md (用户全局偏好)
/// 2. /project/STAR.md (项目根 - 团队共享)
/// 3. /project/src/STAR.md (子目录 - 模块特定)
///
/// 后面的级别优先级更高，内容会被合并注入到 system prompt。
pub fn load_merged_project_context(start_dir: &Path) -> Option<String> {
    let cache_key = start_dir.to_path_buf();
    let ttl = project_context_cache_ttl();

    // 检查多级缓存
    if let Ok(cache) = multi_level_cache().lock() {
        if let Some(entry) = cache.get(&cache_key) {
            if entry.checked_at.elapsed() < ttl {
                // 验证所有文件指纹是否变化
                let mut all_fresh = true;
                for (path, cached_ms, cached_len) in &entry.file_fingerprints {
                    if let Some((current_ms, current_len)) = file_fingerprint(path) {
                        if current_ms != *cached_ms || current_len != *cached_len {
                            all_fresh = false;
                            break;
                        }
                    } else {
                        // 文件被删除
                        all_fresh = false;
                        break;
                    }
                }
                if all_fresh {
                    return entry.merged_content.clone();
                }
            }
        }
    }

    // 收集所有级别的上下文文件
    let files = collect_context_files(start_dir);

    // 计算每个文件的指纹
    let fingerprints: Vec<(PathBuf, Option<u128>, u64)> = files
        .iter()
        .map(|p| {
            let (ms, len) = file_fingerprint(p).unwrap_or((None, 0));
            (p.clone(), ms, len)
        })
        .collect();

    // 合并内容
    let merged = load_and_merge_context_files(&files);

    // 更新缓存
    if let Ok(mut cache) = multi_level_cache().lock() {
        cache.insert(
            cache_key,
            MultiLevelCacheEntry {
                file_fingerprints: fingerprints,
                checked_at: Instant::now(),
                merged_content: merged.clone(),
            },
        );
    }

    merged
}

/// 获取所有已加载的上下文文件路径（用于 UI 显示和调试）
pub fn get_context_file_paths(start_dir: &Path) -> Vec<PathBuf> {
    collect_context_files(start_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("starcode-ctx-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn agents_md_is_a_recognized_candidate() {
        let dir = tmp_dir("agents");
        std::fs::write(dir.join("AGENTS.md"), "hi").unwrap();
        assert_eq!(find_context_file_in_dir(&dir), Some(dir.join("AGENTS.md")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_md_still_wins_over_agents_md() {
        let dir = tmp_dir("claude-first");
        std::fs::write(dir.join("AGENTS.md"), "a").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "c").unwrap();
        assert_eq!(find_context_file_in_dir(&dir), Some(dir.join("CLAUDE.md")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_file_shadows_every_other_candidate() {
        let dir = tmp_dir("override");
        std::fs::write(dir.join("STAR.md"), "s").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "c").unwrap();
        std::fs::write(dir.join("AGENTS.override.md"), "o").unwrap();
        assert_eq!(
            find_context_file_in_dir(&dir),
            Some(dir.join("AGENTS.override.md"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
