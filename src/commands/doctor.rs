use crate::commands::execution::{CommandContext, CommandResult};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;

pub async fn run(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut report = String::from("# 🩺 StarCode Doctor Report\n\n");
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());

    // 1. Check Project Context
    let context_file = crate::utils::project_context::find_project_context_file(&cwd);
    if let Some(path) = context_file {
        report.push_str(&format!(
            "✅ Project Context: Found at `{}`\n",
            path.display()
        ));
    } else {
        report.push_str(
            "⚠️ Project Context: Missing `STAR.md` / `STARCODE.md` (Run `/init` to create)\n",
        );
    }

    // 2. Check API Key
    // We can't easily access the private API key here without passing it down,
    // but we can check if the environment variable is set.
    if std::env::var("STAR_API_KEY").is_ok() {
        report.push_str("✅ API Key: STAR_API_KEY is set\n");
    } else {
        // It might be in settings, but let's just warn about env var
        report.push_str("ℹ️ API Key: STAR_API_KEY env var not set (might be in settings)\n");
    }

    // 3. Check Essential Tools
    let tools = [
        ("git", "--version"),
        ("cargo", "--version"),
        ("node", "--version"),
        ("npm", "--version"),
        ("python", "--version"),
    ];

    report.push_str("\n## Toolchain Status\n");

    for (tool, arg) in tools {
        match Command::new(tool).arg(arg).output() {
            Ok(output) => {
                if output.status.success() {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    report.push_str(&format!("✅ {}: {}\n", tool, version));
                } else {
                    report.push_str(&format!("❌ {}: Error executing command\n", tool));
                }
            }
            Err(_) => {
                report.push_str(&format!("⚪ {}: Not found\n", tool));
            }
        }
    }

    // 4. Check Permissions Mode
    // We need to access config if possible, but CommandContext doesn't expose Config directly yet.
    // However, we can check the process arguments or global state if we had access.
    // For now, skip.
    report.push_str("\n## Harness Health\n");

    let eval_path = cwd.join(".star").join("eval-results.json");
    match load_eval_summary(&eval_path) {
        Ok(Some(summary)) => {
            report.push_str(&format!(
                "✅ Eval Harness: {:.1}% pass ({} total, {} failed) from `{}`\n",
                summary.pass_rate * 100.0,
                summary.total,
                summary.failed,
                summary.path.display()
            ));
            report.push_str(&format!("   Last Eval Run: {}\n", summary.run_at));
            report.push_str(&format!("   Schema: v{}\n", summary.schema_version));
            if !summary.failed_rule_counts.is_empty() {
                report.push_str(&format!(
                    "   Failed Rules: {}\n",
                    summary
                        .failed_rule_counts
                        .iter()
                        .map(|(rule, count)| format!("{}={}", rule, count))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(None) => {
            report.push_str("⚠️ Eval Harness: Missing `.star/eval-results.json` (Run `/eval`)\n");
        }
        Err(err) => {
            report.push_str(&format!(
                "❌ Eval Harness: Failed to read report ({})\n",
                err
            ));
        }
    }

    let transcript_path = ctx
        .state
        .transcript_path
        .clone()
        .unwrap_or_else(|| crate::ui::utils::transcript::default_transcript_path(&cwd));
    if ctx.state.transcript_enabled {
        report.push_str(&format!(
            "✅ Transcript Trace: Enabled at `{}`\n",
            transcript_path.display()
        ));
    } else {
        report.push_str(&format!(
            "⚠️ Transcript Trace: Disabled (expected path `{}`)\n",
            transcript_path.display()
        ));
    }

    match load_transcript_summary(&transcript_path) {
        Ok(Some(summary)) => {
            report.push_str(&format!(
                "✅ Latest Trace Run: `{}` with {} events ({} decision traces)\n",
                summary.latest_run_id, summary.events_in_run, summary.decision_events
            ));
            report.push_str(&format!(
                "   Last Event: {} at {}\n",
                summary.last_event, summary.last_ts
            ));
            if !summary.decision_counts.is_empty() {
                report.push_str(&format!(
                    "   Decision Mix: {}\n",
                    summary
                        .decision_counts
                        .iter()
                        .map(|(event, count)| format!("{}={}", event, count))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(None) => {
            report.push_str("ℹ️ Transcript Trace: No transcript events recorded yet\n");
        }
        Err(err) => {
            report.push_str(&format!(
                "❌ Transcript Trace: Failed to parse transcript ({})\n",
                err
            ));
        }
    }

    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(report).with_streaming(false));

    Ok(())
}

#[derive(Debug, PartialEq)]
struct EvalDoctorSummary {
    path: std::path::PathBuf,
    schema_version: u32,
    run_at: String,
    total: usize,
    failed: usize,
    pass_rate: f64,
    failed_rule_counts: BTreeMap<String, usize>,
}

fn load_eval_summary(path: &Path) -> Result<Option<EvalDoctorSummary>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {}", path.display(), e))?;

    let failed_rule_counts = value["summary"]["failed_rule_counts"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_u64().map(|count| (key.clone(), count as usize))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(Some(EvalDoctorSummary {
        path: path.to_path_buf(),
        schema_version: value["schema_version"].as_u64().unwrap_or(0) as u32,
        run_at: value["run_at"].as_str().unwrap_or("unknown").to_string(),
        total: value["summary"]["total"].as_u64().unwrap_or(0) as usize,
        failed: value["summary"]["failed"].as_u64().unwrap_or(0) as usize,
        pass_rate: value["summary"]["pass_rate"].as_f64().unwrap_or(0.0),
        failed_rule_counts,
    }))
}

#[derive(Debug, PartialEq)]
struct TranscriptDoctorSummary {
    latest_run_id: String,
    events_in_run: usize,
    decision_events: usize,
    last_event: String,
    last_ts: String,
    decision_counts: BTreeMap<String, usize>,
}

fn load_transcript_summary(path: &Path) -> Result<Option<TranscriptDoctorSummary>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let file = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let reader = BufReader::new(file);
    let mut tail = VecDeque::new();
    const MAX_TAIL_LINES: usize = 400;

    for line in reader.lines() {
        let line = line.map_err(|e| format!("read {}: {}", path.display(), e))?;
        if line.trim().is_empty() {
            continue;
        }
        if tail.len() == MAX_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    if tail.is_empty() {
        return Ok(None);
    }

    let parsed = tail
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    let Some(latest_run_id) = parsed
        .iter()
        .rev()
        .find_map(|value| value["run_id"].as_str().map(|run_id| run_id.to_string()))
    else {
        return Ok(None);
    };

    let mut events_in_run = 0usize;
    let mut decision_events = 0usize;
    let mut decision_counts = BTreeMap::new();
    let mut last_event = "unknown".to_string();
    let mut last_ts = "unknown".to_string();

    for value in parsed {
        if value["run_id"].as_str() != Some(latest_run_id.as_str()) {
            continue;
        }

        events_in_run += 1;
        if let Some(event) = value["event"].as_str() {
            last_event = event.to_string();
            if is_decision_trace_event(event) {
                decision_events += 1;
                *decision_counts.entry(event.to_string()).or_insert(0) += 1;
            }
        }
        if let Some(ts) = value["ts"].as_str() {
            last_ts = ts.to_string();
        }
    }

    Ok(Some(TranscriptDoctorSummary {
        latest_run_id,
        events_in_run,
        decision_events,
        last_event,
        last_ts,
        decision_counts,
    }))
}

fn is_decision_trace_event(event: &str) -> bool {
    matches!(
        event,
        "routing_context_resolved"
            | "dynamic_context_resolved"
            | "auto_plan_decision"
            | "context_compression"
            | "tool_shortlist_selected"
            | "verification_required"
            | "auto_verification_injected"
    )
}
