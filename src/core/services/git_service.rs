use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub branch: String,
    pub has_staged: bool,
    pub has_modified: bool,
    pub has_untracked: bool,
}

#[derive(Clone)]
struct GitCacheEntry {
    created_at: Instant,
    value: Option<String>,
}

fn file_history_cache() -> &'static Mutex<HashMap<String, GitCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, GitCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recent_changes_cache() -> &'static Mutex<HashMap<String, GitCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, GitCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 一次轮询的**总**时间预算，不是每个子进程各给一份。
///
/// 以前这里是三条命令连着跑（`symbolic-ref` → 失败再 `rev-parse` →
/// `status --porcelain`），每条各自一个 10s 超时，最坏 30s 才返回一次
/// 「拿不到状态」。而状态栏要的三样东西 `git status --porcelain --branch`
/// 一次就给全了 —— 分支名就在输出第一行 `## `。
const GIT_POLL_BUDGET: Duration = Duration::from_secs(10);

/// 跑一条 git 命令，最多给 `budget`。
///
/// **`kill_on_drop(true)` 是这里的关键，不是保险起见。** tokio 的默认值是
/// `false`：也就是说 `tokio::time::timeout` 超时只是把 future 丢掉，**子进程
/// 还在跑**。慢盘上（WSL 的 `/mnt/d`）`git status` 自己就超过 10s，于是每一轮
/// 超时都留下一个仍在扫整个工作区的 `git status`，而轮询几秒后又启新的。几个
/// 并存的 `git status` 抢同一块慢 I/O —— 这就是「首次特别慢」不但没修好、还会
/// 越拖越长、且日志里反复出现 `status --porcelain timed out` 的原因。
async fn run_git(cwd: &Path, args: &[&str], budget: Duration) -> Option<std::process::Output> {
    let mut command = tokio::process::Command::new("git");
    command.args(args).current_dir(cwd).kill_on_drop(true);

    match tokio::time::timeout(budget, command.output()).await {
        Ok(Ok(output)) if output.status.success() => Some(output),
        Ok(Ok(_)) => None,
        Ok(Err(_)) => None,
        Err(_) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[GIT] `git {}` timed out after {}s",
                args.join(" "),
                budget.as_secs()
            ));
            None
        }
    }
}

/// `## main...origin/main [ahead 1]` → `main`；detached HEAD 原样保留。
fn branch_name(header: &str) -> String {
    // 空仓库的头是 `## No commits yet on master`，把前缀砍掉。
    let header = header.strip_prefix("No commits yet on ").unwrap_or(header);
    let name = header.split("...").next().unwrap_or(header);
    let name = name.split('[').next().unwrap_or(name);
    name.trim().to_string()
}

/// 解析 `git status --porcelain --branch` 的输出。
///
/// 返回 `None` = 「这里不是工作树」，调用方据此停止轮询，而不是上报一个空白分支。
fn parse_status_output(stdout: &str) -> Option<GitStatus> {
    let mut status = GitStatus::default();
    let mut saw_branch_header = false;

    for line in stdout.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            status.branch = branch_name(header);
            saw_branch_header = true;
            continue;
        }

        if line.len() < 2 {
            continue;
        }
        let (staged, modified) = line.split_at(1);
        let (modified, _) = modified.split_at(1);

        match staged {
            "M" | "A" | "D" | "R" | "C" => status.has_staged = true,
            "?" => status.has_untracked = true,
            _ => {}
        }

        match modified {
            "M" | "D" => status.has_modified = true,
            "?" => status.has_untracked = true,
            _ => {}
        }
    }

    if !saw_branch_header || status.branch.is_empty() {
        return None;
    }
    Some(status)
}

pub async fn get_git_status<P: AsRef<Path>>(cwd: P) -> Option<GitStatus> {
    let output = run_git(
        cwd.as_ref(),
        &["status", "--porcelain", "--branch"],
        GIT_POLL_BUDGET,
    )
    .await?;
    parse_status_output(&String::from_utf8_lossy(&output.stdout))
}

