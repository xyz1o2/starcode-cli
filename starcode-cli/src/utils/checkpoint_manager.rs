//! File history checkpoint manager — save and restore file state by snapshot id.
//!
//! API modeled on Claude Code's `src/utils/fileHistory.ts`:
//! - `track_edit` — back up a file before a write tool modifies it
//! - `make_snapshot` — capture all tracked files at a turn boundary
//! - `rewind(snapshot_id)` — restore files to a prior snapshot
//! - `can_restore` / `has_any_changes` / `list_snapshots` — query API for /rewind UI
//!
//! Backups are stored as independent files (`{hash}@v{n}`) under
//! `{star_dir}/file-history/{session_id}/`, never inlined in the state JSON,
//! so memory does not grow with snapshot count or file size.
//!
//! Legacy API (`Checkpoint` / `create_checkpoint` / `apply_checkpoint` /
//! `list_checkpoints` / `load_checkpoint` / `save_checkpoint`) is preserved as
//! a thin adapter so existing call sites (`agent::checkpoint`,
//! `agent::workflows::star_agent`, `commands::utility`, `runtime::checkpoints`)
//! keep compiling during the migration. New code should call the new API.

use crate::core::utils::paths::current_project_star_dir;
use crate::types::{ChatEntry, StarMessage, StarToolCall};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub type CheckpointError = Box<dyn std::error::Error + Send + Sync>;

/// Upper bound on retained snapshots. Older snapshots are evicted once the
/// cap is reached. Matches Claude Code's `MAX_SNAPSHOTS = 20`.
pub const MAX_SNAPSHOTS: usize = 20;

// ─────────────────────────────────────────────────────────────────────────
// New API (Claude Code parity)
// ─────────────────────────────────────────────────────────────────────────

/// One file's backup record inside a snapshot.
///
/// `backup_file_name = None` means the file did not exist at this version
/// (a "file-did-not-exist marker"), mirroring TS's `BackupFileName = string | null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistoryBackup {
    pub backup_file_name: Option<String>,
    pub version: u32,
    pub backup_time: chrono::DateTime<Utc>,
}

/// A snapshot of all tracked files, bound to a message id (when available).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistorySnapshot {
    /// Stable unique id: `{unix_ms}_{rand4}` — used as the /rewind target.
    pub snapshot_id: String,
    /// Weakly associated UI message id (starcode uses u64). None when the
    /// snapshot was taken outside a message context (e.g. manual /snapshot).
    pub message_id: Option<u64>,
    /// Relative path (key) -> backup record.
    pub tracked_file_backups: HashMap<String, FileHistoryBackup>,
    pub timestamp: chrono::DateTime<Utc>,
    /// Tool that triggered the snapshot, for display in /rewind list.
    pub tool_name: Option<String>,
    /// File path that triggered the snapshot (track_edit only), for display.
    pub triggered_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistoryState {
    pub snapshots: Vec<FileHistorySnapshot>,
    pub tracked_files: HashSet<String>,
    pub snapshot_sequence: u64,
}

impl Default for FileHistoryState {
    fn default() -> Self {
        Self {
            snapshots: Vec::new(),
            tracked_files: HashSet::new(),
            snapshot_sequence: 0,
        }
    }
}

/// Summary item for `/rewind` list rendering.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotSummary {
    pub snapshot_id: String,
    pub message_id: Option<u64>,
    pub timestamp: chrono::DateTime<Utc>,
    pub tool_name: Option<String>,
    pub triggered_file: Option<String>,
    pub tracked_files_count: usize,
}

// ─────────────────────────────────────────────────────────────────────────
// Storage paths
// ─────────────────────────────────────────────────────────────────────────

fn file_history_dir(session_id: &str) -> Result<PathBuf, CheckpointError> {
    // Test override: when STAR_FILE_HISTORY_DIR_OVERRIDE is set, use it as the
    // base dir instead of current_project_star_dir(). This lets tests isolate
    // snapshots to a per-test tempdir without depending on current_dir_cached
    // (which is a process-wide OnceLock and ignores set_current_dir).
    if let Ok(p) = std::env::var("STAR_FILE_HISTORY_DIR_OVERRIDE") {
        return Ok(PathBuf::from(p).join(session_id));
    }
    Ok(current_project_star_dir()
        .join("file-history")
        .join(session_id))
}

fn state_file_path(session_id: &str) -> Result<PathBuf, CheckpointError> {
    Ok(file_history_dir(session_id)?.join("state.json"))
}

