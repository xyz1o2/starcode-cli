//! 轻量会话内触发器（对标 Claude Code /schedule、/triggers）。
//!
//! 本工程没有完整的 cron 引擎，这里覆盖"给未来的自己发一条消息"的核心语义：
//! - `add_trigger` 把一个一次性消息持久化到 `.star/triggers.json`，并记录到期时间戳；
//! - `list_triggers` 列出尚未到期的触发器（附剩余时间）；
//! - `remove_trigger` 按 id 移除；
//! - `secs_until_hhmm` 把 `HH:MM` 换算成今天最近一次出现的相对秒数（已过则算到明天）。
//!
//! 本模块只负责存储与时间换算；到期消息由 UI 按所属会话投递。

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// 命令与后台到期任务不能同时读改写同一份文件。
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// 一条已排程的触发。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub message: String,
    /// 到期时刻的 Unix 秒。
    pub fires_at_epoch: i64,
}

pub(crate) fn triggers_path() -> PathBuf {
    let cwd = crate::core::utils::paths::current_dir_cached();
    if let Some(path) = std::env::var_os("STAR_TRIGGERS_FILE").filter(|path| !path.is_empty()) {
        return cwd.join(path);
    }
    crate::core::utils::paths::project_star_dir(cwd).join("triggers.json")
}

fn load_triggers(path: &Path) -> Vec<Trigger> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    if raw.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_triggers(path: &Path, triggers: &[Trigger]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let json = serde_json::to_string_pretty(triggers).unwrap_or_default();
    let _ = std::fs::write(path, json);
}

/// 过期项在 JSON 里保留的宽限期。清理必须晚于到期时刻一段时间，否则一个刚好在
/// 到期瞬间执行的 `add_trigger` 会把还没来得及 `take_trigger` 的触发误删。
const STALE_GRACE_SECS: i64 = 300;

/// 新增一条 `secs` 秒后到期的触发，返回其 id。
pub fn add_trigger(message: &str, secs: u64) -> String {
    add_trigger_at(&triggers_path(), message, secs)
}

pub(crate) fn add_trigger_at(path: &Path, message: &str, secs: u64) -> String {
    let _guard = STORE_LOCK.lock().expect("trigger store lock poisoned");
    let now = chrono::Utc::now().timestamp();
    let mut triggers = load_triggers(path);
    // 顺手清理早已过期的项，避免 JSON 无限膨胀
    triggers.retain(|t| t.fires_at_epoch > now - STALE_GRACE_SECS);
    // id 用毫秒时间戳；同一毫秒内连续两次 add 会撞号，加后缀去重
    let base = format!("t{}", chrono::Utc::now().timestamp_millis());
    let mut id = base.clone();
    let mut suffix = 1;
    while triggers.iter().any(|t| t.id == id) {
        id = format!("{}-{}", base, suffix);
        suffix += 1;
    }
    triggers.push(Trigger {
        id: id.clone(),
        message: message.to_string(),
        fires_at_epoch: now.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX)),
    });
    save_triggers(path, &triggers);
    id
}

/// 按 id 移除触发；不存在则返回 false。
pub fn remove_trigger(id: &str) -> bool {
    take_trigger_at(&triggers_path(), id)
}

/// 「取出并删除」：按创建时的文件查找，避免切目录后误读别的项目。
pub(crate) fn take_trigger_at(path: &Path, id: &str) -> bool {
    remove_trigger_at(path, id)
}

pub(crate) fn remove_trigger_at(path: &Path, id: &str) -> bool {
    let _guard = STORE_LOCK.lock().expect("trigger store lock poisoned");
    let mut triggers = load_triggers(path);
    let before = triggers.len();
    triggers.retain(|t| t.id != id);
    if triggers.len() == before {
        return false;
    }
    save_triggers(path, &triggers);
    true
}

pub fn take_trigger(id: &str) -> bool {
    take_trigger_at(&triggers_path(), id)
}

