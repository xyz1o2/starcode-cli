//! 动态工作流：Node 执行控制流，Rust 派发受权限门保护的 Agent 调用。
//!
//! 每次运行拥有独立目录，成功调用原子写入 journal；恢复创建新运行并复用匹配的
//! 调用结果，不修改来源运行。进程、取消和并发上限由宿主控制，脚本不持有 API key。

use super::{WorkflowInvocation, WorkflowProcessGroup};
use crate::core::tools::tools::{ToolError, ToolResult};
use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const MAX_AGENT_CALLS: usize = 64;
const RUNTIME: &str = include_str!("workflow_runtime.cjs");
type Backend = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

#[derive(Default, Deserialize, Serialize)]
struct Journal {
    version: u32,
    context: String,
    calls: BTreeMap<String, CachedCall>,
    status: String,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct CachedCall {
    key: String,
    value: Value,
}

fn digest(value: &Value) -> String {
    crate::core::utils::paths::sha256_hex(value.to_string().as_bytes())
}

async fn save_journal(root: &Path, journal: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?;
    let tmp = root.join(format!("journal-{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, root.join("journal.json"))
        .await
        .map_err(|e| format!("Failed to publish workflow journal: {e}"))
}

fn agent_arguments(message: &Value) -> Result<(String, String, Value, Option<Value>), String> {
    let prompt = message["prompt"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("agent prompt must be nonempty")?;
    let options = message["opts"]
        .as_object()
        .ok_or("agent options must be an object")?;
    for key in options.keys() {
        if ![
            "label",
            "phase",
            "model",
            "effort",
            "subagent_type",
            "max_rounds",
            "schema",
            "isolation",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unsupported agent option: {key}"));
        }
    }
    let label = options
        .get("label")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("agent requires a unique nonempty label")?
        .to_owned();
    let schema = options.get("schema").cloned();
    if let Some(schema) = &schema {
        jsonschema::validator_for(schema).map_err(|e| format!("Invalid agent schema: {e}"))?;
    }
    let mut arguments = json!({"prompt": prompt, "description": label, "background": false});
    for key in [
        "model",
        "effort",
        "subagent_type",
        "max_rounds",
        "isolation",
    ] {
        if let Some(value) = options.get(key) {
            arguments[key] = value.clone();
        }
    }
    let key = digest(&json!({"prompt": prompt, "options": options}));
    if let Some(schema) = &schema {
        arguments["prompt"] = Value::String(format!(
            "{prompt}\n\nReturn only a JSON value matching this schema:\n{schema}"
        ));
    }
    Ok((label, key, arguments, schema))
}

fn parse_agent_output(output: String, schema: Option<&Value>) -> Result<Value, String> {
    let Some(schema) = schema else {
        return Ok(Value::String(output));
    };
    let value: Value = serde_json::from_str(output.trim())
        .map_err(|e| format!("Agent did not return valid JSON: {e}"))?;
    let validator = jsonschema::validator_for(schema).map_err(|e| e.to_string())?;
    if let Err(error) = validator.validate(&value) {
        return Err(format!("Agent output does not match schema: {error}"));
    }
    Ok(value)
}

pub(super) async fn execute(
    invocation: &WorkflowInvocation,
    signal: Option<tokio_util::sync::CancellationToken>,
    update: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<ToolResult, String> {
    let params = &invocation.params;
    let config = invocation.config.clone();
    let client = invocation
        .parent_client
        .clone()
        .ok_or("Dynamic workflows require the calling agent client")?;
    let parent_effort = invocation.parent_effort.clone();
    let parent_identity = invocation.parent_identity.clone();
    let mut local_config = (*config).clone();
    local_config.agent_identity = invocation.parent_identity.clone();
    local_config.recursion_depth = local_config.recursion_depth.saturating_add(1);
    let parent_context = invocation.parent_context.clone().unwrap_or_default();
    let executor = Arc::new(crate::agent::tool_executor::ToolExecutor::new(
        client.clone(),
        config
            .runtime_tool_registry()
            .ok_or("Workflow Agent registry unavailable")?,
        config.runtime_message_bus(),
        Arc::new(local_config),
    ));
    executor.set_agent_parent_context(&parent_context);
    executor.set_agent_parent_client(&client);
    if let Some(effort) = invocation.parent_effort.clone() {
        executor.set_agent_parent_thinking_effort(effort);
    }
    let cancel = signal.unwrap_or_default().child_token();
    let backend_cancel = cancel.clone();
    let backend: Backend = Arc::new(move |arguments| {
        let executor = executor.clone();
        let cancel = backend_cancel.clone();
        Box::pin(async move {
            let call = crate::types::StarToolCall {
                id: format!("workflow-agent-{}", uuid::Uuid::new_v4()),
                call_type: "function".into(),
                function: crate::types::StarToolCallFunction {
                    name: "Agent".into(),
                    arguments: arguments.to_string(),
                },
            };
            let result = executor.execute(&call, None, Some(cancel)).await;
            if result.success {
                Ok(result.llm_content.or(result.output).unwrap_or_default())
            } else {
                Err(result
                    .error
                    .or(result.output)
                    .unwrap_or_else(|| "Workflow agent failed".into()))
            }
        })
    });
    let workflows = config.storage().project_workflows_dir();
    let source = match (&params.script, &params.workflow) {
        (Some(source), None) => source.clone(),
        (None, Some(name)) => {
            super::validate_workflow_name(name)?;
            tokio::fs::read_to_string(workflows.join(format!("{name}.js")))
                .await
                .map_err(|e| e.to_string())?
        }
        _ => return Err("Provide exactly one of script or workflow".into()),
    };
    let context = digest(
        &json!({"project": config.target_dir(), "model": client.model,
        "provider": client.provider_id, "endpoint": client.base_url, "effort": parent_effort,
        "script": crate::core::utils::paths::sha256_hex(source.as_bytes()), "args": params.args}),
    );
    run(
        &workflows.join("runs"),
        config.target_dir(),
        source,
        params.args.clone().unwrap_or(Value::Null),
        params.resume_from_run_id.as_deref(),
        context,
        params.max_concurrency.unwrap_or(4),
        params.timeout_secs.unwrap_or(300),
        cancel,
        backend,
        update,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run(
    runs: &Path,
    cwd: &Path,
    script: String,
    args: Value,
    resume: Option<&str>,
    context: String,
    concurrency: usize,
    timeout_secs: u64,
    cancel: tokio_util::sync::CancellationToken,
    backend: Backend,
    update: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<ToolResult, String> {
    if !(1..=16).contains(&concurrency) || !(1..=1800).contains(&timeout_secs) {
        return Err("max_concurrency must be 1..16 and timeout_secs must be 1..1800".into());
    }
    if script.len() > 524288 {
        return Err("Workflow script exceeds 512 KiB".into());
    }
    if cancel.is_cancelled() {
        return Err("Workflow cancelled before launch".into());
    }
    let mut journal = Journal {
        version: 1,
        context: context.clone(),
        status: "running".into(),
        ..Default::default()
    };
    if let Some(id) = resume {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid workflow run ID")?;
        let bytes = tokio::fs::read(runs.join(id).join("journal.json"))
            .await
            .map_err(|e| format!("Cannot read workflow journal: {e}"))?;
        let previous: Journal =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid workflow journal: {e}"))?;
        if previous.version != 1 {
            return Err("Unsupported workflow journal version".into());
        }
        if previous.context == context {
            journal.calls = previous.calls;
        }
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    let root = runs.join(&run_id);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(root.join("script.js"), &script)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(root.join("args.json"), args.to_string())
        .await
        .map_err(|e| e.to_string())?;
    save_journal(&root, &journal).await?;
    if let Some(update) = &update {
        update(format!("Workflow run: {run_id}"));
    }

    let result = drive(
        cwd,
        script,
        args,
        concurrency,
        timeout_secs,
        &cancel,
        backend,
        &root,
        &mut journal,
        update,
    )
    .await;
    // 失败时也保留已落盘的成功调用，后续恢复不会再执行它们。
    journal.status = if result.is_ok() {
        "completed"
    } else if cancel.is_cancelled() {
        "cancelled"
    } else {
        "failed"
    }
    .into();
    journal.error = result.as_ref().err().cloned();
    save_journal(&root, &journal).await?;
    let data = json!({"run_id": run_id, "status": journal.status, "result": result.as_ref().ok(), "journal": root.join("journal.json")});
    match result {
        Ok(value) => Ok(ToolResult {
            output: value.to_string(),
            llm_content: Some(data.to_string()),
            return_display: Some(format!("Workflow {run_id} completed")),
            error: None,
            data: Some(data),
        }),
        Err(error) => Ok(ToolResult {
            output: format!("Workflow {run_id} failed: {error}"),
            llm_content: Some(data.to_string()),
            return_display: None,
            error: Some(ToolError {
                error_type: "workflow_error".into(),
                message: error,
            }),
            data: Some(data),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive(
    cwd: &Path,
    script: String,
    args: Value,
    concurrency: usize,
    timeout_secs: u64,
    cancel: &tokio_util::sync::CancellationToken,
    backend: Backend,
    root: &Path,
    journal: &mut Journal,
    update: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<Value, String> {
    let mut command = tokio::process::Command::new("node");
    command
        .args(["--eval", RUNTIME])
        .current_dir(cwd)
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Dynamic workflows require Node.js on PATH: {e}"))?;
    let mut group = WorkflowProcessGroup {
        #[cfg(unix)]
        id: child.id().ok_or("Workflow child PID missing")?,
        armed: true,
    };
    let mut stdin = child.stdin.take().ok_or("Workflow stdin missing")?;
    let stdout = child.stdout.take().ok_or("Workflow stdout missing")?;
    let mut stderr = child.stderr.take().ok_or("Workflow stderr missing")?;
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        bytes
    });
    stdin
        .write_all(format!("{}\n", json!({"script":script,"args":args})).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(stdout).lines();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut calls = FuturesUnordered::<
        BoxFuture<'static, (Value, String, String, Result<Value, String>)>,
    >::new();
    let mut labels = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let execution = async {
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err("Workflow cancelled".into()),
                _ = tokio::time::sleep_until(deadline) => return Err("Workflow timed out".into()),
                completed = calls.next(), if !calls.is_empty() => {
                    let (id, label, key, result) = completed.ok_or("Workflow call disappeared")?;
                    if let Ok(value) = &result {
                        journal.calls.insert(label, CachedCall { key, value: value.clone() });
                        save_journal(root, journal).await?;
                    }
                    let response = match result { Ok(value) => json!({"id":id,"value":value}), Err(error) => json!({"id":id,"error":error}) };
                    stdin.write_all(format!("{response}\n").as_bytes()).await.map_err(|e| e.to_string())?;
                }
                line = lines.next_line() => {
                    let line = line.map_err(|e| e.to_string())?.ok_or("Workflow process ended without a result")?;
                    if line.len() > 4 * 1024 * 1024 { return Err("Workflow message exceeds 4 MiB".into()); }
                    let message: Value = serde_json::from_str(&line).map_err(|e| format!("Invalid workflow message: {e}"))?;
                    match message["type"].as_str() {
                        Some("agent") => {
                            let (label, key, arguments, schema) = agent_arguments(&message)?;
                            if !labels.insert(label.clone()) || labels.len() > MAX_AGENT_CALLS { return Err("Duplicate agent label or workflow exceeds 64 calls".into()); }
                            let id = message["id"].clone();
                            if let Some(cached) = journal.calls.get(&label).filter(|entry| entry.key == key) {
                                stdin.write_all(format!("{}\n", json!({"id":id,"value":cached.value})).as_bytes()).await.map_err(|e| e.to_string())?;
                            } else {
                                let backend = backend.clone(); let semaphore = semaphore.clone(); let cancellation = cancel.clone();
                                calls.push(Box::pin(async move {
                                    let result = async {
                                        let _permit = tokio::select! { _ = cancellation.cancelled() => return Err("Workflow cancelled".into()), permit = semaphore.acquire_owned() => permit.map_err(|e| e.to_string())? };
                                        if cancellation.is_cancelled() { return Err("Workflow cancelled".into()); }
                                        parse_agent_output(backend(arguments).await?, schema.as_ref())
                                    }.await;
                                    (id, label, key, result)
                                }));
                            }
                        }
                        Some("phase") | Some("log") => { if let Some(update) = &update { update(message.to_string()); } }
                        Some("result") => {
                            if !calls.is_empty() { return Err("Workflow returned with unfinished agents".into()); }
                            return Ok(message["value"].clone());
                        }
                        Some("error") => return Err(message["error"].as_str().unwrap_or("Workflow script failed").into()),
                        _ => return Err("Unsupported workflow message".into()),
                    }
                }
            }
        }
    }.await;
    if execution.is_err() {
        cancel.cancel();
    }
    // 取消下游后排空，确保 Agent 的实际循环已交接，不留下脱离工作流的写操作。
    while let Some((_id, label, key, result)) = calls.next().await {
        if let Ok(value) = result {
            journal.calls.insert(label, CachedCall { key, value });
            save_journal(root, journal).await?;
        }
    }
    drop(stdin);
    if execution.is_err() {
        drop(group);
        let _ = child.kill().await;
    } else {
        let status = child.wait().await.map_err(|e| e.to_string())?;
        group.armed = false;
        if !status.success() {
            return Err(format!("Workflow process exited with {status}"));
        }
    }
    let _ = stderr_task.await;
    execution
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn local_backend(calls: Arc<AtomicUsize>) -> Backend {
        Arc::new(move |arguments| {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(arguments["prompt"].as_str().unwrap_or_default().to_owned())
            })
        })
    }

    async fn run_script(
        runs: &Path,
        script: &str,
        resume: Option<&str>,
        backend: Backend,
    ) -> ToolResult {
        run(
            runs,
            runs,
            script.to_owned(),
            json!({"key": "value"}),
            resume,
            "test-context".into(),
            2,
            10,
            tokio_util::sync::CancellationToken::new(),
            backend,
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn agent_parallel_pipeline_and_resume_reuse_successful_calls() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let calls = Arc::new(AtomicUsize::new(0));
        let script = r#"
export const meta = { name: "local", description: "test" };
const values = await parallel([
  () => agent("first", { label: "first" }),
  () => agent("second", { label: "second" }),
]);
const output = await pipeline(values, async (value) => value.toUpperCase());
return { values, output, args };
"#;
        let first = run_script(&runs, script, None, local_backend(calls.clone())).await;
        assert!(first.error.is_none(), "{:?}", first.error);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let data = first.data.unwrap();
        let run_id = data["run_id"].as_str().unwrap();
        let journal_path = runs.join(run_id).join("journal.json");
        assert!(journal_path.exists());

        let resumed = run_script(&runs, script, Some(run_id), local_backend(calls.clone())).await;
        assert!(resumed.error.is_none(), "{:?}", resumed.error);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "resume must not repeat successful agent calls"
        );
        let result: Value = serde_json::from_str(&resumed.output).unwrap();
        assert_eq!(result["output"], json!(["FIRST", "SECOND"]));
        assert_eq!(result["args"]["key"], "value");
    }

    #[tokio::test]
    async fn changed_script_does_not_reuse_old_journal_entries() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let calls = Arc::new(AtomicUsize::new(0));
        let first = run_script(
            &runs,
            r#"const value = await agent("old", { label: "same" }); return value;"#,
            None,
            local_backend(calls.clone()),
        )
        .await;
        let run_id = first.data.unwrap()["run_id"].as_str().unwrap().to_owned();
        let next = run_script(
            &runs,
            r#"const value = await agent("new", { label: "same" }); return value;"#,
            Some(&run_id),
            local_backend(calls.clone()),
        )
        .await;
        assert!(next.error.is_none(), "{:?}", next.error);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(next.output, "\"new\"");
    }

    #[test]
    fn agent_options_require_stable_labels_and_reject_unknown_fields() {
        let missing = json!({"type":"agent", "prompt":"x", "opts":{}});
        assert!(agent_arguments(&missing).unwrap_err().contains("label"));
        let unknown = json!({"type":"agent", "prompt":"x", "opts":{"label":"x", "shell":"rm"}});
        assert!(agent_arguments(&unknown)
            .unwrap_err()
            .contains("Unsupported"));
    }
}
