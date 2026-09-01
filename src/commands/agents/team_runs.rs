use super::team_definitions::{
    find_team_agent, normalize_agent_name, resolve_team_agents, team_run_mode_label,
};
use super::team_definitions::{validate_team_run_id, TEAM_AGENT_CATALOG};
use super::*;

pub(super) async fn list_agent_team_catalog(ctx: CommandContext<'_>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
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
    if args.limit == 0 {
        return Err("limit must be >= 1".to_string());
    }

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd);
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
    let storage = crate::core::config::storage::Storage::new(cwd);
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

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd.clone());

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
