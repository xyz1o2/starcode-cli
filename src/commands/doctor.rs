use crate::commands::execution::{CommandContext, CommandResult};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 构建完整的 doctor 报告，不依赖任何 UI 状态。
///
/// `/doctor`（TUI）和 `starcode doctor`（CLI）必须给出同一份内容——环境类
/// 失败恰恰发生在 TUI 起不来的时候，那时用户只能用 CLI。所以诊断逻辑全部
/// 放这里，两条入口只负责"往哪儿输出"。
pub async fn build_report(transcript_path: Option<PathBuf>) -> String {
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

    // 2. 配置文件：能不能解析，比"存在不存在"重要得多。
    //    provider_store 遇到坏文件会静默回退到默认值，用户 downstream 只会
    //    看到 "API key required"。这里把解析结果直接摆出来。
    report.push_str("\n## Configuration\n");
    report.push_str(&render_config_section().await);

    // 3. Check Essential Tools
    let tools = [
        ("git", "--version"),
        ("cargo", "--version"),
        ("node", "--version"),
        ("npm", "--version"),
        ("python", "--version"),
        // 搜索工具依赖 ripgrep；grep/find 在大仓上不够用，缺了会让
        // "跨文件定位"这一类任务直接降级。
        ("rg", "--version"),
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

    // 4. 日志可写性：日志目录不可写时，所有 "[INIT] 卡在哪一步" 的面包屑都落不了地，
    //    用户和我们都失去唯一的诊断途径。
    report.push_str("\n## Logging\n");
    report.push_str(&render_logging_section());

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

    let transcript_path = transcript_path
        .unwrap_or_else(|| crate::ui::utils::transcript::default_transcript_path(&cwd));
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

    report
}

/// TUI 入口：`/doctor`。
pub async fn run(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let transcript_path = if ctx.state.transcript_enabled {
        ctx.state.transcript_path.clone()
    } else {
        None
    };
    let report = build_report(transcript_path).await;

    ctx.state
        .chat_history
        .push(crate::types::ChatEntry::assistant(report).with_streaming(false));

    Ok(())
}

/// CLI 入口：`starcode doctor`。不经过 TUI，环境出问题时也能跑。
pub async fn run_cli() -> String {
    build_report(None).await
}

/// 检查全局/项目配置文件是否存在、内容是否可解析，并顺带报告生效的
/// provider 凭据状态（只报"有没有"，绝不把 key 打出来）。
async fn render_config_section() -> String {
    use crate::core::config::json_with_comments::parse_json_with_comments;
    use crate::core::config::models::ProviderConfig;
    use crate::core::config::provider_store::ProviderStore;
    use crate::core::config::settings_manager::UserSettings;
    use crate::core::config::storage::Storage;

    let mut out = String::new();

    // 全局配置：provider_store 实际读写的那个文件
    let global_path = Storage::global_star_dir().join("user-settings.json");
    match std::fs::read_to_string(&global_path) {
        Ok(content) => {
            // 复用 provider_store 的两段式判定，保证这里报的口径和加载时一致
            if parse_json_with_comments::<UserSettings>(&content).is_ok() {
                out.push_str(&format!(
                    "✅ Global Config: parses as user-settings at `{}`\n",
                    global_path.display()
                ));
            } else if parse_json_with_comments::<ProviderConfig>(&content).is_ok() {
                out.push_str(&format!(
                    "⚠️ Global Config: `{}` is legacy providers.json format — it will be migrated on next start\n",
                    global_path.display()
                ));
            } else {
                out.push_str(&format!(
                    "❌ Global Config: `{}` exists but is NOT valid JSON — provider config is being ignored. Fix or delete it.\n",
                    global_path.display()
                ));
            }
        }
        Err(_) => {
            out.push_str(&format!(
                "ℹ️ Global Config: `{}` not found (providers may still come from env vars)\n",
                global_path.display()
            ));
        }
    }

    // 项目级 settings
    let project_path =
        Storage::new(std::env::current_dir().unwrap_or_default()).workspace_settings_path();
    match std::fs::read_to_string(&project_path) {
        Ok(content) => match parse_json_with_comments::<UserSettings>(&content) {
            Ok(_) => out.push_str(&format!(
                "✅ Project Config: parses at `{}`\n",
                project_path.display()
            )),
            Err(err) => out.push_str(&format!(
                "❌ Project Config: `{}` is NOT valid JSON ({}): project settings ignored\n",
                project_path.display(),
                err
            )),
        },
        Err(_) => out.push_str(&format!(
            "ℹ️ Project Config: `{}` not found\n",
            project_path.display()
        )),
    }

    // 生效的凭据：env 优先于文件，和 provider_resolution 的优先级一致
    match ProviderStore::new().load().await {
        Ok(config) => {
            let active = config.active_provider_id.clone();
            let api_key = active
                .as_deref()
                .and_then(|id| config.providers.get(id).and_then(|p| p.api_key.as_ref()));
            let base_url = active
                .as_deref()
                .and_then(|id| config.providers.get(id).and_then(|p| p.base_url.as_ref()));

            out.push_str(&format!(
                "- Active Provider: {}\n",
                active.as_deref().unwrap_or("<none>")
            ));
            if std::env::var("STAR_API_KEY").is_ok() {
                out.push_str("✅ API Key: STAR_API_KEY env var is set (overrides file)\n");
            } else if api_key.is_some() {
                out.push_str("✅ API Key: set in config file\n");
            } else {
                out.push_str("❌ API Key: not set — chat will fail with 'API key required'\n");
            }
            if std::env::var("STAR_BASE_URL").is_ok() {
                out.push_str("✅ Base URL: STAR_BASE_URL env var is set (overrides file)\n");
            } else if let Some(url) = base_url {
                if url == crate::core::config::providers::PLACEHOLDER_BASE_URL {
                    out.push_str("❌ Base URL: placeholder value in config — set STAR_BASE_URL or the provider base_url\n");
                } else {
                    out.push_str(&format!("✅ Base URL: {}\n", url));
                }
            } else {
                out.push_str("⚠️ Base URL: not set (provider default will be used, if any)\n");
            }
        }
        Err(err) => out.push_str(&format!("❌ Provider Config: failed to load ({})\n", err)),
    }

    out
}

/// 日志目录是否真的写得进去——is_log_enabled 只看开关，不验证落地。
fn render_logging_section() -> String {
    let mut out = String::new();
    if !crate::utils::logging::is_log_enabled() {
        out.push_str(
            "⚠️ Logging: disabled by STAR_LOG_ENABLED=0 — no diagnostics will be recorded\n",
        );
        return out;
    }
    let dir = crate::utils::logging::agent_log_path();
    let Some(parent) = dir.parent() else {
        out.push_str("⚠️ Logging: cannot resolve log directory\n");
        return out;
    };
    // 真写一次再删，比检查目录存在更可靠（只读挂载/权限不足时目录可能存在但写不了）
    let probe = parent.join(".doctor_write_probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            out.push_str(&format!(
                "✅ Logging: writable, agent log at `{}`\n",
                dir.display()
            ));
        }
        Err(err) => out.push_str(&format!(
            "❌ Logging: cannot write to `{}` ({}) — diagnostics are being lost\n",
            parent.display(),
            err
        )),
    }
    out
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
