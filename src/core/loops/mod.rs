/// 持久化循环调度：普通任务的读改写在进程内串行，磁盘替换不暴露半份 JSON。
/// 调度计算使用宽整数直接跳到未来，不逐次补跑；错误返回调用方，非法旧间隔停用。
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// UI 命令与后台 tick 共享同一把锁，防止读改写覆盖；不提供跨进程互斥。
static LOOP_STORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn interval_seconds(minutes: u64) -> Result<i64, String> {
    i64::try_from(minutes)
        .ok()
        .and_then(|value| value.checked_mul(60))
        .filter(|value| *value > 0)
        .ok_or_else(|| crate::core::i18n::t1("cmd.loop.invalid_interval"))
}

/// 保持原有节拍，常数时间跳过遗漏周期；无法表示未来时间时返回 None。
fn next_scheduled_time(next: i64, now: i64, interval: i64) -> Option<i64> {
    if interval <= 0 {
        return None;
    }
    if next > now {
        return Some(next);
    }
    let steps = (i128::from(now) - i128::from(next)) / i128::from(interval) + 1;
    i64::try_from(i128::from(next) + steps * i128::from(interval)).ok()
}

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
    /// 明确完成信号的计数；派发本身不代表完成。
    #[serde(default)]
    pub completion_count: u64,
    /// 最近一次明确完成的时间。
    #[serde(default)]
    pub last_completed_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopStore {
    pub tasks: Vec<LoopTask>,
}

pub fn loops_file_path(project_root: &Path) -> PathBuf {
    crate::core::goal_tracking::persistence::project_state_dir(project_root).join("loops.json")
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

    let content = serde_json::to_vec_pretty(store)
        .map_err(|e| format!("Failed to serialize loop store: {}", e))?;

    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let write = || -> std::io::Result<()> {
            let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            temporary.write_all(&content)?;
            temporary.as_file().sync_all()?;
            temporary.persist(&path).map_err(|error| error.error)?;
            Ok(())
        };
        write().map_err(|error| format!("Failed to write {}: {}", path.display(), error))
    })
    .await
    .map_err(|error| error.to_string())?
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
    let interval_secs = interval_seconds(interval_minutes)?;
    let name = name.trim().to_string();
    let prompt = prompt.trim().to_string();
    if name.is_empty() {
        return Err(crate::core::i18n::t1("cmd.loop.empty_name"));
    }
    if prompt.is_empty() {
        return Err(crate::core::i18n::t1("cmd.loop.empty_prompt"));
    }

    let _guard = LOOP_STORE_LOCK.lock().await;
    let mut store = load_store(project_root).await?;
    if store.tasks.iter().any(|t| t.name == name) {
        return Err(crate::core::i18n::t1("cmd.loop.duplicate").replace("{name}", &name));
    }

    let now = Utc::now().timestamp();
    let next_run_at = now
        .checked_add(interval_secs)
        .ok_or_else(|| crate::core::i18n::t1("cmd.loop.invalid_interval"))?;

    let task = LoopTask {
        name,
        prompt,
        interval_minutes,
        enabled: true,
        created_at: now,
        updated_at: now,
        last_run_at: None,
        next_run_at,
        completion_count: 0,
        last_completed_at: None,
    };

    store.tasks.push(task.clone());
    save_store(project_root, &store).await?;

    Ok(task)
}

/// 暂停不丢失任务；从暂停恢复时等待一个完整间隔，不补跑暂停期间的任务。
pub async fn set_task_enabled(
    project_root: &Path,
    name: &str,
    enabled: bool,
) -> Result<Option<LoopTask>, String> {
    let _guard = LOOP_STORE_LOCK.lock().await;
    let mut store = load_store(project_root).await?;
    let Some(task) = store.tasks.iter_mut().find(|task| task.name == name.trim()) else {
        return Ok(None);
    };
    if task.enabled != enabled {
        let now = Utc::now().timestamp();
        if enabled {
            task.next_run_at = now
                .checked_add(interval_seconds(task.interval_minutes)?)
                .ok_or_else(|| crate::core::i18n::t1("cmd.loop.invalid_interval"))?;
        }
        task.enabled = enabled;
        task.updated_at = now;
        let task = task.clone();
        save_store(project_root, &store).await?;
        return Ok(Some(task));
    }
    Ok(Some(task.clone()))
}

