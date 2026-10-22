//! `/fewer-permission-prompts` — mine local history for read-only calls that
//! keep asking for approval, then suggest (or apply) a narrow project allowlist.
//!
//! Claude Code's version is a bundled skill. The useful, testable core is a
//! deterministic classifier: find repeated `Bash`/MCP calls, reject compound or
//! mutating commands, and only then write an allow rule. Keeping that core local
//! means the command works without a model, and makes the safety boundary
//! auditable in tests.
//!
//! Design choices:
//! - Preview by default. A rule that skips confirmation is a security decision,
//!   so writing requires `--yes` (or `--apply`).
//! - Rules land in `.star/settings.local.json`, the same layer `/permissions`
//!   uses. Repository-shared `.star/settings.json` is never modified.
//! - `ask`/`deny` rules always win: candidates they cover are not suggested.
//! - Prefix rules are only generated for commands whose observed invocation is
//!   read-only and whose shape is safe to generalize. Compound shell commands,
//!   redirections, command substitution, `find -exec`, `grep --pre`, and MCP
//!   tools with mutating verbs are excluded.

use crate::commands::activity::{self, ActivityScan, ToolCallRecord};
use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::policy::settings_rules::{self, PermissionRuleSpec, RuleVerdict};
use crate::core::policy::SettingsPermissions;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_DAYS: i64 = 30;
const DEFAULT_MIN_COUNT: usize = 2;
const USAGE: &str = concat!(
    "Usage: /fewer-permission-prompts [--days N] [--min-count N] [--yes]\n",
    "\n",
    "Scans this project's saved sessions and `.star/transcript.jsonl` for repeated\n",
    "read-only Bash and MCP calls, then proposes a project-local allowlist.\n",
    "\n",
    "  --days N       Look back N days (default 30; 0 = all recorded history)\n",
    "  --min-count N  Only suggest rules seen at least N times (default 2)\n",
    "  --yes, --apply Add the suggested rules to `.star/settings.local.json`\n",
    "  --dry-run      Force preview mode (the default)\n",
    "  -h, --help     Show this help\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    days: i64,
    min_count: usize,
    apply: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        days: DEFAULT_DAYS,
        min_count: DEFAULT_MIN_COUNT,
        apply: false,
    };
    let mut dry_run = false;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].trim();
        match arg {
            "--yes" | "--apply" => options.apply = true,
            "--dry-run" => dry_run = true,
            "--days" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--days requires a value".to_string())?;
                options.days = parse_non_negative(value, "--days")?;
                index += 1;
            }
            "--min-count" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--min-count requires a value".to_string())?;
                options.min_count = parse_non_negative_usize(value, "--min-count")?;
                index += 1;
            }
            _ if arg.starts_with("--days=") => {
                options.days = parse_non_negative(&arg["--days=".len()..], "--days")?;
            }
            _ if arg.starts_with("--min-count=") => {
                options.min_count =
                    parse_non_negative_usize(&arg["--min-count=".len()..], "--min-count")?;
            }
            _ => {
                return Err(format!("Unknown argument `{}`.\n\n{}", arg, USAGE));
            }
        }
        index += 1;
    }

    if options.min_count == 0 {
        return Err("--min-count must be at least 1".to_string());
    }
    if dry_run && options.apply {
        return Err("--dry-run cannot be combined with --yes/--apply".to_string());
    }
    Ok(options)
}

fn parse_non_negative(raw: &str, flag: &str) -> Result<i64, String> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| format!("{} expects a non-negative integer, got `{}`", flag, raw))
        .and_then(|value| {
            if value < 0 {
                Err(format!("{} expects a non-negative integer", flag))
            } else {
                Ok(value)
            }
        })
}

