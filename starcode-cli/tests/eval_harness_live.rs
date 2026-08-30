//! Eval harness integration tests.
//!
//! These run against the on-disk `eval/` task files and fixtures.
//! Run from the crate root (`starcode-cli-main/starcode-cli`):
//!
//! ```bash
//! cargo test --test eval_harness_live
//! ```

use std::path::PathBuf;

fn eval_dir() -> PathBuf {
    std::env::current_dir()
        .expect("cwd")
        .join("eval")
}

#[tokio::test]
async fn live_tasks_skip_without_api_key() {
    let tasks = eval_dir().join("live-tasks.json");
    let out = std::env::temp_dir().join("starcode-eval-live-test.json");

    let report = starcode_cli::agent::eval_harness::run_eval(&tasks, &out, 1)
        .await
        .expect("run_eval should succeed");

    let live = report.live_results.expect("live_results present");
    assert_eq!(live.len(), 10, "live-tasks.json should define 10 tasks");
    // Without STAR_API_KEY every live task is skipped, not crashed.
    assert!(
        live.iter().all(|r| !r.executed),
        "no live task should execute without STAR_API_KEY"
    );
    assert!(
        live.iter().all(|r| r.skip_reason.is_some()),
        "skipped tasks carry a skip_reason"
    );
}

#[tokio::test]
async fn markdown_report_renders_live_section() {
    let tasks = eval_dir().join("live-tasks.json");
    let out = std::env::temp_dir().join("starcode-eval-live-test.json");

    let report = starcode_cli::agent::eval_harness::run_eval(&tasks, &out, 1)
        .await
        .expect("run_eval should succeed");

    let md = starcode_cli::agent::eval_harness::eval_report_to_markdown(&report);
    assert!(md.contains("# Eval Report"));
    assert!(md.contains("## Live tasks"));
    assert!(md.contains("live_fix_discount"));
}

#[tokio::test]
async fn mechanism_tasks_run_and_report() {
    let tasks = eval_dir().join("tasks.json");
    let out = std::env::temp_dir().join("starcode-eval-mech-test.json");

    let report = starcode_cli::agent::eval_harness::run_eval(&tasks, &out, 1)
        .await
        .expect("run_eval should succeed");

    for r in &report.results {
        if !r.passed {
            println!("FAILED {} rules={:?}", r.id, r.outcome.failed_rules);
        }
    }
    println!("summary: {} total, {} passed", report.summary.total, report.summary.passed);

    assert_eq!(report.summary.total, 16, "tasks.json should define 16 tasks");
    assert!(report.summary.passed >= 14, "mechanism layer should mostly pass");
    assert!(report.results.len() == 16);
}