pub async fn remove_task(project_root: &Path, name: &str) -> Result<bool, String> {
    let _guard = LOOP_STORE_LOCK.lock().await;
    let name = name.trim();
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
    let _guard = LOOP_STORE_LOCK.lock().await;
    let mut store = load_store(project_root).await?;
    let mut due = Vec::new();
    let mut dirty = false;

    for task in &mut store.tasks {
        if !task.enabled {
            continue;
        }
        let interval = interval_seconds(task.interval_minutes).ok();
        let next =
            interval.and_then(|seconds| next_scheduled_time(task.next_run_at, now_ts, seconds));
        let (Some(interval), Some(next)) = (interval, next) else {
            // 旧文件中的非法间隔或时间上界不能卡死整个调度器。
            task.enabled = false;
            task.updated_at = now_ts;
            dirty = true;
            continue;
        };
        if task.next_run_at > now_ts {
            continue;
        }

        // 重启后不补跑积压；最多允许迟到两个周期，并保持原有节拍。
        let elapsed = i128::from(now_ts) - i128::from(task.next_run_at);
        let should_run = elapsed <= i128::from(interval) * 2;
        task.next_run_at = next;
        task.updated_at = now_ts;
        if should_run {
            task.last_run_at = Some(now_ts);
            due.push(task.clone());
        }
        dirty = true;
    }

    if dirty {
        save_store(project_root, &store).await?;
    }

    Ok(due)
}

/// 记录完成信息；只有明确收到该任务完成信号的调用方才应使用。
pub async fn mark_task_completed(project_root: &Path, task_name: &str) -> Result<(), String> {
    let _guard = LOOP_STORE_LOCK.lock().await;
    let mut store = load_store(project_root).await?;
    if let Some(task) = store.tasks.iter_mut().find(|t| t.name == task_name) {
        task.completion_count = task.completion_count.saturating_add(1);
        task.last_completed_at = Some(chrono::Utc::now().timestamp());
        task.updated_at = chrono::Utc::now().timestamp();
        save_store(project_root, &store).await?;
    }
    Ok(())
}

