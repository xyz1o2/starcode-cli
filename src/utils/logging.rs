use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::core::utils::paths::current_project_star_dir;

pub fn is_log_enabled() -> bool {
    static LOG_ENABLED: OnceLock<bool> = OnceLock::new();
    *LOG_ENABLED.get_or_init(|| {
        std::env::var("STAR_LOG_ENABLED")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    })
}

fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..16 {
        if dir.join("Cargo.toml").is_file() || dir.join(".git").is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn log_dir() -> PathBuf {
    static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
    LOG_DIR
        .get_or_init(|| {
            if let Ok(p) = std::env::var("STAR_LOG_DIR") {
                let p = p.trim();
                if !p.is_empty() {
                    return PathBuf::from(p);
                }
            }
            if let Some(project_root) = find_project_root() {
                return project_root.join(".star").join("logs");
            }
            current_project_star_dir().join("logs")
        })
        .clone()
}

pub fn debug_log_path() -> PathBuf {
    static DEBUG_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
    DEBUG_LOG_PATH
        .get_or_init(|| log_dir().join("starcode_debug.log"))
        .clone()
}

pub fn debug_log_path_display() -> String {
    debug_log_path().to_string_lossy().to_string()
}

pub fn agent_log_path() -> PathBuf {
    static AGENT_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
    AGENT_LOG_PATH
        .get_or_init(|| log_dir().join("agent.log"))
        .clone()
}

pub fn agent_log_path_display() -> String {
    agent_log_path().to_string_lossy().to_string()
}

pub fn is_log_stderr_enabled() -> bool {
    static LOG_STDERR_ENABLED: OnceLock<bool> = OnceLock::new();
    *LOG_STDERR_ENABLED.get_or_init(|| {
        std::env::var("STAR_LOG_STDERR")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(false)
    })
}

pub fn is_verbose_debug_logging_enabled() -> bool {
    static VERBOSE_DEBUG_LOGGING_ENABLED: OnceLock<bool> = OnceLock::new();
    *VERBOSE_DEBUG_LOGGING_ENABLED.get_or_init(|| {
        for key in ["STAR_VERBOSE_DEBUG_LOG", "STAR_VERBOSE_LOGGING"] {
            if let Ok(v) = std::env::var(key) {
                let v = v.to_lowercase();
                return !(v == "0" || v == "false" || v == "off");
            }
        }
        false
    })
}

fn idle_worker_log_enabled() -> bool {
    static IDLE_WORKER_LOG_ENABLED: OnceLock<bool> = OnceLock::new();
    *IDLE_WORKER_LOG_ENABLED.get_or_init(|| {
        std::env::var("STAR_LOG_IDLE_WORKER")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(false)
    })
}

fn should_skip_debug_log_line(line: &str) -> bool {
    if line.starts_with("[DEBUG]") && !is_verbose_debug_logging_enabled() {
        return true;
    }

    if line == "[Worker] Waiting for message on rx..." {
        return !idle_worker_log_enabled();
    }

    false
}

struct LogWriters {
    debug: Mutex<Option<File>>,
    agent: Mutex<Option<File>>,
}

fn log_writers() -> &'static LogWriters {
    static WRITERS: OnceLock<LogWriters> = OnceLock::new();
    WRITERS.get_or_init(|| LogWriters {
        debug: Mutex::new(None),
        agent: Mutex::new(None),
    })
}

fn append_log_line(path: PathBuf, slot: &Mutex<Option<File>>, line: &str) {
    if let Ok(mut file_guard) = slot.lock() {
        if file_guard.is_none() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let opened = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);
            let Ok(file) = opened else {
                return;
            };
            *file_guard = Some(file);
        }

        let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        if let Some(file) = file_guard.as_mut() {
            let _ = writeln!(file, "[{}] {}", ts, line);
        }
    }
}

pub fn append_debug_log_line(line: &str) {
    if !is_log_enabled() || should_skip_debug_log_line(line) {
        return;
    }
    append_log_line(debug_log_path(), &log_writers().debug, line);
}

pub fn append_agent_log_line(line: &str) {
    if !is_log_enabled() {
        return;
    }
    append_log_line(agent_log_path(), &log_writers().agent, line);
}

// ============ 增强的日志功能 - 任务追踪和调试 ============

/// 生成任务 ID
pub fn generate_task_id() -> String {
    format!("task_{}", chrono::Local::now().format("%Y%m%d_%H%M%S_%3f"))
}

/// 记录任务启动
pub fn log_task_start(task_id: &str, description: &str) {
    let msg = format!("🚀 TASK START [{}]: {}", task_id, description);
    append_debug_log_line(&msg);
}

/// 记录任务完成
pub fn log_task_end(task_id: &str, success: bool) {
    let status = if success { "✓ SUCCESS" } else { "✗ FAILED" };
    let msg = format!("🏁 TASK END [{}]: {}", task_id, status);
    append_debug_log_line(&msg);
}

/// 记录 LLM 请求
pub fn log_llm_request(model: &str, msg_count: usize, tool_count: usize) {
    let msg = format!(
        "🤖 LLM REQUEST: {} ({} messages, {} tools)",
        model, msg_count, tool_count
    );
    append_debug_log_line(&msg);
}

/// 记录 LLM 响应
pub fn log_llm_response(finish_reason: &str, tokens: usize) {
    let msg = format!(
        "🤖 LLM RESPONSE: finish_reason={}, {} tokens",
        finish_reason, tokens
    );
    append_debug_log_line(&msg);
}

/// 记录工具调用
pub fn log_tool_call(tool_name: &str, args: &str) {
    // 按字符截断：参数里带中文（搜索词、写入内容）时字节切片会 panic —— 而这行日志
    // 每次工具调用都会走。
    let preview = crate::utils::string_utils::truncate_with_ellipsis(args, 100);
    let msg = format!("🔧 TOOL CALL: {} | {}", tool_name, preview);
    append_debug_log_line(&msg);
}

/// 记录工具执行结果
pub fn log_tool_result(tool_name: &str, success: bool, output_len: usize) {
    let status = if success { "✓" } else { "✗" };
    let msg = format!(
        "{} TOOL RESULT [{}]: {} bytes",
        status, tool_name, output_len
    );
    append_debug_log_line(&msg);
}

/// 记录循环检测
pub fn log_loop_detection(loop_type: &str, iteration: u32) {
    let msg = format!("🔄 LOOP DETECTION: {} (iter {})", loop_type, iteration);
    append_debug_log_line(&msg);
}

/// 记录阶段进展
pub fn log_phase(phase: &str, status: &str) {
    let msg = format!("📍 [{}] {}", phase, status);
    append_debug_log_line(&msg);
}
