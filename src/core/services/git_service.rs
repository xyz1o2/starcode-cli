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

pub async fn get_git_status<P: AsRef<Path>>(cwd: P) -> Option<GitStatus> {
    let mut status = GitStatus::default();

    // Add timeout to prevent blocking on slow filesystems (e.g., WSL2 network drives)
    const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    let branch_output = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&cwd)
            .output(),
    )
    .await;

    let branch_output = match branch_output {
        Ok(result) => result,
        Err(_) => {
            crate::utils::logging::append_debug_log_line("[GIT] symbolic-ref timed out");
            return None;
        }
    };

    if let Ok(output) = branch_output {
        if output.status.success() {
            status.branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        } else {
            let rev_output = tokio::time::timeout(
                GIT_TIMEOUT,
                tokio::process::Command::new("git")
                    .args(["rev-parse", "--short", "HEAD"])
                    .current_dir(&cwd)
                    .output(),
            )
            .await;

            let rev_output = match rev_output {
                Ok(result) => result,
                Err(_) => {
                    crate::utils::logging::append_debug_log_line("[GIT] rev-parse timed out");
                    return None;
                }
            };

            if let Ok(rev) = rev_output {
                if rev.status.success() {
                    status.branch = String::from_utf8_lossy(&rev.stdout).trim().to_string();
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
    } else {
        return None;
    }

    let status_output = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&cwd)
            .output(),
    )
    .await;

    let status_output = match status_output {
        Ok(result) => result,
        Err(_) => {
            crate::utils::logging::append_debug_log_line("[GIT] status --porcelain timed out");
            return None;
        }
    };

    if let Ok(output) = status_output {
        if output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            for line in out_str.lines() {
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
        }
    }

    Some(status)
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
