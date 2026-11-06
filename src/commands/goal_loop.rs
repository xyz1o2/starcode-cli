use crate::commands::execution::{CommandContext, CommandResult};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum GoalLoopCommand {
    /// Start a goal loop: repeatedly hand a goal to the agent until it is done
    ///
    /// `goal` 传已有 goal 的 id 前缀即绑定该目标；没匹配上就把整段当作
    /// 新 goal 的标题创建。目标完成后循环会自动收口（`/goal done <id>`）。
    #[command(arg_required_else_help = true)]
    Start {
        /// Goal id prefix (binds an existing goal) — no match = new goal title
        #[arg(required = true, num_args = 1..)]
        goal: Vec<String>,
        /// Trigger interval in minutes
        #[arg(long, default_value_t = 5)]
        every_minutes: u64,
        /// Max agent iterations before the loop stops itself
        #[arg(long, default_value_t = 10)]
        max_iterations: u64,
    },
    /// List goal loops
    List,
    /// Remove a goal loop by name (see /goal-loop list)
    #[command(arg_required_else_help = true)]
    Stop {
        /// Goal loop name
        name: String,
    },
}

pub async fn execute_goal_loop_command(
    ctx: CommandContext<'_>,
    cmd: GoalLoopCommand,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    execute_goal_loop_command_in(ctx, cmd, &cwd).await
}

/// `cwd` 显式传入：单测要在临时目录里建/删 goal 和 loop，不能写进开发者自己的
/// `.star/goals.json` / `.star/goal_loops.json`。
pub(crate) async fn execute_goal_loop_command_in(
    ctx: CommandContext<'_>,
    cmd: GoalLoopCommand,
    cwd: &std::path::Path,
) -> CommandResult {
    let out = match cmd {
        GoalLoopCommand::Start {
            goal,
            every_minutes,
            max_iterations,
        } => start_goal_loop(cwd, &goal.join(" "), every_minutes, max_iterations).await,
        GoalLoopCommand::List => render_goal_loops(cwd).await,
        GoalLoopCommand::Stop { name } => stop_goal_loop(cwd, &name).await,
    };

    match out {
        Ok(msg) => {
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
            Ok(())
        }
        Err(e) => Err(e),
    }
}

type LoopOutput = Result<String, String>;

/// 项目级 goal 存储。不复用 `parity::goal_manager()` —— 那个读进程 cwd，
/// 单测没法定向到临时目录。
fn goal_manager_in(
    cwd: &std::path::Path,
) -> Result<crate::core::goal_tracking::GoalManager, String> {
    let dir = cwd.join(".star");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("goals.json");
    let mut mgr =
        crate::core::goal_tracking::GoalManager::new(Some(path.to_string_lossy().as_ref()));
    mgr.load()?;
    Ok(mgr)
}

/// goal loop 名字取标题 slug；重名自动加 `-2`、`-3` 后缀。
async fn derive_loop_name(cwd: &std::path::Path, goal_title: &str) -> LoopOutput {
    let existing = crate::core::loops::list_goal_loops(cwd).await?;
    let base = crate::commands::skills::slugify(goal_title);
    let base = if base.is_empty() {
        "goal-loop".to_string()
    } else {
        base.chars().take(24).collect()
    };

    let mut name = base.clone();
    let mut n = 1usize;
    while existing.iter().any(|t| t.name == name) {
        n += 1;
        name = format!("{}-{}", base, n);
    }
    Ok(name)
}