/// 列出尚未到期的触发：`(id, message, 剩余时间描述)`。
pub fn list_triggers() -> Vec<(String, String, String)> {
    let _guard = STORE_LOCK.lock().expect("trigger store lock poisoned");
    let now = chrono::Utc::now().timestamp();
    load_triggers(&triggers_path())
        .into_iter()
        .filter(|t| t.fires_at_epoch > now)
        .map(|t| {
            let eta = format_eta(t.fires_at_epoch - now);
            (t.id, t.message, eta)
        })
        .collect()
}

fn format_eta(secs: i64) -> String {
    if secs < 60 {
        format!("in {}s", secs)
    } else {
        format!("in {}s (≈{} min)", secs, secs / 60)
    }
}

/// 把 `HH:MM` 换算成今天最近一次的相对秒数；若该时刻已过则算到明天。
pub fn secs_until_hhmm(hhmm: &str) -> Result<u64, String> {
    let parts: Vec<&str> = hhmm.split(':').collect();
    if parts.len() != 2 {
        return Err("`at` needs a time like 14:30".to_string());
    }
    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| "invalid hour in `at <HH:MM>`".to_string())?;
    let minute: u32 = parts[1]
        .parse()
        .map_err(|_| "invalid minute in `at <HH:MM>`".to_string())?;
    if hour > 23 || minute > 59 {
        return Err("time out of range (HH:MM)".to_string());
    }
    let now = Local::now();
    let target = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)
        .ok_or_else(|| "invalid time".to_string())?;
    let secs = (target - now.naive_local()).num_seconds();
    Ok(if secs <= 0 { secs + 86_400 } else { secs } as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把存储重定向到临时文件，避免测试写进仓库 cwd 下的 `.star/`（那会让测试
    /// 依赖 cwd，也会在工作树里留下垃圾文件）。
    ///
    /// `STAR_TRIGGERS_FILE` 是进程全局的，所以这些测试必须串行：默认 `cargo test`
    /// 并行跑同一模块内的测试，不加锁会互相读到对方的存储路径。
    struct TempStore {
        path: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    impl TempStore {
        fn new(tag: &str) -> Self {
            let guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
            let path = std::env::temp_dir().join(format!(
                "starcode-triggers-{}-{}.json",
                tag,
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            std::env::set_var("STAR_TRIGGERS_FILE", &path);
            TempStore {
                path,
                _guard: guard,
            }
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            std::env::remove_var("STAR_TRIGGERS_FILE");
        }
    }

    #[test]
    fn secs_until_hhmm_parses_and_clamps() {
        // 合法输入总是落在 (0, 86400] 区间
        for hhmm in ["00:00", "12:34", "23:59"] {
            let secs = secs_until_hhmm(hhmm).unwrap();
            assert!(secs > 0 && secs <= 86_400, "{} -> {}", hhmm, secs);
        }
        assert!(secs_until_hhmm("24:00").is_err());
        assert!(secs_until_hhmm("12:60").is_err());
        assert!(secs_until_hhmm("noon").is_err());
    }

    #[test]
    fn add_remove_list_roundtrip() {
        let _store = TempStore::new("roundtrip");

        let id = add_trigger("ping", 3_600);
        assert!(!id.is_empty());
        assert!(list_triggers().iter().any(|(i, _, _)| i == &id));

        // remove 幂等：第一次成功，第二次报告"不存在"
        assert!(remove_trigger(&id));
        assert!(!remove_trigger(&id));
        assert!(list_triggers().is_empty());
    }

    #[test]
    fn take_trigger_is_the_cancellation_check() {
        let _store = TempStore::new("take");

        let id = add_trigger("fire once", 3_600);
        // 第一次 take 成功（应当发送），第二次失败（不会重复发送）
        assert!(take_trigger(&id));
        assert!(!take_trigger(&id));

        // 已被 /schedule remove 取消的触发，到期时 take 不到 → 不发送
        let cancelled = add_trigger("cancelled", 3_600);
        assert!(remove_trigger(&cancelled));
        assert!(!take_trigger(&cancelled));
    }

    #[test]
    fn ids_are_unique_within_the_same_millisecond() {
        let _store = TempStore::new("ids");

        let ids: Vec<String> = (0..5).map(|_| add_trigger("m", 3_600)).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids must be unique: {:?}", ids);
    }
}
