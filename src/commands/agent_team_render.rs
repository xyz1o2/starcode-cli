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
