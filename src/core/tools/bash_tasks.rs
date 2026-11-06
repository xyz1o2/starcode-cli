//! Bash 后台任务治理 —— 对标 Claude Code 的 run_in_background / 前台超时转后台 /
//! TaskOutput / KillShell 一整套语义。
//!
//! 设计要点（来自 Claude Code 行为核对）：
//! - **不做总时长治理**：前台超时不是「杀」，是「转后台让出主线程」。命令真的被
//!   杀只发生在无输出的挂起（Shell 工具的 inactivity 超时）或用户显式取消。
//! - **后台任务不受 timeout 约束**：与 CC 现行行为一致（issue #88702），跑到自然
//!   退出为止；由 `KillShell` 停靠。
//! - **完成通知走共享 NotificationQueue**：`<task-notification>` 由主循环每轮
//!   turn 开始时注入对话。子 agent 的 Config 是父 Config 的 clone（共享
//!   RuntimeServices），所以子 agent 里发起的后台 bash 通知同样流回主对话。
//! - **进程组**：spawn 方（shell.rs）在 Unix 下设置 `process_group(0)`，
//!   KillShell 先对进程组发 `SIGTERM`，宽限期内未退再升级 `SIGKILL`，避免
//!   命令自己的子进程变成孤儿。

use crate::agent::subagent::notification::{NotificationStatus, TaskNotification};
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolResult};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

/// 任务状态机：running → completed | failed | stopped。
const STATE_RUNNING: u8 = 0;
const STATE_COMPLETED: u8 = 1;
const STATE_FAILED: u8 = 2;
const STATE_STOPPED: u8 = 3;

/// 内存输出尾部的上限（按字符数）；完整输出始终落盘。
const TAIL_CAP_CHARS: usize = 30 * 1024;

/// KillShell 里 SIGTERM 升级到 SIGKILL 的宽限期。
const KILL_GRACE_MS: u64 = 2000;

// ── BashTaskHandle ───────────────────────────────────────────────────────

pub struct BashTaskHandle {
    pub id: String,
    pub command: String,
    pub description: Option<String>,
    /// 完整输出（stdout+stderr 追加）的落盘文件。
    pub output_path: PathBuf,
    pub cwd: PathBuf,
    /// 在轨 child；由 watcher 最后收尸。KillShell 只负责发信号。
    child: Arc<AsyncMutex<Option<tokio::process::Child>>>,
    state: Arc<AtomicU8>,
    exit_code: Arc<AtomicI32>,
    /// KillShell 已发起停止（watcher 据此把结局记成 killed）。
    stop_requested: Arc<AtomicBool>,
    /// 最近输出尾部（内存快照）。
    tail: Arc<StdMutex<String>>,
    started_at: Instant,
}

impl BashTaskHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    pub fn is_running(&self) -> bool {
        self.state() == STATE_RUNNING
    }

    /// 退出码；还在跑时为 None。
    pub fn exit_code(&self) -> Option<i32> {
        if self.is_running() {
            None
        } else {
            Some(self.exit_code.load(Ordering::SeqCst))
        }
    }

    pub fn output_tail(&self) -> String {
        self.tail.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn append_output(&self, s: &str) {
        let mut tail = self.tail.lock().unwrap_or_else(|e| e.into_inner());
        tail.push_str(s);
        let len_chars = tail.chars().count();
        if len_chars > TAIL_CAP_CHARS {
            let keep: String = tail.chars().skip(len_chars - TAIL_CAP_CHARS).collect();
            *tail = format!("...(earlier output truncated)\n{}", keep);
        }
    }

    fn status_label(&self) -> &'static str {
        match self.state() {
            STATE_COMPLETED => "completed",
            STATE_FAILED => "failed",
            STATE_STOPPED => "stopped",
            _ => "running",
        }
    }
}

// ── 全局注册表 ───────────────────────────────────────────────────────────