fn parse_non_negative_usize(raw: &str, flag: &str) -> Result<usize, String> {
    raw.trim()
        .parse::<usize>()
        .map_err(|_| format!("{} expects a positive integer, got `{}`", flag, raw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateKind {
    Bash,
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    rule: String,
    count: usize,
    kind: CandidateKind,
}

#[derive(Debug, Clone, Default)]
struct Summary {
    scanned_tool_calls: usize,
    bash_seen: usize,
    bash_skipped: usize,
    mcp_seen: usize,
    mcp_skipped: usize,
    already_allowed: usize,
    blocked_by_rule: usize,
    candidates: Vec<Candidate>,
}

pub async fn run(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args
        .iter()
        .any(|arg| arg == "-h" || arg == "--help" || arg == "help")
    {
        reply(ctx.state, USAGE);
        return Ok(());
    }

    let options = parse_options(&args)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cutoff = if options.days == 0 {
        None
    } else {
        Utc::now().checked_sub_signed(Duration::days(options.days))
    };
    let scan = activity::scan_project_activity(&cwd, cutoff);
    let existing = SettingsPermissions::from_project(&cwd);
    let summary = summarize(&scan, &existing, options.min_count);

    if options.apply && !summary.candidates.is_empty() {
        let added = apply_candidates(&cwd, &summary.candidates)?;
        let _ = ctx.agent_tx.send(AgentRequest::ReloadPermissions).await;
        reply(
            ctx.state,
            render_applied(&cwd, &summary, &added, options.days),
        );
    } else {
        reply(ctx.state, render_preview(&cwd, &scan, &summary, &options));
    }
    Ok(())
}

fn reply(state: &mut crate::ui::state::store::ChatState, msg: impl Into<String>) {
    state
        .chat_history
        .push(ChatEntry::assistant(msg).with_streaming(false));
}

/// Run every tool call through the read-only classifiers and aggregate by rule.
/// Existing `allow` rules are reported separately; `ask`/`deny` rules suppress a
/// suggestion entirely.
fn summarize(scan: &ActivityScan, existing: &SettingsPermissions, min_count: usize) -> Summary {
    let mut counts: BTreeMap<(CandidateKind, String), usize> = BTreeMap::new();
    let mut summary = Summary {
        scanned_tool_calls: scan.tool_calls.len(),
        ..Summary::default()
    };

    for call in &scan.tool_calls {
        if is_bash_tool(&call.tool) {
            summary.bash_seen += 1;
            match call.command().and_then(classify_bash_command) {
                Some(rule) => *counts.entry((CandidateKind::Bash, rule)).or_insert(0) += 1,
                None => summary.bash_skipped += 1,
            }
        } else if activity::split_mcp_tool(&call.tool).is_some() {
            summary.mcp_seen += 1;
            match classify_mcp_tool(&call.tool) {
                Some(rule) => *counts.entry((CandidateKind::Mcp, rule)).or_insert(0) += 1,
                None => summary.mcp_skipped += 1,
            }
        }
    }

    for ((kind, rule), count) in counts {
        if count < min_count {
            continue;
        }
        let sample = sample_arguments(kind, &rule);
        match existing.evaluate(&tool_name(kind, &rule), &sample) {
            Some((RuleVerdict::Allow, _)) => summary.already_allowed += count,
            Some((RuleVerdict::Ask | RuleVerdict::Deny, _)) => summary.blocked_by_rule += count,
            None => summary.candidates.push(Candidate { rule, count, kind }),
        }
    }

    summary.candidates.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.rule.cmp(&right.rule))
    });
    summary
}

fn is_bash_tool(tool: &str) -> bool {
    matches!(
        tool.to_ascii_lowercase().as_str(),
        "bash" | "shell" | "run_shell_command" | "terminal"
    )
}

fn tool_name(kind: CandidateKind, rule: &str) -> String {
    match kind {
        CandidateKind::Bash => "Bash".to_string(),
        CandidateKind::Mcp => rule.to_string(),
    }
}

fn sample_arguments(kind: CandidateKind, rule: &str) -> Value {
    match kind {
        CandidateKind::Bash => {
            let prefix = rule
                .strip_prefix("Bash(")
                .and_then(|rest| rest.strip_suffix(":*)"))
                .unwrap_or("true");
            json!({ "command": prefix })
        }
        CandidateKind::Mcp => json!({}),
    }
}

/// Return a `Bash(prefix:*)` rule only when the observed command is a simple,
/// read-only invocation. Earlier shell metacharacters are rejected even when
/// each segment would be harmless: an allow rule is about avoiding prompts, not
/// about teaching the classifier a shell grammar in production.
fn classify_bash_command(command: &str) -> Option<String> {
    let command = command.trim();
    if command.is_empty() || command.contains('\n') || command.contains('\r') {
        return None;
    }
    if has_shell_control(command) {
        return None;
    }

    let words: Vec<&str> = command.split_whitespace().collect();
    let binary = words.first()?.to_ascii_lowercase();
    if binary.contains('/') || binary.contains('\\') || binary.contains('=') {
        return None;
    }
    if matches!(
        binary.as_str(),
        "sudo"
            | "env"
            | "time"
            | "command"
            | "exec"
            | "nohup"
            | "watch"
            | "xargs"
            | "bash"
            | "sh"
            | "zsh"
            | "pwsh"
            | "cmd"
    ) {
        return None;
    }

    if binary == "git" {
        return classify_git(&words);
    }

    let prefix = match binary.as_str() {
        // Prefix rules are safe here: none of these commands has a write or
        // execute flag, and redirections/compound shell syntax were rejected.
        "ls" | "pwd" | "cat" | "head" | "tail" | "wc" | "stat" | "which" | "where" | "type"
        | "echo" | "printf" | "du" | "df" | "basename" | "dirname" | "realpath" | "whoami"
        | "uname" => binary.as_str(),
        _ => return None,
    };
    Some(format!("Bash({}:*)", prefix))
}

