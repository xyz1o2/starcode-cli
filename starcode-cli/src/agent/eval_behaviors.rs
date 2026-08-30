//! P1: Agent behavior evaluation — tool call chain verification, finish signal detection, context efficiency
//!
//! Unlike Layer 1 (routing decision), this layer validates the agent's **actual runtime behavior**:
//! - Whether tool sequence matches expectations
//! - Whether there are forbidden tool calls
//! - Whether it finishes within a reasonable number of turns
//! - Context efficiency

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Behavior expectation definition ──────────────────────────────────────────

/// Tool sequence step constraint
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolSequenceStep {
    /// Tool name (e.g. "Read", "Glob", "Edit", "Bash")
    pub tool: String,
    /// Optional argument pattern (regex substring match)
    #[serde(default)]
    pub args_pattern: Option<String>,
    /// Call count constraint
    #[serde(default)]
    pub count: Option<CountConstraint>,
}

/// Call count constraint
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CountConstraint {
    #[serde(default)]
    pub min: Option<usize>,
    #[serde(default)]
    pub max: Option<usize>,
    #[serde(default)]
    pub exact: Option<usize>,
}

/// Behavior expectation (optional, only effective when task's `behavior` field exists)
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct BehaviorExpectation {
    /// Expected tool call sequence (order matching)
    #[serde(default)]
    pub tool_sequence: Option<Vec<ToolSequenceStep>>,
    /// Forbidden tool call patterns (e.g. "Bash(rm)", "Bash(git push --force)")
    #[serde(default)]
    pub forbidden_tools: Option<Vec<String>>,
    /// Maximum allowed turns
    #[serde(default)]
    pub max_turns: Option<usize>,
    /// Required tool names
    #[serde(default)]
    pub must_include_tools: Option<Vec<String>>,
    /// Expected files to create (glob pattern)
    #[serde(default)]
    pub files_created: Option<Vec<String>>,
    /// Expected files to modify (glob pattern)
    #[serde(default)]
    pub files_modified: Option<Vec<String>>,
}

// ── Tool name normalization ─────────────────────────────────────────

/// 归一化工具名，使别名与真实注册名等价（如 read_file/view_file、
/// grep/search、str_replace_editor/replace）。断言按归一化后的名字匹配，
/// 避免"提示词用别名、schema 用注册名"导致的误报。
fn canonical_tool_name(name: &str) -> String {
    crate::core::tools::constants::canonical_tool_name(name)
}

// ── Agent finish signal ────────────────────────────────────────

/// Agent finish signal classification
#[derive(Debug, PartialEq, Eq, Serialize, Clone)]
pub enum FinishSignal {
    /// Agent has explicit finish signal and artifacts are verifiable
    TrueFinish,
    /// Agent called finish/explicit-done type tool
    ExplicitFinish,
    /// Agent stopped calling tools without explicit finish
    ImplicitFinish,
    /// Agent claims completion but verification fails
    FalseFinish { reason: String },
    /// Reached max_turns limit
    TurnLimit,
    /// Tool execution error interrupted
    ToolError { tool: String, error: String },
}

impl FinishSignal {
    pub fn label(&self) -> &'static str {
        match self {
            FinishSignal::TrueFinish => "true_finish",
            FinishSignal::ExplicitFinish => "explicit_finish",
            FinishSignal::ImplicitFinish => "implicit_finish",
            FinishSignal::FalseFinish { .. } => "false_finish",
            FinishSignal::TurnLimit => "turn_limit",
            FinishSignal::ToolError { .. } => "tool_error",
        }
    }
}

// ── Context efficiency metrics ────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct ContextEfficiency {
    /// Total turns (LLM call count)
    pub total_turns: usize,
    /// Total tool call count
    pub total_tool_calls: usize,
    /// Redundant read count (reading same file path repeatedly)
    pub redundant_reads: usize,
    /// Effective tool call count (Write/Edit/Bash etc.)
    pub effective_tool_calls: usize,
    /// Effective tool call ratio
    pub effective_ratio: f64,
}

