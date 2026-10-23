use crate::commands::agent_team_support::TeamRunRecord;
use chrono::Utc;
use std::path::Path;

fn format_unix_timestamp(ts: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn isolation_label(git_mode: bool) -> &'static str {
    if git_mode {
        "git-worktree"
    } else {
        "shared(non-git)"
    }
}

pub(crate) fn render_team_runs_list(
    runs_root: &Path,
    runs: &[TeamRunRecord],
    limit: usize,
) -> String {
    let mut lines = vec![
        "# Agent Team Runs".to_string(),
        "".to_string(),
        format!("- runs_root: `{}`", runs_root.display()),
        format!("- total: {}", runs.len()),
        format!("- showing: {} (limit={})", runs.len().min(limit), limit),
        "".to_string(),
    ];

    for run in runs.iter().take(limit) {
        let success_count = run.members.iter().filter(|m| m.success).count();
        let failed_count = run.members.len().saturating_sub(success_count);
        let changed_members = run.members.iter().filter(|m| m.has_changes).count();
        lines.push(format!("## `{}`", run.run_id));
        lines.push(format!(
            "- created_at: `{}`",
            format_unix_timestamp(run.created_at)
        ));
        lines.push(format!("- command_cwd: `{}`", run.command_cwd));
        lines.push(format!("- source_target: `{}`", run.source_target));
        lines.push(format!("- isolation: {}", isolation_label(run.git_mode)));
        lines.push(format!("- mode: `{}`", run.mode));
        lines.push(format!("- rounds: {}", run.rounds));
        lines.push(format!(
            "- shared_memory_items: {}",
            run.shared_memory.len()
        ));
        if let Some(repo_root) = run.repo_root.as_ref() {
            lines.push(format!("- repo_root: `{}`", repo_root));
        }
        if let Some(base_head) = run.base_head.as_ref() {
            lines.push(format!("- base_head: `{}`", base_head));
        }
        lines.push(format!(
            "- result: {} success / {} failed / {} changed members",
            success_count, failed_count, changed_members
        ));
        lines.push(format!(
            "- members: {}",
            run.members
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "- apply_hint: `/agents team apply {} --strategy manual`",
            run.run_id
        ));
        lines.push(format!(
            "- apply_check_hint: `/agents team apply {} --strategy ours --dry-run`",
            run.run_id
        ));
        lines.push(format!(
            "- inspect_hint: `/agents team show-run {}`",
            run.run_id
        ));
        lines.push(format!("- clean_hint: `/agents team clean {}`", run.run_id));
        lines.push("".to_string());
    }

    lines.join("\n")
}