pub fn get_file_history(path: &Path, n: usize) -> Option<String> {
    if !path.exists() {
        return None;
    }

    let cache_key = format!(
        "{}::{}",
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .display(),
        n
    );
    if let Some(cached) = get_cached_value(file_history_cache(), &cache_key) {
        return cached;
    }

    let output = Command::new("git")
        .args([
            "log",
            "-p",
            &format!("-n {}", n),
            "--pretty=format:Commit: %h%nAuthor: %an%nDate: %ad%nSummary: %s%n",
            "--",
            path.to_str().unwrap_or(""),
        ])
        .output()
        .ok()?;

    let value = if output.status.success() {
        let content = String::from_utf8_lossy(&output.stdout).to_string();
        if content.trim().is_empty() {
            None
        } else {
            Some(limit_git_context_chars(content))
        }
    } else {
        None
    };

    set_cached_value(file_history_cache(), cache_key, value.clone());
    value
}

pub fn get_recent_changes(n: usize) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let cache_key = format!("{}::{}", cwd.display(), n);
    if let Some(cached) = get_cached_value(recent_changes_cache(), &cache_key) {
        return cached;
    }

    let output = Command::new("git")
        .args([
            "log",
            &format!("-n {}", n),
            "--name-only",
            "--pretty=format:---%nCommit: %h%nSummary: %s%nFiles:",
        ])
        .current_dir(&cwd)
        .output()
        .ok()?;

    let value = if output.status.success() {
        Some(limit_git_context_chars(
            String::from_utf8_lossy(&output.stdout).to_string(),
        ))
    } else {
        None
    };

    set_cached_value(recent_changes_cache(), cache_key, value.clone());
    value
}

pub async fn detect_repo_root(cwd: &Path) -> Option<PathBuf> {
    let output = tokio::process::Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

pub async fn rev_parse(repo_root: &Path, rev: &str) -> Result<String, String> {
    let output = run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C").arg(repo_root).arg("rev-parse").arg(rev);
            c
        },
        "git rev-parse",
    )
    .await?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn worktree_remove_force(repo_root: &Path, work_dir: &Path) {
    let _ = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(work_dir)
        .output()
        .await;
}

pub async fn worktree_add(
    repo_root: &Path,
    work_dir: &Path,
    base_head: &str,
) -> Result<(), String> {
    if let Some(parent) = work_dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }

    run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C")
                .arg(repo_root)
                .arg("worktree")
                .arg("add")
                .arg("--detach")
                .arg(work_dir)
                .arg(base_head);
            c
        },
        "git worktree add",
    )
    .await?;

    Ok(())
}

pub async fn is_working_tree_clean(repo_root: &Path) -> Result<bool, String> {
    let output = run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C").arg(repo_root).arg("status").arg("--porcelain");
            c
        },
        "git status --porcelain",
    )
    .await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

pub async fn collect_member_patch(
    work_dir: &Path,
    patch_path: &Path,
) -> Result<(bool, usize), String> {
    let _ = run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C").arg(work_dir).arg("add").arg("-A");
            c
        },
        "git add -A",
    )
    .await?;

    let patch_output = run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C")
                .arg(work_dir)
                .arg("diff")
                .arg("--cached")
                .arg("--binary");
            c
        },
        "git diff --cached --binary",
    )
    .await?;

    if let Some(parent) = patch_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }

    tokio::fs::write(patch_path, &patch_output.stdout)
        .await
        .map_err(|e| format!("failed to write patch {}: {}", patch_path.display(), e))?;

    let names_output = run_git_output(
        {
            let mut c = tokio::process::Command::new("git");
            c.arg("-C")
                .arg(work_dir)
                .arg("diff")
                .arg("--cached")
                .arg("--name-only");
            c
        },
        "git diff --cached --name-only",
    )
    .await?;

    let names_text = String::from_utf8_lossy(&names_output.stdout);
    let changed_files = names_text.lines().filter(|l| !l.trim().is_empty()).count();
    Ok((changed_files > 0, changed_files))
}