fn classify_git(words: &[&str]) -> Option<String> {
    let subcommand = words.get(1)?.to_ascii_lowercase();
    let prefix = match subcommand.as_str() {
        // `git diff` accepts `--output`; it cannot execute a command, but it can
        // write a file. Keep it read-only by rejecting that flag on the observed
        // invocation. The prefix covers normal uses.
        "diff" if !has_flag(words, "--output") => "git diff",
        // These subcommands inspect repository state and have no command-exec
        // flags. `git status` is the common source of repeated prompts.
        "status" | "log" | "show" | "rev-parse" | "ls-files" => {
            return Some(format!("Bash({}:*)", format!("git {}", subcommand)))
        }
        _ => return None,
    };
    Some(format!("Bash({}:*)", prefix))
}

fn has_flag(words: &[&str], flag: &str) -> bool {
    words
        .iter()
        .skip(2)
        .any(|word| *word == flag || word.starts_with(&format!("{}=", flag)))
}

/// Conservative MCP read-only heuristic. A tool is only suggested when its name
/// starts with a known inspection verb and contains no mutating verb. This is a
/// heuristic, so the command always previews before writing.
fn classify_mcp_tool(name: &str) -> Option<String> {
    let (_server, tool) = activity::split_mcp_tool(name)?;
    let tool = tool.to_ascii_lowercase();
    if tool.is_empty() {
        return None;
    }
    const MUTATING: &[&str] = &[
        "add",
        "apply",
        "change",
        "clear",
        "close",
        "comment",
        "commit",
        "copy",
        "create",
        "delete",
        "deploy",
        "download",
        "edit",
        "execute",
        "exec",
        "grant",
        "install",
        "login",
        "logout",
        "merge",
        "modify",
        "move",
        "open",
        "patch",
        "post",
        "publish",
        "purge",
        "push",
        "put",
        "remove",
        "rename",
        "reopen",
        "replace",
        "revoke",
        "run",
        "save",
        "send",
        "set",
        "share",
        "sign",
        "store",
        "submit",
        "transfer",
        "trigger",
        "uninstall",
        "update",
        "upload",
        "write",
    ];
    // Tool names use snake_case today, but camelCase names from older servers
    // should not slip past the verb check.
    let mut normalized = String::new();
    for (index, ch) in tool.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            normalized.push('_');
        }
        normalized.push(ch.to_ascii_lowercase());
    }
    let tokens: Vec<&str> = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.iter().any(|token| MUTATING.contains(token)) {
        return None;
    }

    let safe_prefix = [
        "get", "list", "search", "read", "fetch", "find", "query", "lookup", "count", "describe",
        "status", "info", "whoami", "resolve", "preview", "inspect",
    ];
    let starts_safe = safe_prefix
        .iter()
        .any(|prefix| tool == *prefix || tool.starts_with(&format!("{}_", prefix)));
    let ends_safe = ["snapshot", "screenshot"]
        .iter()
        .any(|suffix| tool == *suffix || tool.ends_with(&format!("_{}", suffix)));
    if starts_safe || ends_safe {
        Some(name.to_string())
    } else {
        None
    }
}

/// True when the command contains shell syntax that makes a prefix rule a bad
/// idea. Quotes are honored so a literal `>` inside `echo "a > b"` stays safe.
fn has_shell_control(command: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '`' => return true,
            '$' if chars.peek() == Some(&'(') => return true,
            ';' | '|' | '&' | '<' | '>' if !in_single && !in_double => return true,
            _ => {}
        }
    }
    in_single || in_double
}