/// 兼容旧调用方的半周期完成去重，不代表调度派发已经完成。
pub fn is_task_recently_completed(task: &LoopTask, now_ts: i64) -> bool {
    match (
        task.last_completed_at,
        interval_seconds(task.interval_minutes),
    ) {
        (Some(last), Ok(interval)) => {
            let elapsed = i128::from(now_ts) - i128::from(last);
            elapsed >= 0 && elapsed < i128::from(interval) / 2
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, minutes: u64, next: i64) -> LoopTask {
        LoopTask {
            name: name.into(),
            prompt: "check the build".into(),
            interval_minutes: minutes,
            enabled: true,
            created_at: 0,
            updated_at: 0,
            last_run_at: None,
            next_run_at: next,
            completion_count: 0,
            last_completed_at: None,
        }
    }

    #[tokio::test]
    async fn invalid_inputs_do_not_create_storage() {
        let dir = tempfile::tempdir().unwrap();
        for minutes in [0, u64::MAX, i64::MAX as u64 / 60 + 1, i64::MAX as u64 / 60] {
            assert!(add_task(dir.path(), "bad".into(), minutes, "prompt".into())
                .await
                .is_err());
        }
        assert!(add_task(dir.path(), "  ".into(), 1, "prompt".into())
            .await
            .is_err());
        assert!(add_task(dir.path(), "valid".into(), 1, "  ".into())
            .await
            .is_err());
        assert!(!loops_file_path(dir.path()).parent().unwrap().exists());
    }

    #[tokio::test]
    async fn names_and_prompts_are_trimmed_before_duplicate_check() {
        let dir = tempfile::tempdir().unwrap();
        let added = add_task(dir.path(), " check ".into(), 1, " prompt ".into())
            .await
            .unwrap();
        assert_eq!(added.name, "check");
        assert_eq!(added.prompt, "prompt");
        assert_eq!(added.next_run_at - added.created_at, 60);
        assert!(add_task(dir.path(), "check ".into(), 1, "another".into())
            .await
            .is_err());
        assert_eq!(list_tasks(dir.path()).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn due_tasks_run_once_and_keep_the_original_cadence() {
        let dir = tempfile::tempdir().unwrap();
        save_store(
            dir.path(),
            &LoopStore {
                tasks: vec![task("due", 1, 100)],
            },
        )
        .await
        .unwrap();
        let due = tick_and_collect_due_tasks(dir.path(), 170).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].next_run_at, 220);
        assert_eq!(due[0].last_run_at, Some(170));
        assert_eq!(due[0].completion_count, 0);
        assert!(tick_and_collect_due_tasks(dir.path(), 170)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(list_tasks(dir.path()).await.unwrap()[0].next_run_at, 220);
    }

    #[tokio::test]
    async fn stale_tasks_skip_backlog_in_constant_time() {
        let dir = tempfile::tempdir().unwrap();
        save_store(
            dir.path(),
            &LoopStore {
                tasks: vec![task("old", 1, i64::MIN)],
            },
        )
        .await
        .unwrap();
        assert!(tick_and_collect_due_tasks(dir.path(), 1_000)
            .await
            .unwrap()
            .is_empty());
        let old = list_tasks(dir.path()).await.unwrap().remove(0);
        assert!(old.enabled);
        assert!(old.next_run_at > 1_000 && old.next_run_at <= 1_060);
        assert_eq!((i128::from(old.next_run_at) - i128::from(i64::MIN)) % 60, 0);
        assert_eq!(old.last_run_at, None);
    }

    #[tokio::test]
    async fn invalid_legacy_intervals_and_time_overflow_are_disabled() {
        let dir = tempfile::tempdir().unwrap();
        save_store(
            dir.path(),
            &LoopStore {
                tasks: vec![
                    task("zero", 0, 0),
                    task("overflow", u64::MAX, 0),
                    task("end", 1, i64::MAX),
                ],
            },
        )
        .await
        .unwrap();
        assert!(tick_and_collect_due_tasks(dir.path(), i64::MAX)
            .await
            .unwrap()
            .is_empty());
        assert!(list_tasks(dir.path())
            .await
            .unwrap()
            .iter()
            .all(|task| !task.enabled));
        assert_eq!(
            next_scheduled_time(i64::MIN, i64::MIN, 60),
            Some(i64::MIN + 60)
        );
        assert_eq!(next_scheduled_time(0, i64::MAX, 60), None);
    }

    #[tokio::test]
    async fn pause_resume_and_repeated_resume_have_stable_schedules() {
        let dir = tempfile::tempdir().unwrap();
        save_store(
            dir.path(),
            &LoopStore {
                tasks: vec![task("work", 5, 100)],
            },
        )
        .await
        .unwrap();
        let paused = set_task_enabled(dir.path(), " work ", false)
            .await
            .unwrap()
            .unwrap();
        assert!(!paused.enabled);
        assert_eq!(paused.next_run_at, 100);
        assert!(tick_and_collect_due_tasks(dir.path(), 101)
            .await
            .unwrap()
            .is_empty());
        let before = Utc::now().timestamp();
        let resumed = set_task_enabled(dir.path(), "work", true)
            .await
            .unwrap()
            .unwrap();
        let after = Utc::now().timestamp();
        assert!(resumed.enabled);
        assert!((before + 300..=after + 300).contains(&resumed.next_run_at));
        let repeated = set_task_enabled(dir.path(), "work", true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repeated.next_run_at, resumed.next_run_at);
        assert_eq!(repeated.updated_at, resumed.updated_at);
    }

    #[tokio::test]
    async fn readonly_and_missing_mutations_do_not_create_storage() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_tasks(dir.path()).await.unwrap().is_empty());
        assert!(tick_and_collect_due_tasks(dir.path(), 100)
            .await
            .unwrap()
            .is_empty());
        assert!(!remove_task(dir.path(), "missing").await.unwrap());
        assert!(set_task_enabled(dir.path(), "missing", true)
            .await
            .unwrap()
            .is_none());
        assert!(!loops_file_path(dir.path()).parent().unwrap().exists());
    }

    #[tokio::test]
    async fn concurrent_production_mutations_do_not_lose_updates() {
        let dir = tempfile::tempdir().unwrap();
        save_store(
            dir.path(),
            &LoopStore {
                tasks: vec![task("due", 1, 100), task("remove", 1, 200)],
            },
        )
        .await
        .unwrap();
        let (added, due, removed) = tokio::join!(
            add_task(dir.path(), "new".into(), 1, "prompt".into()),
            tick_and_collect_due_tasks(dir.path(), 100),
            remove_task(dir.path(), "remove"),
        );
        added.unwrap();
        assert_eq!(due.unwrap().len(), 1);
        assert!(removed.unwrap());
        let tasks = list_tasks(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|task| task.name == "new"));
        assert_eq!(
            tasks
                .iter()
                .find(|task| task.name == "due")
                .unwrap()
                .next_run_at,
            160
        );
    }

    #[tokio::test]
    async fn failed_storage_operations_preserve_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = loops_file_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "invalid JSON").unwrap();
        assert!(add_task(dir.path(), "new".into(), 1, "prompt".into())
            .await
            .is_err());
        assert!(remove_task(dir.path(), "new").await.is_err());
        assert!(set_task_enabled(dir.path(), "new", false).await.is_err());
        assert!(tick_and_collect_due_tasks(dir.path(), 100).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid JSON");

        let blocked = tempfile::tempdir().unwrap();
        let target = loops_file_path(blocked.path());
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep"), "unchanged").unwrap();
        assert!(save_store(blocked.path(), &LoopStore::default())
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep")).unwrap(),
            "unchanged"
        );
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[tokio::test]
    async fn nested_project_commands_share_one_loop_store() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("src/nested");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(loops_file_path(&nested), loops_file_path(dir.path()));
        add_task(&nested, "work".into(), 1, "prompt".into())
            .await
            .unwrap();
        assert_eq!(list_tasks(dir.path()).await.unwrap().len(), 1);
        assert!(remove_task(dir.path(), "work").await.unwrap());
        assert!(list_tasks(&nested).await.unwrap().is_empty());
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
    crate::core::goal_tracking::persistence::project_state_dir(project_root).join("goal_loops.json")
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
    let path = crate::core::goal_tracking::persistence::project_goals_path(project_root);
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
