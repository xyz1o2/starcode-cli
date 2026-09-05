use crate::agent::eval_behaviors::{BehaviorExpectation, BehaviorFailure, FinishSignal};
use crate::agent::eval_runner::AgentRunResult;
use crate::agent::policies::automation::{
    auto_plan_enabled, auto_plan_max_history, detect_skip_verification_pattern,
    evaluate_auto_plan_gate,
};
use crate::agent::router::Router;
use crate::core::routing::{RequestComplexity, RoutingEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const EVAL_REPORT_SCHEMA_VERSION: u32 = 4;
const EVAL_HARNESS_NAME: &str = "routing_decision_harness";
const DEFAULT_TRIALS: usize = 1;

#[derive(Debug, Deserialize)]
struct EvalTaskFile {
    version: u32,
    tasks: Vec<EvalTask>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct EvalTask {
    id: String,
    prompt: String,
    #[serde(default)]
    history_len: Option<usize>,
    expected: EvalExpected,
    /// P1: behavior expectation (optional)
    #[serde(default)]
    behavior: Option<BehaviorExpectation>,
    /// P2: E2E task config (optional)
    #[serde(default)]
    e2e: Option<E2ETaskConfig>,
    /// P3: live task config (optional) — runs a real LLM against a fixture
    #[serde(default)]
    live: Option<LiveTaskConfig>,
}

/// P3: Live task config — runs a real LLM agent against an isolated fixture
/// project and verifies the outcome with shell commands.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LiveTaskConfig {
    /// Fixture directory name under `eval/fixtures/` (e.g. "rust-lab")
    pub fixture: String,
    /// Verification commands run in the fixture copy after the agent finishes.
    /// A command is considered passed when it exits with code 0.
    #[serde(default)]
    pub verify: Vec<String>,
    /// Agent max session turns (injected into Config.max_session_turns)
    #[serde(default = "default_live_max_turns")]
    pub max_turns: usize,
    /// Overall timeout seconds for the whole live run (incl. verify)
    #[serde(default = "default_live_timeout")]
    pub timeout_secs: u64,
    /// Optional model override (defaults to STAR_MODEL)
    #[serde(default)]
    pub model: Option<String>,
}

fn default_live_max_turns() -> usize {
    25
}

fn default_live_timeout() -> u64 {
    420
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct EvalExpected {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub complexity: Option<String>,
    #[serde(default)]
    pub auto_plan: Option<bool>,
    #[serde(default)]
    pub skip_verification: Option<bool>,
    /// Expected model selection (e.g. "fast_model", "default", "cheap_model")
    #[serde(default)]
    pub model: Option<String>,
    /// Expected plan_mode trigger
    #[serde(default)]
    pub plan_mode: Option<bool>,
    /// Expected triggered skill name
    #[serde(default)]
    pub skill: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EvalActual {
    pub kind: String,
    pub complexity: String,
    pub auto_plan: bool,
    pub skip_verification: bool,
    pub model: String,
    pub plan_mode: bool,
    pub skill: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EvalFailure {
    pub rule: String,
    pub expected: Value,
    pub actual: Value,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct EvalOutcome {
    pub passed: bool,
    pub actual: EvalActual,
    pub failures: Vec<EvalFailure>,
    pub failed_rules: Vec<String>,
    /// P1: behavior evaluation result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior: Option<BehaviorEvalResult>,
}

// Keep the EvalResult definition above...
// E2E results at EvalReport level, not per trial

#[derive(Debug, Serialize)]
pub struct EvalTraceStep {
    pub step: String,
    pub detail: String,
    pub data: Value,
}

#[derive(Debug, Serialize)]
pub struct EvalTrial {
    pub id: String,
    pub trial_num: usize,
    pub trials_total: usize,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub trace: Vec<EvalTraceStep>,
}

#[derive(Debug, Serialize)]
pub struct EvalTaskSnapshot {
    pub id: String,
    pub prompt: String,
    pub history_len: usize,
    pub expected: EvalExpected,
}

#[derive(Debug, Serialize)]
pub struct EvalResult {
    pub id: String,
    pub passed: bool,
    pub trial_num: usize,
    pub trials_total: usize,
    pub task: EvalTaskSnapshot,
    pub trial: EvalTrial,
    pub outcome: EvalOutcome,
}

#[derive(Debug, Serialize)]
pub struct EvalSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub pass_rate: f64,
    pub failed_rule_counts: BTreeMap<String, usize>,
    /// Multi-trial stats: run count per task
    pub trials_per_task: usize,
    /// pass@1: at least 1 pass rate (single trial)
    pub pass_at_1: f64,
    /// pass@3: at least 1 pass in 3 trials
    pub pass_at_3: f64,
    /// pass^k: all trials pass rate
    pub pass_all: f64,
}

#[derive(Debug, Serialize)]
pub struct EvalEnv {
    pub auto_plan_enabled: bool,
    pub auto_plan_max_history: usize,
}

#[derive(Debug, Serialize)]
pub struct EvalHarness {
    pub name: String,
    pub task_file_version: u32,
    pub trace_format: String,
}

#[derive(Debug, Serialize)]
pub struct EvalArtifacts {
    pub tasks_path: String,
    pub report_path: String,
}

#[derive(Debug, Serialize)]
pub struct EvalReport {
    pub schema_version: u32,
    pub run_id: String,
    pub run_at: String,
    pub harness: EvalHarness,
    pub artifacts: EvalArtifacts,
    pub summary: EvalSummary,
    pub env: EvalEnv,
    pub results: Vec<EvalResult>,
    /// P2: E2E evaluation results (one per task with e2e config)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e2e_results: Option<Vec<E2EResult>>,
    /// P3: live evaluation results (real LLM runs, one per task with live config)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_results: Option<Vec<LiveEvalResult>>,
}

// ── P2: E2E task config ──────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct E2ETaskConfig {
    /// Builder agent max turns
    #[serde(default = "default_e2e_max_turns")]
    pub max_turns: usize,
    /// Builder agent timeout in seconds
    #[serde(default = "default_e2e_timeout")]
    pub timeout_secs: u64,
    /// Evaluator acceptance criteria list
    #[serde(default)]
    pub criteria: Vec<E2ECriterion>,
    /// Expected files to create
    #[serde(default)]
    pub files_created: Option<Vec<String>>,
    /// Expected files to modify
    #[serde(default)]
    pub files_modified: Option<Vec<String>>,
}

fn default_e2e_max_turns() -> usize {
    30
}
fn default_e2e_timeout() -> u64 {
    300
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct E2ECriterion {
    pub name: String,
    pub check: String,
}

/// E2E evaluation result (P2).
/// Note: actual agent runs require a real LLM + API key environment.
/// Current implementation provides skeleton — task config, result slots, report integration.
/// Full E2E agent launch will be connected when the environment is ready.
#[derive(Debug, Serialize, Clone)]
pub struct E2EResult {
    /// Whether E2E evaluation was executed
    pub executed: bool,
    /// Builder agent run result
    pub builder: Option<AgentRunResult>,
    /// Evaluator verdict (reserved)
    pub evaluator_verdict: Option<String>,
    /// Acceptance criteria results
    pub criteria_results: Vec<CriterionResult>,
    /// Failed rules
    pub failed_rules: Vec<String>,
    /// Overall passed
    pub passed: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct CriterionResult {
    pub name: String,
    pub check: String,
    pub passed: bool,
    pub detail: String,
}

// ── P3: Live eval result ─────────────────────────────────────────

/// Result of a single live task (real LLM run against a fixture copy).
#[derive(Debug, Serialize, Clone)]
pub struct LiveEvalResult {
    pub id: String,
    pub executed: bool,
    pub skip_reason: Option<String>,
    pub passed: bool,
    pub failed_rules: Vec<String>,
    pub finish_signal: Option<String>,
    pub total_turns: usize,
    pub tool_calls: usize,
    pub duration_ms: u64,
    /// Total LLM tokens consumed (prompt + completion, summed across turns)
    #[serde(default)]
    pub total_tokens: u64,
    pub tool_call_summary: Vec<ToolCallSummary>,
    pub verify: Vec<VerifyResult>,
    pub final_message: String,
    pub agent_error: Option<String>,
    /// Behavior-rule failures (e.g. forbidden tool used, missing tool)
    pub behavior_failures: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ToolCallSummary {
    pub tool: String,
    pub success: bool,
    pub args: Value,
}

#[derive(Debug, Serialize, Clone)]
pub struct VerifyResult {
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub output_tail: String,
}

// ── Result extension ──────────────────────────────────────────────

/// EvalResult extension: behavior evaluation result
#[derive(Debug, Serialize, Clone)]
pub struct BehaviorEvalResult {
    pub executed: bool,
    pub passed: bool,
    pub finish_signal: Option<FinishSignal>,
    pub failures: Vec<BehaviorFailure>,
    pub failed_rules: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalBaseline {
    pub created_at: String,
    pub git_commit: String,
    pub schema_version: u32,
    pub tasks: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct BaselineDelta {
    pub task_id: String,
    pub change: BaselineChange,
}

#[derive(Debug, Serialize)]
pub enum BaselineChange {
    Regression {
        expected: serde_json::Value,
        actual: serde_json::Value,
    },
    /// Task existed in baseline but not in current run
    Removed,
    /// Task existed in current run but not in baseline
    New,
}

pub async fn run_eval(
    tasks_path: &Path,
    output_path: &Path,
    trials: usize,
) -> Result<EvalReport, String> {
    let trials = if trials == 0 { DEFAULT_TRIALS } else { trials };
    let tasks_content = tokio::fs::read_to_string(tasks_path)
        .await
        .map_err(|e| format!("Failed to read tasks file {}: {}", tasks_path.display(), e))?;
    let task_file: EvalTaskFile = serde_json::from_str(&tasks_content)
        .map_err(|e| format!("Failed to parse tasks file: {}", e))?;

    let run_at = chrono::Utc::now().to_rfc3339();
    let run_id = format!("eval-{}", chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"));

    // Separate L1 (routing), L2 (behavior) and P3 (live) tasks
    let mut routing_results = Vec::new();
    let mut e2e_results = Vec::new();
    let mut live_results = Vec::new();

    for task in task_file.tasks {
        let has_e2e = task.e2e.is_some();
        let has_live = task.live.is_some();

        if has_live {
            // P3: live task — real LLM run against an isolated fixture copy
            let live_result = evaluate_live_task(&task).await;
            live_results.push(live_result);
        } else if has_e2e {
            // P2: E2E task — generate skeleton result, don't run agent here (needs API key and isolated environment)
            let e2e_result = evaluate_e2e_task_stub(&task, &run_id);
            e2e_results.push(e2e_result);
        } else {
            // L1 + L2: routing decision + behavior evaluation
            for trial_num in 1..=trials {
                let result = evaluate_task_case(task.clone(), &run_id, trial_num, trials);
                routing_results.push(result);
            }
        }
    }

    let summary = build_summary(&routing_results, trials);

    let e2e_slot = if e2e_results.is_empty() {
        None
    } else {
        Some(e2e_results)
    };

    let live_slot = if live_results.is_empty() {
        None
    } else {
        Some(live_results)
    };

    let report = EvalReport {
        schema_version: EVAL_REPORT_SCHEMA_VERSION,
        run_id: run_id.clone(),
        run_at,
        harness: EvalHarness {
            name: EVAL_HARNESS_NAME.to_string(),
            task_file_version: task_file.version,
            trace_format: "inline".to_string(),
        },
        artifacts: EvalArtifacts {
            tasks_path: tasks_path.display().to_string(),
            report_path: output_path.display().to_string(),
        },
        summary,
        env: EvalEnv {
            auto_plan_enabled: auto_plan_enabled(),
            auto_plan_max_history: auto_plan_max_history(),
        },
        results: routing_results,
        e2e_results: e2e_slot,
        live_results: live_slot,
    };

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("Failed to serialize report: {}", e))?;
    tokio::fs::write(output_path, report_json)
        .await
        .map_err(|e| format!("Failed to write report: {}", e))?;

    Ok(report)
}

fn evaluate_task_case(
    task: EvalTask,
    run_id: &str,
    trial_num: usize,
    trials_total: usize,
) -> EvalResult {
    let started_at = chrono::Utc::now().to_rfc3339();
    let started = Instant::now();
    let history_len = task.history_len.unwrap_or(0);
    let mut trace = Vec::new();

    let complexity = Router::classify(&task.prompt, history_len);
    trace.push(EvalTraceStep {
        step: "classify".to_string(),
        detail: format!(
            "router classified complexity as {}",
            complexity_label(complexity)
        ),
        data: json!({
            "complexity": complexity_label(complexity),
        }),
    });

    let context = Router::build_context(
        &task.prompt,
        history_len,
        None,
        "default".to_string(),
        None,
        None,
    );
    trace.push(EvalTraceStep {
        step: "build_context".to_string(),
        detail: format!(
            "routing context resolved complexity={} with history_len={}",
            complexity_label(complexity),
            history_len
        ),
        data: json!({
            "history_len": history_len,
            "default_model": context.default_model,
            "request_complexity": complexity_label(complexity),
        }),
    });

    // Model routing evaluation
    let routing_engine = RoutingEngine::new();
    let routing_decision = routing_engine.route(&context);
    let model = routing_decision.model;
    let model_source = routing_decision.metadata.source;
    trace.push(EvalTraceStep {
        step: "model_routing".to_string(),
        detail: format!(
            "model routing selected {} via {} strategy",
            model, model_source
        ),
        data: json!({
            "model": model,
            "source": model_source,
            "reasoning": routing_decision.metadata.reasoning,
        }),
    });

    // plan_mode detection: complex task + history under threshold → plan_mode
    let auto_plan_gate = evaluate_auto_plan_gate(&complexity, history_len);
    let auto_plan = auto_plan_gate.should_generate;
    let plan_mode = auto_plan;
    trace.push(EvalTraceStep {
        step: "auto_plan_gate".to_string(),
        detail: if auto_plan {
            "auto plan gate passed".to_string()
        } else {
            "auto plan gate blocked".to_string()
        },
        data: json!({
            "enabled": auto_plan_enabled(),
            "history_len": history_len,
            "max_history": auto_plan_max_history(),
            "request_complexity": complexity_label(complexity),
            "reason": auto_plan_gate.reason,
            "result": auto_plan,
        }),
    });

    let matched_skip_pattern = detect_skip_verification_pattern(&task.prompt);
    let skip_verification = matched_skip_pattern.is_some();
    trace.push(EvalTraceStep {
        step: "skip_verification_scan".to_string(),
        detail: if let Some(pattern) = matched_skip_pattern {
            format!("matched skip-verification pattern: {}", pattern)
        } else {
            "no skip-verification pattern matched".to_string()
        },
        data: json!({
            "matched_pattern": matched_skip_pattern,
            "result": skip_verification,
        }),
    });

    // Skill detection: heuristic matching based on prompt keywords
    let skill = detect_skill(&task.prompt);
    trace.push(EvalTraceStep {
        step: "skill_detect".to_string(),
        detail: if let Some(ref s) = skill {
            format!("detected skill hint: {}", s)
        } else {
            "no skill hint detected".to_string()
        },
        data: json!({
            "skill": skill,
        }),
    });

    let actual = EvalActual {
        kind: "general".to_string(),
        complexity: complexity_label(complexity).to_string(),
        auto_plan,
        skip_verification,
        model: model_label(&model, &model_source),
        plan_mode,
        skill,
    };

    let mut failures = Vec::new();
    record_optional_failure(
        &mut failures,
        "kind",
        task.expected.kind.as_ref().map(|value| json!(value)),
        json!(actual.kind),
    );
    record_optional_failure(
        &mut failures,
        "complexity",
        task.expected.complexity.as_ref().map(|value| json!(value)),
        json!(actual.complexity),
    );
    record_optional_failure(
        &mut failures,
        "auto_plan",
        task.expected.auto_plan.map(|value| json!(value)),
        json!(actual.auto_plan),
    );
    record_optional_failure(
        &mut failures,
        "skip_verification",
        task.expected.skip_verification.map(|value| json!(value)),
        json!(actual.skip_verification),
    );
    record_optional_failure(
        &mut failures,
        "model",
        task.expected.model.as_ref().map(|value| json!(value)),
        json!(actual.model),
    );
    record_optional_failure(
        &mut failures,
        "plan_mode",
        task.expected.plan_mode.map(|value| json!(value)),
        json!(actual.plan_mode),
    );
    record_optional_failure(
        &mut failures,
        "skill",
        task.expected.skill.as_ref().map(|value| json!(value)),
        json!(actual.skill),
    );

    // ── P1: behavior evaluation ──────────────────────────────────────
    let behavior_result = task.behavior.as_ref().map(|behavior| {
        // Generate simulated tool call records to verify behavior expectations
        let simulated_calls = simulate_tool_calls_from_prompt(&task.prompt, behavior);
        let efficiency = crate::agent::eval_behaviors::compute_efficiency(
            &simulated_calls,
            simulated_calls.len().max(1),
        );
        let signal = crate::agent::eval_behaviors::detect_finish_signal(
            &simulated_calls,
            None,
            behavior.max_turns.unwrap_or(30),
            simulated_calls.len(),
        );
        let result = crate::agent::eval_behaviors::validate_behavior(
            &simulated_calls,
            behavior,
            &signal,
            &efficiency,
        );
        trace.push(EvalTraceStep {
            step: "behavior_eval".to_string(),
            detail: format!(
                "behavior eval: passed={}, finish={}, efficiency_ratio={:.2}",
                result.passed,
                result.finish_signal.label(),
                result.efficiency.effective_ratio,
            ),
            data: json!({
                "passed": result.passed,
                "finish_signal": result.finish_signal.label(),
                "failed_rules": result.failed_rules,
                "efficiency": {
                    "total_turns": result.efficiency.total_turns,
                    "total_tool_calls": result.efficiency.total_tool_calls,
                    "redundant_reads": result.efficiency.redundant_reads,
                    "effective_ratio": result.efficiency.effective_ratio,
                },
            }),
        });
        BehaviorEvalResult {
            executed: true,
            passed: result.passed,
            finish_signal: Some(result.finish_signal),
            failures: result.failures.clone(),
            failed_rules: result.failed_rules.clone(),
        }
    });

    // Merge behavior evaluation failures into routing failures
    if let Some(ref br) = behavior_result {
        for rule in &br.failed_rules {
            failures.push(EvalFailure {
                rule: format!("behavior.{}", rule),
                expected: json!("expected"),
                actual: json!("actual"),
                message: format!("Behavior rule '{}' failed", rule),
            });
        }
    }

    let failed_rules = failures
        .iter()
        .map(|failure| failure.rule.clone())
        .collect::<Vec<_>>();
    // Routing failure + behavior failure → either failure makes overall passed = false
    let routing_passed = failed_rules.iter().all(|r| r.starts_with("behavior."));
    let behavior_passed = behavior_result.as_ref().map(|b| b.passed).unwrap_or(true);
    let passed = routing_passed && behavior_passed;
    let finished_at = chrono::Utc::now().to_rfc3339();
    let duration_ms = started.elapsed().as_millis() as u64;

    EvalResult {
        id: task.id.clone(),
        passed,
        trial_num,
        trials_total,
        task: EvalTaskSnapshot {
            id: task.id,
            prompt: task.prompt,
            history_len,
            expected: task.expected,
        },
        trial: EvalTrial {
            id: format!("{}#{}", run_id, trial_num),
            trial_num,
            trials_total,
            started_at,
            finished_at,
            duration_ms,
            trace,
        },
        outcome: EvalOutcome {
            passed,
            actual,
            failures,
            failed_rules,
            behavior: behavior_result,
        },
    }
}

/// Generate simulated tool calls based on prompt and behavior expectations
///
/// Note: This is P1's **offline mode** (does not start a real agent).
/// Real agent runs require E2E (P2) / CI environment via `run_agent_with_trace`.
fn simulate_tool_calls_from_prompt(
    prompt: &str,
    behavior: &BehaviorExpectation,
) -> Vec<crate::agent::eval_behaviors::ToolCallRecord> {
    use crate::agent::eval_behaviors::ToolCallRecord;

    // If tool_sequence is specified, use it as simulation
    if let Some(ref seq) = behavior.tool_sequence {
        return seq
            .iter()
            .map(|step| ToolCallRecord {
                tool_name: step.tool.clone(),
                arguments: serde_json::json!({"pattern": step.args_pattern.as_deref().unwrap_or("*")}),
                success: true,
                output_summary: format!("Simulated {} call", step.tool),
            })
            .collect();
    }

    // Otherwise, generate heuristically based on prompt keywords
    let lower = prompt.to_lowercase();
    let mut calls = Vec::new();

    if lower.contains("Grep") || lower.contains("find") {
        calls.push(ToolCallRecord {
            tool_name: "Grep".to_string(),
            arguments: json!({"pattern": "search_term"}),
            success: true,
            output_summary: "Simulated search".to_string(),
        });
    }
    if lower.contains("read") || lower.contains("file") {
        calls.push(ToolCallRecord {
            tool_name: "Read".to_string(),
            arguments: json!({"file_path": "src/lib.rs"}),
            success: true,
            output_summary: "Simulated read".to_string(),
        });
    }
    if lower.contains("edit") || lower.contains("write") {
        calls.push(ToolCallRecord {
            tool_name: "Edit".to_string(),
            arguments: json!({"file_path": "src/lib.rs", "old": "old", "new": "new"}),
            success: true,
            output_summary: "Simulated edit".to_string(),
        });
    }
    if lower.contains("Bash") || lower.contains("test") {
        calls.push(ToolCallRecord {
            tool_name: "Bash".to_string(),
            arguments: json!({"command": "cargo test"}),
            success: true,
            output_summary: "Simulated bash".to_string(),
        });
    }

    if calls.is_empty() {
        calls.push(ToolCallRecord {
            tool_name: "Read".to_string(),
            arguments: json!({"file_path": "src/main.rs"}),
            success: true,
            output_summary: "Default simulated read".to_string(),
        });
    }

    calls
}

/// P2: E2E task skeleton evaluation
///
/// Generates an E2E result marked as `executed: false`.
/// When the environment is ready (API key + isolated worktree), real builder + evaluator will be run via `run_agent_with_trace`.
fn evaluate_e2e_task_stub(task: &EvalTask, _run_id: &str) -> E2EResult {
    let e2e_config = task.e2e.as_ref();

    let criteria_results = e2e_config
        .map(|config| {
            config
                .criteria
                .iter()
                .map(|c| CriterionResult {
                    name: c.name.clone(),
                    check: c.check.clone(),
                    passed: false,
                    detail: "E2E not executed: requires API key and git worktree isolation"
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    E2EResult {
        executed: false,
        builder: None,
        evaluator_verdict: None,
        criteria_results,
        failed_rules: vec!["e2e_not_executed".to_string()],
        passed: false,
    }
}

fn build_summary(results: &[EvalResult], trials_per_task: usize) -> EvalSummary {
    let total = results.len();
    let passed = results.iter().filter(|result| result.passed).count();
    let failed = total.saturating_sub(passed);
    let mut failed_rule_counts = BTreeMap::new();

    for result in results.iter().filter(|result| !result.passed) {
        for rule in &result.outcome.failed_rules {
            *failed_rule_counts.entry(rule.clone()).or_insert(0) += 1;
        }
    }

    // Group by task_id to calculate pass@k
    let mut task_trials: BTreeMap<&str, Vec<bool>> = BTreeMap::new();
    for result in results {
        task_trials
            .entry(&result.id)
            .or_default()
            .push(result.passed);
    }

    let unique_tasks = task_trials.len();
    // pass@1: at least 1 pass (single trial)
    let pass_at_1_count = task_trials
        .values()
        .filter(|trials| trials.first().copied().unwrap_or(false))
        .count();
    // pass@k: at least 1 pass across all trials
    let pass_at_k_count = task_trials
        .values()
        .filter(|trials| trials.iter().any(|p| *p))
        .count();
    // pass^k: all trials pass
    let pass_all_count = task_trials
        .values()
        .filter(|trials| trials.iter().all(|p| *p))
        .count();

    EvalSummary {
        total,
        passed,
        failed,
        pass_rate: if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        },
        failed_rule_counts,
        trials_per_task,
        pass_at_1: if unique_tasks == 0 {
            0.0
        } else {
            pass_at_1_count as f64 / unique_tasks as f64
        },
        pass_at_3: if unique_tasks == 0 {
            0.0
        } else {
            pass_at_k_count as f64 / unique_tasks as f64
        },
        pass_all: if unique_tasks == 0 {
            0.0
        } else {
            pass_all_count as f64 / unique_tasks as f64
        },
    }
}

fn record_optional_failure(
    failures: &mut Vec<EvalFailure>,
    rule: &str,
    expected: Option<Value>,
    actual: Value,
) {
    let Some(expected) = expected else {
        return;
    };

    if expected == actual {
        return;
    }

    failures.push(EvalFailure {
        rule: rule.to_string(),
        message: format!(
            "{} expected={} actual={}",
            rule,
            render_value(&expected),
            render_value(&actual)
        ),
        expected,
        actual,
    });
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn complexity_label(complexity: RequestComplexity) -> &'static str {
    match complexity {
        RequestComplexity::Simple => "simple",
        RequestComplexity::Medium => "medium",
        RequestComplexity::Complex => "complex",
    }
}

/// Model label: combine model name + routing strategy into an evaluable label
fn model_label(model: &str, source: &str) -> String {
    // Summarize: for routing decisions that need verification in eval, return strategy label
    match source {
        "performance" => match model {
            m if m == "default" => "default".to_string(),
            _ => "fast_model".to_string(),
        },
        "cost_optimization" => "cheap_model".to_string(),
        "user_override" => "user_override".to_string(),
        "default" => "default".to_string(),
        _ => format!("{}:{}", source, model),
    }
}

/// Heuristic skill detection: based on prompt keywords
fn detect_skill(prompt: &str) -> Option<String> {
    let lower = prompt.to_lowercase();
    if lower.contains("test") {
        return Some("spec-test".to_string());
    }
    if lower.contains("commit") {
        return Some("commit".to_string());
    }
    if lower.contains("pr") || lower.contains("pull request") {
        return Some("git-pr".to_string());
    }
    if lower.contains("explain") {
        return Some("explain".to_string());
    }
    if lower.contains("Grep") {
        return Some("Grep".to_string());
    }
    None
}

/// Save regression baseline to file
pub async fn save_baseline(report: &EvalReport, baseline_path: &Path) -> Result<String, String> {
    let mut tasks_map = BTreeMap::new();
    for result in &report.results {
        // Only save first trial result per task (deduplicate)
        if result.trial_num == 1 {
            tasks_map.insert(
                result.id.clone(),
                json!({
                    "expected": result.task.expected,
                    "actual": {
                        "kind": result.outcome.actual.kind,
                        "complexity": result.outcome.actual.complexity,
                        "auto_plan": result.outcome.actual.auto_plan,
                        "skip_verification": result.outcome.actual.skip_verification,
                        "model": result.outcome.actual.model,
                        "plan_mode": result.outcome.actual.plan_mode,
                        "skill": result.outcome.actual.skill,
                    },
                }),
            );
        }
    }

    let git_commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let baseline = EvalBaseline {
        created_at: chrono::Utc::now().to_rfc3339(),
        git_commit,
        schema_version: EVAL_REPORT_SCHEMA_VERSION,
        tasks: tasks_map,
    };

    let baseline_json = serde_json::to_string_pretty(&baseline)
        .map_err(|e| format!("Failed to serialize baseline: {}", e))?;
    if let Some(parent) = baseline_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    tokio::fs::write(baseline_path, &baseline_json)
        .await
        .map_err(|e| format!("Failed to write baseline: {}", e))?;

    Ok(baseline_json)
}

/// Compare current report with baseline, return list of deltas
pub async fn compare_baseline(
    report: &EvalReport,
    baseline_path: &Path,
) -> Result<Vec<BaselineDelta>, String> {
    let baseline_content = tokio::fs::read_to_string(baseline_path)
        .await
        .map_err(|e| format!("Failed to read baseline {}: {}", baseline_path.display(), e))?;
    let baseline: EvalBaseline = serde_json::from_str(&baseline_content)
        .map_err(|e| format!("Failed to parse baseline: {}", e))?;

    // Collect current report's best-of-trial results (aggregate by task_id, take first pass)
    let mut current_map: BTreeMap<String, &EvalResult> = BTreeMap::new();
    for result in &report.results {
        current_map.entry(result.id.clone()).or_insert(result);
    }

    let mut deltas = Vec::new();

    // Check each task in baseline for regression
    for (task_id, baseline_actual) in &baseline.tasks {
        match current_map.get(task_id.as_str()) {
            Some(current) => {
                let current_actual = json!({
                    "kind": current.outcome.actual.kind,
                    "complexity": current.outcome.actual.complexity,
                    "auto_plan": current.outcome.actual.auto_plan,
                    "skip_verification": current.outcome.actual.skip_verification,
                    "model": current.outcome.actual.model,
                    "plan_mode": current.outcome.actual.plan_mode,
                    "skill": current.outcome.actual.skill,
                });
                let bl_actual = &baseline_actual["actual"];
                if bl_actual != &current_actual {
                    deltas.push(BaselineDelta {
                        task_id: task_id.clone(),
                        change: BaselineChange::Regression {
                            expected: bl_actual.clone(),
                            actual: current_actual,
                        },
                    });
                }
            }
            None => {
                deltas.push(BaselineDelta {
                    task_id: task_id.clone(),
                    change: BaselineChange::Removed,
                });
            }
        }
    }

    // Check new tasks in current report not present in baseline
    for task_id in current_map.keys() {
        if !baseline.tasks.contains_key(task_id.as_str()) {
            deltas.push(BaselineDelta {
                task_id: task_id.clone(),
                change: BaselineChange::New,
            });
        }
    }

    Ok(deltas)
}

// ── P3: Live evaluation (real LLM against an isolated fixture copy) ──────

/// Run a real LLM agent on the given live task.
///
/// Steps:
/// 1. Resolve LLM credentials from the environment (STAR_API_KEY / STAR_BASE_URL /
///    STAR_OPENAI_COMPATIBLE). Missing credentials → skipped result.
/// 2. Copy `eval/fixtures/{fixture}` to a temp dir (self-contained git repo).
/// 3. Build a Config/Agent pointed at the fixture copy and run it with
///    `run_agent_with_trace` under a timeout.
/// 4. Run each `verify` command in the fixture copy (cwd = fixture).
/// 5. Evaluate behavior rules (must_include/forbidden/max_turns/files).
/// 6. Clean up the fixture copy.
async fn evaluate_live_task(task: &EvalTask) -> LiveEvalResult {
    let started = Instant::now();
    let live = task.live.as_ref().expect("live config required");
    let Some(fixture_dir) = resolve_fixture_dir(&live.fixture) else {
        return LiveEvalResult {
            id: task.id.clone(),
            executed: false,
            skip_reason: Some(format!(
                "fixture '{}' not found under eval/fixtures/",
                live.fixture
            )),
            passed: false,
            failed_rules: vec!["fixture_missing".to_string()],
            finish_signal: None,
            total_turns: 0,
            tool_calls: 0,
            duration_ms: 0,
            total_tokens: 0,
            tool_call_summary: vec![],
            verify: vec![],
            final_message: String::new(),
            agent_error: None,
            behavior_failures: vec![],
        };
    };

    let Some(api_key) = std::env::var("STAR_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
    else {
        return LiveEvalResult {
            id: task.id.clone(),
            executed: false,
            skip_reason: Some(
                "STAR_API_KEY not set — configure credentials to run live tasks".to_string(),
            ),
            passed: false,
            failed_rules: vec!["no_api_key".to_string()],
            finish_signal: None,
            total_turns: 0,
            tool_calls: 0,
            duration_ms: 0,
            total_tokens: 0,
            tool_call_summary: vec![],
            verify: vec![],
            final_message: String::new(),
            agent_error: None,
            behavior_failures: vec![],
        };
    };

    let base_url = std::env::var("STAR_BASE_URL").ok();
    let openai_compatible = std::env::var("STAR_OPENAI_COMPATIBLE").ok().map(|v| {
        matches!(
            v.trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    });
    let model = live
        .model
        .clone()
        .or_else(|| std::env::var("STAR_MODEL").ok());

    // 1. Copy fixture to a temp dir (keeps the fixture pristine; the copy
    //    carries its own .git so the agent gets full git context).
    let tmp_root = std::env::temp_dir().join(format!("starcode-eval-{}", uuid::Uuid::new_v4()));
    let copy_path = tmp_root.join(&live.fixture);
    let mut fail = |rules: Vec<String>, msg: String| LiveEvalResult {
        id: task.id.clone(),
        executed: true,
        skip_reason: Some(msg),
        passed: false,
        failed_rules: rules,
        finish_signal: None,
        total_turns: 0,
        tool_calls: 0,
        duration_ms: started.elapsed().as_millis() as u64,
        total_tokens: 0,
        tool_call_summary: vec![],
        verify: vec![],
        final_message: String::new(),
        agent_error: None,
        behavior_failures: vec![],
    };

    // `cp -r <fixture> <tmp_root>/` needs tmp_root to exist first, otherwise
    // cp renames the fixture into the target path itself.
    let prepared = std::fs::create_dir_all(&tmp_root).is_ok();
    let copy_ok = prepared
        && tokio::process::Command::new("cp")
            .arg("-r")
            .arg(&fixture_dir)
            .arg(&tmp_root)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
    if !copy_ok {
        let _ = std::fs::remove_dir_all(&tmp_root);
        return fail(
            vec!["fixture_copy_failed".to_string()],
            format!("failed to copy fixture to {}", copy_path.display()),
        );
    }

    // 2. Build Config + Client pointed at the fixture copy.
    let mut params = crate::core::config::ConfigParameters::default();
    params.session_id = uuid::Uuid::new_v4().to_string();
    params.target_dir = copy_path.clone();
    params.cwd = copy_path.clone();
    params.model = model.clone().unwrap_or_default();
    params.max_session_turns = Some(live.max_turns as i32);
    // Headless eval: never block on permission confirmations — there is no
    // UI to answer them, and a pending confirmation would hang the run
    // until the 600s confirmation timeout.
    params.approval_mode = Some(crate::core::policy::types::ApprovalMode::Yolo);

    let mut config = crate::core::config::Config::new(params);
    let init_result = config.initialize().await;
    let config = match init_result {
        Ok(_) => std::sync::Arc::new(config),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp_root);
            return fail(
                vec!["config_init_failed".to_string()],
                format!("config init failed: {e}"),
            );
        }
    };

    let client =
        crate::llm::client::StarClient::new(&api_key, model, base_url, openai_compatible, None);

    // 3. Switch the process cwd to the fixture copy and override the
    //    process-level project-root cache, so the agent works inside the
    //    copy (not the pristine fixture). Restored afterwards.
    let original_cwd = std::env::current_dir().ok();
    let switched_cwd = std::env::set_current_dir(&copy_path).is_ok();
    if !switched_cwd {
        let _ = std::fs::remove_dir_all(&tmp_root);
        return fail(
            vec!["cwd_switch_failed".to_string()],
            format!("failed to chdir into {}", copy_path.display()),
        );
    }
    crate::agent::hooks::override_project_root(copy_path.clone());

    // 3. Run the agent under a timeout.
    let mut agent = crate::agent::Agent::new(client, config);
    let run_config = crate::agent::eval_runner::AgentRunConfig {
        max_turns: live.max_turns,
        ..Default::default()
    };

    let run_result = tokio::time::timeout(
        std::time::Duration::from_secs(live.timeout_secs),
        crate::agent::eval_runner::run_agent_with_trace(&mut agent, &task.prompt, &run_config),
    )
    .await;

    // Restore process cwd immediately after the agent run so verify commands
    // and subsequent tasks run from a known location.
    let _ = original_cwd.as_ref().map(|d| std::env::set_current_dir(d));

    let agent_run = match run_result {
        Ok(result) => result,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&tmp_root);
            return fail(
                vec!["agent_timeout".to_string()],
                format!("agent run timed out after {}s", live.timeout_secs),
            );
        }
    };

    // 4. Run verify commands in the fixture copy.
    let mut verify_results = Vec::new();
    for command in &live.verify {
        let verify_out = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .current_dir(&copy_path)
                .output(),
        )
        .await;
        match verify_out {
            Ok(Ok(output)) => {
                let passed = output.status.success();
                let tail = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                verify_results.push(VerifyResult {
                    command: command.clone(),
                    passed,
                    exit_code: output.status.code(),
                    output_tail: tail,
                });
            }
            Ok(Err(e)) => verify_results.push(VerifyResult {
                command: command.clone(),
                passed: false,
                exit_code: None,
                output_tail: format!("failed to run: {e}"),
            }),
            Err(_) => verify_results.push(VerifyResult {
                command: command.clone(),
                passed: false,
                exit_code: None,
                output_tail: "timed out after 120s".to_string(),
            }),
        }
    }

    // 5. Evaluate behavior rules against the real tool call history.
    let tool_calls = &agent_run.tool_calls;
    let mut behavior_failures: Vec<String> = Vec::new();
    let mut failed_rules: Vec<String> = Vec::new();

    if let Some(behavior) = &task.behavior {
        let efficiency =
            crate::agent::eval_behaviors::compute_efficiency(tool_calls, tool_calls.len().max(1));
        let signal = crate::agent::eval_behaviors::detect_finish_signal(
            tool_calls,
            if agent_run.final_message.is_empty() {
                None
            } else {
                Some(&agent_run.final_message)
            },
            behavior.max_turns.unwrap_or(30),
            agent_run.total_turns,
        );
        let result = crate::agent::eval_behaviors::validate_behavior(
            tool_calls,
            behavior,
            &signal,
            &efficiency,
        );
        behavior_failures.extend(result.failures.iter().map(|f| f.message.clone()));
        failed_rules.extend(result.failed_rules.iter().cloned());
    }

    // 6. Overall pass: no agent error, verify all green, no behavior failures.
    if agent_run.error.is_some() {
        failed_rules.push("agent_error".to_string());
    }
    if verify_results.iter().any(|v| !v.passed) {
        failed_rules.push("verify_failed".to_string());
    }
    if matches!(agent_run.finish_signal, FinishSignal::FalseFinish { .. }) {
        failed_rules.push("false_finish".to_string());
    }
    if matches!(agent_run.finish_signal, FinishSignal::TurnLimit) {
        failed_rules.push("turn_limit".to_string());
    }
    if matches!(agent_run.finish_signal, FinishSignal::ToolError { .. }) {
        failed_rules.push("tool_error".to_string());
    }

    let tool_call_summary: Vec<ToolCallSummary> = tool_calls
        .iter()
        .map(|tc| ToolCallSummary {
            tool: tc.tool_name.clone(),
            success: tc.success,
            args: tc.arguments.clone(),
        })
        .collect();

    let passed = agent_run.error.is_none()
        && verify_results.iter().all(|v| v.passed)
        && behavior_failures.is_empty()
        && !matches!(
            agent_run.finish_signal,
            FinishSignal::FalseFinish { .. }
                | FinishSignal::TurnLimit
                | FinishSignal::ToolError { .. }
        );

    let duration_ms = started.elapsed().as_millis() as u64;
    let result = LiveEvalResult {
        id: task.id.clone(),
        executed: true,
        skip_reason: None,
        passed,
        failed_rules: failed_rules.clone(),
        finish_signal: Some(agent_run.finish_signal.label().to_string()),
        total_turns: agent_run.total_turns,
        tool_calls: tool_calls.len(),
        duration_ms,
        total_tokens: agent_run.prompt_tokens + agent_run.completion_tokens,
        tool_call_summary,
        verify: verify_results,
        final_message: agent_run.final_message.clone(),
        agent_error: agent_run.error.clone(),
        behavior_failures,
    };

    // 7. Clean up the fixture copy.
    let _ = std::fs::remove_dir_all(&tmp_root);
    result
}

/// Resolve `eval/fixtures/<name>` relative to the current working directory.
fn resolve_fixture_dir(name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let candidate = cwd.join("eval").join("fixtures").join(name);
    if candidate.is_dir() {
        return Some(candidate);
    }
    None
}

// ── Markdown report ────────────────────────────────────────────────────

/// Render an eval report as a human-readable markdown summary.
///
/// Includes: run metadata, L1/L2 routing summary, live (P3) results with
/// problem profiling (finish signal, verify output, tool call evidence),
/// and failed-rule aggregation.
pub fn eval_report_to_markdown(report: &EvalReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Eval Report — `{}`\n\n", report.run_id));
    out.push_str(&format!(
        "- Run at: {}\n- Schema: v{}\n- Tasks file: {}\n- Report: {}\n\n",
        report.run_at,
        report.schema_version,
        report.artifacts.tasks_path,
        report.artifacts.report_path,
    ));

    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "| Metric | Value |\n|---|---|\n| Total | {} |\n| Passed | {} |\n| Failed | {} |\n| Pass rate | {:.1}% |\n",
        report.summary.total,
        report.summary.passed,
        report.summary.failed,
        report.summary.pass_rate * 100.0,
    ));
    if !report.summary.failed_rule_counts.is_empty() {
        out.push_str("\n### Failed rule distribution\n\n");
        out.push_str("| Rule | Count |\n|---|---|\n");
        for (rule, count) in &report.summary.failed_rule_counts {
            out.push_str(&format!("| `{}` | {} |\n", rule, count));
        }
    }

    if let Some(live) = &report.live_results {
        out.push_str("\n## Live tasks (real LLM runs)\n\n");
        for r in live {
            out.push_str(&render_live_result_markdown(r));
        }
    }

    if let Some(e2e) = &report.e2e_results {
        out.push_str("\n## E2E tasks\n\n");
        for (i, r) in e2e.iter().enumerate() {
            out.push_str(&format!(
                "- **E2E task #{}**: executed={} passed={}\n",
                i + 1,
                r.executed,
                r.passed
            ));
        }
    }

    out
}

fn render_live_result_markdown(r: &LiveEvalResult) -> String {
    let mut out = String::new();
    let status = if !r.executed {
        "⬜ skipped"
    } else if r.passed {
        "✅ passed"
    } else {
        "❌ failed"
    };
    out.push_str(&format!("### {} — {}\n\n", r.id, status));
    if let Some(reason) = &r.skip_reason {
        out.push_str(&format!("> {}\n\n", reason));
        return out;
    }
    out.push_str(&format!(
        "- Finish signal: `{}`\n- Turns: {} | Tool calls: {} | Duration: {} ms | Tokens: {}\n",
        r.finish_signal.as_deref().unwrap_or("n/a"),
        r.total_turns,
        r.tool_calls,
        r.duration_ms,
        r.total_tokens,
    ));
    if !r.failed_rules.is_empty() {
        out.push_str(&format!("- Failed rules: {}\n", r.failed_rules.join(", ")));
    }
    if !r.behavior_failures.is_empty() {
        out.push_str("\n#### Behavior failures\n\n");
        for f in &r.behavior_failures {
            out.push_str(&format!("- {}\n", f));
        }
    }
    if !r.verify.is_empty() {
        out.push_str("\n#### Verification\n\n");
        for v in &r.verify {
            let mark = if v.passed { "✅" } else { "❌" };
            out.push_str(&format!(
                "- {} `{}` (exit {})\n",
                mark,
                v.command,
                v.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ));
            if !v.passed && !v.output_tail.is_empty() {
                out.push_str(&format!("```\n{}\n```\n", v.output_tail));
            }
        }
    }
    out.push_str("\n#### Tool calls (evidence)\n\n");
    out.push_str("| Tool | Success | Args |\n|---|---|---|\n");
    for tc in &r.tool_call_summary {
        let args = tc.args.to_string();
        let args = crate::utils::string_utils::truncate_chars(&args, 100);
        out.push_str(&format!(
            "| `{}` | {} | `{}` |\n",
            tc.tool,
            if tc.success { "✅" } else { "❌" },
            args
        ));
    }
    if !r.final_message.is_empty() {
        let msg = r
            .final_message
            .lines()
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "\n#### Final message (truncated)\n\n```\n{}\n```\n",
            msg
        ));
    }
    if let Some(err) = &r.agent_error {
        out.push_str(&format!("\n#### Agent error\n\n```\n{}\n```\n", err));
    }
    out.push('\n');
    out
}
