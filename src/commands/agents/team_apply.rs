use super::team_definitions::{
    normalize_agent_name, validate_team_run_id, TeamAgentDef, TeamMemberWorkspace,
};
use super::*;

/// `git apply --3way` 是否留下了冲突标记。
///
/// 这一条是必需的：带冲突时 git 的退出码依然是 0，只在输出里写
/// "Applied patch to 'x' with conflicts." 与 `U <path>` 行。
fn applied_with_conflicts(output: &std::process::Output) -> bool {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined
        .lines()
        .any(|line| line.trim_start().starts_with("with conflicts"))
        || combined.contains(" with conflicts")
}

/// 从 `--3way` 的冲突输出里取文件名。
///
/// `git apply --3way` 报冲突的方式与 `git am` 不同：它打印
/// "Applied patch to 'file.txt' with conflicts." 和 `U file.txt`，
/// 而 `collect_apply_conflict_files` 认的是 `error: patch failed:` 那套。
fn conflict_files_from_apply_output(output: &std::process::Output) -> Vec<String> {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut files: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for raw in combined.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Applied patch to '") {
            if let Some(end) = rest.find('\'') {
                files.insert(rest[..end].to_string());
            }
        }
        // 未合并路径列表：`U file.txt` / `UU file.txt`
        let mut chars = line.chars();
        if let Some(first) = chars.next() {
            let rest = chars.as_str();
            if matches!(first, 'U' | 'A' | 'D') && (rest.starts_with(' ') || rest.starts_with('U'))
            {
                let path = rest.trim_start_matches('U').trim();
                if !path.is_empty() {
                    files.insert(path.to_string());
                }
            }
        }
    }
    files.into_iter().collect()
}

pub(super) async fn apply_team_run(
    ctx: CommandContext<'_>,
    args: AgentTeamApplyArgs,
) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    apply_team_run_in(ctx, args, &cwd).await
}

