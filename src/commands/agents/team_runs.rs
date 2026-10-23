use super::team_definitions::{
    find_team_agent, normalize_agent_name, resolve_team_agents, team_run_mode_label,
};
use super::team_definitions::{validate_team_run_id, TEAM_AGENT_CATALOG};
use super::*;

pub(super) async fn list_agent_team_catalog(ctx: CommandContext<'_>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    list_agent_team_catalog_in(ctx, &cwd).await
}

/// `cwd` 走参数：单测要在临时目录里验证预设清单的渲染，
/// 否则会读到开发者自己 `.star/agent-teams.json` 里的内容。
pub(super) async fn list_agent_team_catalog_in(
    ctx: CommandContext<'_>,
    cwd: &std::path::Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
    let (project_presets, user_presets) = list_team_presets(&storage).await?;

    let mut lines = vec![
        "# Agent Teams".to_string(),
        "".to_string(),
        "Built-in Team Members:".to_string(),
    ];

    for def in TEAM_AGENT_CATALOG {
        let aliases = if def.aliases.is_empty() {
            "-".to_string()
        } else {
            def.aliases.join(", ")
        };
        lines.push(format!(
            "- `{}` (task_type: `{}`): {} | aliases: {}",
            def.cli_name, def.task_type, def.description, aliases
        ));
    }

    lines.push("".to_string());
    lines.push("Preset Teams:".to_string());
    lines.push(format!(
        "## Project ({})",
        team_preset_file_path(TeamPresetScope::Project, &storage).display()
    ));
    if project_presets.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for p in &project_presets {
            lines.push(format!(
                "- `{}`: agents={} target={} mode={} rounds={} objective={}",
                p.name,
                p.agents.join(","),
                p.target.clone().unwrap_or_else(|| ".".to_string()),
                p.mode.clone().unwrap_or_else(|| "parallel".to_string()),
                p.rounds.unwrap_or(1),
                p.objective
                    .clone()
                    .unwrap_or_else(|| "(required at run time)".to_string())
            ));
        }
    }

    lines.push(format!(
        "## User ({})",
        team_preset_file_path(TeamPresetScope::User, &storage).display()
    ));
    if user_presets.is_empty() {
        lines.push("- (empty)".to_string());
    } else {
        for p in &user_presets {
            lines.push(format!(
                "- `{}`: agents={} target={} mode={} rounds={} objective={}",
                p.name,
                p.agents.join(","),
                p.target.clone().unwrap_or_else(|| ".".to_string()),
                p.mode.clone().unwrap_or_else(|| "parallel".to_string()),
                p.rounds.unwrap_or(1),
                p.objective
                    .clone()
                    .unwrap_or_else(|| "(required at run time)".to_string())
            ));
        }
    }

    lines.push("".to_string());
    lines.push("Examples:".to_string());
    lines.push(
        "- `/agents team run --agents search,analyzer --target src trace command execution flow`"
            .to_string(),
    );
    lines.push("- `/agents team save rust-refactor --agents analyzer,editor --target src --max-steps 6 --description \"Rust refactoring template\"`".to_string());
    lines.push(
        "- `/agents team run --team rust-refactor refactor provider parsing flow`".to_string(),
    );
    lines.push("- `/agents team run --agents analyzer,editor --mode pipeline --rounds 2 先分析后修改并复查`".to_string());
    lines.push("- `/agents team apply <run-id> --strategy manual`".to_string());
    lines.push("- `/agents team apply <run-id> --strategy ours --dry-run`".to_string());
    lines.push("- `/agents team show-run <run-id>`".to_string());
    lines.push("- `/agents team runs --limit 20`".to_string());
    lines.push("- `/agents team clean <run-id>`".to_string());
    lines.push(
        "- `/agents team run --agents all --parallelism 3 --timeout-secs 120 全量检查工具调用路径`"
            .to_string(),
    );

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );
    Ok(())
}

pub(super) async fn list_team_runs(
    ctx: CommandContext<'_>,
    args: AgentTeamRunsArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    list_team_runs_in(ctx, args, &cwd).await
}

pub(super) async fn list_team_runs_in(
    ctx: CommandContext<'_>,
    args: AgentTeamRunsArgs,
    cwd: &std::path::Path,
) -> CommandResult {
    if args.limit == 0 {
        return Err("limit must be >= 1".to_string());
    }

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
    let runs = scan_team_run_records(&storage).await?;

    if runs.is_empty() {
        let msg = format!(
            "ℹ️ no team runs found in `{}`",
            team_runs_root(&storage).display()
        );
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    }

    let text = render_team_runs_list(&team_runs_root(&storage), &runs, args.limit);
    ctx.state
        .chat_history
        .push(ChatEntry::assistant(text).with_streaming(false));
    Ok(())
}