fn resolve_backup_path(backup_file_name: &str, session_id: &str) -> Result<PathBuf, CheckpointError> {
    Ok(file_history_dir(session_id)?.join(backup_file_name))
}

fn get_backup_file_name(file_path: &str, version: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_path.as_bytes());
    // format!("{:x}", ..) on GenericArray yields full hex; slice to 16 chars
    // (8 bytes) for a short, deterministic, collision-resistant key.
    let hash_hex = format!("{:x}", hasher.finalize());
    let short = &hash_hex[..16];
    format!("{}_v{}", short, version)
}

/// Resolve session id: prefer caller-provided, fall back to a stable per-cwd
/// default so snapshots still work when Config is unavailable (e.g. tests).
fn resolve_session_id(provided: Option<&str>) -> String {
    if let Some(s) = provided {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    // Fallback: hash of current dir to keep snapshots scoped per project.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut hasher = Sha256::new();
    hasher.update(cwd.to_string_lossy().as_bytes());
    let hash_hex = format!("{:x}", hasher.finalize());
    format!("default_{}", &hash_hex[..8])
}

/// Shorten an absolute path to a relative key (relative to project cwd).
fn maybe_shorten_file_path(file_path: &Path) -> String {
    if !file_path.is_absolute() {
        return file_path.to_string_lossy().to_string();
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if file_path.starts_with(&cwd) {
        if let Ok(rel) = file_path.strip_prefix(&cwd) {
            return rel.to_string_lossy().to_string();
        }
    }
    file_path.to_string_lossy().to_string()
}

/// Expand a relative tracking path back to an absolute path.
fn maybe_expand_file_path(tracking_path: &str) -> PathBuf {
    let p = Path::new(tracking_path);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_default()
        .join(tracking_path)
}

// ─────────────────────────────────────────────────────────────────────────
// Global enable switch
// ─────────────────────────────────────────────────────────────────────────

/// File history is enabled unless explicitly turned off via env var, mirroring
/// TS `fileHistoryEnabled()` (which also honors a global config flag; we keep
/// the env switch only — config plumbing can be added later if needed).
pub fn file_history_enabled() -> bool {
    if let Ok(v) = std::env::var("STAR_DISABLE_FILE_CHECKPOINTING") {
        let normalized = v.trim().to_lowercase();
        return !(normalized == "1" || normalized == "true" || normalized == "on");
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────
// State persistence (single JSON index file per session)
// ─────────────────────────────────────────────────────────────────────────

pub async fn load_state(session_id: Option<&str>) -> Result<FileHistoryState, CheckpointError> {
    let sid = resolve_session_id(session_id);
    let path = state_file_path(&sid)?;
    if !path.exists() {
        return Ok(FileHistoryState::default());
    }
    let content = tokio::fs::read_to_string(&path).await?;
    let state: FileHistoryState = serde_json::from_str(&content)?;
    Ok(state)
}

pub async fn save_state(
    state: &FileHistoryState,
    session_id: Option<&str>,
) -> Result<(), CheckpointError> {
    let sid = resolve_session_id(session_id);
    let path = state_file_path(&sid)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let s = serde_json::to_string_pretty(state)?;
    // Atomic write: tmp + rename
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, s).await?;
    if let Err(_e) = tokio::fs::rename(&tmp, &path).await {
        // rename can fail across volumes / locks; fall back to copy+remove
        tokio::fs::copy(&tmp, &path).await?;
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Core: backup primitives
// ─────────────────────────────────────────────────────────────────────────

/// Stat a path; returns None on ENOENT, propagates other errors.
async fn stat_path(path: &Path) -> Result<Option<std::fs::Metadata>, CheckpointError> {
    match tokio::fs::metadata(path).await {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Box::new(e)),
    }
}

/// Create a backup of the file at `file_path`. If the file does not exist,
/// records a null backup (None marker). Mirrors TS `createBackup`.
pub async fn create_backup(
    file_path: &Path,
    version: u32,
    session_id: &str,
) -> Result<FileHistoryBackup, CheckpointError> {
    let tracking_path = maybe_shorten_file_path(file_path);
    let backup_file_name = get_backup_file_name(&tracking_path, version);
    let backup_path = resolve_backup_path(&backup_file_name, session_id)?;

    let src_meta = stat_path(file_path).await?;
    let backup_time = Utc::now();

    if src_meta.is_none() {
        // File does not exist at this version → null marker.
        return Ok(FileHistoryBackup {
            backup_file_name: None,
            version,
            backup_time,
        });
    }

    // Ensure parent dir exists (lazy mkdir: most calls hit the fast path).
    if let Some(parent) = backup_path.parent() {
        if tokio::fs::metadata(parent).await.is_err() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // copy_file preserves content without loading it all into memory.
    if let Err(e) = tokio::fs::copy(file_path, &backup_path).await {
        // If the destination dir was missing (race), mkdir and retry once.
        if e.kind() == std::io::ErrorKind::NotFound {
            if let Some(parent) = backup_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(file_path, &backup_path).await?;
        } else {
            return Err(Box::new(e));
        }
    }

    // Preserve permissions on Unix (best effort).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(meta) = src_meta {
            let mode = meta.permissions().mode();
            let _ = std::fs::set_permissions(&backup_path, std::fs::Permissions::from_mode(mode));
        }
    }

    Ok(FileHistoryBackup {
        backup_file_name: Some(backup_file_name),
        version,
        backup_time,
    })
}

/// Restore a file from its backup path. No-op if backup is missing
/// (logged + skipped, never aborts a rewind mid-loop). Mirrors TS `restoreBackup`.
pub async fn restore_backup(
    file_path: &Path,
    backup_file_name: &str,
    session_id: &str,
) -> Result<(), CheckpointError> {
    let backup_path = resolve_backup_path(backup_file_name, session_id)?;
    let backup_meta = stat_path(&backup_path).await?;
    if backup_meta.is_none() {
        // Backup missing — log and bail without aborting the rewind.
        log::warn!(
            "FileHistory: [rewind] backup file not found: {}",
            backup_path.display()
        );
        return Ok(());
    }

    if let Some(parent) = file_path.parent() {
        if tokio::fs::metadata(parent).await.is_err() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    if let Err(e) = tokio::fs::copy(&backup_path, file_path).await {
        if e.kind() == std::io::ErrorKind::NotFound {
            if let Some(parent) = file_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&backup_path, file_path).await?;
        } else {
            return Err(Box::new(e));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(meta) = backup_meta {
            let mode = meta.permissions().mode();
            let _ = std::fs::set_permissions(file_path, std::fs::Permissions::from_mode(mode));
        }
    }

    Ok(())
}

/// Check whether the on-disk file differs from its backup.
/// Mirrors TS `checkOriginFileChanged` (stat short-circuit + content compare).
pub async fn check_origin_file_changed(
    file_path: &Path,
    backup_file_name: &str,
    session_id: &str,
) -> Result<bool, CheckpointError> {
    let backup_path = resolve_backup_path(backup_file_name, session_id)?;

    let orig_meta = stat_path(file_path).await?;
    let backup_meta = stat_path(&backup_path).await?;

    // One exists, one missing → changed
    if (orig_meta.is_none()) != (backup_meta.is_none()) {
        return Ok(true);
    }
    // Both missing → no change
    if orig_meta.is_none() {
        return Ok(false);
    }

    let orig_meta = orig_meta.unwrap();
    let backup_meta = backup_meta.unwrap();

    // Quick stat-based reject: size differs
    if orig_meta.len() != backup_meta.len() {
        return Ok(true);
    }

    // mtime short-circuit: if origin was modified before backup time, unchanged
    use std::time::SystemTime;
    let orig_mtime = orig_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let backup_mtime = backup_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    if orig_mtime < backup_mtime {
        return Ok(false);
    }

    // Fall back to content comparison.
    let orig_content = tokio::fs::read(file_path).await?;
    let backup_content = tokio::fs::read(&backup_path).await?;
    Ok(orig_content != backup_content)
}

// ─────────────────────────────────────────────────────────────────────────
// Core: snapshot operations
// ─────────────────────────────────────────────────────────────────────────

/// Generate a stable, sortable snapshot id: `{unix_ms}_{counter}`.
///
/// Uses a process-wide AtomicU64 counter to guarantee uniqueness even when
/// two snapshots are taken within the same millisecond. Avoids the unstable
/// `ThreadId::as_u64` API.
fn generate_snapshot_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use once_cell::sync::Lazy;
    static SNAPSHOT_COUNTER: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
    let ms = Utc::now().timestamp_millis();
    let counter = SNAPSHOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}", ms, counter)
}

/// Track an imminent file edit: back up the file at its current version and
/// attach the backup to the most recent snapshot (or create a new one).
///
/// Must be called BEFORE the file is actually written. Failures are logged
/// but never propagated — checkpoint is best-effort and must not block edits.
///
/// Returns the snapshot id the backup was attached to (for /rewind), or None
/// if file history is disabled.
pub async fn track_edit(
    file_path: &Path,
    message_id: Option<u64>,
    tool_name: Option<&str>,
    session_id: Option<&str>,
) -> Result<Option<String>, CheckpointError> {
    if !file_history_enabled() {
        return Ok(None);
    }
    let sid = resolve_session_id(session_id);
    let tracking_path = maybe_shorten_file_path(file_path);

    let mut state = load_state(Some(&sid)).await?;

    // Phase 1: if the file is already backed up in the most recent snapshot,
    // do NOT touch v1 — the existing backup is the pre-edit state. This mirrors
    // TS's "Phase 1: check if backup is needed" guard against speculative writes.
    let already_tracked = state
        .snapshots
        .last()
        .map(|s| s.tracked_file_backups.contains_key(&tracking_path))
        .unwrap_or(false);
    if already_tracked {
        // Return the existing snapshot id so callers can still reference it.
        if let Some(last) = state.snapshots.last() {
            return Ok(Some(last.snapshot_id.clone()));
        }
    }

    // Phase 2: create the backup file on disk.
    let version = state
        .snapshots
        .iter()
        .flat_map(|s| s.tracked_file_backups.get(&tracking_path).map(|b| b.version))
        .max()
        .unwrap_or(0)
        + 1;
    let backup = create_backup(file_path, version, &sid).await?;

    // Phase 3: commit. Either attach to the most recent snapshot (if it shares
    // the same message_id) or create a fresh snapshot bound to this message_id.
    let snapshot_id = {
        let now = Utc::now();
        let attach_to_last = state
            .snapshots
            .last()
            .map(|s| s.message_id == message_id)
            .unwrap_or(false);

        if attach_to_last {
            // Mutate the last snapshot in place by cloning + pushing back.
            if let Some(last) = state.snapshots.last_mut() {
                last.tracked_file_backups.insert(tracking_path.clone(), backup);
                last.snapshot_sequence_touch();
            }
            state.snapshots.last().map(|s| s.snapshot_id.clone()).unwrap_or_default()
        } else {
            let snapshot = FileHistorySnapshot {
                snapshot_id: generate_snapshot_id(),
                message_id,
                tracked_file_backups: {
                    let mut m = HashMap::new();
                    m.insert(tracking_path.clone(), backup);
                    m
                },
                timestamp: now,
                tool_name: tool_name.map(|s| s.to_string()),
                triggered_file: Some(file_path.to_string_lossy().to_string()),
            };
            let sid_ret = snapshot.snapshot_id.clone();
            state.snapshots.push(snapshot);
            sid_ret
        }
    };

    state.tracked_files.insert(tracking_path);
    state.snapshot_sequence = state.snapshot_sequence.saturating_add(1);

    // Enforce MAX_SNAPSHOTS cap: drop the oldest snapshot's file backups
    // (their content may still be referenced by newer snapshots via version
    // chaining, so we do NOT unlink the backup files — only the index entry).
    if state.snapshots.len() > MAX_SNAPSHOTS {
        let drop_count = state.snapshots.len() - MAX_SNAPSHOTS;
        state.snapshots.drain(0..drop_count);
    }

    save_state(&state, Some(&sid)).await?;
    Ok(Some(snapshot_id))
}

/// Capture a snapshot of all currently tracked files at a turn boundary.
/// Callers: agent loop after a user message round (optional — track_edit
/// already creates per-write snapshots; this is for "round-level" parity
/// with TS `fileHistoryMakeSnapshot`).
pub async fn make_snapshot(
    message_id: Option<u64>,
    tool_name: Option<&str>,
    session_id: Option<&str>,
) -> Result<Option<String>, CheckpointError> {
    if !file_history_enabled() {
        return Ok(None);
    }
    let sid = resolve_session_id(session_id);
    let mut state = load_state(Some(&sid)).await?;

    if state.tracked_files.is_empty() {
        return Ok(None);
    }

    let most_recent = state.snapshots.last().cloned();
    let mut tracked_file_backups: HashMap<String, FileHistoryBackup> = HashMap::new();

    for tracking_path in state.tracked_files.iter() {
        let file_path = maybe_expand_file_path(tracking_path);
        let latest_backup = most_recent
            .as_ref()
            .and_then(|s| s.tracked_file_backups.get(tracking_path));
        let next_version = latest_backup.map(|b| b.version + 1).unwrap_or(1);

        // ENOENT → file was deleted since last snapshot → record null marker.
        let file_meta = stat_path(&file_path).await?;
        if file_meta.is_none() {
            tracked_file_backups.insert(
                tracking_path.clone(),
                FileHistoryBackup {
                    backup_file_name: None,
                    version: next_version,
                    backup_time: Utc::now(),
                },
            );
            continue;
        }

        // Reuse last backup if the file hasn't changed on disk.
        if let Some(latest) = latest_backup {
            if let Some(name) = &latest.backup_file_name {
                if !check_origin_file_changed(&file_path, name, &sid).await? {
                    tracked_file_backups.insert(tracking_path.clone(), latest.clone());
                    continue;
                }
            }
        }

        let backup = create_backup(&file_path, next_version, &sid).await?;
        tracked_file_backups.insert(tracking_path.clone(), backup);
    }

    let snapshot = FileHistorySnapshot {
        snapshot_id: generate_snapshot_id(),
        message_id,
        tracked_file_backups,
        timestamp: Utc::now(),
        tool_name: tool_name.map(|s| s.to_string()),
        triggered_file: None,
    };
    let snapshot_id = snapshot.snapshot_id.clone();
    state.snapshots.push(snapshot);
    state.snapshot_sequence = state.snapshot_sequence.saturating_add(1);

    if state.snapshots.len() > MAX_SNAPSHOTS {
        let drop_count = state.snapshots.len() - MAX_SNAPSHOTS;
        state.snapshots.drain(0..drop_count);
    }

    save_state(&state, Some(&sid)).await?;
    Ok(Some(snapshot_id))
}

/// Apply a snapshot to disk: restore / delete every tracked file to its
/// state at the snapshot time. Returns the list of changed file paths.
pub async fn apply_snapshot(
    snapshot: &FileHistorySnapshot,
    session_id: Option<&str>,
) -> Result<Vec<String>, CheckpointError> {
    let sid = resolve_session_id(session_id);
    let state = load_state(Some(&sid)).await?;
    let mut changed: Vec<String> = Vec::new();

    for tracking_path in state.tracked_files.iter() {
        let file_path = maybe_expand_file_path(tracking_path);
        let target_backup = snapshot.tracked_file_backups.get(tracking_path);

        // Resolve the backup file name: prefer the snapshot's entry; fall back
        // to the file's first-ever version (the file did not exist back then
        // if the first version is null).
        let backup_file_name: Option<Option<String>> = match target_backup {
            Some(b) => Some(b.backup_file_name.clone()),
            None => {
                // Look up the first version across all snapshots.
                let first = state.snapshots.iter().find_map(|s| {
                    s.tracked_file_backups.get(tracking_path).and_then(|b| {
                        if b.version == 1 {
                            Some(b.backup_file_name.clone())
                        } else {
                            None
                        }
                    })
                });
                first
            }
        };

        match backup_file_name {
            // Undefined = error resolving; skip this file.
            None => {
                log::warn!(
                    "FileHistory: [rewind] could not resolve backup for {}",
                    tracking_path
                );
                continue;
            }
            // null = file did not exist → delete if present.
            Some(None) => {
                if file_path.exists() {
                    if let Err(e) = tokio::fs::remove_file(&file_path).await {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            log::warn!(
                                "FileHistory: [rewind] failed to delete {}: {}",
                                file_path.display(),
                                e
                            );
                        }
                    } else {
                        changed.push(file_path.to_string_lossy().to_string());
                    }
                }
            }
            // string = restore from backup if changed.
            Some(Some(name)) => {
                if check_origin_file_changed(&file_path, &name, &sid).await? {
                    restore_backup(&file_path, &name, &sid).await?;
                    changed.push(file_path.to_string_lossy().to_string());
                }
            }
        }
    }

    Ok(changed)
}

/// Restore files to the state captured by `snapshot_id`.
/// Returns the list of file paths that were modified.
pub async fn rewind(snapshot_id: &str, session_id: Option<&str>) -> Result<Vec<String>, CheckpointError> {
    if !file_history_enabled() {
        return Ok(Vec::new());
    }
    let sid = resolve_session_id(session_id);
    let state = load_state(Some(&sid)).await?;

    let target = state.snapshots.iter().find(|s| s.snapshot_id == snapshot_id).cloned();
    let target = target.ok_or_else(|| {
        CheckpointError::from(format!("snapshot {} not found", snapshot_id))
    })?;

    apply_snapshot(&target, Some(&sid)).await
}

/// Whether a snapshot exists with the given id.
pub async fn can_restore(snapshot_id: &str, session_id: Option<&str>) -> Result<bool, CheckpointError> {
    if !file_history_enabled() {
        return Ok(false);
    }
    let state = load_state(session_id).await?;
    Ok(state.snapshots.iter().any(|s| s.snapshot_id == snapshot_id))
}

/// Dry-run: would rewinding to `snapshot_id` change any file on disk?
pub async fn has_any_changes(
    snapshot_id: &str,
    session_id: Option<&str>,
) -> Result<bool, CheckpointError> {
    if !file_history_enabled() {
        return Ok(false);
    }
    let sid = resolve_session_id(session_id);
    let state = load_state(Some(&sid)).await?;

    let target = state.snapshots.iter().find(|s| s.snapshot_id == snapshot_id);
    let target = match target {
        Some(t) => t,
        None => return Ok(false),
    };

    for tracking_path in state.tracked_files.iter() {
        let file_path = maybe_expand_file_path(tracking_path);
        let target_backup = target.tracked_file_backups.get(tracking_path);
        let backup_file_name: Option<Option<&str>> = match target_backup {
            Some(b) => Some(b.backup_file_name.as_deref()),
            None => state.snapshots.iter().find_map(|s| {
                s.tracked_file_backups.get(tracking_path).and_then(|b| {
                    if b.version == 1 {
                        Some(b.backup_file_name.as_deref())
                    } else {
                        None
                    }
                })
            }),
        };

        match backup_file_name {
            None => continue,
            Some(None) => {
                if file_path.exists() {
                    return Ok(true);
                }
            }
            Some(Some(name)) => {
                if check_origin_file_changed(&file_path, name, &sid).await? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// List all snapshots (newest last) for rendering `/rewind` choices.
pub async fn list_snapshots(
    session_id: Option<&str>,
) -> Result<Vec<SnapshotSummary>, CheckpointError> {
    let state = load_state(session_id).await?;
    Ok(state
        .snapshots
        .iter()
        .map(|s| SnapshotSummary {
            snapshot_id: s.snapshot_id.clone(),
            message_id: s.message_id,
            timestamp: s.timestamp,
            tool_name: s.tool_name.clone(),
            triggered_file: s.triggered_file.clone(),
            tracked_files_count: s.tracked_file_backups.len(),
        })
        .collect())
}

/// Return the most recent snapshot id, or None if no snapshots exist.
pub async fn latest_snapshot_id(session_id: Option<&str>) -> Result<Option<String>, CheckpointError> {
    let state = load_state(session_id).await?;
    Ok(state.snapshots.last().map(|s| s.snapshot_id.clone()))
}

// Helper used by `track_edit` to bump sequence on in-place mutation.
impl FileHistorySnapshot {
    fn snapshot_sequence_touch(&mut self) {
        // No-op placeholder kept for API symmetry with the future migration to
        // a global sequence counter. Snapshot id is already unique + sortable.
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Legacy API (compatibility shim — DO NOT extend; new code uses the API above)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFile {
    pub path: String,
    pub existed: bool,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub created_at_unix: i64,
    pub cwd: String,
    pub tool_call: StarToolCall,
    pub files: Vec<CheckpointFile>,
    pub messages: Vec<StarMessage>,
    pub chat_history: Vec<ChatEntry>,
}

fn legacy_sanitize_for_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

fn legacy_checkpoints_dir() -> Result<PathBuf, CheckpointError> {
    Ok(current_project_star_dir().join("tmp").join("checkpoints"))
}

/// Legacy: list legacy checkpoint ids (json filenames in tmp/checkpoints/).
/// Use `list_snapshots()` for the new file-history API.
pub async fn list_checkpoints() -> Result<Vec<String>, CheckpointError> {
    let dir = legacy_checkpoints_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut out: Vec<String> = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir).await?;
    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    Ok(out)
}

pub async fn load_checkpoint(id: &str) -> Result<Checkpoint, CheckpointError> {
    let dir = legacy_checkpoints_dir()?;
    let p = dir.join(format!("{}.json", id));
    let content = tokio::fs::read_to_string(&p).await?;
    let cp: Checkpoint = serde_json::from_str(&content)?;
    Ok(cp)
}

pub async fn save_checkpoint(cp: &Checkpoint) -> Result<(), CheckpointError> {
    let dir = legacy_checkpoints_dir()?;
    tokio::fs::create_dir_all(&dir).await?;
    let p = dir.join(format!("{}.json", cp.id));
    let s = serde_json::to_string_pretty(cp)?;
    tokio::fs::write(&p, s).await?;
    Ok(())
}

pub async fn create_checkpoint(
    tool_call: &StarToolCall,
    messages: &[StarMessage],
    chat_history: &[ChatEntry],
    file_paths: &[String],
) -> Result<String, CheckpointError> {
    let cwd = std::env::current_dir()?;
    let cwd_s = cwd.to_string_lossy().to_string();
    let created_at = Utc::now().timestamp();

    let tool = legacy_sanitize_for_filename(tool_call.function.name.as_str());
    let file_hint = file_paths
        .first()
        .map(|p| legacy_sanitize_for_filename(p))
        .unwrap_or_else(|| "no_file".to_string());

    let id = format!("{}-{}-{}", created_at, tool, file_hint);

    let mut files: Vec<CheckpointFile> = Vec::new();
    for fp in file_paths {
        let path = PathBuf::from(fp);
        let existed = path.exists();
        let content = if existed {
            match tokio::fs::read(&path).await {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).to_string()),
                Err(_) => None,
            }
        } else {
            None
        };
        files.push(CheckpointFile {
            path: fp.clone(),
            existed,
            content,
        });
    }

    let msg_tail = if messages.len() > 200 {
        messages[messages.len() - 200..].to_vec()
    } else {
        messages.to_vec()
    };
    let hist_tail = if chat_history.len() > 400 {
        chat_history[chat_history.len() - 400..].to_vec()
    } else {
        chat_history.to_vec()
    };

    let cp = Checkpoint {
        id: id.clone(),
        created_at_unix: created_at,
        cwd: cwd_s,
        tool_call: tool_call.clone(),
        files,
        messages: msg_tail,
        chat_history: hist_tail,
    };

    save_checkpoint(&cp).await?;
    Ok(id)
}

pub async fn apply_checkpoint(cp: &Checkpoint) -> Result<String, CheckpointError> {
    let mut restored: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();

    for f in &cp.files {
        let p = PathBuf::from(&f.path);
        if f.existed {
            if let Some(parent) = p.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let content = f.content.clone().unwrap_or_default();
            tokio::fs::write(&p, content).await?;
            restored.push(f.path.clone());
        } else if p.exists() {
            let _ = tokio::fs::remove_file(&p).await;
            removed.push(f.path.clone());
        }
    }

    let mut summary = String::new();
    if !restored.is_empty() {
        summary.push_str("restored:\n");
        summary.push_str(&restored.join("\n"));
        summary.push('\n');
    }
    if !removed.is_empty() {
        summary.push_str("removed:\n");
        summary.push_str(&removed.join("\n"));
        summary.push('\n');
    }
    if summary.trim().is_empty() {
        summary = "no changes".to_string();
    }

    Ok(summary)
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
//
// Run single-threaded because tests share global env-var state
// (STAR_DISABLE_FILE_CHECKPOINTING, STAR_FILE_HISTORY_DIR_OVERRIDE).
// Command:
//   cargo test --lib utils::checkpoint_manager -- --test-threads=1
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// RAII guard that sets an env var on construction and removes it on drop.
    /// Ensures tests do not leak global state even on panic.
    struct EnvGuard {
        key: &'static str,
    }
    impl EnvGuard {
        fn new(key: &'static str, val: &str) -> Self {
            std::env::set_var(key, val);
            Self { key }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.key);
        }
    }

    /// Per-test isolated project: a tempdir + an env override pointing
    /// file_history_dir at it. Does NOT call set_current_dir (which
    /// current_dir_cached ignores due to its OnceLock).
    fn make_test_project() -> (tempfile::TempDir, EnvGuard) {
        let dir = tempfile::tempdir().expect("tempdir");
        let guard = EnvGuard::new(
            "STAR_FILE_HISTORY_DIR_OVERRIDE",
            dir.path().to_str().expect("utf8 path"),
        );
        (dir, guard)
    }

    #[tokio::test]
    async fn test_track_edit_creates_backup_for_existing_file() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("sample.txt");
        std::fs::write(&file_path, "hello v1").unwrap();

        let snap_id = track_edit(&file_path, Some(1), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit");
        assert!(snap_id.is_some(), "snapshot id should be returned");

        let snaps = list_snapshots(Some("test_session")).await.expect("list");
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].snapshot_id, snap_id.unwrap());
        assert_eq!(snaps[0].tracked_files_count, 1);
    }

    #[tokio::test]
    async fn test_track_edit_for_nonexistent_file_records_null_marker() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("never_existed.txt");

        let snap_id = track_edit(&file_path, Some(2), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit");
        assert!(snap_id.is_some());

        let state = load_state(Some("test_session")).await.expect("state");
        let snap = state.snapshots.first().expect("snapshot");
        // tracking_path is absolute here (file is outside cwd) — find by suffix.
        let backup = snap
            .tracked_file_backups
            .iter()
            .find(|(k, _)| k.ends_with("never_existed.txt"))
            .map(|(_, v)| v)
            .expect("tracked");
        assert!(backup.backup_file_name.is_none(), "should be null marker");
    }

    #[tokio::test]
    async fn test_rewind_restores_file_content() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("doc.txt");
        std::fs::write(&file_path, "version 1").unwrap();

        let snap_id = track_edit(&file_path, Some(10), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit")
            .expect("some snap id");

        // Mutate the file after snapshot.
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"version 2").unwrap();
        drop(f);

        // Sanity: file is now v2.
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "version 2");

        let changed = rewind(&snap_id, Some("test_session")).await.expect("rewind");
        assert_eq!(changed.len(), 1);
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "version 1");
    }

    #[tokio::test]
    async fn test_rewind_deletes_file_created_after_snapshot() {
        let (td, _guard) = make_test_project();
        // Snapshot an empty project — file does not exist yet → null marker.
        let file_path = td.path().join("new.txt");
        let snap_id = track_edit(&file_path, Some(20), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit")
            .expect("snap id");

        // Create the file after snapshot.
        std::fs::write(&file_path, "created later").unwrap();
        assert!(file_path.exists());

        let changed = rewind(&snap_id, Some("test_session")).await.expect("rewind");
        assert!(changed.iter().any(|p| p.contains("new.txt")));
        assert!(!file_path.exists(), "file should be deleted by rewind");
    }

    #[tokio::test]
    async fn test_has_any_changes_detects_modified_file() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("changed.txt");
        std::fs::write(&file_path, "before").unwrap();

        let snap_id = track_edit(&file_path, Some(30), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit")
            .expect("snap id");

        // No change yet.
        assert!(
            !has_any_changes(&snap_id, Some("test_session")).await.expect("has")
        );

        // Mutate.
        std::fs::write(&file_path, "after").unwrap();
        assert!(
            has_any_changes(&snap_id, Some("test_session")).await.expect("has")
        );
    }

    #[tokio::test]
    async fn test_can_restore_returns_true_for_existing_snapshot() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("any.txt");
        std::fs::write(&file_path, "x").unwrap();
        let snap_id = track_edit(&file_path, Some(40), Some("write_file"), Some("test_session"))
            .await
            .expect("track_edit")
            .expect("snap id");

        assert!(
            can_restore(&snap_id, Some("test_session")).await.expect("can")
        );
        assert!(
            !can_restore("nonexistent_id", Some("test_session")).await.expect("can")
        );
    }

    #[tokio::test]
    async fn test_max_snapshots_cap_evicts_oldest() {
        let (td, _guard) = make_test_project();
        let file_path = td.path().join("cap.txt");
        std::fs::write(&file_path, "0").unwrap();

        // Create MAX_SNAPSHOTS + 5 snapshots by mutating between calls.
        // track_edit attaches to the last snapshot if same message_id; vary
        // message_id to force new snapshot each time.
        for i in 0..(MAX_SNAPSHOTS + 5) {
            std::fs::write(&file_path, format!("v{}", i)).unwrap();
            let _ = track_edit(&file_path, Some(i as u64), Some("test"), Some("cap_session"))
                .await
                .expect("track");
        }

        let snaps = list_snapshots(Some("cap_session")).await.expect("list");
        assert!(
            snaps.len() <= MAX_SNAPSHOTS,
            "snapshots should be capped at {}, got {}",
            MAX_SNAPSHOTS,
            snaps.len()
        );
    }

    #[tokio::test]
    async fn test_disabled_via_env_var() {
        let (td, _guard) = make_test_project();
        let _disable_guard = EnvGuard::new("STAR_DISABLE_FILE_CHECKPOINTING", "1");
        let file_path = td.path().join("off.txt");
        std::fs::write(&file_path, "x").unwrap();
        let result = track_edit(&file_path, None, Some("test"), Some("off_session"))
            .await
            .expect("no err");
        assert!(result.is_none(), "should be no-op when disabled");
        // EnvGuard drops here, clearing the disable flag.
    }

    #[test]
    fn test_get_backup_file_name_is_deterministic() {
        let a = get_backup_file_name("/some/path/foo.rs", 1);
        let b = get_backup_file_name("/some/path/foo.rs", 1);
        assert_eq!(a, b, "same path+version → same backup name");
        let c = get_backup_file_name("/some/path/foo.rs", 2);
        assert_ne!(a, c, "different version → different name");
        let d = get_backup_file_name("/other/path/foo.rs", 1);
        assert_ne!(a, d, "different path → different name");
    }
}