async fn run_git_output(
    mut cmd: tokio::process::Command,
    desc: &str,
) -> Result<std::process::Output, String> {
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("{} failed to start: {}", desc, e))?;
    if !output.status.success() {
        return Err(format!("{} failed: {}", desc, summarize_output(&output)));
    }
    Ok(output)
}

fn summarize_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout: {}; stderr: {}", stdout, stderr),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("exit status {}", output.status),
    }
}

fn git_context_cache_ttl() -> Duration {
    Duration::from_secs(
        std::env::var("STAR_GIT_CONTEXT_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120)
            .clamp(5, 1800),
    )
}

fn limit_git_context_chars(content: String) -> String {
    let limit = std::env::var("STAR_GIT_CONTEXT_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8_000)
        .clamp(512, 32_000);
    if content.chars().count() <= limit {
        return content;
    }
    content.chars().take(limit).collect()
}

fn get_cached_value(
    cache: &'static Mutex<HashMap<String, GitCacheEntry>>,
    key: &str,
) -> Option<Option<String>> {
    let ttl = git_context_cache_ttl();
    let mut guard = cache.lock().expect("git context cache lock poisoned");
    if let Some(entry) = guard.get(key) {
        if entry.created_at.elapsed() <= ttl {
            return Some(entry.value.clone());
        }
    }
    guard.remove(key);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `git status --porcelain --branch` 的输出里，分支名独占第一行 `## `。
    /// 解析必须跳过它 —— 否则 `##` 会被当成一条「已暂存」记录，
    /// 状态栏永远显示「有暂存改动」。
    #[test]
    fn the_branch_header_is_not_counted_as_a_change() {
        let status = parse_status_output("## main...origin/main [ahead 1]\n").expect("parsed");
        assert_eq!(status.branch, "main");
        assert!(!status.has_staged, "分支头不是改动");
        assert!(!status.has_modified);
        assert!(!status.has_untracked);
    }

    #[test]
    fn branch_name_drops_the_upstream_and_divergence_suffix() {
        assert_eq!(branch_name("main...origin/main [ahead 1]"), "main");
        assert_eq!(branch_name("main"), "main");
        // detached HEAD 没有 `...`，也没有 upstream —— 原样保留，
        // 比退化成一段 commit sha 有用。
        assert_eq!(branch_name("HEAD (no branch)"), "HEAD (no branch)");
        // 空仓库的头多一层前缀。
        assert_eq!(branch_name("No commits yet on master"), "master");
    }

    #[test]
    fn staged_modified_and_untracked_are_all_reported() {
        let status = parse_status_output(
            "## feature/x\nM  staged.rs\n M modified.rs\n?? new.rs\nA  added.rs\n",
        )
        .expect("parsed");

        assert_eq!(status.branch, "feature/x");
        assert!(status.has_staged, "`M ` 是已暂存");
        assert!(status.has_modified, "` M` 是已修改未暂存");
        assert!(status.has_untracked, "`??` 是未跟踪");
    }

    #[test]
    fn a_clean_worktree_reports_only_the_branch() {
        let status = parse_status_output("## main\n").expect("parsed");
        assert_eq!(status.branch, "main");
        assert!(!status.has_staged);
        assert!(!status.has_modified);
        assert!(!status.has_untracked);
    }

    /// 没有 `## ` 头 = 不是工作树（或者 git 报错了）。必须返回 `None` 让轮询
    /// 停止，而不是上报一个空分支 —— 后者会让状态栏显示一个空白分支，
    /// 看起来像「在一个真的仓库里但没分支」。
    #[test]
    fn output_without_a_branch_header_is_rejected() {
        assert!(parse_status_output("").is_none());
        assert!(parse_status_output("fatal: not a git repository\n").is_none());
        assert!(parse_status_output(" M src/main.rs\n").is_none());
    }
}

fn set_cached_value(
    cache: &'static Mutex<HashMap<String, GitCacheEntry>>,
    key: String,
    value: Option<String>,
) {
    let mut guard = cache.lock().expect("git context cache lock poisoned");
    guard.insert(
        key,
        GitCacheEntry {
            created_at: Instant::now(),
            value,
        },
    );
}
