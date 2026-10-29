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

// ============ 容量治理：轮转 + 保留策略 ============
//
// 两次实测教训让这段必须有：
// - watcher 空转 bug 把单文件 `starcode_debug.log` 写到了 2.5 GB
//   （2429 万行），磁盘被一个进程悄悄吃满；
// - Claude Code 的形态是「按会话拆文件 + cleanupPeriodDays 兜底回收」，
//   单文件 -- 对一个常驻 CLI 来说，**必须有硬上限**。
//
// 方案：大小轮转（`log` → `.1` → `.2` → ... → `.N` 链，N 之外丢弃）+
// 启动时按保留天数清理旧文件（`STAR_LOG_RETENTION_DAYS`，默认 30，
// 设 0 关闭清理）。链长 × 单文件上限给出磁盘占用硬顶：debug 日志
// 8 MB × 5 ≈ 40 MB，agent 日志 4 MB × 5 ≈ 20 MB，与当年 2.5 GB 的
// 量级差 60 倍。

/// `starcode_debug.log` 单文件上限。
const DEBUG_LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// `agent.log` 单文件上限（内容更稀疏，给小一点）。
const AGENT_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 轮转链长度：保留 `.1` ~ `.N` 共 N 份历史。
const ROTATE_KEEP: u32 = 4;
/// 保留天数默认值（`STAR_LOG_RETENTION_DAYS=0` 关闭清理）。
const DEFAULT_LOG_RETENTION_DAYS: u64 = 30;

/// 每行写出的路径长度。`[2026-10-29 16:22:01.791] ` + 换行，取个整。
const LINE_OVERHEAD_BYTES: u64 = 32;

/// 轮转链的成员名：`starcode_debug.log` → `starcode_debug.log.1`。
fn rotated_name(path: &std::path::Path, n: u32) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}", n));
    PathBuf::from(name)
}

/// 链式改名：删掉最老的 `.N`，其余依次后移，最后当前文件接上 `.1`。
///
/// 多进程同时rotate 是允许的竞争（两个进程都在写同一项目日志时）：
/// Windows 上持由句柄重命名可能失败 —— 失败就什么都不动，主文件继续
/// 下一次写入时按「继承的大小再超一次阈值」重试。宁可迟到，不丢数据。
fn rotate_chain(path: &std::path::Path, keep: u32) {
    let _ = std::fs::remove_file(rotated_name(path, keep));
    for i in (1..keep).rev() {
        let from = rotated_name(path, i);
        let to = rotated_name(path, i + 1);
        let _ = std::fs::remove_file(&to);
        if std::fs::rename(&from, &to).is_err() {
            // 历史少一份不致命
        }
    }
    let first = rotated_name(path, 1);
    let _ = std::fs::remove_file(&first);
    let _ = std::fs::rename(path, &first);
}