fn apply_candidates(cwd: &Path, candidates: &[Candidate]) -> Result<Vec<String>, String> {
    // Validate every rule before the first write so a malformed candidate cannot
    // leave the settings file half-updated.
    for candidate in candidates {
        PermissionRuleSpec::parse(&candidate.rule).ok_or_else(|| {
            format!(
                "Internal error: generated rule `{}` is not valid",
                candidate.rule
            )
        })?;
    }

    let mut added = Vec::new();
    for candidate in candidates {
        if settings_rules::add_local_rule(cwd, RuleVerdict::Allow, &candidate.rule)?.is_some() {
            added.push(candidate.rule.clone());
        }
    }
    Ok(added)
}

fn render_preview(cwd: &Path, scan: &ActivityScan, summary: &Summary, options: &Options) -> String {
    let window = if options.days == 0 {
        "all recorded history".to_string()
    } else {
        format!("the last {} day(s)", options.days)
    };
    let mut out = format!(
        "# Fewer permission prompts\n\nScanned {} local source(s) and {} tool call(s) from {}.\n",
        scan.sources, summary.scanned_tool_calls, window
    );

    if scan.sources == 0 || summary.scanned_tool_calls == 0 {
        out.push_str(
            "\nNo local tool-call history was found. Sessions are used when saved; otherwise \
             StarCode reads `.star/transcript.jsonl`. Enable transcripts with `STAR_TRANSCRIPT=1` \
             and run a few turns before trying again.\n",
        );
        return out;
    }

    if summary.candidates.is_empty() {
        out.push_str("\nNo new safe allow rules to suggest.\n");
    } else {
        out.push_str(&format!(
            "\n## Suggested allow rules (seen at least {} time(s))\n\n",
            options.min_count
        ));
        for candidate in &summary.candidates {
            let kind = match candidate.kind {
                CandidateKind::Bash => "read-only Bash",
                CandidateKind::Mcp => "read-only MCP",
            };
            out.push_str(&format!(
                "- `{}` — {} call(s), {}\n",
                candidate.rule, candidate.count, kind
            ));
        }
        out.push_str(&format!(
            "\nNothing has been written. Run `/fewer-permission-prompts --yes` to add \
             {} rule(s) to `{}`.\n",
            summary.candidates.len(),
            settings_rules::local_settings_path(cwd).display()
        ));
        out.push_str(
            "Prefix rules cover future arguments; review them before applying. \
             `ask`/`deny` rules keep precedence.\n",
        );
    }

    out.push_str("\n## What was skipped\n");
    out.push_str(&format!(
        "- Bash: {} seen, {} not read-only or not safely generalizable\n",
        summary.bash_seen, summary.bash_skipped
    ));
    out.push_str(&format!(
        "- MCP: {} seen, {} not recognized as read-only\n",
        summary.mcp_seen, summary.mcp_skipped
    ));
    if summary.already_allowed > 0 {
        out.push_str(&format!(
            "- {} call(s) already covered by an allow rule\n",
            summary.already_allowed
        ));
    }
    if summary.blocked_by_rule > 0 {
        out.push_str(&format!(
            "- {} call(s) covered by an `ask` or `deny` rule; not suggested\n",
            summary.blocked_by_rule
        ));
    }
    if scan.malformed > 0 {
        out.push_str(&format!(
            "- {} malformed source line/file(s) were ignored\n",
            scan.malformed
        ));
    }
    out
}