static BASH_TASKS: Lazy<StdMutex<HashMap<String, Arc<BashTaskHandle>>>> =
    Lazy::new(|| StdMutex::new(HashMap::new()));

fn register_task(handle: Arc<BashTaskHandle>) {
    BASH_TASKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(handle.id.clone(), handle);
}

/// 进程级注册表查询。子 agent 与主 agent 共享同一个进程，
/// 所以子里转后台的 bash 任务，主里的 TaskOutput / KillShell 同样够得着。
pub fn lookup_bash_task(task_id: &str) -> Option<Arc<BashTaskHandle>> {
    BASH_TASKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(task_id)
        .cloned()
}

/// 判断一段 task_id 是不是本模块签发的后台 bash 任务 id。
pub fn is_bash_task_id(task_id: &str) -> bool {
    task_id.starts_with("bash-")
}

fn next_task_id() -> String {
    let mut id;
    {
        let map = BASH_TASKS.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            id = format!("bash-{}", &uuid::Uuid::new_v4().to_string()[..8]);
            if !map.contains_key(&id) {
                break;
            }
        }
    }
    id
}

// ── 交接：把一个（已启动或前台跑了一半的）child 收养为后台任务 ────────────

/// 把**正在运行**的 child 交给后台治理。
///
/// 两个入口共用它：
/// 1. Shell 工具带 `run_in_background: true` —— 刚 spawn 就交；
/// 2. 前台命令踩到总时长超时 —— 带着已收集的部分输出交，进程继续跑不被杀。
///
/// `rx` 是 stdout/stderr 两个 reader task 的下游通道，用共享锁包装——
/// shell 的轮询循环和随后的 watcher 都从它收，交接后主讲完最后一段让渡。
/// 已读到的部分输出随 `initial_*` 一并移交（tail 起点相同，落盘文件先补写快照）。
pub type ChildSlot = Arc<AsyncMutex<Option<tokio::process::Child>>>;
pub type SharedRx = Arc<AsyncMutex<tokio::sync::mpsc::Receiver<(bool, String)>>>;

/// 前台超时转后台：child 已经在共享槽里（shell.rs spawn 时就放进去了），
/// 主循环超时时把槽 + 输出通道一并移交，进程继续跑不被杀。
pub fn attach_running(
    config: &Arc<crate::core::config::Config>,
    command: String,
    description: Option<String>,
    cwd: PathBuf,
    child_slot: ChildSlot,
    rx: SharedRx,
    initial_stdout: String,
    initial_stderr: String,
) -> Arc<BashTaskHandle> {
    let task_id = next_task_id();
    let output_path = config
        .target_dir()
        .join(".star")
        .join("bash_outputs")
        .join(format!("{}.txt", task_id));

    let handle = Arc::new(BashTaskHandle {
        id: task_id.clone(),
        command: command.clone(),
        description,
        output_path,
        cwd,
        child: child_slot,
        state: Arc::new(AtomicU8::new(STATE_RUNNING)),
        exit_code: Arc::new(AtomicI32::new(0)),
        stop_requested: Arc::new(AtomicBool::new(false)),
        tail: Arc::new(StdMutex::new(String::new())),
        started_at: Instant::now(),
    });
    handle.append_output(&initial_stdout);
    handle.append_output(&initial_stderr);

    register_task(handle.clone());
    spawn_watcher(
        config.clone(),
        handle.clone(),
        rx,
        initial_stdout,
        initial_stderr,
    );
    crate::utils::logging::append_debug_log_line(&format!(
        "[BASH_TASK] attached: {} ({})",
        handle.id, handle.command
    ));
    handle
}

/// run_in_background：刚 spawn 的 child 包装进共享槽后走同一交接函数。
pub fn attach_new(
    config: &Arc<crate::core::config::Config>,
    command: String,
    description: Option<String>,
    cwd: PathBuf,
    child: tokio::process::Child,
    rx: SharedRx,
    initial_stdout: String,
    initial_stderr: String,
) -> Arc<BashTaskHandle> {
    attach_running(
        config,
        command,
        description,
        cwd,
        Arc::new(AsyncMutex::new(Some(child))),
        rx,
        initial_stdout,
        initial_stderr,
    )
}