pub(crate) fn render_team_run_details(run: &TeamRunRecord) -> String {
    let success_count = run.members.iter().filter(|m| m.success).count();
    let failed_count = run.members.len().saturating_sub(success_count);
    let changed_members = run.members.iter().filter(|m| m.has_changes).count();

    let mut lines = vec![
        "# Agent Team Run Record".to_string(),
        "".to_string(),
        format!("- run_id: `{}`", run.run_id),
        format!("- created_at: `{}`", format_unix_timestamp(run.created_at)),
        format!("- mode: `{}`", run.mode),
        format!("- rounds: {}", run.rounds),
        format!("- command_cwd: `{}`", run.command_cwd),
        format!("- source_target: `{}`", run.source_target),
        format!("- isolation: {}", isolation_label(run.git_mode)),
        format!(
            "- result(final_round): {} success / {} failed / {} changed",
            success_count, failed_count, changed_members
        ),
    ];

    if let Some(repo_root) = run.repo_root.as_ref() {
        lines.push(format!("- repo_root: `{}`", repo_root));
    }
    if let Some(base_head) = run.base_head.as_ref() {
        lines.push(format!("- base_head: `{}`", base_head));
    }

    if !run.round_traces.is_empty() {
        lines.push("".to_string());
        lines.push("## Rounds".to_string());
        for trace in &run.round_traces {
            lines.push(format!(
                "### Round {} [{}ms] success={} failed={} changed={}",
                trace.round,
                trace.duration_ms,
                trace.success_count,
                trace.failed_count,
                trace.changed_members
            ));
            let preview = trace.member_summaries.iter().take(16);
            for item in preview {
                lines.push(format!("- {}", item));
            }
        }
    }

    if !run.shared_memory.is_empty() {
        lines.push("".to_string());
        lines.push("## Shared Memory".to_string());
        for item in &run.shared_memory {
            lines.push(format!("- {}", item));
        }
    }

    lines.push("".to_string());
    lines.push("## Members".to_string());
    if run.members.is_empty() {
        lines.push("- (empty after member filter)".to_string());
    } else {
        for member in &run.members {
            let status = if member.success { "✅" } else { "❌" };
            lines.push(format!(
                "### {} `{}` (`{}`) [round={} {}ms]",
                status, member.name, member.internal_id, member.round, member.duration_ms
            ));
            lines.push(format!("- summary: {}", member.summary));
            lines.push(format!("- work_dir: `{}`", member.work_dir));
            lines.push(format!("- target: `{}`", member.target));
            lines.push(format!("- isolation_mode: `{}`", member.isolation_mode));
            lines.push(format!("- changed_files: {}", member.changed_files));
            if member.has_changes {
                lines.push(format!("- patch: `{}`", member.patch_path));
            }
            if let Some(error) = member.error.as_ref().filter(|e| !e.trim().is_empty()) {
                lines.push(format!("- error: {}", error));
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent_team_support::{
        TeamRunMemberRecord, TeamRunRecord, TeamRunRoundRecord,
    };

    fn member(name: &str, success: bool, changed: bool) -> TeamRunMemberRecord {
        TeamRunMemberRecord {
            name: name.to_string(),
            internal_id: format!("{name}-id"),
            work_dir: format!("/tmp/{name}"),
            target: ".".to_string(),
            isolation_mode: "git-worktree".to_string(),
            patch_path: if changed {
                format!("/tmp/{name}.patch")
            } else {
                String::new()
            },
            has_changes: changed,
            changed_files: if changed { 2 } else { 0 },
            success,
            summary: format!("{name} did the thing"),
            error: None,
            duration_ms: 42,
            round: 1,
        }
    }

    fn run(run_id: &str) -> TeamRunRecord {
        TeamRunRecord {
            run_id: run_id.to_string(),
            created_at: 1_700_000_000,
            command_cwd: "/repo".to_string(),
            source_target: ".".to_string(),
            git_mode: true,
            repo_root: Some("/repo".to_string()),
            base_head: Some("abc1234".to_string()),
            rounds: 1,
            mode: "parallel".to_string(),
            round_traces: Vec::new(),
            shared_memory: Vec::new(),
            members: vec![member("alice", true, true), member("bob", false, false)],
        }
    }

    #[test]
    fn the_runs_list_reports_counts_and_every_follow_up_command() {
        let runs = vec![run("run-1")];
        let out = render_team_runs_list(Path::new("/repo/.star/agent-teams/runs"), &runs, 10);

        assert!(out.starts_with("# Agent Team Runs"), "{out}");
        assert!(out.contains("- total: 1"), "{out}");
        assert!(out.contains("- showing: 1 (limit=10)"), "{out}");
        assert!(out.contains("## `run-1`"), "{out}");
        assert!(out.contains("- isolation: git-worktree"), "{out}");
        assert!(
            out.contains("- result: 1 success / 1 failed / 1 changed members"),
            "{out}"
        );
        // 四个后续动作一个都不能少，否则用户只能靠翻文档
        assert!(
            out.contains("/agents team apply run-1 --strategy manual"),
            "{out}"
        );
        assert!(
            out.contains("/agents team apply run-1 --strategy ours --dry-run"),
            "{out}"
        );
        assert!(out.contains("/agents team show-run run-1"), "{out}");
        assert!(out.contains("/agents team clean run-1"), "{out}");
    }

    #[test]
    fn the_runs_list_honours_the_limit_but_still_reports_the_total() {
        let runs = vec![run("run-1"), run("run-2"), run("run-3")];
        let out = render_team_runs_list(Path::new("/runs"), &runs, 2);

        assert!(out.contains("- total: 3"), "{out}");
        assert!(out.contains("- showing: 2 (limit=2)"), "{out}");
        assert!(out.contains("run-1") && out.contains("run-2"), "{out}");
        assert!(!out.contains("run-3"), "{out}");
    }

    #[test]
    fn a_non_git_run_is_labelled_as_shared() {
        let mut record = run("run-1");
        record.git_mode = false;
        let out = render_team_runs_list(Path::new("/runs"), &[record], 5);
        assert!(out.contains("- isolation: shared(non-git)"), "{out}");
    }

    #[test]
    fn an_empty_run_list_still_renders_a_header() {
        let out = render_team_runs_list(Path::new("/runs"), &[], 5);
        assert!(out.contains("- total: 0"), "{out}");
        assert!(out.contains("- showing: 0 (limit=5)"), "{out}");
    }

    #[test]
    fn details_render_rounds_shared_memory_and_each_member() {
        let mut record = run("run-1");
        record.round_traces.push(TeamRunRoundRecord {
            round: 1,
            objective: "fix the build".to_string(),
            success_count: 1,
            failed_count: 1,
            changed_members: 1,
            duration_ms: 1200,
            member_summaries: vec!["alice: patched".to_string(), "bob: failed".to_string()],
        });
        record
            .shared_memory
            .push("build is broken on main".to_string());

        let out = render_team_run_details(&record);
        assert!(out.starts_with("# Agent Team Run Record"), "{out}");
        assert!(
            out.contains("- result(final_round): 1 success / 1 failed / 1 changed"),
            "{out}"
        );
        assert!(out.contains("## Rounds"), "{out}");
        assert!(
            out.contains("### Round 1 [1200ms] success=1 failed=1 changed=1"),
            "{out}"
        );
        assert!(out.contains("- alice: patched"), "{out}");
        assert!(out.contains("## Shared Memory"), "{out}");
        assert!(out.contains("- build is broken on main"), "{out}");
        assert!(out.contains("## Members"), "{out}");
        assert!(
            out.contains("### ✅ `alice` (`alice-id`) [round=1 42ms]"),
            "{out}"
        );
        assert!(
            out.contains("### ❌ `bob` (`bob-id`) [round=1 42ms]"),
            "{out}"
        );
    }

    /// 没改动的成员不该出现 patch 行；改了的一定要有，否则用户找不到 diff。
    #[test]
    fn patch_lines_only_appear_for_members_that_changed_files() {
        let out = render_team_run_details(&run("run-1"));
        assert!(out.contains("- patch: `/tmp/alice.patch`"), "{out}");
        assert!(!out.contains("/tmp/bob.patch"), "{out}");
    }

    #[test]
    fn member_errors_are_shown_but_blank_ones_are_dropped() {
        let mut record = run("run-1");
        record.members[1].error = Some("  ".to_string());
        let out = render_team_run_details(&record);
        assert!(!out.contains("- error:"), "{out}");

        record.members[1].error = Some("tests failed".to_string());
        let out = render_team_run_details(&record);
        assert!(out.contains("- error: tests failed"), "{out}");
    }

    #[test]
    fn a_run_without_members_says_the_filter_matched_nothing() {
        let mut record = run("run-1");
        record.members.clear();
        let out = render_team_run_details(&record);
        assert!(out.contains("- (empty after member filter)"), "{out}");
        assert!(!out.contains("## Rounds"), "{out}");
        assert!(!out.contains("## Shared Memory"), "{out}");
    }

    #[test]
    fn optional_repo_fields_are_omitted_when_absent() {
        let mut record = run("run-1");
        record.repo_root = None;
        record.base_head = None;
        let out = render_team_run_details(&record);
        assert!(!out.contains("- repo_root:"), "{out}");
        assert!(!out.contains("- base_head:"), "{out}");
    }

    #[test]
    fn timestamps_render_as_utc_and_fall_back_to_raw_values() {
        let out = render_team_run_details(&run("run-1"));
        assert!(out.contains("2023-11-14"), "{out}");
        assert!(out.contains("UTC"), "{out}");

        let mut record = run("run-1");
        record.created_at = i64::MAX;
        let out = render_team_run_details(&record);
        assert!(out.contains(&i64::MAX.to_string()), "{out}");
    }
}
