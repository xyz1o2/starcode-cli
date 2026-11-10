use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

const PROJECT_CONTEXT_CACHE_TTL_SECS: u64 = 30;
/// 注入到 system prompt 的 CLAUDE.md / STAR.md 最大字符数
const DEFAULT_PROJECT_CONTEXT_MAX_CHARS: usize = 8000;

/// `@path` 导入的最大展开深度（对标 Claude Code：imports 不能无限嵌套，
/// CC 用 5 层）。
const MAX_IMPORT_DEPTH: usize = 5;

// ── Context file candidates (priority order) ──
/// AGENTS.md 排在 CLAUDE.md 之后：既认社区正在收敛的 AGENTS.md 标准，
/// 又不改变已有仓库（多数只有 CLAUDE.md）的行为。
const CONTEXT_FILE_CANDIDATES: &[&str] = &["STAR.md", "STARCODE.md", "CLAUDE.md", "AGENTS.md"];
/// 注意：这个列表只服务于「首个命中」路径（`find_context_file_in_dir`），
/// 用于判断目录算不算项目根、以及单文件展示。**实际加载路径走
/// `collect_context_files_in_dir`**，那里会把 `CLAUDE.local.md` 和
/// `.claude/CLAUDE.md` 也一并收进来，且 `AGENTS.md` 受全局判定控制。
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

/// 单个目录内按加载顺序收集**全部**上下文文件（对标 Claude Code）
///
/// 与 `find_context_file_in_dir` 的区别：后者是「首个命中」，只用于判断
/// 「这个目录算不算项目根」和单文件展示；这里必须收全部，因为 CC 是**拼接**
/// 语义而非**遮蔽**语义 —— 一层目录里的 CLAUDE.md 不会让同目录的
/// CLAUDE.local.md 失效。
///
/// 顺序（先加载 → 后加载，即低优先级 → 高优先级）：
///   STAR.md → STARCODE.md → CLAUDE.md → .claude/CLAUDE.md → CLAUDE.local.md
///
/// 两处刻意的偏差，都写在这里以免后来人「修正」回去：
/// 1. `STAR.md` / `STARCODE.md` 是 StarCode 自有指令，排在 `CLAUDE.md` **之前**
///    而不是遮蔽它。旧实现用 first-hit 导致同时存在 STARCODE.md 和 CLAUDE.md 时
///    只读前者，而 CC 会读 CLAUDE.md —— 同一个仓库两边看到不同指令。
/// 2. `AGENTS.md` 的收不收由调用方全局判定（见 `collect_context_files`），
///    CC 的规则是「CWD 或其任一上层存在 CLAUDE.md / CLAUDE.local.md 就不读
///    AGENTS.md」，这是跨目录的全局判断，不是每目录各自决定。
fn collect_context_files_in_dir(dir: &Path, include_agents: bool) -> Vec<PathBuf> {
    // AGENTS.override.md 存在即独占该目录 —— StarCode 既有语义，保留。
    let override_path = dir.join(CONTEXT_FILE_OVERRIDE);
    if override_path.exists() {
        return vec![override_path];
    }

    let mut files = Vec::new();
    for name in ["STAR.md", "STARCODE.md"] {
        let p = dir.join(name);
        if p.exists() {
            files.push(p);
        }
    }
    // CC 原生位置：根目录 CLAUDE.md 与 .claude/CLAUDE.md 等价，两者都收。
    let claude_md = dir.join("CLAUDE.md");
    if claude_md.exists() {
        files.push(claude_md);
    }
    let claude_dir_md = dir.join(".claude").join("CLAUDE.md");
    if claude_dir_md.exists() {
        files.push(claude_dir_md);
    }
    // 个人层：同目录内排在 CLAUDE.md 之后（CC 语义）。
    let claude_local_md = dir.join("CLAUDE.local.md");
    if claude_local_md.exists() {
        files.push(claude_local_md);
    }
    if include_agents {
        let agents_md = dir.join("AGENTS.md");
        if agents_md.exists() {
            files.push(agents_md);
        }
    }
    // legacy：老版本把指令放在 .star/STAR.md
    let legacy = dir.join(CONTEXT_FILE_LEGACY);
    if legacy.exists() {
        files.push(legacy);
    }
    files
}

