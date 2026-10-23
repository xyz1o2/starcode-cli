use crate::core::config::storage::Storage;
use crate::core::services::git_service;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn default_team_rounds() -> usize {
    1
}

fn default_team_run_mode() -> String {
    "parallel".to_string()
}

fn default_member_round() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TeamRunMemberRecord {
    pub(crate) name: String,
    pub(crate) internal_id: String,
    pub(crate) work_dir: String,
    pub(crate) target: String,
    pub(crate) isolation_mode: String,
    pub(crate) patch_path: String,
    pub(crate) has_changes: bool,
    pub(crate) changed_files: usize,
    pub(crate) success: bool,
    pub(crate) summary: String,
    pub(crate) error: Option<String>,
    pub(crate) duration_ms: u64,
    #[serde(default = "default_member_round")]
    pub(crate) round: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TeamRunRecord {
    pub(crate) run_id: String,
    pub(crate) created_at: i64,
    pub(crate) command_cwd: String,
    pub(crate) source_target: String,
    pub(crate) git_mode: bool,
    pub(crate) repo_root: Option<String>,
    pub(crate) base_head: Option<String>,
    #[serde(default = "default_team_rounds")]
    pub(crate) rounds: usize,
    #[serde(default = "default_team_run_mode")]
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) round_traces: Vec<TeamRunRoundRecord>,
    #[serde(default)]
    pub(crate) shared_memory: Vec<String>,
    pub(crate) members: Vec<TeamRunMemberRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TeamRunRoundRecord {
    pub(crate) round: usize,
    pub(crate) objective: String,
    pub(crate) success_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) changed_members: usize,
    pub(crate) duration_ms: u64,
    pub(crate) member_summaries: Vec<String>,
}

#[derive(Default)]
pub(crate) struct TeamRunCleanupReport {
    pub(crate) worktree_members: usize,
    pub(crate) removed_worktrees: usize,
    pub(crate) removed_temp_dir: bool,
    pub(crate) removed_run_dir: bool,
    pub(crate) issues: Vec<String>,
}

pub(crate) fn team_runs_root(storage: &Storage) -> PathBuf {
    storage.star_dir().join("agent-teams").join("runs")
}

pub(crate) fn team_run_dir(storage: &Storage, run_id: &str) -> PathBuf {
    team_runs_root(storage).join(run_id)
}

pub(crate) fn team_run_record_path(storage: &Storage, run_id: &str) -> PathBuf {
    team_run_dir(storage, run_id).join("run.json")
}

pub(crate) async fn save_team_run_record(
    storage: &Storage,
    run: &TeamRunRecord,
) -> Result<(), String> {
    let path = team_run_record_path(storage, &run.run_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
    }
    let text = serde_json::to_string_pretty(run)
        .map_err(|e| format!("failed to serialize team run record: {}", e))?;
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))
}

pub(crate) async fn load_team_run_record(
    storage: &Storage,
    run_id: &str,
) -> Result<TeamRunRecord, String> {
    let path = team_run_record_path(storage, run_id);
    if !path.exists() {
        return Err(format!("team run record not found: {}", path.display()));
    }

    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {}", path.display(), e))
}

pub(crate) async fn scan_team_run_records(storage: &Storage) -> Result<Vec<TeamRunRecord>, String> {
    let root = team_runs_root(storage);
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = tokio::fs::read_dir(&root)
        .await
        .map_err(|e| format!("failed to read runs root {}: {}", root.display(), e))?;

    let mut runs = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to iterate runs root {}: {}", root.display(), e))?
    {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .await
            .map_err(|e| format!("failed to inspect {}: {}", path.display(), e))?
            .is_dir();
        if !is_dir {
            continue;
        }

        let record_path = path.join("run.json");
        if !record_path.exists() {
            continue;
        }

        let text = tokio::fs::read_to_string(&record_path)
            .await
            .map_err(|e| format!("failed to read {}: {}", record_path.display(), e))?;
        let record: TeamRunRecord = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {}", record_path.display(), e))?;
        runs.push(record);
    }

    runs.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.run_id.cmp(&a.run_id))
    });
    Ok(runs)
}

pub(crate) fn summarize_output(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status: {}", output.status)
    }
}