pub(super) async fn show_team_run(
    ctx: CommandContext<'_>,
    args: AgentTeamShowRunArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    show_team_run_in(ctx, args, &cwd).await
}

pub(super) async fn show_team_run_in(
    ctx: CommandContext<'_>,
    args: AgentTeamShowRunArgs,
    cwd: &std::path::Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
    let run_id = validate_team_run_id(&args.run_id)?;
    let mut run = load_team_run_record(&storage, &run_id).await?;

    if let Some(members) = args.members.as_ref() {
        let filter: HashSet<String> = members.iter().map(|m| normalize_agent_name(m)).collect();
        run.members
            .retain(|m| filter.contains(&normalize_agent_name(&m.name)));
    }

    if args.json {
        let text = serde_json::to_string_pretty(&run)
            .map_err(|e| format!("failed to serialize run record: {}", e))?;
        ctx.state.chat_history.push(
            ChatEntry::assistant(format!(
                "```json
{}
```",
                text
            ))
            .with_streaming(false),
        );
        return Ok(());
    }

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(render_team_run_details(&run)).with_streaming(false));
    Ok(())
}

pub(super) async fn clean_team_runs(
    ctx: CommandContext<'_>,
    args: AgentTeamCleanArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    clean_team_runs_in(ctx, args, &cwd).await
}

pub(super) async fn clean_team_runs_in(
    ctx: CommandContext<'_>,
    args: AgentTeamCleanArgs,
    cwd: &std::path::Path,
) -> CommandResult {
    if !args.all
        && args
            .run_id
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(
            "usage: `/agents team clean <run-id>` or `/agents team clean --all`".to_string(),
        );
    }

    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());

    let run_ids: Vec<String> = if args.all {
        scan_team_run_records(&storage)
            .await?
            .into_iter()
            .map(|r| r.run_id)
            .collect()
    } else {
        vec![validate_team_run_id(
            args.run_id.as_deref().unwrap_or_default(),
        )?]
    };

    if run_ids.is_empty() {
        let msg = format!(
            "ℹ️ no team runs to clean in `{}`",
            team_runs_root(&storage).display()
        );
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    }

    let mut ok_count = 0usize;
    let mut warn_count = 0usize;
    let mut lines = vec![
        "# Agent Team Clean".to_string(),
        "".to_string(),
        format!(
            "- scope: {}",
            if args.all { "all runs" } else { "single run" }
        ),
        format!("- requested_runs: {}", run_ids.len()),
        "".to_string(),
    ];

    for run_id in run_ids {
        let safe_run_id = match validate_team_run_id(&run_id) {
            Ok(id) => id,
            Err(e) => {
                warn_count += 1;
                lines.push(format!("- ⚠️ `{}` skipped: {}", run_id, e));
                continue;
            }
        };

        let run = match load_team_run_record(&storage, &safe_run_id).await {
            Ok(r) => r,
            Err(e) => {
                warn_count += 1;
                lines.push(format!("- ⚠️ `{}` skipped: {}", safe_run_id, e));
                continue;
            }
        };

        let report = cleanup_team_run_artifacts(&storage, &safe_run_id, &run).await;

        if report.issues.is_empty() {
            ok_count += 1;
            lines.push(format!(
                "- ✅ `{}` cleaned: worktrees={}/{} temp_dir_removed={} run_dir_removed={}",
                safe_run_id,
                report.removed_worktrees,
                report.worktree_members,
                report.removed_temp_dir,
                report.removed_run_dir
            ));
        } else {
            warn_count += 1;
            lines.push(format!(
                "- ⚠️ `{}` cleaned with issues: worktrees={}/{} temp_dir_removed={} run_dir_removed={}",
                safe_run_id,
                report.removed_worktrees,
                report.worktree_members,
                report.removed_temp_dir,
                report.removed_run_dir
            ));
            for err in report.issues {
                lines.push(format!("- issue: {}", err));
            }
        }
    }

    lines.push("".to_string());
    lines.push(format!(
        "- summary: success={} with_warnings={}",
        ok_count, warn_count
    ));

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_team_presets::TeamPresetStore;
    use crate::commands::agent_team_support::{save_team_run_record, TeamRunMemberRecord};

    fn ctx_parts() -> (
        crate::ui::state::ChatState,
        tokio::sync::mpsc::Sender<crate::runtime::messages::AgentRequest>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        // rx 必须活着，否则命令里的 send 会立刻失败
        std::mem::forget(rx);
        (crate::ui::state::ChatState::test_fixture(), tx)
    }

    fn last(state: &crate::ui::state::ChatState) -> String {
        state
            .chat_history
            .last()
            .map(|e| e.content.clone())
            .unwrap_or_default()
    }

    fn run_record(run_id: &str, created_at: i64, member: &str, ok: bool) -> TeamRunRecord {
        TeamRunRecord {
            run_id: run_id.to_string(),
            created_at,
            command_cwd: "/repo".to_string(),
            source_target: ".".to_string(),
            git_mode: false,
            repo_root: None,
            base_head: None,
            rounds: 1,
            mode: "parallel".to_string(),
            round_traces: Vec::new(),
            shared_memory: Vec::new(),
            members: vec![TeamRunMemberRecord {
                name: member.to_string(),
                internal_id: member.to_string(),
                work_dir: "/tmp/w".to_string(),
                target: ".".to_string(),
                isolation_mode: "shared(non-git)".to_string(),
                patch_path: String::new(),
                has_changes: false,
                changed_files: 0,
                success: ok,
                summary: "done".to_string(),
                error: None,
                duration_ms: 5,
                round: 1,
            }],
        }
    }

    // ── 目录清单 ────────────────────────────────────────────────

    /// 清单是用户唯一能看到"有哪些成员可选"的地方，成员名必须对得上
    /// `--agents` 能接受的名字。
    #[tokio::test]
    async fn the_catalog_lists_every_member_and_the_examples() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        list_agent_team_catalog_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("# Agent Teams"), "{out}");
        for def in TEAM_AGENT_CATALOG {
            assert!(out.contains(&format!("`{}`", def.cli_name)), "{out}");
        }
        assert!(out.contains("Preset Teams:"), "{out}");
        assert!(out.contains("- (empty)"), "{out}");
        assert!(
            out.contains("/agents team run --agents search,analyzer"),
            "{out}"
        );
        assert!(out.contains("/agents team show-run <run-id>"), "{out}");
    }

    #[tokio::test]
    async fn the_catalog_renders_saved_presets() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        let path = team_preset_file_path(TeamPresetScope::Project, &storage);
        save_team_preset_store(
            &path,
            &TeamPresetStore {
                teams: vec![TeamPreset {
                    name: "rust-refactor".to_string(),
                    description: None,
                    agents: vec!["analyzer".to_string(), "editor".to_string()],
                    target: Some("src".to_string()),
                    max_steps: Some(6),
                    parallelism: None,
                    mode: Some("pipeline".to_string()),
                    rounds: Some(2),
                    timeout_secs: None,
                    dry_run: None,
                    objective: Some("refactor".to_string()),
                    updated_at: 1,
                }],
            },
        )
        .await
        .unwrap();

        let (mut state, tx) = ctx_parts();
        list_agent_team_catalog_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(
            out.contains("`rust-refactor`: agents=analyzer,editor"),
            "{out}"
        );
        assert!(out.contains("target=src"), "{out}");
        assert!(out.contains("mode=pipeline"), "{out}");
        assert!(out.contains("rounds=2"), "{out}");
        assert!(out.contains("objective=refactor"), "{out}");
    }

    // ── runs ────────────────────────────────────────────────────

    #[tokio::test]
    async fn runs_reports_an_empty_project_with_the_searched_path() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        list_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamRunsArgs { limit: 10 },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("ℹ️ no team runs found in `"), "{out}");
        assert!(out.contains("agent-teams"), "{out}");
    }

    #[tokio::test]
    async fn runs_renders_records_with_the_requested_limit() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        for (id, ts) in [("run-a", 100), ("run-b", 200), ("run-c", 300)] {
            save_team_run_record(&storage, &run_record(id, ts, "alice", true))
                .await
                .unwrap();
        }

        let (mut state, tx) = ctx_parts();
        list_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamRunsArgs { limit: 2 },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("# Agent Team Runs"), "{out}");
        assert!(out.contains("- total: 3"), "{out}");
        assert!(out.contains("- showing: 2 (limit=2)"), "{out}");
        assert!(out.contains("run-c") && out.contains("run-b"), "{out}");
        assert!(!out.contains("run-a"), "{out}");
    }

    #[tokio::test]
    async fn a_zero_limit_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = list_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamRunsArgs { limit: 0 },
            dir.path(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "limit must be >= 1");
    }

    // ── show-run ────────────────────────────────────────────────

    #[tokio::test]
    async fn show_run_renders_markdown_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &run_record("run-a", 1, "alice", true))
            .await
            .unwrap();

        let (mut state, tx) = ctx_parts();
        show_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamShowRunArgs {
                run_id: "run-a".to_string(),
                json: false,
                members: None,
            },
            dir.path(),
        )
        .await
        .unwrap();

        assert!(last(&state).starts_with("# Agent Team Run Record"));
    }

    #[tokio::test]
    async fn show_run_can_emit_json_inside_a_fence() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &run_record("run-a", 1, "alice", true))
            .await
            .unwrap();

        let (mut state, tx) = ctx_parts();
        show_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamShowRunArgs {
                run_id: "run-a".to_string(),
                json: true,
                members: None,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("```json"), "{out}");
        assert!(out.trim_end().ends_with("```"), "{out}");
        assert!(out.contains("\"run_id\": \"run-a\""), "{out}");
    }

    /// `--members` 用同一套名字归一：`Alice` 也要能筛到 `alice`。
    #[tokio::test]
    async fn show_run_filters_members_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        let mut record = run_record("run-a", 1, "alice", true);
        record.members.push(TeamRunMemberRecord {
            name: "bob".to_string(),
            internal_id: "bob".to_string(),
            work_dir: "/tmp/b".to_string(),
            target: ".".to_string(),
            isolation_mode: "shared(non-git)".to_string(),
            patch_path: String::new(),
            has_changes: false,
            changed_files: 0,
            success: false,
            summary: "failed".to_string(),
            error: None,
            duration_ms: 5,
            round: 1,
        });
        save_team_run_record(&storage, &record).await.unwrap();

        let (mut state, tx) = ctx_parts();
        show_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamShowRunArgs {
                run_id: "run-a".to_string(),
                json: false,
                members: Some(vec!["ALICE".to_string()]),
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("`alice`"), "{out}");
        assert!(!out.contains("`bob`"), "{out}");
    }

    #[tokio::test]
    async fn show_run_rejects_a_path_traversing_id() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = show_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamShowRunArgs {
                run_id: "../../etc".to_string(),
                json: false,
                members: None,
            },
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("invalid run_id"), "{err}");
    }

    #[tokio::test]
    async fn show_run_on_a_missing_record_reports_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = show_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamShowRunArgs {
                run_id: "ghost".to_string(),
                json: false,
                members: None,
            },
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("team run record not found:"), "{err}");
    }

    // ── clean ───────────────────────────────────────────────────

    #[tokio::test]
    async fn clean_without_an_id_or_all_states_the_usage() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        let err = clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: None,
                all: false,
            },
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("/agents team clean <run-id>"), "{err}");
        assert!(err.contains("--all"), "{err}");

        // 空白 id 等同于没给
        let err = clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: Some("   ".to_string()),
                all: false,
            },
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("usage:"), "{err}");
    }

    #[tokio::test]
    async fn clean_all_on_an_empty_project_says_nothing_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, tx) = ctx_parts();
        clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: None,
                all: true,
            },
            dir.path(),
        )
        .await
        .unwrap();

        assert!(last(&state).contains("no team runs to clean"));
    }

    #[tokio::test]
    async fn clean_removes_a_single_run_and_reports_the_counts() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &run_record("run-a", 1, "alice", true))
            .await
            .unwrap();
        save_team_run_record(&storage, &run_record("run-b", 2, "bob", true))
            .await
            .unwrap();

        let (mut state, tx) = ctx_parts();
        clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: Some("run-a".to_string()),
                all: false,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- scope: single run"), "{out}");
        assert!(out.contains("- requested_runs: 1"), "{out}");
        assert!(out.contains("✅ `run-a` cleaned"), "{out}");
        assert!(
            out.contains("- summary: success=1 with_warnings=0"),
            "{out}"
        );

        // 只清指定的那一个
        let remaining = scan_team_run_records(&storage).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].run_id, "run-b");
    }

    #[tokio::test]
    async fn clean_all_sweeps_every_run() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        for (id, ts) in [("run-a", 1), ("run-b", 2)] {
            save_team_run_record(&storage, &run_record(id, ts, "alice", true))
                .await
                .unwrap();
        }

        let (mut state, tx) = ctx_parts();
        clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: None,
                all: true,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- scope: all runs"), "{out}");
        assert!(out.contains("- requested_runs: 2"), "{out}");
        assert!(
            out.contains("- summary: success=2 with_warnings=0"),
            "{out}"
        );
        assert!(scan_team_run_records(&storage).await.unwrap().is_empty());
    }

    /// 单个 run 出错不能让整批停下：其余 run 仍要清掉，并汇总成警告。
    #[tokio::test]
    async fn clean_reports_a_missing_run_as_a_warning_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &run_record("run-a", 1, "alice", true))
            .await
            .unwrap();

        let (mut state, tx) = ctx_parts();
        clean_team_runs_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            AgentTeamCleanArgs {
                run_id: Some("ghost".to_string()),
                all: false,
            },
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("⚠️ `ghost` skipped:"), "{out}");
        assert!(
            out.contains("- summary: success=0 with_warnings=1"),
            "{out}"
        );
    }
}
