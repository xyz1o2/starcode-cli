use crate::agent::eval_harness::BaselineChange;
use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;
use std::path::PathBuf;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut tasks_path: Option<PathBuf> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut report_md_path: Option<PathBuf> = None;
    let mut trials: usize = 1;
    let mut save_baseline_path: Option<PathBuf> = None;
    let mut baseline_path: Option<PathBuf> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--tasks" => {
                if let Some(value) = args.get(i + 1) {
                    tasks_path = Some(PathBuf::from(value));
                    i += 2;
                    continue;
                } else {
                    return Err("--tasks requires a path".to_string());
                }
            }
            "--out" => {
                if let Some(value) = args.get(i + 1) {
                    output_path = Some(PathBuf::from(value));
                    i += 2;
                    continue;
                } else {
                    return Err("--out requires a path".to_string());
                }
            }
            "--report-md" => {
                if let Some(value) = args.get(i + 1).filter(|v| !v.starts_with("--")) {
                    report_md_path = Some(PathBuf::from(value));
                    i += 2;
                } else {
                    return Err("--report-md requires a path".to_string());
                }
                continue;
            }
            "--trials" => {
                if let Some(value) = args.get(i + 1) {
                    trials = value
                        .parse::<usize>()
                        .map_err(|_| format!("--trials must be a positive integer, got: {}", value))?;
                    if trials == 0 {
                        return Err("--trials must be at least 1".to_string());
                    }
                    i += 2;
                    continue;
                } else {
                    return Err("--trials requires a number (e.g. --trials 3)".to_string());
                }
            }
            "--save-baseline" => {
                if let Some(value) = args.get(i + 1).filter(|v| !v.starts_with("--")) {
                    save_baseline_path = Some(PathBuf::from(value));
                    i += 2;
                } else {
                    save_baseline_path = Some(cwd.join(".star").join("eval-baseline.json"));
                    i += 1;
                }
                continue;
            }
            "--baseline" => {
                if let Some(value) = args.get(i + 1) {
                    baseline_path = Some(PathBuf::from(value));
                    i += 2;
                    continue;
                } else {
                    return Err("--baseline requires a path".to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    let tasks_path = tasks_path.unwrap_or_else(|| cwd.join("eval").join("tasks.json"));
    let output_path = output_path.unwrap_or_else(|| cwd.join(".star").join("eval-results.json"));

    let report =
        crate::agent::eval_harness::run_eval(&tasks_path, &output_path, trials).await?;
    let summary = &report.summary;

    let mut message = format!(
        "Eval results: {} total, {} passed, {} failed ({:.1}% pass)",
        summary.total,
        summary.passed,
        summary.failed,
        summary.pass_rate * 100.0,
    );

    // Live (P3) 结果摘要
    if let Some(live) = &report.live_results {
        let passed = live.iter().filter(|r| r.passed).count();
        let skipped = live.iter().filter(|r| !r.executed).count();
        message.push_str(&format!(
            ". Live: {}/{} passed ({} skipped)",
            passed,
            live.len(),
            skipped
        ));
        let failed_live: Vec<&str> = live
            .iter()
            .filter(|r| r.executed && !r.passed)
            .map(|r| r.id.as_str())
            .collect();
        if !failed_live.is_empty() {
            message.push_str(&format!(
                ". Live failed: {}",
                failed_live.join(", ")
            ));
        }
    }

    // 生成 markdown 报告
    if let Some(md_path) = &report_md_path {
        let md = crate::agent::eval_harness::eval_report_to_markdown(&report);
        if let Some(parent) = md_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(md_path, md).map_err(|e| format!("Failed to write report: {e}"))?;
        message.push_str(&format!(". Markdown report: {}", md_path.display()));
    }

    // 多轮统计
    if summary.trials_per_task > 1 {
        message.push_str(&format!(
            ", pass@1={:.1}%, pass@{k}={pass_k:.1}%, pass^{k}={pass_all:.1}%",
            summary.pass_at_1 * 100.0,
            k = summary.trials_per_task,
            pass_k = summary.pass_at_3 * 100.0,
            pass_all = summary.pass_all * 100.0,
        ));
    }
    message.push_str(&format!(". Report: {}", output_path.display()));

    if summary.failed > 0 {
        let failed_ids: Vec<String> = report
            .results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| {
                if r.outcome.failed_rules.is_empty() {
                    format!("{}#{}", r.id, r.trial_num)
                } else {
                    format!("{}#{}[{}]", r.id, r.trial_num, r.outcome.failed_rules.join(","))
                }
            })
            .collect();
        if !failed_ids.is_empty() {
            message.push_str(&format!(". Failed: {}", failed_ids.join(", ")));
        }
    }

    // 对比基线
    if let Some(ref baseline_path) = baseline_path {
        match crate::agent::eval_harness::compare_baseline(&report, baseline_path).await {
            Ok(deltas) if deltas.is_empty() => {
                message.push_str(". Baseline: no regressions.");
            }
            Ok(deltas) => {
                let regressions: Vec<_> = deltas
                    .iter()
                    .filter_map(|d| match &d.change {
                        BaselineChange::Regression { .. } => Some(format!("{}[REGRESSION]", d.task_id)),
                        BaselineChange::Removed => Some(format!("{}[REMOVED]", d.task_id)),
                        BaselineChange::New => Some(format!("{}[NEW]", d.task_id)),
                    })
                    .collect();
                message.push_str(&format!(
                    ". Baseline: {} changes ({}).",
                    deltas.len(),
                    regressions.join(", ")
                ));
            }
            Err(e) => {
                message.push_str(&format!(". Baseline compare failed: {}", e));
            }
        }
    }

    // 保存基线
    if let Some(ref baseline_path) = save_baseline_path {
        match crate::agent::eval_harness::save_baseline(&report, baseline_path).await {
            Ok(_) => {
                message.push_str(&format!(". Baseline saved: {}", baseline_path.display()));
            }
            Err(e) => {
                message.push_str(&format!(". Baseline save failed: {}", e));
            }
        }
    }

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(message).with_streaming(false));

    Ok(())
}