fn render_applied(cwd: &Path, summary: &Summary, added: &[String], days: i64) -> String {
    let window = if days == 0 {
        "all recorded history".to_string()
    } else {
        format!("the last {} day(s)", days)
    };
    let mut out = format!(
        "# Permission allowlist updated\n\nScanned {} tool call(s) from {}.\n",
        summary.scanned_tool_calls, window
    );
    if added.is_empty() {
        out.push_str("\nNo new rule needed to be written.\n");
    } else {
        out.push_str(&format!(
            "\nAdded {} rule(s) to `{}`:\n",
            added.len(),
            settings_rules::local_settings_path(cwd).display()
        ));
        for rule in added {
            out.push_str(&format!("- `{}`\n", rule));
        }
        out.push_str("\nPermission rules were reloaded for the running session.\n");
    }
    if summary.blocked_by_rule > 0 || summary.bash_skipped > 0 || summary.mcp_skipped > 0 {
        out.push_str(&format!(
            "\nSkipped: {} Bash call(s), {} MCP call(s), {} call(s) protected by ask/deny.\n",
            summary.bash_skipped, summary.mcp_skipped, summary.blocked_by_rule
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::activity::{ActivityScan, ToolCallRecord};

    fn call(tool: &str, arguments: Value) -> ToolCallRecord {
        ToolCallRecord {
            tool: tool.to_string(),
            arguments,
            timestamp: None,
        }
    }

    fn bash(command: &str) -> ToolCallRecord {
        call("Bash", json!({ "command": command }))
    }

    fn mcp(name: &str) -> ToolCallRecord {
        call(name, json!({}))
    }

    fn with_rule(
        mut permissions: SettingsPermissions,
        bucket: RuleVerdict,
        raw: &str,
    ) -> SettingsPermissions {
        let rule = PermissionRuleSpec::parse(raw).unwrap();
        match bucket {
            RuleVerdict::Allow => permissions.allow.push(rule),
            RuleVerdict::Ask => permissions.ask.push(rule),
            RuleVerdict::Deny => permissions.deny.push(rule),
        }
        permissions
    }

    #[test]
    fn parses_apply_and_window_flags() {
        let options = parse_options(&[
            "--days=7".to_string(),
            "--min-count".to_string(),
            "3".to_string(),
            "--yes".to_string(),
        ])
        .unwrap();
        assert_eq!(
            options,
            Options {
                days: 7,
                min_count: 3,
                apply: true,
            }
        );
        assert_eq!(parse_options(&[]).unwrap().days, DEFAULT_DAYS);
        assert!(parse_options(&["--dry-run".to_string(), "--yes".to_string()]).is_err());
        assert!(parse_options(&["--days".to_string()]).is_err());
        assert!(parse_options(&["--min-count=0".to_string()]).is_err());
        assert!(parse_options(&["--wat".to_string()]).is_err());
    }

    #[test]
    fn classifies_common_read_only_bash_commands() {
        assert_eq!(
            classify_bash_command("git status --short"),
            Some("Bash(git status:*)".to_string())
        );
        assert_eq!(
            classify_bash_command("git log --oneline -5"),
            Some("Bash(git log:*)".to_string())
        );
        assert_eq!(
            classify_bash_command("ls -la"),
            Some("Bash(ls:*)".to_string())
        );
        assert_eq!(
            classify_bash_command("cat 'file with spaces.txt'"),
            Some("Bash(cat:*)".to_string())
        );
        assert_eq!(
            classify_bash_command("echo 'a > b'"),
            Some("Bash(echo:*)".to_string()),
            "a literal redirection inside quotes is not shell syntax"
        );
    }

    #[test]
    fn rejects_compound_or_mutating_bash_commands() {
        for command in [
            "ls && rm -rf src",
            "echo hi > file",
            "cat $(secret)",
            "cat `secret`",
            "git branch feature",
            "git remote add origin url",
            "git diff --output=patch.diff",
            "rm -rf src",
            "find . -delete",
            "python script.py",
            "bash -c 'ls'",
            "env FOO=bar ls",
            "ls 'unterminated",
            "date -s '2026-01-01'",
            "hostname evil",
            "file -C",
        ] {
            assert_eq!(
                classify_bash_command(command),
                None,
                "expected `{}` to be rejected",
                command
            );
        }
    }

    #[test]
    fn classifies_only_read_only_mcp_tools() {
        assert_eq!(
            classify_mcp_tool("mcp__github__list_issues"),
            Some("mcp__github__list_issues".to_string())
        );
        assert_eq!(
            classify_mcp_tool("mcp__filesystem__read_file"),
            Some("mcp__filesystem__read_file".to_string())
        );
        assert_eq!(
            classify_mcp_tool("mcp__browser__take_snapshot"),
            Some("mcp__browser__take_snapshot".to_string())
        );
        assert_eq!(classify_mcp_tool("mcp__github__create_issue"), None);
        assert_eq!(classify_mcp_tool("mcp__github__update_issue"), None);
        assert_eq!(classify_mcp_tool("mcp__github__createIssue"), None);
        assert_eq!(
            classify_mcp_tool("mcp__search__search_and_replace"),
            None,
            "a read-only prefix must not hide a mutating verb"
        );
        assert_eq!(
            classify_mcp_tool("mcp__vault__get_and_delete_secret"),
            None,
            "the mutating verb wins even when the tool starts with get_"
        );
        assert_eq!(
            classify_mcp_tool("mcp__exa__search"),
            Some("mcp__exa__search".to_string())
        );
        assert_eq!(classify_mcp_tool("Bash"), None);
    }

    #[test]
    fn summarize_filters_short_unsafe_allowed_and_blocked_entries() {
        let scan = ActivityScan {
            sources: 1,
            tool_calls: vec![
                bash("git status"),
                bash("git status --short"),
                bash("ls -la"),
                bash("ls src"),
                bash("rm -rf target"),
                mcp("mcp__github__list_issues"),
                mcp("mcp__github__list_issues"),
                mcp("mcp__github__create_issue"),
                mcp("mcp__github__create_issue"),
            ],
            ..ActivityScan::default()
        };
        let existing = with_rule(
            with_rule(
                SettingsPermissions::default(),
                RuleVerdict::Allow,
                "Bash(git status:*)",
            ),
            RuleVerdict::Deny,
            "mcp__github__list_issues",
        );

        let summary = summarize(&scan, &existing, 2);
        assert_eq!(summary.scanned_tool_calls, 9);
        assert_eq!(summary.bash_seen, 5);
        assert_eq!(summary.bash_skipped, 1);
        assert_eq!(summary.mcp_seen, 4);
        assert_eq!(summary.mcp_skipped, 2);
        assert_eq!(summary.already_allowed, 2);
        assert_eq!(summary.blocked_by_rule, 2);
        assert_eq!(
            summary.candidates,
            vec![Candidate {
                rule: "Bash(ls:*)".to_string(),
                count: 2,
                kind: CandidateKind::Bash,
            }]
        );
    }

    #[test]
    fn summarize_honors_min_count_and_sorts_by_frequency() {
        let scan = ActivityScan {
            sources: 1,
            tool_calls: vec![
                bash("ls"),
                bash("ls"),
                bash("pwd"),
                bash("pwd"),
                bash("pwd"),
                bash("git status"),
            ],
            ..ActivityScan::default()
        };
        let summary = summarize(&scan, &SettingsPermissions::default(), 2);
        assert_eq!(
            summary
                .candidates
                .iter()
                .map(|candidate| (candidate.rule.as_str(), candidate.count))
                .collect::<Vec<_>>(),
            vec![("Bash(pwd:*)", 3), ("Bash(ls:*)", 2)]
        );
    }

    #[test]
    fn preview_explains_preview_mode_and_skips() {
        let scan = ActivityScan {
            sources: 2,
            tool_calls: vec![bash("ls"), bash("ls"), bash("rm -rf x")],
            ..ActivityScan::default()
        };
        let summary = summarize(&scan, &SettingsPermissions::default(), 2);
        let options = Options {
            days: 30,
            min_count: 2,
            apply: false,
        };
        let rendered = render_preview(Path::new("/repo"), &scan, &summary, &options);
        assert!(rendered.contains("Bash(ls:*)"));
        assert!(rendered.contains("--yes"));
        assert!(rendered.contains("Bash: 3 seen, 1 not read-only"));
        assert!(rendered.contains("Nothing has been written"));
    }

    #[test]
    fn preview_reports_missing_history_without_dividing_by_zero() {
        let scan = ActivityScan::default();
        let summary = summarize(&scan, &SettingsPermissions::default(), 2);
        let rendered = render_preview(
            Path::new("/repo"),
            &scan,
            &summary,
            &Options {
                days: 0,
                min_count: 2,
                apply: false,
            },
        );
        assert!(rendered.contains("No local tool-call history"));
        assert!(rendered.contains("STAR_TRANSCRIPT=1"));
    }

    #[test]
    fn apply_writes_only_the_local_settings_layer() {
        let dir = std::env::temp_dir().join(format!(
            "starcode-fewer-permissions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".star")).unwrap();

        let candidates = vec![Candidate {
            rule: "Bash(ls:*)".to_string(),
            count: 3,
            kind: CandidateKind::Bash,
        }];
        let added = apply_candidates(&dir, &candidates).unwrap();
        assert_eq!(added, vec!["Bash(ls:*)".to_string()]);

        let path = settings_rules::local_settings_path(&dir);
        let raw = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["permissions"]["allow"], json!(["Bash(ls:*)"]));
        assert!(
            !dir.join(".star/settings.json").exists(),
            "shared project settings must not be modified"
        );

        // The same candidate is already present, so a second apply is a no-op.
        let added_again = apply_candidates(&dir, &candidates).unwrap();
        assert!(added_again.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applied_report_mentions_reload_and_added_rules() {
        let summary = Summary {
            scanned_tool_calls: 5,
            ..Summary::default()
        };
        let rendered = render_applied(
            Path::new("/repo"),
            &summary,
            &["Bash(ls:*)".to_string()],
            30,
        );
        assert!(rendered.contains("Bash(ls:*)"));
        assert!(rendered.contains("reloaded"));
        assert!(rendered.contains("last 30 day(s)"));
    }
}
