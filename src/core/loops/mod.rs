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

// ═══════════════════════════════════════════════════════════════════════════
// Goal loop（`/goal-loop`）
//
// 与 [`LoopTask`] 的区别：它绑定一个 goal（`.star/goals.json`），每轮把
// 目标上下文交给 agent 推进一步，并在以下任一条件满足时**自动收口**：
//   - goal 已 Completed / Cancelled（用户 `/goal done`、`/goal progress 100`）
//   - goal 被删除（goals.json 里查不到）
//   - 迭代数达到 `max_iterations`
// 持久化在独立的 `.star/goal_loops.json`，与 loops.json 互不干扰。
// ═══════════════════════════════════════════════════════════════════════════

/// `/goal-loop` 任务：定时把某个 goal 交给 agent 循环推进，直到目标完成、
/// 迭代数用尽或手动 `stop`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopTask {
    pub name: String,
    /// 绑定的 goal 完整 id（`.star/goals.json` 里的 UUID）
    pub goal_id: String,
    /// goal 标题快照 —— goals.json 删了之后列表里还能看得懂
    pub goal_title: String,
    pub interval_minutes: u64,
    pub max_iterations: u64,
    /// 已触发的轮数（每次到期触发 +1，重启不重置）
    #[serde(default)]
    pub iterations_run: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    pub next_run_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalLoopStore {
    pub tasks: Vec<GoalLoopTask>,
}

pub fn goal_loops_file_path(project_root: &Path) -> PathBuf {
    project_root.join(".star").join("goal_loops.json")
}

