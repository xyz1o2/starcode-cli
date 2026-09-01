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