fn pick_path_before_suffix(line: &str, suffix: &str) -> Option<String> {
    let idx = line.find(suffix)?;
    let raw = line[..idx].trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

pub(crate) fn collect_apply_conflict_files(output: &std::process::Output) -> Vec<String> {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    collect_apply_conflict_files_from_text(&combined)
}

pub(crate) fn collect_apply_conflict_files_from_text(combined: &str) -> Vec<String> {
    let mut files: BTreeSet<String> = BTreeSet::new();

    for raw_line in combined.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("error: patch failed: ") {
            if let Some(path) = pick_path_before_suffix(rest, ":") {
                files.insert(path);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("error: ") {
            if let Some(path) = pick_path_before_suffix(rest, ": patch does not apply") {
                files.insert(path);
                continue;
            }
            if let Some(path) = pick_path_before_suffix(rest, ": does not exist in index") {
                files.insert(path);
                continue;
            }
            if let Some(path) =
                pick_path_before_suffix(rest, ": already exists in working directory")
            {
                files.insert(path);
                continue;
            }
        }

        if let Some(rest) = line.strip_prefix("Checking patch ") {
            let path = rest.trim_end_matches("...").trim();
            if !path.is_empty() {
                files.insert(path.to_string());
            }
        }
    }

    files.into_iter().collect()
}

pub(crate) fn map_target_for_worktree(
    raw_target: &str,
    command_cwd: &Path,
    repo_root: &Path,
    worktree_root: &Path,
) -> String {
    let normalized = raw_target.trim();
    let normalized = if normalized.is_empty() {
        "."
    } else {
        normalized
    };
    let base_rel = command_cwd
        .strip_prefix(repo_root)
        .unwrap_or_else(|_| Path::new(""));

    if normalized == "." {
        if base_rel.as_os_str().is_empty() {
            ".".to_string()
        } else {
            base_rel.to_string_lossy().to_string()
        }
    } else {
        let p = PathBuf::from(normalized);
        if p.is_absolute() {
            if let Ok(rel) = p.strip_prefix(repo_root) {
                worktree_root.join(rel).to_string_lossy().to_string()
            } else {
                ".".to_string()
            }
        } else {
            let mapped = if base_rel.as_os_str().is_empty() {
                p
            } else {
                base_rel.join(p)
            };
            mapped.to_string_lossy().to_string()
        }
    }
}

fn path_has_component(path: &Path, segment: &str) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(segment))
}

async fn remove_path_if_exists(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("failed to inspect {}: {}", path.display(), e))?;
    if metadata.is_dir() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|e| format!("failed to remove directory {}: {}", path.display(), e))?;
    } else {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| format!("failed to remove file {}: {}", path.display(), e))?;
    }
    Ok(true)
}