pub(crate) async fn load_goal_loop_store(project_root: &Path) -> Result<GoalLoopStore, String> {
    let path = goal_loops_file_path(project_root);
    if !path.exists() {
        return Ok(GoalLoopStore::default());
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    if content.trim().is_empty() {
        return Ok(GoalLoopStore::default());
    }

    serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

pub(crate) async fn save_goal_loop_store(
    project_root: &Path,
    store: &GoalLoopStore,
) -> Result<(), String> {
    let path = goal_loops_file_path(project_root);
    ensure_parent_dir(&path).await?;

    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize goal loop store: {}", e))?;

    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

pub async fn list_goal_loops(project_root: &Path) -> Result<Vec<GoalLoopTask>, String> {
    let store = load_goal_loop_store(project_root).await?;
    Ok(store.tasks)
}

/// 新增一个 goal loop。同名的任务（包括已收口的）不允许重复创建，
/// 否则 `stop <name>` 就说不清删的是哪一个。
pub async fn add_goal_loop(
    project_root: &Path,
    name: String,
    goal_id: String,
    goal_title: String,
    interval_minutes: u64,
    max_iterations: u64,
) -> Result<GoalLoopTask, String> {
    if name.trim().is_empty() {
        return Err("goal loop name cannot be empty".to_string());
    }
    if goal_id.trim().is_empty() {
        return Err("goal_id cannot be empty".to_string());
    }
    if interval_minutes == 0 {
        return Err("interval_minutes must be greater than 0".to_string());
    }
    if max_iterations == 0 {
        return Err("max_iterations must be greater than 0".to_string());
    }

    let mut store = load_goal_loop_store(project_root).await?;
    if store.tasks.iter().any(|t| t.name == name) {
        return Err(format!("goal loop '{}' already exists", name));
    }

    let now = Utc::now().timestamp();
    let interval_secs = (interval_minutes as i64).saturating_mul(60);

    let task = GoalLoopTask {
        name,
        goal_id,
        goal_title,
        interval_minutes,
        max_iterations,
        iterations_run: 0,
        enabled: true,
        finished: false,
        finish_reason: None,
        last_run_at: None,
        next_run_at: now.saturating_add(interval_secs),
        created_at: now,
        updated_at: now,
    };

    store.tasks.push(task.clone());
    save_goal_loop_store(project_root, &store).await?;

    Ok(task)
}

pub async fn remove_goal_loop(project_root: &Path, name: &str) -> Result<bool, String> {
    let mut store = load_goal_loop_store(project_root).await?;
    let before = store.tasks.len();
    store.tasks.retain(|t| t.name != name);

    if store.tasks.len() == before {
        return Ok(false);
    }

    save_goal_loop_store(project_root, &store).await?;
    Ok(true)
}

/// 读取 `.star/goals.json` 里某个 goal 的状态快照；文件或该 goal 不存在时返回 `None`。
async fn load_goal_snapshot(
    project_root: &Path,
    goal_id: &str,
) -> Result<Option<crate::core::goal_tracking::Goal>, String> {
    let path = project_root.join(".star").join("goals.json");
    if !path.exists() {
        return Ok(None);
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let goals: Vec<crate::core::goal_tracking::Goal> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    Ok(goals.into_iter().find(|g| g.id == goal_id))
}

/// 生成某一轮要发给 agent 的目标上下文；goal 已不存在时返回 `None`。
pub async fn goal_prompt_for(project_root: &Path, goal_id: &str) -> Result<Option<String>, String> {
    Ok(load_goal_snapshot(project_root, goal_id)
        .await?
        .map(|goal| crate::core::goal_tracking::GoalPrompts::new().generate_goal_prompt(&goal)))
}

/// 从 goals.json 读出 goal 的当前状态（找不到 → `None`）。
async fn goal_status_snapshot(
    project_root: &Path,
    goal_id: &str,
) -> Result<Option<crate::core::goal_tracking::GoalStatus>, String> {
    Ok(load_goal_snapshot(project_root, goal_id)
        .await?
        .map(|g| g.status))
}

/// 收口一个 task：仅在内存里标记 finished 与原因，落盘交给调用方统一处理。
fn finish_task(task: &mut GoalLoopTask, now_ts: i64, reason: &str) {
    task.finished = true;
    task.finish_reason = Some(reason.to_string());
    task.updated_at = now_ts;
}

/// 到期收集。调度语义与 [`tick_and_collect_due_tasks`] 一致（过期太久跳过后重排，
/// 不补跑错过的轮次），额外的 goal-loop 收口逻辑：
///   - goal Completed / Cancelled → finished（不触发本轮）
///   - goal 在 goals.json 里查不到 → finished(`goal_missing`)
///   - 本轮触发后 `iterations_run` 已达 `max_iterations` → finished(`max_iterations`)
///
/// 返回本轮到期待触发的任务（含最新的 `iterations_run`）。
pub async fn tick_and_collect_due_goal_loops(
    project_root: &Path,
    now_ts: i64,
) -> Result<Vec<GoalLoopTask>, String> {
    use crate::core::goal_tracking::GoalStatus;

    let mut store = load_goal_loop_store(project_root).await?;
    let mut due = Vec::new();
    let mut dirty = false;

    for task in &mut store.tasks {
        if !task.enabled || task.finished {
            continue;
        }

        match goal_status_snapshot(project_root, &task.goal_id).await? {
            // goal 已经不在了（被 `/goal rm` 删掉）——循环没法再推进，收口。
            None => {
                finish_task(task, now_ts, "goal_missing");
                dirty = true;
                continue;
            }
            Some(status) if status == GoalStatus::Completed => {
                finish_task(task, now_ts, "goal_completed");
                dirty = true;
                continue;
            }
            Some(status) if status == GoalStatus::Cancelled => {
                finish_task(task, now_ts, "goal_cancelled");
                dirty = true;
                continue;
            }
            Some(_) => {}
        }

        // 迭代数用尽：不再触发，直接收口。
        if task.iterations_run >= task.max_iterations {
            finish_task(task, now_ts, "max_iterations");
            dirty = true;
            continue;
        }

        // 状态行是快照，不复述过期字段。
        task.goal_title = load_goal_snapshot(project_root, &task.goal_id)
            .await?
            .map(|g| g.title)
            .unwrap_or_else(|| task.goal_title.clone());

        let interval_secs = (task.interval_minutes as i64).saturating_mul(60);

        // If next_run_at is far in the past (e.g., session resume after hours),
        // skip missed executions and just schedule the next one.
        let elapsed = now_ts.saturating_sub(task.next_run_at);
        if elapsed > interval_secs * 2 {
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
            task.iterations_run = task.iterations_run.saturating_add(1);
            task.updated_at = now_ts;
            due.push(task.clone());

            // 已经是最后一轮：照常触发本轮，之后自动收口，下个 tick 不会再跑。
            if task.iterations_run >= task.max_iterations {
                finish_task(task, now_ts, "max_iterations");
            }
            dirty = true;
        }
    }

    if dirty {
        save_goal_loop_store(project_root, &store).await?;
    }

    Ok(due)
}
