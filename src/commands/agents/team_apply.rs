use super::team_definitions::{
    normalize_agent_name, validate_team_run_id, TeamAgentDef, TeamMemberWorkspace,
};
use super::*;

pub(super) async fn apply_team_run(
    ctx: CommandContext<'_>,
    args: AgentTeamApplyArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let storage = crate::core::config::storage::Storage::new(cwd.clone());
    let run_id = validate_team_run_id(&args.run_id)?;
    let run = load_team_run_record(&storage, &run_id).await?;

    if !run.git_mode {
        return Err(format!(
            "team run `{}` was not created in git-worktree mode; no apply artifacts available",
            run_id
        ));
    }

    let repo_root = git_service::detect_repo_root(&cwd)
        .await
        .ok_or_else(|| "current directory is not inside a git repository".to_string())?;

    if let Some(expected_root) = run.repo_root.as_ref() {
        let expected = PathBuf::from(expected_root);
        let expected = expected.canonicalize().unwrap_or(expected);
        let actual = repo_root.canonicalize().unwrap_or(repo_root.clone());
        if expected != actual {
            return Err(format!(
                "run repo_root mismatch. run=`{}` current=`{}`",
                expected.display(),
                actual.display()
            ));
        }
    }

    if args.require_clean {
        let clean = git_service::is_working_tree_clean(&repo_root).await?;
        if !clean {
            return Err(
                "working tree is not clean. commit/stash changes or remove `--require-clean`."
                    .to_string(),
            );
        }
    }

    if args.base_head_check {
        let expected = run.base_head.as_ref().ok_or_else(|| {
            "run record has no base_head; cannot use `--base-head-check`".to_string()
        })?;
        let current = git_service::rev_parse(&repo_root, "HEAD").await?;
        if expected.trim() != current.trim() {
            return Err(format!(
                "base_head mismatch: run=`{}` current=`{}`",
                expected, current
            ));
        }
    }

    let filter_members = args.members.as_ref().map(|members| {
        members
            .iter()
            .map(|m| normalize_agent_name(m))
            .collect::<HashSet<String>>()
    });

    let mut selected_members: Vec<&TeamRunMemberRecord> = Vec::new();
    for member in &run.members {
        if let Some(filter) = filter_members.as_ref() {
            if !filter.contains(&normalize_agent_name(&member.name)) {
                continue;
            }
        }
        if member.has_changes {
            selected_members.push(member);
        }
    }

    if selected_members.is_empty() {
        ctx.state.chat_history.push(
            ChatEntry::assistant(format!(
                "ℹ️ run `{}` has no matching changed members to apply.",
                run_id
            ))
            .with_streaming(false),
        );
        return Ok(());
    }

    let mut lines = vec![
        "# Agent Team Apply".to_string(),
        "".to_string(),
        format!("- run_id: `{}`", run_id),
        format!(
            "- strategy: `{}`",
            format!("{:?}", args.strategy).to_lowercase()
        ),
        format!("- dry_run: {}", args.dry_run),
        format!("- require_clean: {}", args.require_clean),
        format!("- base_head_check: {}", args.base_head_check),
        format!("- auto_clean: {}", args.auto_clean),
        format!("- repo_root: `{}`", repo_root.display()),
        format!("- members_to_apply: {}", selected_members.len()),
        "".to_string(),
    ];

    match args.strategy {
        TeamApplyStrategy::Manual => {
            lines.push("## Manual Steps".to_string());
            for member in selected_members {
                lines.push(format!("### `{}`", member.name));
                lines.push(format!("- patch: `{}`", member.patch_path));
                lines.push(format!("- changed_files: {}", member.changed_files));
                lines.push(format!(
                    "- check_command: `git -C \"{}\" apply --check --3way --whitespace=nowarn \"{}\"`",
                    repo_root.display(),
                    member.patch_path
                ));
                lines.push(format!(
                    "- apply_command: `git -C \"{}\" apply --3way --whitespace=nowarn \"{}\"`",
                    repo_root.display(),
                    member.patch_path
                ));
            }
            if args.dry_run {
                lines.push("".to_string());
                lines.push("- note: strategy=manual 本身不会自动落盘，`--dry-run` 仅用于强调先执行 check_command。".to_string());
            }
        }
        TeamApplyStrategy::Ours => {
            let mut ok_count = 0usize;
            let mut conflict_count = 0usize;
            let mut missing_patch = 0usize;
            let section_title = if args.dry_run {
                "## Apply Check Results"
            } else {
                "## Apply Results"
            };
            lines.push(section_title.to_string());

            for member in selected_members {
                let patch_path = PathBuf::from(&member.patch_path);
                if !patch_path.exists() {
                    missing_patch += 1;
                    lines.push(format!(
                        "- ❌ `{}` patch file not found: {}",
                        member.name,
                        patch_path.display()
                    ));
                    continue;
                }

                let mut cmd = tokio::process::Command::new("git");
                cmd.arg("-C")
                    .arg(&repo_root)
                    .arg("apply")
                    .arg("--3way")
                    .arg("--whitespace=nowarn");
                if args.dry_run {
                    cmd.arg("--check");
                }
                cmd.arg(&patch_path);
                let output = cmd
                    .output()
                    .await
                    .map_err(|e| format!("failed to execute git apply: {}", e))?;

                if output.status.success() {
                    ok_count += 1;
                    if args.dry_run {
                        lines.push(format!(
                            "- ✅ `{}` check passed (would apply): {}",
                            member.name, member.patch_path
                        ));
                    } else {
                        lines.push(format!(
                            "- ✅ `{}` applied ({})",
                            member.name, member.patch_path
                        ));
                    }
                } else {
                    conflict_count += 1;
                    let summary = summarize_output(&output);
                    if args.dry_run {
                        lines.push(format!(
                            "- ⚠️ `{}` check failed (would conflict): {}",
                            member.name, summary
                        ));
                    } else {
                        lines.push(format!(
                            "- ⚠️ `{}` skipped (ours keeps current changes): {}",
                            member.name, summary
                        ));
                    }
                    let files = collect_apply_conflict_files(&output);
                    if !files.is_empty() {
                        lines.push(format!("- conflict_files: {}", files.join(", ")));
                    }
                }
            }

            lines.push("".to_string());
            if args.dry_run {
                lines.push(format!(
                    "- summary: would_apply={} would_conflict={} missing_patch={}",
                    ok_count, conflict_count, missing_patch
                ));
            } else {
                lines.push(format!(
                    "- summary: applied={} conflicted={} missing_patch={}",
                    ok_count, conflict_count, missing_patch
                ));
            }

            if args.auto_clean && !args.dry_run {
                lines.push("".to_string());
                let report = cleanup_team_run_artifacts(&storage, &run_id, &run).await;
                if report.issues.is_empty() {
                    lines.push(format!(
                        "- auto_clean: ✅ done (worktrees={}/{} temp_dir_removed={} run_dir_removed={})",
                        report.removed_worktrees,
                        report.worktree_members,
                        report.removed_temp_dir,
                        report.removed_run_dir
                    ));
                } else {
                    lines.push(format!(
                        "- auto_clean: ⚠️ partial (worktrees={}/{} temp_dir_removed={} run_dir_removed={})",
                        report.removed_worktrees,
                        report.worktree_members,
                        report.removed_temp_dir,
                        report.removed_run_dir
                    ));
                    for issue in report.issues {
                        lines.push(format!("- issue: {}", issue));
                    }
                }
            } else if args.auto_clean && args.dry_run {
                lines.push("".to_string());
                lines.push("- note: `--dry-run` 下不会执行 `--auto-clean`。".to_string());
            }
        }
    }

    if matches!(args.strategy, TeamApplyStrategy::Manual) && args.auto_clean {
        lines.push("".to_string());
        lines.push(
            "- note: `--auto-clean` 在 strategy=manual 下不会立即生效；请手动应用 patch 后再执行 `/agents team clean <run-id>`。"
                .to_string(),
        );
    }

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join(
            "
",
        ))
        .with_streaming(false),
    );
    Ok(())
}