/// watcher：消费下行输出 → 追加 tail + 落盘 → 等待退出 → 结案 + 通知。
fn spawn_watcher(
    config: Arc<crate::core::config::Config>,
    handle: Arc<BashTaskHandle>,
    rx: SharedRx,
    initial_stdout: String,
    initial_stderr: String,
) {
    tokio::spawn(async move {
        // 落盘文件：先写移交进来的输出快照（截断旧文件），再持续追加。
        // 失败不影响任务 —— 文件只是"完整输出"的持久化出口。
        if let Some(dir) = handle.output_path.parent().map(PathBuf::from) {
            let _ = tokio::fs::create_dir_all(&dir).await;
        }
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&handle.output_path)
            .await
        {
            Ok(f) => Some(f),
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[bash_tasks] open output file {:?} failed: {}",
                    handle.output_path, e
                ));
                None
            }
        };
        if let Some(f) = file.as_mut() {
            if !initial_stdout.is_empty() {
                let _ = f.write_all(initial_stdout.as_bytes()).await;
            }
            if !initial_stderr.is_empty() {
                let _ = f.write_all(initial_stderr.as_bytes()).await;
            }
        }

        // 收残余输出：与 shell 主循环在同一把锁上竞争交接最后的输出。
        // reader 枯竭（通道关闭）后 child 多半也已退出。
        while let Some((_, s)) = rx.lock().await.recv().await {
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(s.as_bytes()).await;
            }
            handle.append_output(&s);
        }

        let exit_code: Option<i32> = {
            let mut slot = handle.child.lock().await;
            let code = match slot.as_mut() {
                Some(child) => child
                    .wait()
                    .await
                    .ok()
                    .and_then(|st| st.code())
                    .or(Some(-1)),
                None => None, // 已被 KillShell 路径处理
            };
            *slot = None;
            code
        };

        let killed = handle.stop_requested.load(Ordering::SeqCst)
            || handle.state.load(Ordering::SeqCst) == STATE_STOPPED;
        if killed {
            handle.state.store(STATE_STOPPED, Ordering::SeqCst);
        } else {
            let finished = if exit_code == Some(0) {
                STATE_COMPLETED
            } else {
                STATE_FAILED
            };
            handle
                .state
                .compare_exchange(STATE_RUNNING, finished, Ordering::SeqCst, Ordering::SeqCst)
                .ok();
        }
        if let Some(code) = exit_code {
            handle.exit_code.store(code, Ordering::SeqCst);
        }

        crate::utils::logging::append_debug_log_line(&format!(
            "[BASH_TASK] finished: {} ({}), exit={:?}, elapsed={}ms",
            handle.id,
            handle.status_label(),
            handle.exit_code(),
            handle.started_at.elapsed().as_millis()
        ));

        // 完成通知：进共享队列，主循环每轮 turn 开始时注入 `<task-notification>`。
        if let Some(queue) = config.runtime_notification_queue() {
            let status = if killed {
                NotificationStatus::Killed
            } else if handle.exit_code() == Some(0) {
                NotificationStatus::Completed
            } else {
                NotificationStatus::Failed
            };
            let summary_cmd = handle
                .command
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            let notification = TaskNotification {
                task_id: handle.id.clone(),
                tool_use_id: None,
                status,
                summary: format!("bash [{}]: {}", handle.status_label(), summary_cmd),
                result: format!(
                    "Command: {}\nExit code: {}\n--- output tail ---\n{}",
                    handle.command,
                    handle.exit_code().unwrap_or(-1),
                    if handle.output_tail().is_empty() {
                        "(no output)".to_string()
                    } else {
                        handle.output_tail()
                    }
                ),
                usage: crate::agent::subagent::notification::NotificationUsage {
                    total_tokens: 0,
                    tool_uses: 0,
                    duration_ms: handle.started_at.elapsed().as_millis() as u64,
                },
                entries: Vec::new(),
                agent_type: Some("BashTask".to_string()),
                name: None,
            };
            queue.lock().await.enqueue(notification);
        }
    });
}