async fn start_goal_loop(
    cwd: &std::path::Path,
    goal_arg: &str,
    every_minutes: u64,
    max_iterations: u64,
) -> LoopOutput {
    use crate::core::goal_tracking::{GoalPriority, GoalStatus};

    let title = goal_arg.trim().to_string();
    if title.is_empty() {
        return Ok(
            "Usage: `/goal-loop start <goal-id-prefix | goal title> [--every N] [--max N]`"
                .to_string(),
        );
    }

    let mut mgr = goal_manager_in(cwd)?;

    let (goal_id, goal_title) = {
        let words: Vec<&str> = title.split_whitespace().collect();
        let prefix = words[0];
        let hits: Vec<String> = mgr
            .get_all_goals()
            .iter()
            .filter(|g| g.id.starts_with(prefix))
            .map(|g| g.id.clone())
            .collect();
        match hits.len() {
            1 => {
                let g = mgr.get_goal(&hits[0]).unwrap();
                // 已收口的 goal 再挂一个循环，只会落地即停 —— 直接拒绝更清楚。
                if g.status == GoalStatus::Completed || g.status == GoalStatus::Cancelled {
                    return Err(format!(
                        "goal `{}` is already {} — loops only run on active goals.",
                        crate::commands::parity::short_id(&hits[0]),
                        if g.status == GoalStatus::Completed {
                            "completed"
                        } else {
                            "cancelled"
                        }
                    ));
                }
                (hits[0].clone(), g.title.clone())
            }
            0 => {
                let id = mgr.create_goal(&title, None, GoalPriority::Medium);
                (id, title.clone())
            }
            n => {
                return Err(format!(
                    "`{}` matches {} goals — use more characters or a full goal id.",
                    prefix, n
                ))
            }
        }
    };

    let name = derive_loop_name(cwd, &goal_title).await?;
    let task = crate::core::loops::add_goal_loop(
        cwd,
        name,
        goal_id.clone(),
        goal_title.clone(),
        every_minutes,
        max_iterations,
    )
    .await
    .map_err(|e| format!("Failed to create goal loop: {}", e))?;

    let next_time = crate::commands::loop_cmd::format_ts(task.next_run_at);
    let sid = crate::commands::parity::short_id(&goal_id);
    Ok(format!(
        "✅ 已创建 goal-loop\n\n\
         - 名称: `{}`\n\
         - 目标: **{}** (`{}`)\n\
         - 节奏: 每 {} 分钟 · 最多 {} 轮\n\
         - 下次触发: {}\n\n\
         收口方式（任一即停）：\n\
         - 目标完成 → `/goal done {}`，下一轮自动结束（`goal_completed`）\n\
         - 手动移除 → `/goal-loop stop {}`\n\
         - 跑满 {} 轮自动停（`max_iterations`）",
        task.name,
        goal_title,
        sid,
        task.interval_minutes,
        task.max_iterations,
        next_time,
        sid,
        task.name,
        task.max_iterations
    ))
}

async fn render_goal_loops(cwd: &std::path::Path) -> LoopOutput {
    let tasks = crate::core::loops::list_goal_loops(cwd)
        .await
        .map_err(|e| format!("Failed to list goal loops: {}", e))?;

    if tasks.is_empty() {
        return Ok(
            "当前没有 goal-loop。使用 `/goal-loop start <goal | title>`（或先用 `/goal <title>` 建目标）。".to_string(),
        );
    }

    let now = chrono::Utc::now().timestamp();
    let mut lines = vec!["# /goal-loop 任务列表\n".to_string()];
    for t in &tasks {
        let (state, extra) = if t.finished {
            (
                format!("finished ({})", t.finish_reason.as_deref().unwrap_or("?")),
                String::new(),
            )
        } else {
            let in_minutes = ((t.next_run_at - now).max(0) as f64 / 60.0).ceil() as i64;
            (
                "running".to_string(),
                format!(
                    "\n  - next: {} (约 {} 分钟后)",
                    crate::commands::loop_cmd::format_ts(t.next_run_at),
                    in_minutes
                ),
            )
        };
        lines.push(format!(
            "- `{}` [{}]\n  - goal: **{}** (`{}`)\n  - every: {} min · iteration {}/{}\n  - goal id: {}{}",
            t.name,
            state,
            t.goal_title,
            crate::commands::parity::short_id(&t.goal_id),
            t.interval_minutes,
            t.iterations_run,
            t.max_iterations,
            t.goal_id,
            extra
        ));
    }
    lines.push(String::new());
    lines.push("`/goal-loop stop <name>` 移除；`/goal done <id>` 完成目标并自动收口。".to_string());
    Ok(lines.join("\n"))
}