/// 启动时一次性清理过期日志（对标 Claude Code 的 `cleanupPeriodDays`）。
///
/// 只按**文件名前缀 + mtime**收走写出过的东西：`starcode_debug.log*`、
/// `agent.log*`。死项目目录里 30 天前最后的日志也一起收走 —— 主文件
/// 同样算：正在跑的进程绝不可能让主文件 30 天不写入，不会误伤活会话。
fn cleanup_expired_logs_once() {
    static DONE: OnceLock<()> = OnceLock::new();
    if DONE.set(()).is_err() {
        return;
    }
    let days: u64 = std::env::var("STAR_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_LOG_RETENTION_DAYS);
    if days == 0 {
        return;
    }
    let Some(cutoff) = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days * 24 * 3600))
    else {
        return;
    };
    let dir = log_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_log_chain = name.starts_with("starcode_debug.log") || name.starts_with("agent.log");
        if !is_log_chain {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if let Ok(modified) = meta.modified() {
            if modified < cutoff {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// 一个日志槽位：懒打开的文件句柄 + 自打开以来的写入量。
struct LogSlot {
    file: Option<File>,
    /// 到阈值触发轮转。**新会话打开文件时继承磁盘上现有大小** ——
    /// 上一次会话留下的巨型文件,哪怕一字节没写也过阈值,绝不让
    /// "旧账清不清得掉"取决于进程活得久不久。
    bytes_written: u64,
}

impl LogSlot {
    fn fresh() -> Self {
        Self {
            file: None,
            bytes_written: 0,
        }
    }
}

struct LogWriters {
    debug: Mutex<LogSlot>,
    agent: Mutex<LogSlot>,
}

fn log_writers() -> &'static LogWriters {
    static WRITERS: OnceLock<LogWriters> = OnceLock::new();
    WRITERS.get_or_init(|| {
        // 在任何文件被本进程打开之前做保留清理:既不和自己持有的句柄
        // 打架,也把「清理时刚好有个活进程在写」的窗口压到零。
        cleanup_expired_logs_once();
        LogWriters {
            debug: Mutex::new(LogSlot::fresh()),
            agent: Mutex::new(LogSlot::fresh()),
        }
    })
}

fn append_log_line(path: PathBuf, slot: &Mutex<LogSlot>, line: &str, max_bytes: u64) {
    if let Ok(mut slot_guard) = slot.lock() {
        if slot_guard.file.is_none() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let disk_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let opened = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);
            let Ok(file) = opened else {
                return;
            };
            slot_guard.file = Some(file);
            slot_guard.bytes_written = disk_size;
        }

        if let Some(file) = slot_guard.file.as_mut() {
            let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(file, "[{}] {}", ts, line);
        }
        slot_guard.bytes_written += line.len() as u64 + LINE_OVERHEAD_BYTES;

        if slot_guard.bytes_written >= max_bytes {
            // 到点轮转。先放手再改名:Windows/FAT/网络盘上,持句柄重命名
            // 并不总被允许。轮转失败时旧文件原位不动,下一次写入的
            // 「继承磁盘大小」逻辑会再在阈值上踩一脚,自动重试。
            slot_guard.file = None;
            rotate_chain(&path, ROTATE_KEEP);
            // 不在这里重开:懒打开的下一次调用自然接管。成功轮转后
            // 新文件为空,继承到 size 0。
        }
    }
}

pub fn append_debug_log_line(line: &str) {
    if !is_log_enabled() || should_skip_debug_log_line(line) {
        return;
    }
    append_log_line(debug_log_path(), &log_writers().debug, line, DEBUG_LOG_MAX_BYTES);
}

pub fn append_agent_log_line(line: &str) {
    if !is_log_enabled() {
        return;
    }
    append_log_line(agent_log_path(), &log_writers().agent, line, AGENT_LOG_MAX_BYTES);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_name_appends_suffix() {
        let p = std::path::Path::new("logs/starcode_debug.log");
        assert_eq!(
            rotated_name(p, 2),
            std::path::PathBuf::from("logs/starcode_debug.log.2")
        );
    }

    #[test]
    fn rotate_chain_shifts_history_and_drops_oldest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let main = temp.path().join("t.log");
        std::fs::write(&main, "current").unwrap();
        std::fs::write(temp.path().join("t.log.1"), "old-1").unwrap();
        std::fs::write(temp.path().join("t.log.2"), "old-2").unwrap();
        std::fs::write(temp.path().join("t.log.3"), "oldest").unwrap();

        rotate_chain(&main, 3);

        assert!(!main.exists(), "current 应改名为 .1");
        assert_eq!(std::fs::read_to_string(temp.path().join("t.log.1")).unwrap(), "current");
        assert_eq!(std::fs::read_to_string(temp.path().join("t.log.2")).unwrap(), "old-1");
        assert_eq!(std::fs::read_to_string(temp.path().join("t.log.3")).unwrap(), "old-2");
        assert!(
            !temp.path().join("t.log.4").exists(),
            "keep=3 时 .4 不该出现,最老的一份直接丢弃"
        );
    }

    #[test]
    fn rotate_chain_tolerates_missing_chain_members() {
        let temp = tempfile::tempdir().expect("tempdir");
        let main = temp.path().join("t.log");
        std::fs::write(&main, "only").unwrap();
        rotate_chain(&main, 4);
        assert!(!main.exists());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("t.log.1")).unwrap(),
            "only"
        );
    }
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