// ── Behavior verification result ──────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct BehaviorResult {
    /// Overall passed
    pub passed: bool,
    /// Finish signal
    pub finish_signal: FinishSignal,
    /// Context efficiency
    pub efficiency: ContextEfficiency,
    /// Tool sequence match result
    pub sequence_match: Option<SequenceMatch>,
    /// Forbidden tool violations
    pub forbidden_violations: Vec<String>,
    /// Missing required tools
    pub missing_tools: Vec<String>,
    /// Failed rules list
    pub failed_rules: Vec<String>,
    /// All failure details
    pub failures: Vec<BehaviorFailure>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SequenceMatch {
    /// Matched steps / total expected steps
    pub matched_steps: usize,
    pub total_steps: usize,
    pub violations: Vec<SequenceViolation>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SequenceViolation {
    pub step_index: usize,
    pub expected_tool: String,
    pub actual_tool: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct BehaviorFailure {
    pub rule: String,
    pub message: String,
    pub detail: Value,
}

// ── Tool call record ──────────────────────────────────────────

/// Single tool call record
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    /// Serialized tool arguments JSON
    pub arguments: Value,
    /// Whether tool execution succeeded
    pub success: bool,
    /// Tool output summary (truncated)
    pub output_summary: String,
}

// ── Verification functions ──────────────────────────────────────────────

/// Verify tool call history matches behavior expectations
pub fn validate_behavior(
    tool_calls: &[ToolCallRecord],
    behavior: &BehaviorExpectation,
    finish_signal: &FinishSignal,
    efficiency: &ContextEfficiency,
) -> BehaviorResult {
    let mut failures: Vec<BehaviorFailure> = Vec::new();
    let mut failed_rules: Vec<String> = Vec::new();

    // 1. Tool sequence verification
    let sequence_match = behavior
        .tool_sequence
        .as_ref()
        .map(|seq| validate_sequence(tool_calls, seq, &mut failures));

    if let Some(ref seq_match) = sequence_match {
        if seq_match.matched_steps != seq_match.total_steps {
            failed_rules.push("tool_sequence".to_string());
        }
    }

    // 2. Forbidden tool detection
    let forbidden_violations: Vec<String> = behavior
        .forbidden_tools
        .as_ref()
        .map(|forbidden| {
            tool_calls
                .iter()
                .filter(|tc| {
                    forbidden.iter().any(|f| {
                        let f_lower = f.to_lowercase();
                        canonical_tool_name(&tc.tool_name).to_lowercase().contains(&f_lower)
                            || format!("{}({})", canonical_tool_name(&tc.tool_name), tc.arguments)
                                .to_lowercase()
                                .contains(&f_lower)
                    })
                })
                .map(|tc| format!("{} (args: {})", tc.tool_name, tc.arguments))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for violation in &forbidden_violations {
        failed_rules.push("forbidden_tools".to_string());
        failures.push(BehaviorFailure {
            rule: "forbidden_tools".to_string(),
            message: format!("Forbidden tool call: {}", violation),
            detail: serde_json::json!({ "call": violation }),
        });
    }

    // 3. Required tool detection
    let missing_tools: Vec<String> = behavior
        .must_include_tools
        .as_ref()
        .map(|required| {
            required
                .iter()
                .filter(|req| {
                    let req_canonical = canonical_tool_name(&req.to_lowercase());
                    !tool_calls.iter().any(|tc| {
                        canonical_tool_name(&tc.tool_name).to_lowercase().contains(&req_canonical)
                    })
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    for missing in &missing_tools {
        failed_rules.push("must_include_tools".to_string());
        failures.push(BehaviorFailure {
            rule: "must_include_tools".to_string(),
            message: format!("Required tool not called: {}", missing),
            detail: serde_json::json!({ "missing_tool": missing }),
        });
    }

    // 4. Max turns check
    if let Some(max_turns) = behavior.max_turns {
        if efficiency.total_turns > max_turns {
            failed_rules.push("max_turns".to_string());
            failures.push(BehaviorFailure {
                rule: "max_turns".to_string(),
                message: format!(
                    "Exceeded max turns: {} > {}",
                    efficiency.total_turns, max_turns
                ),
                detail: serde_json::json!({
                    "actual_turns": efficiency.total_turns,
                    "max_turns": max_turns,
                }),
            });
        }
    }

    // 5. Finish signal check
    if matches!(
        finish_signal,
        FinishSignal::FalseFinish { .. } | FinishSignal::ToolError { .. }
    ) {
        failed_rules.push("finish_signal".to_string());
        failures.push(BehaviorFailure {
            rule: "finish_signal".to_string(),
            message: format!("Unhealthy finish signal: {}", finish_signal.label()),
            detail: serde_json::json!({ "signal": finish_signal.label() }),
        });
    }

    BehaviorResult {
        passed: failures.is_empty(),
        finish_signal: finish_signal.clone(),
        efficiency: efficiency.clone(),
        sequence_match,
        forbidden_violations,
        missing_tools,
        failed_rules,
        failures,
    }
}

/// Verify tool call sequence
fn validate_sequence(
    actual: &[ToolCallRecord],
    expected: &[ToolSequenceStep],
    failures: &mut Vec<BehaviorFailure>,
) -> SequenceMatch {
    let mut violations = Vec::new();
    let total_steps = expected.len();

    for (step_idx, expected_step) in expected.iter().enumerate() {
        let actual_call = actual.get(step_idx);

        match actual_call {
            Some(tc) => {
                let tool_match = canonical_tool_name(&tc.tool_name)
                    .to_lowercase()
                    .contains(&expected_step.tool.to_lowercase());

                let args_match = expected_step
                    .args_pattern
                    .as_ref()
                    .map(|pat| {
                        tc.arguments
                            .to_string()
                            .to_lowercase()
                            .contains(&pat.to_lowercase())
                    })
                    .unwrap_or(true);

                if !tool_match || !args_match {
                    violations.push(SequenceViolation {
                        step_index: step_idx,
                        expected_tool: expected_step.tool.clone(),
                        actual_tool: Some(tc.tool_name.clone()),
                        detail: if !tool_match {
                            format!(
                                "Tool mismatch: expected_pattern={} actual={}",
                                expected_step.tool, tc.tool_name
                            )
                        } else {
                            format!(
                                "Args mismatch: expected_pattern={} actual_args={}",
                                expected_step.args_pattern.as_deref().unwrap_or("*"),
                                tc.arguments
                            )
                        },
                    });
                    failures.push(BehaviorFailure {
                        rule: "tool_sequence".to_string(),
                        message: format!(
                            "Step {}: expected tool matching '{}', got '{}'",
                            step_idx, expected_step.tool, tc.tool_name
                        ),
                        detail: serde_json::json!({
                            "step": step_idx,
                            "expected": expected_step.tool,
                            "actual": tc.tool_name,
                        }),
                    });
                }
            }
            None => {
                violations.push(SequenceViolation {
                    step_index: step_idx,
                    expected_tool: expected_step.tool.clone(),
                    actual_tool: None,
                    detail: format!("Missing tool at step {step_idx}"),
                });
                failures.push(BehaviorFailure {
                    rule: "tool_sequence".to_string(),
                    message: format!(
                        "Step {}: expected '{}' but agent made no more tool calls",
                        step_idx, expected_step.tool
                    ),
                    detail: serde_json::json!({
                        "step": step_idx,
                        "expected": expected_step.tool,
                        "actual": null,
                    }),
                });
            }
        }
    }

    // Count constraint verification
    for expected_step in expected {
        if let Some(ref count) = expected_step.count {
            let actual_count = actual
                .iter()
                .filter(|tc| {
                    tc.tool_name
                        .to_lowercase()
                        .contains(&expected_step.tool.to_lowercase())
                })
                .count();

            if let Some(exact) = count.exact {
                if actual_count != exact {
                    violations.push(SequenceViolation {
                        step_index: 0,
                        expected_tool: expected_step.tool.clone(),
                        actual_tool: None,
                        detail: format!(
                            "Count mismatch for {}: expected={exact} actual={actual_count}",
                            expected_step.tool
                        ),
                    });
                }
            }
            if let Some(min) = count.min {
                if actual_count < min {
                    violations.push(SequenceViolation {
                        step_index: 0,
                        expected_tool: expected_step.tool.clone(),
                        actual_tool: None,
                        detail: format!(
                            "Count below minimum for {}: min={min} actual={actual_count}",
                            expected_step.tool
                        ),
                    });
                }
            }
            if let Some(max) = count.max {
                if actual_count > max {
                    violations.push(SequenceViolation {
                        step_index: 0,
                        expected_tool: expected_step.tool.clone(),
                        actual_tool: None,
                        detail: format!(
                            "Count above maximum for {}: max={max} actual={actual_count}",
                            expected_step.tool
                        ),
                    });
                }
            }
        }
    }

    let matched_steps = total_steps.saturating_sub(violations.len());

    SequenceMatch {
        matched_steps,
        total_steps,
        violations,
    }
}

/// Calculate context efficiency
pub fn compute_efficiency(
    tool_calls: &[ToolCallRecord],
    total_turns: usize,
) -> ContextEfficiency {
    let total_tool_calls = tool_calls.len();

    // Detect redundant reads (same tool + same arguments called multiple times)
    let mut read_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut redundant_reads = 0usize;

    for tc in tool_calls {
        if is_read_tool(&tc.tool_name) {
            let key = format!("{}:{}", tc.tool_name, tc.arguments);
            let count = read_counts.entry(key).or_insert(0);
            *count += 1;
        }
    }

    for count in read_counts.values() {
        if *count > 1 {
            redundant_reads += count - 1;
        }
    }

    let effective_tool_calls = tool_calls
        .iter()
        .filter(|tc| is_effective_tool(&tc.tool_name))
        .count();

    let effective_ratio = if total_tool_calls == 0 {
        0.0
    } else {
        effective_tool_calls as f64 / total_tool_calls as f64
    };

    ContextEfficiency {
        total_turns,
        total_tool_calls,
        redundant_reads,
        effective_tool_calls,
        effective_ratio,
    }
}

/// Detect finish signal
pub fn detect_finish_signal(
    tool_calls: &[ToolCallRecord],
    final_message: Option<&str>,
    max_turns: usize,
    total_turns: usize,
) -> FinishSignal {
    if total_turns >= max_turns {
        return FinishSignal::TurnLimit;
    }

    // Check for tool execution errors
    for tc in tool_calls {
        if !tc.success {
            return FinishSignal::ToolError {
                tool: tc.tool_name.clone(),
                error: tc.output_summary.clone(),
            };
        }
    }

    // Check for explicit finish signal
    let has_explicit_finish = tool_calls.iter().any(|tc| {
        let name = tc.tool_name.to_lowercase();
        name.contains("finish") || name.contains("exit_plan_mode") || name.contains("complete")
    });

    // Check if final message implies completion
    let message_implies_completion = final_message.map_or(false, |msg| {
        let lower = msg.to_lowercase();
        lower.contains("done")
            || lower.contains("finished")
            || lower.contains("completed")
    });

    if has_explicit_finish && message_implies_completion {
        FinishSignal::TrueFinish
    } else if has_explicit_finish {
        FinishSignal::ExplicitFinish
    } else if message_implies_completion && !tool_calls.is_empty() {
        // Agent made tool calls then claimed completion, but no explicit finish tool
        FinishSignal::ImplicitFinish
    } else if message_implies_completion && tool_calls.is_empty() {
        // 0 tool calls + claimed completion = possible FalseFinish
        FinishSignal::FalseFinish {
            reason: "agent claimed completion without any tool calls".to_string(),
        }
    } else {
        FinishSignal::ImplicitFinish
    }
}

fn is_read_tool(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("read")
        || lower.contains("Glob")
        || lower.contains("Grep")
        || lower.contains("Grep")
        || lower == "listdir"
}

fn is_effective_tool(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("write")
        || lower.contains("edit")
        || lower.contains("Bash")
        || lower.contains("command")
        || lower.contains("create")
}