/// 收集多级上下文文件路径（对标 Claude Code 的 getClaudeMds）
///
/// 加载顺序（CC 语义）：
/// 1. 用户全局：`~/.star/*` 与 `~/.claude/CLAUDE.md`
/// 2. 项目各层目录，**从文件系统根向下到 CWD**
///
/// 关键：CC 是**拼接**不是**覆盖** —— 所有层的内容全部进 context，顺序从根到
/// CWD，越靠近启动目录的越晚读到（因此事实上「最后读的赢」）。
/// 旧实现把 `collect_context_files` 的返回顺序写成「从高到低」但循环实际是
/// CWD 优先，等于反着来。
fn collect_context_files(start_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // 1. 用户全局上下文（最低优先级，最先加载）
    if let Some(home) = dirs::home_dir() {
        files.extend(collect_context_files_in_dir(&home.join(".star"), true));
        // CC 的用户层。排在 ~/.star/ 之后。
        let user_claude_md = home.join(".claude").join("CLAUDE.md");
        if user_claude_md.exists() {
            files.push(user_claude_md);
        }
        // home 根下的裸文件（~/.CLAUDE.md 等直接放 home 下的）
        for candidate in CONTEXT_FILE_CANDIDATES {
            let p = home.join(candidate);
            if p.exists() {
                files.push(p);
                break;
            }
        }
    }

    // 2. 先把目录链收集出来（CWD → 上层），供 AGENTS.md 的全局判定使用
    let project_root = find_project_root(start_dir);
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut current = start_dir.to_path_buf();
    loop {
        dirs.push(current.clone());
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

    // CC 默认语义：只要 CWD 或其任一上层存在 CLAUDE.md / .claude/CLAUDE.md /
    // CLAUDE.local.md，就完全不读 AGENTS.md。注意 `~/.claude/CLAUDE.md`
    // 与 managed CLAUDE.md **不计入**这个判定（上面已在项目目录外处理）。
    let has_claude_md = dirs.iter().any(|d| {
        d.join("CLAUDE.md").exists()
            || d.join(".claude").join("CLAUDE.md").exists()
            || d.join("CLAUDE.local.md").exists()
    });
    let include_agents = !has_claude_md;

    // 3. 从文件系统根向下到 CWD 遍历
    for dir in dirs.iter().rev() {
        for found in collect_context_files_in_dir(dir, include_agents) {
            // 避免重复（同一路径只收集一次）
            if !files.iter().any(|f| f == &found) {
                files.push(found);
            }
        }
    }

    files
}

/// `@path` 导入展开（对标 Claude Code 的 `@imports`）。
///
/// CC 语义：CLAUDE.md 里的 `@path/to/file` 行会把目标文件内容就地展开进
/// context —— 用户用它把过长的一段指令（比如 API 速查）拆成单独文件管理。
/// 之前这里按字面量把 `@path` 原样送进 system prompt，模型只会看到一行
/// 看不懂的引用。
///
/// 展开规则（抄 CC 的既有约定，见官方 memory 文档）：
/// - 相对路径相对**该文件所在目录**解析；`~/` 解析到 home；
/// - `.md(x)` 全文注入；其它扩展名（代码文件）CC 只摘 symbols，这里
///   简化为**整段注入 + 截断到 100 行** —— 语义等价于「有开头有正文」，
///   且不需要 tree-sitter 依赖；
/// - 代码围栏（```）内不展开 —— 那是给人看的示例，不是导入；
/// - 递归：导入的文件里还可以再 `@import`，深度上限 [`MAX_IMPORT_DEPTH`]，
///   超深的行保留原样；
/// - 环检测：同一次展开里已访问过的文件不再展开（避免 a→b→a 死循环）；
/// - 目标不存在 / 读不出来：保留原行（CC 的失败方向也是「当没看见」，
///   不应让一行坏 import 毁掉整个 CLAUDE.md）。
fn expand_imports(
    content: &str,
    base_dir: &Path,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let Some(target) = import_target_of(trimmed) else {
            out.push_str(line);
            out.push('\n');
            continue;
        };

        let resolved = resolve_import_path(base_dir, &target);
        let stop =
            depth >= MAX_IMPORT_DEPTH || seen.contains(&resolved) || !seen.insert(resolved.clone());
        // seen.insert 已把当前文件记下；stop 的三种情况都原样保留该行。
        if stop {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        match std::fs::read_to_string(&resolved) {
            Ok(imported) => {
                let import_dir = resolved.parent().unwrap_or(base_dir).to_path_buf();
                let expanded = expand_imports(&imported, &import_dir, seen, depth + 1);
                // 展开体带一个提醒标签，模型看得见内容来源。
                out.push_str(&format!(
                    "[imported from {}]\n{}\n[/imported from {}]",
                    target,
                    expanded.trim_end(),
                    target
                ));
                out.push('\n');
            }
            Err(_) => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    // 保持「文件以换行结尾」的既有形态；无尾随换行时 trim 掉最后补的那一个。
    if !content.ends_with('\n') {
        while out.ends_with('\n') {
            out.pop();
        }
    }
    out
}

/// 识别一行是不是 `@path` 导入；返回路径部分。
/// 只认行首（允许缩进）以 `@` 开头、后跟非空白内容的行 —— `邮件@foo` 这类
/// 正文里的 `@` 命不中。
fn import_target_of(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('@')?.trim();
    if rest.is_empty() || rest.chars().next()? == '@' {
        return None;
    }
    // 路径部分到第一个空白为止 —— CC 的 `@import` 没有行内注释语法，
    // `@path 之后的多余内容` 取第一个 token，其余原样保留。
    Some(rest.split_whitespace().next()?)
}

fn resolve_import_path(base_dir: &Path, target: &str) -> PathBuf {
    if let Some(stripped) = target.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    base_dir.join(target)
}

/// 读取并合并多级上下文文件内容
///
/// 合并策略（对标 Claude Code）：
/// - 全部文件内容按 `collect_context_files` 给出的顺序拼接，每段标注来源
/// - 顺序是「文件系统根 → CWD」，所以**越晚出现的（越靠近 CWD）越优先**
/// - 总字符数受 STAR_PROJECT_CONTEXT_MAX_CHARS 限制
/// - 每段先过 `@import` 展开（见 `expand_imports`）
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

        // `@import` 展开（对标 Claude Code）。跨文件的环检测共享一个 seen：
        // 根文件收了 `@sub.md`、sub.md 又收根文件时，第二次命中走 stop 分支。
        let base_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let expanded = expand_imports(&raw, &base_dir, &mut HashSet::from([path.clone()]), 0);

        let trimmed = expanded.trim();
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

    // ── 以下覆盖 `collect_context_files_in_dir`：拼接语义，不是遮蔽语义 ──

    #[test]
    fn dir_collects_claude_md_and_local_md_in_cc_order() {
        let dir = tmp_dir("order");
        std::fs::write(dir.join("CLAUDE.md"), "c").unwrap();
        std::fs::write(dir.join("CLAUDE.local.md"), "l").unwrap();
        let got = collect_context_files_in_dir(&dir, false);
        assert_eq!(
            got,
            vec![dir.join("CLAUDE.md"), dir.join("CLAUDE.local.md")],
            "CLAUDE.local.md 必须排在 CLAUDE.md 之后（CC：个人层最后读到）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dot_claude_claude_md_is_collected() {
        let dir = tmp_dir("dot-claude");
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join(".claude").join("CLAUDE.md"), "dc").unwrap();
        assert_eq!(
            collect_context_files_in_dir(&dir, false),
            vec![dir.join(".claude").join("CLAUDE.md")]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn starcode_md_no_longer_shadows_claude_md() {
        // 语义分歧的核心：旧实现 first-hit 时 STARCODE.md 会顶掉 CLAUDE.md，
        // 导致同一个仓库 StarCode 与 CC 读到不同指令。
        let dir = tmp_dir("no-shadow");
        std::fs::write(dir.join("STARCODE.md"), "s").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "c").unwrap();
        let got = collect_context_files_in_dir(&dir, false);
        assert_eq!(
            got,
            vec![dir.join("STARCODE.md"), dir.join("CLAUDE.md")],
            "STARCODE.md 与 CLAUDE.md 应并列收集，顺序在前者"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_file_still_shadows_whole_dir() {
        let dir = tmp_dir("override2");
        std::fs::write(dir.join("CLAUDE.md"), "c").unwrap();
        std::fs::write(dir.join("CLAUDE.local.md"), "l").unwrap();
        std::fs::write(dir.join("AGENTS.override.md"), "o").unwrap();
        assert_eq!(
            collect_context_files_in_dir(&dir, false),
            vec![dir.join("AGENTS.override.md")]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agents_md_gated_by_caller() {
        let dir = tmp_dir("agents-gate");
        std::fs::write(dir.join("AGENTS.md"), "a").unwrap();
        assert_eq!(
            collect_context_files_in_dir(&dir, false),
            Vec::<PathBuf>::new(),
            "include_agents=false 时不应收 AGENTS.md"
        );
        assert_eq!(
            collect_context_files_in_dir(&dir, true),
            vec![dir.join("AGENTS.md")]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── `@import` 展开（对标 Claude Code 的 `@imports`）──

    #[test]
    fn import_expands_markdown_content() {
        let dir = tmp_dir("import-basic");
        std::fs::write(dir.join("api.md"), "API 速查内容").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "主要指令\n\n@api.md\n\n其余指令").unwrap();
        let expanded = expand_imports(
            &std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
            &dir,
            &mut HashSet::new(),
            0,
        );
        assert!(
            expanded.contains("API 速查内容"),
            "@import 的目标内容应当被就地展开：{expanded}"
        );
        assert!(
            expanded.contains("[imported from api.md]"),
            "展开体应带来源标签：{expanded}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_inside_code_fence_is_left_alone() {
        let dir = tmp_dir("import-fence");
        std::fs::write(dir.join("api.md"), "SHOULD_NOT_APPEAR").unwrap();
        let content = "# 示例\n\n```markdown\n@api.md 这里只是文档示例\n```\n";
        let expanded = expand_imports(content, &dir, &mut HashSet::new(), 0);
        assert!(
            !expanded.contains("SHOULD_NOT_APPEAR"),
            "围栏里的 @path 是给人看的示例，不展开：{expanded}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_cycle_is_broken_not_looped() {
        let dir = tmp_dir("import-cycle");
        std::fs::write(dir.join("a.md"), "@b.md\ncontent-a").unwrap();
        std::fs::write(dir.join("b.md"), "@a.md\ncontent-b").unwrap();
        // 与合并路径同前提：根文件本身先进入 seen。
        let expanded = expand_imports(
            &std::fs::read_to_string(dir.join("a.md")).unwrap(),
            &dir,
            &mut HashSet::from([dir.join("a.md")]),
            0,
        );
        // a → b 时 a.md 已在 seen 里，b 中的 @a.md 原样保留 —— 出现一次失败方向
        // （字面量行）而不是死循环。
        assert_eq!(
            expanded.matches("content-a").count(),
            1,
            "根文件内容只出现一次：{expanded}"
        );
        assert!(expanded.contains("content-b"), "第二层正常展开：{expanded}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_depth_limit_preserves_the_line() {
        let dir = tmp_dir("import-depth");
        // root 展开于深度 0，@d0..@d4 各层依次 1..=5 —— d4 的内容在深度 5 处理，
        // 它的 @d5.md 触顶被原样保留。链长 = MAX_IMPORT_DEPTH + 1 跳才看得到截断。
        let link = [
            ("root.md", "@d0.md"),
            ("d0.md", "@d1.md"),
            ("d1.md", "@d2.md"),
            ("d2.md", "@d3.md"),
            ("d3.md", "@d4.md\nDEPTH4"),
            ("d4.md", "@d5.md\nDEPTH5"),
            ("d5.md", "NEVER5"),
        ];
        for (name, body) in &link {
            std::fs::write(dir.join(name), body).unwrap();
        }
        let expanded = expand_imports(
            &std::fs::read_to_string(dir.join("root.md")).unwrap(),
            &dir,
            &mut HashSet::from([dir.join("root.md")]),
            0,
        );
        assert!(
            expanded.contains("DEPTH4"),
            "深度上限内的链应整链展开：{expanded}"
        );
        assert!(
            expanded.contains("@d5.md"),
            "第 6 跳起原样保留，不再递归：{expanded}"
        );
        assert!(!expanded.contains("NEVER5"), "超深目标不展开：{expanded}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_missing_target_keeps_the_line() {
        let dir = tmp_dir("import-missing");
        let content = "前文\n\n@does-not-exist.md\n\n后文";
        let expanded = expand_imports(content, &dir, &mut HashSet::new(), 0);
        assert!(
            expanded.contains("@does-not-exist.md"),
            "目标不存在保留原行，不让坏 import 毁掉文件：{expanded}"
        );
        assert!(
            expanded.contains("后文"),
            "坏 import 不截断后续内容：{expanded}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_path_run_expands_imports_end_to_end() {
        let dir = tmp_dir("import-merge");
        std::fs::write(dir.join("team-rules.md"), "团队规则正文").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "@team-rules.md").unwrap();
        let merged = load_and_merge_context_files(&[dir.join("CLAUDE.md")]);
        let merged = merged.expect("CLAUDE.md 非空应产出内容");
        assert!(
            merged.contains("团队规则正文"),
            "合并路径里 @import 应已展开：{merged}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