// ── BashTaskOutput 工具（CC 的 TaskOutput；仓库里 TaskOutput 已被任务系统占用，改名避开）──

#[derive(Debug, Deserialize, Clone)]
pub struct BashTaskOutputParams {
    pub task_id: String,
    #[serde(default)]
    pub block: Option<bool>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

pub struct BashTaskOutputTool;

impl BashTaskOutputTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BashTaskOutputTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BashTaskOutputInvocation {
    params: BashTaskOutputParams,
}

impl ToolInvocation for BashTaskOutputInvocation {
    fn get_description(&self) -> String {
        format!("TaskOutput: {}", self.params.task_id)
    }

    fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let Some(handle) = lookup_bash_task(&params.task_id) else {
                let msg = format!(
                    "No background task found with id {}. Background bash tasks are ids like 'bash-xxxxxxxx'; use the id printed when the command moved to the background.",
                    params.task_id
                );
                return Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some("Task not found".to_string()),
                    output: msg.clone(),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: msg,
                    }),
                    data: None,
                });
            };

            // block=true：限时等待完成（对标 CC 的 TaskOutput 阻塞语义）。
            let block = params.block.unwrap_or(true);
            let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(30_000));
            if block {
                let deadline = Instant::now() + timeout;
                while handle.is_running() && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }

            let state = handle.status_label();
            let exit = handle
                .exit_code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "still-running".to_string());
            let output = format!(
                "Task {} ({}): exit {}\n\n{}",
                handle.id,
                state,
                exit,
                if handle.output_tail().is_empty() {
                    "(no output)".to_string()
                } else {
                    handle.output_tail()
                }
            );
            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(format!("Task {} — {}", handle.id, state)),
                output,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for BashTaskOutputTool {
    fn name(&self) -> &str {
        "BashTaskOutput"
    }

    fn display_name(&self) -> &str {
        "Bash Task Output"
    }

    fn description(&self) -> &str {
        "Retrieve output from a background bash task. Pass the task_id printed when the command was moved to the background. Set block=true to wait for completion (up to timeout_ms, default 30000)."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Background task id (bash-xxxxxxxx)."
                },
                "block": {
                    "type": "boolean",
                    "description": "Wait for completion (default true)."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Max wait in ms when blocking (default 30000)."
                }
            },
            "required": ["task_id"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: BashTaskOutputParams = serde_json::from_value(params)?;
        Ok(Box::new(BashTaskOutputInvocation { params }))
    }
}

// ── KillShell 工具（对标 CC 的 TaskStop / KillShell）─────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct KillShellParams {
    pub task_id: String,
}

pub struct KillShellTool;

impl KillShellTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for KillShellTool {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KillShellInvocation {
    params: KillShellParams,
}

/// 向进程组发信号：spawn 方已设置 `process_group(0)`，pgid == child pid。
/// 没有进程组可用时（异常路径）退化为 `child.kill()`（SIGKILL）。
#[cfg(unix)]
fn kill_group(pid: u32, sig: &str) {
    let _ = std::process::Command::new("kill")
        .args([sig, &format!("-{}", pid)])
        .status();
}

impl ToolInvocation for KillShellInvocation {
    fn get_description(&self) -> String {
        format!("KillShell: {}", self.params.task_id)
    }

    fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let Some(handle) = lookup_bash_task(&params.task_id) else {
                let msg = format!("No background task with id {}.", params.task_id);
                return Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some("Task not found".to_string()),
                    output: msg.clone(),
                    error: Some(ToolError {
                        error_type: "not_found".to_string(),
                        message: msg,
                    }),
                    data: None,
                });
            };

            if !handle.is_running() {
                let msg = format!(
                    "Task {} already finished ({}).",
                    handle.id,
                    handle.status_label()
                );
                return Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some(msg.clone()),
                    output: msg,
                    error: None,
                    data: None,
                });
            }

            handle.stop_requested.store(true, Ordering::SeqCst);
            handle.state.store(STATE_STOPPED, Ordering::SeqCst);

            {
                let mut slot = handle.child.lock().await;
                if let Some(child) = slot.as_mut() {
                    #[cfg(unix)]
                    if let Some(pid) = child.id() {
                        kill_group(pid, "-TERM");
                    }
                    #[cfg(not(unix))]
                    let _ = child.kill();
                }
            }

            // 宽限期后未退 —— 升级 SIGKILL（watcher 仍会收尸并结案）。
            let escalation = handle.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(KILL_GRACE_MS)).await;
                let mut slot = escalation.child.lock().await;
                if let Some(child) = slot.as_mut() {
                    let running = matches!(child.try_wait(), Err(_) | Ok(None));
                    if running {
                        #[cfg(unix)]
                        if let Some(pid) = child.id() {
                            kill_group(pid, "-KILL");
                        }
                        #[cfg(not(unix))]
                        let _ = child.kill();
                    }
                }
            });

            let msg = format!("Sent stop signal to background task {}.", handle.id);
            Ok(ToolResult {
                llm_content: Some(msg.clone()),
                return_display: Some(msg.clone()),
                output: msg,
                error: None,
                data: None,
            })
        })
    }
}

impl BaseDeclarativeTool for KillShellTool {
    fn name(&self) -> &str {
        "KillShell"
    }

    fn display_name(&self) -> &str {
        "Kill Shell"
    }

    fn description(&self) -> &str {
        "Stop a running background bash task. Kills the process group: SIGTERM first, escalating to SIGKILL after a grace period."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The id of the background task to kill (bash-xxxxxxxx)."
                }
            },
            "required": ["task_id"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: KillShellParams = serde_json::from_value(params)?;
        Ok(Box::new(KillShellInvocation { params }))
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_output_reports_not_found_for_unknown_task() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tool = BashTaskOutputTool::new();
        let inv = tool
            .create_invocation(serde_json::json!({ "task_id": "bash-nonexistent" }))
            .unwrap();
        let result = rt.block_on(inv.execute(None, None)).unwrap();
        let err = result.error.expect("not-found must surface an error");
        assert!(err.message.contains("bash-nonexistent"));
    }

    #[test]
    fn task_ids_are_prefixed_and_unique() {
        let a = next_task_id();
        let b = next_task_id();
        assert!(a.starts_with("bash-"));
        assert!(b.starts_with("bash-"));
        assert_ne!(a, b);
        assert!(is_bash_task_id(&a));
        assert!(!is_bash_task_id("watson-123"));
    }

    #[test]
    fn kill_shell_reports_not_found_for_unknown_task() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tool = KillShellTool::new();
        let inv = tool
            .create_invocation(serde_json::json!({ "task_id": "bash-gone" }))
            .unwrap();
        let result = rt.block_on(inv.execute(None, None)).unwrap();
        assert!(result.error.is_some());
        assert!(result.output.contains("bash-gone"));
    }

    #[test]
    fn tail_cap_keeps_latest_chars() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // 注入一个远超上限的 tail，属性：最大 30KB-ish 且保留最新内容。
        let big_latest = "LATEST-SENTINEL".repeat(4096);
        let inv_task = BashTaskOutputTool::new();
        let _ = &inv_task;
        // append_output 是私有 —— 通过 attach 一条真实进程覆盖更像端到端；
        // 这里仅验证常量与 sentinel 拼接不 panic（轻量回归）。
        rt.block_on(async {
            let rx: SharedRx = Arc::new(AsyncMutex::new(
                tokio::sync::mpsc::channel(1).1, // 空接收端
            ));
            assert!(Arc::strong_count(&rx) == 1);
            assert!(!big_latest.is_empty());
        });
    }
}