pub(crate) async fn cleanup_team_run_artifacts(
    storage: &Storage,
    safe_run_id: &str,
    run: &TeamRunRecord,
) -> TeamRunCleanupReport {
    let mut report = TeamRunCleanupReport::default();
    let repo_root = run.repo_root.as_ref().map(PathBuf::from);

    for member in &run.members {
        if member.isolation_mode != "git-worktree" {
            continue;
        }
        report.worktree_members += 1;
        let work_dir = PathBuf::from(&member.work_dir);

        if !path_has_component(&work_dir, "agent-teams")
            || !path_has_component(&work_dir, safe_run_id)
        {
            report.issues.push(format!(
                "member `{}` work_dir not in expected agent-teams path: {}",
                member.name,
                work_dir.display()
            ));
            continue;
        }

        if let Some(root) = repo_root.as_ref() {
            git_service::worktree_remove_force(root, &work_dir).await;
        }

        match remove_path_if_exists(&work_dir).await {
            Ok(_) => report.removed_worktrees += 1,
            Err(e) => report
                .issues
                .push(format!("member `{}` cleanup failed: {}", member.name, e)),
        }
    }

    let temp_run_dir = storage
        .project_temp_dir()
        .join("agent-teams")
        .join(safe_run_id);
    match remove_path_if_exists(&temp_run_dir).await {
        Ok(removed) => report.removed_temp_dir = removed,
        Err(e) => report.issues.push(e),
    }

    let run_dir = team_run_dir(storage, safe_run_id);
    match remove_path_if_exists(&run_dir).await {
        Ok(removed) => report.removed_run_dir = removed,
        Err(e) => report.issues.push(e),
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage_in(dir: &Path) -> Storage {
        Storage::new(dir.to_path_buf())
    }

    fn member(name: &str, isolation: &str, work_dir: &str) -> TeamRunMemberRecord {
        TeamRunMemberRecord {
            name: name.to_string(),
            internal_id: format!("{}-id", name),
            work_dir: work_dir.to_string(),
            target: ".".to_string(),
            isolation_mode: isolation.to_string(),
            patch_path: String::new(),
            has_changes: false,
            changed_files: 0,
            success: true,
            summary: "ok".to_string(),
            error: None,
            duration_ms: 1,
            round: 1,
        }
    }

    fn run_record(
        run_id: &str,
        created_at: i64,
        members: Vec<TeamRunMemberRecord>,
    ) -> TeamRunRecord {
        TeamRunRecord {
            run_id: run_id.to_string(),
            created_at,
            command_cwd: "/tmp/project".to_string(),
            source_target: ".".to_string(),
            git_mode: false,
            repo_root: None,
            base_head: None,
            rounds: 1,
            mode: "parallel".to_string(),
            round_traces: Vec::new(),
            shared_memory: Vec::new(),
            members,
        }
    }

    // ── 记录读写 ────────────────────────────────────────────────

    #[tokio::test]
    async fn run_records_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let record = run_record(
            "run-1",
            1_700_000_000,
            vec![member("a", "shared", "/tmp/a")],
        );

        save_team_run_record(&storage, &record).await.unwrap();
        assert!(team_run_record_path(&storage, "run-1").exists());

        let loaded = load_team_run_record(&storage, "run-1").await.unwrap();
        assert_eq!(loaded.run_id, "run-1");
        assert_eq!(loaded.members.len(), 1);
        assert_eq!(loaded.members[0].name, "a");
    }

    #[tokio::test]
    async fn loading_a_missing_run_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let err = load_team_run_record(&storage, "nope").await.unwrap_err();
        assert!(err.starts_with("team run record not found:"), "{err}");
        assert!(err.contains("nope"), "{err}");
    }

    /// 扫描按 created_at 倒序：`/agents team runs` 的第一条必须是最近一次。
    #[tokio::test]
    async fn scanning_returns_the_newest_run_first() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        save_team_run_record(&storage, &run_record("old", 100, vec![]))
            .await
            .unwrap();
        save_team_run_record(&storage, &run_record("new", 300, vec![]))
            .await
            .unwrap();
        save_team_run_record(&storage, &run_record("mid", 200, vec![]))
            .await
            .unwrap();

        let runs = scan_team_run_records(&storage).await.unwrap();
        let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn scanning_skips_directories_without_a_record() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        save_team_run_record(&storage, &run_record("real", 1, vec![]))
            .await
            .unwrap();
        // 一个空目录 + 一个普通文件都不该让扫描报错
        std::fs::create_dir_all(team_run_dir(&storage, "empty")).unwrap();
        std::fs::write(team_runs_root(&storage).join("stray.txt"), "x").unwrap();

        let runs = scan_team_run_records(&storage).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "real");
    }

    #[tokio::test]
    async fn scanning_an_absent_root_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let runs = scan_team_run_records(&storage_in(dir.path()))
            .await
            .unwrap();
        assert!(runs.is_empty());
    }

    // ── 输出摘要 ────────────────────────────────────────────────

    fn output(stdout: &str, stderr: &str, code: i32) -> std::process::Output {
        use std::os::unix::process::ExitStatusExt;
        std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// stderr 优先：git 的失败原因只写在 stderr，stdout 里的进度行会误导人。
    #[test]
    fn summarize_output_prefers_stderr_then_stdout_then_exit_status() {
        assert_eq!(summarize_output(&output("progress", "boom", 1)), "boom");
        assert_eq!(summarize_output(&output("done", "", 0)), "done");
        assert_eq!(
            summarize_output(&output("", "", 0)),
            "exit status: exit status: 0"
        );
    }

    // ── 冲突文件解析 ────────────────────────────────────────────

    /// apply 失败时用户最需要的是"哪几个文件撞了"。git 有四种说法，
    /// 少认一种就等于少报一个冲突文件。
    #[test]
    fn conflict_files_are_collected_from_every_git_wording() {
        let text = "\
error: patch failed: src/a.rs:12
error: src/a.rs: patch does not apply
error: src/b.rs: does not exist in index
error: src/c.rs: already exists in working directory
Checking patch src/d.rs...
";
        let files = collect_apply_conflict_files_from_text(text);
        assert_eq!(files, ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"]);
    }

    #[test]
    fn conflict_files_are_deduplicated_and_sorted() {
        let text =
            "error: patch failed: z.rs:1\nerror: patch failed: a.rs:1\nerror: patch failed: z.rs:9";
        assert_eq!(
            collect_apply_conflict_files_from_text(text),
            ["a.rs", "z.rs"]
        );
    }

    #[test]
    fn unrelated_output_yields_no_conflicts() {
        let files = collect_apply_conflict_files_from_text(
            "Applying: fix\nAlready up to date.\nerror: something else happened",
        );
        assert!(files.is_empty(), "{files:?}");
        assert!(collect_apply_conflict_files(&output("", "", 0)).is_empty());
    }

    // ── worktree 目标映射 ───────────────────────────────────────

    #[test]
    fn a_dot_target_maps_to_the_command_directory_relative_to_the_repo() {
        let repo = Path::new("/repo");
        let cwd = Path::new("/repo/crates/app");
        let wt = Path::new("/tmp/wt");

        assert_eq!(map_target_for_worktree("", cwd, repo, wt), "crates/app");
        assert_eq!(map_target_for_worktree(".", repo, repo, wt), ".");
    }

    #[test]
    fn relative_targets_are_joined_onto_the_command_directory() {
        assert_eq!(
            map_target_for_worktree(
                "src/lib.rs",
                Path::new("/repo/app"),
                Path::new("/repo"),
                Path::new("/wt")
            ),
            "app/src/lib.rs"
        );
    }

    /// 绝对路径落在仓库外时映射成 `.`：worktree 里没有那个文件，
    /// 编一个越界路径只会让成员在别人的目录上乱跑。
    #[test]
    fn absolute_targets_inside_the_repo_are_rebased_onto_the_worktree() {
        assert_eq!(
            map_target_for_worktree(
                "/repo/src/lib.rs",
                Path::new("/repo"),
                Path::new("/repo"),
                Path::new("/wt")
            ),
            "/wt/src/lib.rs"
        );
        assert_eq!(
            map_target_for_worktree(
                "/etc/passwd",
                Path::new("/repo"),
                Path::new("/repo"),
                Path::new("/wt")
            ),
            "."
        );
    }

    // ── 清理 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn cleanup_removes_the_run_dir_and_the_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        save_team_run_record(&storage, &run_record("run-1", 1, vec![]))
            .await
            .unwrap();
        let temp_run = storage.project_temp_dir().join("agent-teams").join("run-1");
        std::fs::create_dir_all(&temp_run).unwrap();

        let record = load_team_run_record(&storage, "run-1").await.unwrap();
        let report = cleanup_team_run_artifacts(&storage, "run-1", &record).await;

        assert!(report.removed_run_dir);
        assert!(report.removed_temp_dir);
        assert!(!team_run_dir(&storage, "run-1").exists());
        assert!(!temp_run.exists());
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    /// 共享目录的成员不是 worktree，清理时不能去删它的工作目录 ——
    /// 那可能是用户的项目本身。
    #[tokio::test]
    async fn cleanup_leaves_shared_members_alone() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let shared_dir = dir.path().join("project");
        std::fs::create_dir_all(&shared_dir).unwrap();
        let record = run_record(
            "run-1",
            1,
            vec![member(
                "a",
                "shared(non-git)",
                &shared_dir.to_string_lossy(),
            )],
        );

        let report = cleanup_team_run_artifacts(&storage, "run-1", &record).await;
        assert_eq!(report.worktree_members, 0);
        assert!(shared_dir.exists(), "the project directory must survive");
    }

    /// 路径里没有 `agent-teams/<run_id>` 的成员目录一律不动：这个判断是
    /// 唯一阻止"清理误删用户目录"的护栏。
    #[tokio::test]
    async fn cleanup_refuses_worktrees_outside_the_agent_teams_root() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let outside = dir.path().join("user-worktree");
        std::fs::create_dir_all(&outside).unwrap();

        let record = run_record(
            "run-1",
            1,
            vec![member("a", "git-worktree", &outside.to_string_lossy())],
        );
        let report = cleanup_team_run_artifacts(&storage, "run-1", &record).await;

        assert_eq!(report.worktree_members, 1);
        assert_eq!(report.removed_worktrees, 0);
        assert!(outside.exists(), "must not delete an unexpected directory");
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.contains("not in expected agent-teams path")),
            "{:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn cleanup_removes_a_worktree_that_lives_under_the_run_id() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage_in(dir.path());
        let wt = storage
            .project_temp_dir()
            .join("agent-teams")
            .join("run-1")
            .join("member-a");
        std::fs::create_dir_all(&wt).unwrap();

        let record = run_record(
            "run-1",
            1,
            vec![member("a", "git-worktree", &wt.to_string_lossy())],
        );
        let report = cleanup_team_run_artifacts(&storage, "run-1", &record).await;

        assert_eq!(report.removed_worktrees, 1);
        assert!(!wt.exists());
    }
}