/// `cwd` 走参数：单测在临时 git 仓库里验证 apply 的每一条分支。
pub(super) async fn apply_team_run_in(
    ctx: CommandContext<'_>,
    args: AgentTeamApplyArgs,
    cwd: &Path,
) -> CommandResult {
    let storage = crate::core::config::storage::Storage::new(cwd.to_path_buf());
    let run_id = validate_team_run_id(&args.run_id)?;
    let run = load_team_run_record(&storage, &run_id).await?;

    if !run.git_mode {
        return Err(format!(
            "team run `{}` was not created in git-worktree mode; no apply artifacts available",
            run_id
        ));
    }

    let repo_root = git_service::detect_repo_root(cwd)
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

                // `git apply --3way` 在留下冲突标记时**仍然退出 0** —— 它只把
                // 冲突写进 stdout（"Applied patch to 'x' with conflicts."）。
                // 只看退出码会把一个带冲突标记的半成品文件报成"已应用"。
                let conflicted = !args.dry_run && applied_with_conflicts(&output);
                if output.status.success() && !conflicted {
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
                    } else if conflicted {
                        lines.push(format!(
                            "- ⚠️ `{}` left conflict markers (ours keeps current changes): {}",
                            member.name, summary
                        ));
                    } else {
                        lines.push(format!(
                            "- ⚠️ `{}` skipped (ours keeps current changes): {}",
                            member.name, summary
                        ));
                    }
                    let files = if conflicted {
                        conflict_files_from_apply_output(&output)
                    } else {
                        collect_apply_conflict_files(&output)
                    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_team_support::{save_team_run_record, TeamRunMemberRecord};
    use crate::ui::state::ChatState;

    fn harness() -> (
        ChatState,
        tokio::sync::mpsc::Sender<crate::runtime::messages::AgentRequest>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        std::mem::forget(rx);
        (ChatState::test_fixture(), tx)
    }

    fn last(state: &ChatState) -> String {
        state
            .chat_history
            .last()
            .map(|e| e.content.clone())
            .unwrap_or_default()
    }

    fn member(name: &str, has_changes: bool, patch: &Path) -> TeamRunMemberRecord {
        TeamRunMemberRecord {
            name: name.to_string(),
            internal_id: name.to_string(),
            work_dir: "/tmp/w".to_string(),
            target: ".".to_string(),
            isolation_mode: "git-worktree".to_string(),
            patch_path: patch.display().to_string(),
            has_changes,
            changed_files: if has_changes { 1 } else { 0 },
            success: true,
            summary: "ok".to_string(),
            error: None,
            duration_ms: 1,
            round: 1,
        }
    }

    fn record(run_id: &str, repo: &Path, members: Vec<TeamRunMemberRecord>) -> TeamRunRecord {
        TeamRunRecord {
            run_id: run_id.to_string(),
            created_at: 1,
            command_cwd: repo.display().to_string(),
            source_target: ".".to_string(),
            git_mode: true,
            repo_root: Some(repo.display().to_string()),
            base_head: None,
            rounds: 1,
            mode: "parallel".to_string(),
            round_traces: Vec::new(),
            shared_memory: Vec::new(),
            members,
        }
    }

    fn args(run_id: &str) -> AgentTeamApplyArgs {
        AgentTeamApplyArgs {
            run_id: run_id.to_string(),
            strategy: TeamApplyStrategy::Manual,
            members: None,
            dry_run: false,
            require_clean: false,
            base_head_check: false,
            auto_clean: false,
        }
    }

    fn git(dir: &Path, argv: &[&str]) {
        let out = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(argv)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {argv:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// 建一个真仓库：apply 的每一条分支都要真的跑 `git apply`。
    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join("file.txt"), "line one\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "init"]);
    }

    /// 生成一个能干净应用的 patch（在副本上改一行再 diff）。
    fn make_patch(repo: &Path, dst: &Path, new_content: &str) {
        let work = tempfile::tempdir().unwrap();
        git(work.path(), &["init", "-q"]);
        std::fs::write(work.path().join("file.txt"), "line one\n").unwrap();
        git(work.path(), &["add", "."]);
        git(work.path(), &["commit", "-q", "-m", "init"]);
        std::fs::write(work.path().join("file.txt"), new_content).unwrap();
        let out = std::process::Command::new("git")
            .args(["diff"])
            .current_dir(work.path())
            .output()
            .unwrap();
        std::fs::write(dst, out.stdout).unwrap();
        let _ = repo;
    }

    // ── 前置校验 ────────────────────────────────────────────────

    /// 非 worktree 模式没有 patch 可应用，必须说清楚而不是空跑一遍。
    #[tokio::test]
    async fn a_shared_mode_run_cannot_be_applied() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        let mut rec = record("run-1", dir.path(), vec![]);
        rec.git_mode = false;
        save_team_run_record(&storage, &rec).await.unwrap();

        let (mut state, tx) = harness();
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("run-1"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("was not created in git-worktree mode"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn applying_outside_a_git_repository_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &record("run-1", dir.path(), vec![]))
            .await
            .unwrap();

        let (mut state, tx) = harness();
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("run-1"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not inside a git repository"), "{err}");
    }

    #[tokio::test]
    async fn a_run_recorded_against_another_repo_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let other = tempfile::tempdir().unwrap();
        init_repo(other.path());

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &record("run-1", other.path(), vec![]))
            .await
            .unwrap();

        let (mut state, tx) = harness();
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("run-1"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("repo_root mismatch"), "{err}");
    }

    #[tokio::test]
    async fn require_clean_refuses_a_dirty_tree() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        std::fs::write(dir.path().join("file.txt"), "dirty\n").unwrap();

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &record("run-1", dir.path(), vec![]))
            .await
            .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.require_clean = true;
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("working tree is not clean"), "{err}");
    }

    #[tokio::test]
    async fn base_head_check_requires_a_recorded_head() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(&storage, &record("run-1", dir.path(), vec![]))
            .await
            .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.base_head_check = true;
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("no base_head"), "{err}");
    }

    #[tokio::test]
    async fn base_head_check_detects_a_moved_head() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        let mut rec = record("run-1", dir.path(), vec![]);
        rec.base_head = Some("deadbeef".to_string());
        save_team_run_record(&storage, &rec).await.unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.base_head_check = true;
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("base_head mismatch"), "{err}");
    }

    // ── 成员筛选 ────────────────────────────────────────────────

    /// 只有改过文件的成员才需要应用；一个都没有时给出明确说法，
    /// 而不是印一张空表。
    #[tokio::test]
    async fn a_run_without_changed_members_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record(
                "run-1",
                dir.path(),
                vec![member("alice", false, &dir.path().join("a.patch"))],
            ),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("run-1"),
            dir.path(),
        )
        .await
        .unwrap();
        assert!(last(&state).contains("no matching changed members to apply"));
    }

    #[tokio::test]
    async fn the_members_filter_narrows_the_selection_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record(
                "run-1",
                dir.path(),
                vec![
                    member("alice", true, &dir.path().join("a.patch")),
                    member("bob", true, &dir.path().join("b.patch")),
                ],
            ),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.members = Some(vec!["ALICE".to_string()]);
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("- members_to_apply: 1"), "{out}");
        assert!(out.contains("### `alice`"), "{out}");
        assert!(!out.contains("### `bob`"), "{out}");
    }

    // ── manual ──────────────────────────────────────────────────

    /// manual 策略只打印命令，不碰工作区。
    #[tokio::test]
    async fn manual_prints_check_and_apply_commands_without_touching_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("run-1"),
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.starts_with("# Agent Team Apply"), "{out}");
        assert!(out.contains("- strategy: `manual`"), "{out}");
        assert!(out.contains("## Manual Steps"), "{out}");
        assert!(out.contains("apply --check --3way"), "{out}");
        assert!(out.contains("apply --3way"), "{out}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "line one\n",
            "manual must not modify files"
        );
    }

    #[tokio::test]
    async fn manual_with_auto_clean_explains_why_nothing_was_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.auto_clean = true;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        assert!(last(&state).contains("`--auto-clean` 在 strategy=manual 下不会立即生效"));
    }

    // ── ours ────────────────────────────────────────────────────

    #[tokio::test]
    async fn ours_dry_run_checks_without_applying() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        make_patch(dir.path(), &patch, "line one changed\n");

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        a.dry_run = true;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("## Apply Check Results"), "{out}");
        assert!(
            out.contains("✅ `alice` check passed (would apply)"),
            "{out}"
        );
        assert!(
            out.contains("- summary: would_apply=1 would_conflict=0 missing_patch=0"),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "line one\n",
            "--dry-run must not write"
        );
    }

    #[tokio::test]
    async fn ours_applies_a_clean_patch() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        make_patch(dir.path(), &patch, "line one changed\n");

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("## Apply Results"), "{out}");
        assert!(out.contains("✅ `alice` applied"), "{out}");
        assert!(
            out.contains("- summary: applied=1 conflicted=0 missing_patch=0"),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "line one changed\n"
        );
    }

    /// patch 文件丢了不能算"冲突" —— 两者的处理方式完全不同。
    #[tokio::test]
    async fn ours_reports_a_missing_patch_separately() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("gone.patch");
        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("patch file not found"), "{out}");
        assert!(
            out.contains("- summary: applied=0 conflicted=0 missing_patch=1"),
            "{out}"
        );
    }

    /// 冲突时列出具体文件，用户才知道该手工合哪几个。
    #[tokio::test]
    async fn ours_lists_conflict_files_when_a_patch_does_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("conflict.patch");
        make_patch(dir.path(), &patch, "line one changed\n");
        // 先把文件改成 patch 的上下文对不上的样子
        std::fs::write(dir.path().join("file.txt"), "completely different\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "diverge"]);

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        // `--3way` 留下冲突标记时退出码仍是 0，只能靠输出识别 —— 认不出来
        // 就会把带冲突标记的半成品文件报成"已应用"。
        assert!(out.contains("⚠️ `alice` left conflict markers"), "{out}");
        assert!(out.contains("- conflict_files: file.txt"), "{out}");
        assert!(
            out.contains("- summary: applied=0 conflicted=1 missing_patch=0"),
            "{out}"
        );
    }

    /// auto_clean 只在真正应用成功之后才跑，而且 dry-run 下不能删东西。
    #[tokio::test]
    async fn auto_clean_is_deferred_under_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        make_patch(dir.path(), &patch, "line one changed\n");

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        a.dry_run = true;
        a.auto_clean = true;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        assert!(last(&state).contains("`--dry-run` 下不会执行 `--auto-clean`"));
        assert!(
            crate::commands::agent_team_support::team_run_record_path(&storage, "run-1").exists(),
            "the record must survive a dry run"
        );
    }

    #[tokio::test]
    async fn auto_clean_removes_the_run_record_after_a_real_apply() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let patch = dir.path().join("a.patch");
        make_patch(dir.path(), &patch, "line one changed\n");

        let storage = crate::core::config::storage::Storage::new(dir.path().to_path_buf());
        save_team_run_record(
            &storage,
            &record("run-1", dir.path(), vec![member("alice", true, &patch)]),
        )
        .await
        .unwrap();

        let (mut state, tx) = harness();
        let mut a = args("run-1");
        a.strategy = TeamApplyStrategy::Ours;
        a.auto_clean = true;
        apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            a,
            dir.path(),
        )
        .await
        .unwrap();

        let out = last(&state);
        assert!(out.contains("auto_clean:"), "{out}");
        assert!(
            !crate::commands::agent_team_support::team_run_record_path(&storage, "run-1").exists()
        );
    }

    #[tokio::test]
    async fn applying_an_unknown_run_reports_the_missing_record() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let (mut state, tx) = harness();
        let err = apply_team_run_in(
            CommandContext {
                state: &mut state,
                agent_tx: &tx,
            },
            args("ghost"),
            dir.path(),
        )
        .await
        .unwrap_err();
        assert!(err.starts_with("team run record not found:"), "{err}");
    }
}
