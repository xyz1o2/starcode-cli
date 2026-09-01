use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopTask {
    pub name: String,
    pub prompt: String,
    pub interval_minutes: u64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run_at: Option<i64>,
    pub next_run_at: i64,
    /// Track task completions to prevent re-execution
    #[serde(default)]
    pub completion_count: u64,
    /// Last completion timestamp
    #[serde(default)]
    pub last_completed_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopStore {
    pub tasks: Vec<LoopTask>,
}

pub fn loops_file_path(project_root: &Path) -> PathBuf {
    project_root.join(".star").join("loops.json")
}

async fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
        }
    }
    Ok(())
}

pub async fn load_store(project_root: &Path) -> Result<LoopStore, String> {
    let path = loops_file_path(project_root);
    if !path.exists() {
        return Ok(LoopStore::default());
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    if content.trim().is_empty() {
        return Ok(LoopStore::default());
    }

    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

pub async fn save_store(project_root: &Path, store: &LoopStore) -> Result<(), String> {
    let path = loops_file_path(project_root);
    ensure_parent_dir(&path).await?;

    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize loop store: {}", e))?;

    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

pub async fn list_tasks(project_root: &Path) -> Result<Vec<LoopTask>, String> {
    let store = load_store(project_root).await?;
    Ok(store.tasks)
}

pub async fn add_task(
    project_root: &Path,
    name: String,
    interval_minutes: u64,
    prompt: String,
) -> Result<LoopTask, String> {
    if interval_minutes == 0 {
        return Err("interval_minutes must be greater than 0".to_string());
    }
    if name.trim().is_empty() {
        return Err("task name cannot be empty".to_string());
    }
    if prompt.trim().is_empty() {
        return Err("prompt cannot be empty".to_string());
    }

    let mut store = load_store(project_root).await?;
    if store.tasks.iter().any(|t| t.name == name) {
        return Err(format!("loop task '{}' already exists", name));
    }

    let now = Utc::now().timestamp();
    let interval_secs = (interval_minutes as i64).saturating_mul(60);

    let task = LoopTask {
        name,
        prompt,
        interval_minutes,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_run_at: None,
        next_run_at: now.saturating_add(interval_secs),
        completion_count: 0,
        last_completed_at: None,
    };

    store.tasks.push(task.clone());
    save_store(project_root, &store).await?;

    Ok(task)
}

pub async fn remove_task(project_root: &Path, name: &str) -> Result<bool, String> {
    let mut store = load_store(project_root).await?;
    let before = store.tasks.len();
    store.tasks.retain(|t| t.name != name);

    if store.tasks.len() == before {
        return Ok(false);
    }

    save_store(project_root, &store).await?;
    Ok(true)
}

pub async fn tick_and_collect_due_tasks(
    project_root: &Path,
    now_ts: i64,
) -> Result<Vec<LoopTask>, String> {
    let mut store = load_store(project_root).await?;
    let mut due = Vec::new();
    let mut dirty = false;

    for task in &mut store.tasks {
        if !task.enabled || task.interval_minutes == 0 {
            continue;
        }

        let interval_secs = (task.interval_minutes as i64).saturating_mul(60);

        // If next_run_at is far in the past (e.g., session resume after hours),
        // skip missed executions and just schedule the next one.
        // Only trigger if the task was due within the last 2x interval.
        let elapsed = now_ts.saturating_sub(task.next_run_at);
        if elapsed > interval_secs * 2 {
            // Task was due long ago — skip and reschedule
            let mut next_run = task.next_run_at;
            while next_run <= now_ts {
                next_run = next_run.saturating_add(interval_secs);
            }
            task.next_run_at = next_run;
            task.updated_at = now_ts;
            dirty = true;
            continue;
        }

        if task.next_run_at <= now_ts {
            let mut next_run = task.next_run_at;
            while next_run <= now_ts {
                next_run = next_run.saturating_add(interval_secs);
            }

            task.last_run_at = Some(now_ts);
            task.next_run_at = next_run;
            task.updated_at = now_ts;
            due.push(task.clone());
            dirty = true;
        }
    }

    if dirty {
        save_store(project_root, &store).await?;
    }

    Ok(due)
}

/// Mark a task as completed. This prevents re-execution until the next interval.
pub async fn mark_task_completed(project_root: &Path, task_name: &str) -> Result<(), String> {
    let mut store = load_store(project_root).await?;
    if let Some(task) = store.tasks.iter_mut().find(|t| t.name == task_name) {
        task.completion_count += 1;
        task.last_completed_at = Some(chrono::Utc::now().timestamp());
        task.updated_at = chrono::Utc::now().timestamp();
        save_store(project_root, &store).await?;
    }
    Ok(())
}

/// Check if a task was recently completed (within the last interval).
/// Returns true if the task should be skipped.
pub fn is_task_recently_completed(task: &LoopTask, now_ts: i64) -> bool {
    if let Some(last_completed) = task.last_completed_at {
        let interval_secs = (task.interval_minutes as i64).saturating_mul(60);
        let elapsed = now_ts.saturating_sub(last_completed);
        // Skip if completed within the last 50% of the interval
        elapsed < interval_secs / 2
    } else {
        false
    }
}