async fn stop_goal_loop(cwd: &std::path::Path, name: &str) -> LoopOutput {
    let removed = crate::core::loops::remove_goal_loop(cwd, name)
        .await
        .map_err(|e| format!("Failed to remove goal loop: {}", e))?;

    if removed {
        Ok(format!("✅ 已删除 goal-loop `{}`", name))
    } else {
        Ok(format!(
            "⚠️ 未找到 goal-loop `{}`（`/goal-loop list` 查看）",
            name
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::CommandContext;
    use crate::core::goal_tracking::{Goal, GoalPriority, GoalStatus};
    use crate::ui::state::ChatState;
    use chrono::Utc;

    async fn run_in(cwd: &std::path::Path, cmd: GoalLoopCommand) -> Result<String, String> {
        let mut state = ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        execute_goal_loop_command_in(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            cmd,
            cwd,
        )
        .await?;
        Ok(state
            .chat_history
            .last()
            .map(|entry| entry.content.clone())
            .unwrap_or_default())
    }

    fn goal(id: &str, title: &str, status: GoalStatus) -> Goal {
        Goal {
            id: id.to_string(),
            title: title.to_string(),
            description: None,
            status,
            priority: GoalPriority::Medium,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            sub_goals: Vec::new(),
            tags: Vec::new(),
            progress: 0,
            parent_id: None,
            deadline: None,
            milestones: Vec::new(),
        }
    }

    /// 把 goal 列表落盘为 `.star/goals.json`（GoalManager 的存储格式就是 `Vec<Goal>`）。
    fn seed_goals(dir: &std::path::Path, goals: &[Goal]) {
        let dir = dir.join(".star");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("goals.json"),
            serde_json::to_string(&goals).unwrap(),
        )
        .unwrap();
    }

    fn start(prefix: &str, every: u64, max: u64) -> GoalLoopCommand {
        GoalLoopCommand::Start {
            goal: vec![prefix.to_string()],
            every_minutes: every,
            max_iterations: max,
        }
    }

    #[tokio::test]
    async fn an_empty_list_explains_how_to_start() {
        let dir = tempfile::tempdir().unwrap();
        let out = run_in(dir.path(), GoalLoopCommand::List).await.unwrap();
        assert!(out.contains("当前没有 goal-loop"), "{out}");
        assert!(out.contains("/goal-loop start"), "{out}");
    }

    #[tokio::test]
    async fn starting_with_an_id_prefix_binds_that_goal() {
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::InProgress)],
        );

        let out = run_in(dir.path(), start("1111", 5, 10)).await.unwrap();
        assert!(out.contains("Ship the parser"), "{out}");
        assert!(out.contains("`11111111`"), "{out}");
        assert!(out.contains("每 5 分钟"), "{out}");
        assert!(out.contains("`/goal done 11111111`"), "{out}");

        let tasks = crate::core::loops::list_goal_loops(dir.path())
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].goal_id, id);
        assert_eq!(tasks[0].iterations_run, 0);
    }

    #[tokio::test]
    async fn starting_without_a_match_creates_a_new_goal() {
        let dir = tempfile::tempdir().unwrap();

        let out = run_in(dir.path(), start("Write", 3, 4)).await.unwrap();
        assert!(out.contains("已创建 goal-loop"), "{out}");

        // 新 goal 已经写进 goals.json，能被下一次 start 重复绑定。
        let goals_file = dir.path().join(".star").join("goals.json");
        assert!(goals_file.exists(), "new goal must be persisted");
        let tasks = crate::core::loops::list_goal_loops(dir.path())
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].goal_title, "Write");
    }

    #[tokio::test]
    async fn an_ambiguous_id_prefix_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let a = "aaaabbbb-0000-0000-0000-000000000000";
        let b = "aaaacccc-0000-0000-0000-000000000000";
        seed_goals(
            dir.path(),
            &[
                goal(a, "Goal A", GoalStatus::InProgress),
                goal(b, "Goal B", GoalStatus::InProgress),
            ],
        );

        let err = run_in(dir.path(), start("aaaa", 5, 10)).await.unwrap_err();
        assert!(err.contains("matches 2 goals"), "{err}");
    }

    #[tokio::test]
    async fn a_completed_goal_cannot_gain_a_loop() {
        let dir = tempfile::tempdir().unwrap();
        let id = "cafe0000-0000-0000-0000-000000000000";
        seed_goals(dir.path(), &[goal(id, "Done deal", GoalStatus::Completed)]);

        let err = run_in(dir.path(), start("cafe", 5, 10)).await.unwrap_err();
        assert!(err.contains("already completed"), "{err}");
    }

    #[tokio::test]
    async fn the_second_loop_on_the_same_title_gets_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let first = run_in(dir.path(), start("Ship the parser", 5, 10))
            .await
            .unwrap();
        let second = run_in(dir.path(), start("Ship the parser", 5, 10))
            .await
            .unwrap();
        assert!(first.contains("名称: `Ship-the-parser`"), "{first}");
        assert!(second.contains("-2`"), "{second}");
    }

    #[tokio::test]
    async fn a_stored_loop_shows_up_in_the_list() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), start("Ship the parser", 7, 6))
            .await
            .unwrap();

        let listed = run_in(dir.path(), GoalLoopCommand::List).await.unwrap();
        assert!(listed.starts_with("# /goal-loop 任务列表"), "{listed}");
        assert!(listed.contains("`Ship-the-parser` [running]"), "{listed}");
        assert!(listed.contains("iteration 0/6"), "{listed}");
        assert!(listed.contains("every: 7 min"), "{listed}");
    }

    #[tokio::test]
    async fn stop_reports_whether_the_loop_existed() {
        let dir = tempfile::tempdir().unwrap();
        run_in(dir.path(), start("Ship the parser", 5, 10))
            .await
            .unwrap();

        let removed = run_in(
            dir.path(),
            GoalLoopCommand::Stop {
                name: "Ship-the-parser".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(
            removed.contains("已删除 goal-loop `Ship-the-parser`"),
            "{removed}"
        );

        let again = run_in(
            dir.path(),
            GoalLoopCommand::Stop {
                name: "ship-the-parser".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(again.contains("未找到 goal-loop"), "{again}");
    }

    // ── 调度语义 ─────────────────────────────────────────────────────────

    async fn seed_store_with(
        dir: &std::path::Path,
        goal_id: &str,
        next_run_at: i64,
        iterations_run: u64,
        max_iterations: u64,
    ) {
        let task = crate::core::loops::add_goal_loop(
            dir,
            "cached-loop".to_string(),
            goal_id.to_string(),
            "cached goal".to_string(),
            5,
            max_iterations,
        )
        .await
        .unwrap();

        // add 之后直接改调度时间 / 计数，绕过“必须在未来”的默认值。
        let mut store = crate::core::loops::load_goal_loop_store(dir).await.unwrap();
        let now = Utc::now().timestamp();
        let stored = store.tasks.last_mut().unwrap();
        stored.name = task.name.clone();
        stored.next_run_at = if next_run_at < 0 {
            now + next_run_at
        } else {
            next_run_at
        };
        stored.iterations_run = iterations_run;
        crate::core::loops::save_goal_loop_store(dir, &store)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tick_triggers_a_due_loop_and_counts_the_iteration() {
        use chrono::Utc;
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::InProgress)],
        );
        seed_store_with(dir.path(), id, Utc::now().timestamp() - 30, 0, 3).await;

        let due =
            crate::core::loops::tick_and_collect_due_goal_loops(dir.path(), Utc::now().timestamp())
                .await
                .unwrap();
        assert_eq!(due.len(), 1, "到期的那一轮应当触发");
        assert_eq!(due[0].iterations_run, 1);

        // goal 提示词能按 id 取回（含标题），供触发消息用。
        let prompt = crate::core::loops::goal_prompt_for(dir.path(), id)
            .await
            .unwrap();
        assert!(prompt.unwrap().contains("Ship the parser"));
    }

    #[tokio::test]
    async fn tick_finishes_the_loop_when_the_goal_is_completed() {
        use chrono::Utc;
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::InProgress)],
        );
        seed_store_with(dir.path(), id, Utc::now().timestamp() - 30, 0, 3).await;

        // 模拟用户 `/goal done`：直接把状态改成 Completed。
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::Completed)],
        );

        let due =
            crate::core::loops::tick_and_collect_due_goal_loops(dir.path(), Utc::now().timestamp())
                .await
                .unwrap();
        assert!(due.is_empty(), "goal 完成后不再触发");

        let tasks = crate::core::loops::list_goal_loops(dir.path())
            .await
            .unwrap();
        assert!(tasks[0].finished);
        assert_eq!(tasks[0].finish_reason.as_deref(), Some("goal_completed"));
    }

    #[tokio::test]
    async fn tick_finishes_the_loop_when_iterations_run_out() {
        use chrono::Utc;
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::InProgress)],
        );
        // 已跑 1 轮、上限 2 轮：本轮照样触发，之后收口。
        seed_store_with(dir.path(), id, Utc::now().timestamp() - 30, 1, 2).await;

        let due =
            crate::core::loops::tick_and_collect_due_goal_loops(dir.path(), Utc::now().timestamp())
                .await
                .unwrap();
        assert_eq!(due.len(), 1, "最后一轮照样触发");

        let tasks = crate::core::loops::list_goal_loops(dir.path())
            .await
            .unwrap();
        assert!(tasks[0].finished);
        assert_eq!(tasks[0].finish_reason.as_deref(), Some("max_iterations"));

        // 收口后的下一轮不再触发。
        let due =
            crate::core::loops::tick_and_collect_due_goal_loops(dir.path(), Utc::now().timestamp())
                .await
                .unwrap();
        assert!(due.is_empty());
    }

    #[tokio::test]
    async fn tick_finishes_the_loop_when_the_goal_is_deleted() {
        use chrono::Utc;
        let dir = tempfile::tempdir().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        seed_goals(
            dir.path(),
            &[goal(id, "Ship the parser", GoalStatus::InProgress)],
        );
        seed_store_with(dir.path(), id, Utc::now().timestamp() - 30, 0, 3).await;

        // goal 被删除（goals.json 里没有这个 id 了）。
        seed_goals(
            dir.path(),
            &[goal(
                "99999999-0000-0000-0000-000000000000",
                "Other",
                GoalStatus::InProgress,
            )],
        );

        let due =
            crate::core::loops::tick_and_collect_due_goal_loops(dir.path(), Utc::now().timestamp())
                .await
                .unwrap();
        assert!(due.is_empty());

        let tasks = crate::core::loops::list_goal_loops(dir.path())
            .await
            .unwrap();
        assert!(tasks[0].finished);
        assert_eq!(tasks[0].finish_reason.as_deref(), Some("goal_missing"));
    }
}
