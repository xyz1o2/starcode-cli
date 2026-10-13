//! 进程级覆盖表：模型自调的落点。
//!
//! 解析顺序固定为 **override > env > 调用点默认值**。`env` 仍然完整生效，
//! 所以用户现有的环境变量配置不需要迁移；模型只是在它之上多了一层
//! 可写、可溯源、可撤销的覆盖。
//!
//! ## 为什么不用 `std::env::set_var`
//!
//! tokio 是多线程运行时，工具执行在 worker 线程上。其它线程随时可能在读
//! 同一个环境变量，`setenv` 与 `getenv` 并发在 libc 层是数据竞争。
//! 覆盖表用 `RwLock` 表达同样的语义，不碰进程环境。
//!
//! ## 持久化
//!
//! 会话内自调只对当前进程有意义，除非写出去。`persist` 为真的写入落到
//! `Storage::self_tuning_path()`（`~/.star/self-tuning.json`），并在下次启动时
//! 由 `load_persisted` 读回覆盖表 —— 于是"模型自己适应出来的配置"跨会话存活，
//! 用户不必再手写环境变量。

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use super::knobs::{Knob, KnobType};

/// 有效值的来源。`inspect` 报告它，回答"这个值是谁定的"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueSource {
    /// 覆盖层：模型（或持久化回读）在本次运行中写入。
    SelfTuned,
    /// 进程环境变量，沿用用户既有配置。
    Environment,
    /// 覆盖层与 env 都没有，用调用点默认值。
    Default,
}

impl ValueSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueSource::SelfTuned => "self_tuned",
            ValueSource::Environment => "env",
            ValueSource::Default => "default",
        }
    }
}

fn table() -> &'static RwLock<BTreeMap<String, String>> {
    static TABLE: OnceLock<RwLock<BTreeMap<String, String>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// 运维锁定的旋钮列表，以逗号分隔的旋钮 key 指定。
///
/// 这是对标 Claude Code 的 managed-settings `lock`：*锁住的值本身仍是一个任何层
/// 都能设置的普通 key，lock 额外告诉程序只认管理层级的值*。
///
/// 没有这一层的话，运维**故意**设的 `STAR_COMPACT_MAX_TOKENS=80000` 会被模型
/// 静默盖掉 —— 覆盖层优先级高于 env 是设计意图，但对"运维已经拍过板"的项，
/// 这个意图是错的。
pub const ENV_LOCKED: &str = "STAR_SELFTUNING_LOCKED";

/// 一个会话内允许的成功覆盖次数上限。
///
/// 对标 ToolSelf 的 reconfiguration budget。没有预算时，一个发现旋钮能翻的模型
/// 会每轮都翻：先调压缩阈值、再调回来、再调别的，最后比不调更糟。预算是让"不再调"
/// 成为默认路径。
pub const ENV_MAX_SETS: &str = "STAR_SELFTUNING_MAX_SETS";

/// 未设置 `STAR_SELFTUNING_MAX_SETS` 时的每次会话预算。
///
/// 12 次足够一次长会话里"发现上下文吃紧 → 调压缩"这类少量定向调整，又不至于
/// 让模型把这个工具当成日常动作。
pub const DEFAULT_SESSION_BUDGET: usize = 12;

fn locked_keys() -> Vec<String> {
    std::env::var(ENV_LOCKED)
        .unwrap_or_default()
        .split(',')
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect()
}

/// 该旋钮是否被运维锁定。锁定的旋钮拒绝模型写入。
pub fn is_locked(key: &str) -> bool {
    locked_keys().iter().any(|locked| locked == key)
}

/// 全部被锁定的旋钮，供 `inspect` 呈现。
pub fn locked_knob_keys() -> Vec<String> {
    let locked = locked_keys();
    super::knobs::knobs()
        .iter()
        .filter(|knob| locked.iter().any(|key| key == knob.key))
        .map(|knob| knob.key.to_string())
        .collect()
}

fn budget() -> usize {
    std::env::var(ENV_MAX_SETS)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_SESSION_BUDGET)
}

/// 本会话剩余的覆盖预算。
///
/// `reset` 不消耗预算：撤销必须永远可用，否则模型一旦把值调坏就再也回不去 ——
/// 那正是这个预算设计最需要防的失败模式。
pub fn remaining_budget() -> usize {
    let cap = budget();
    let spent = sets_used();
    cap.saturating_sub(spent)
}

fn used_counter() -> &'static std::sync::atomic::AtomicUsize {
    static USED: OnceLock<std::sync::atomic::AtomicUsize> = OnceLock::new();
    USED.get_or_init(|| std::sync::atomic::AtomicUsize::new(0))
}

fn sets_used() -> usize {
    used_counter().load(std::sync::atomic::Ordering::SeqCst)
}

fn consume_budget() {
    used_counter().fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// 仅测试使用：把会话预算计数清零。
///
/// 覆盖表与预算计数器都是进程级状态，只靠串行化不够 —— 一个测试花掉的预算会
/// 让下一个测试莫名其妙地撞上"预算用尽"。与 `tool_search::sticky_test_guard`
/// 同一思路。
#[cfg(test)]
pub(crate) fn reset_budget_for_test() {
    used_counter().store(0, std::sync::atomic::Ordering::SeqCst);
}

/// 写入一个已通过 [`Knob::validate`] 的覆盖值。
///
/// 返回规范化后的值。`persist` 为真时同时落盘，跨会话存活。
/// `rationale` 是模型给出的调参理由：强制填写（见 [`set`] 的调用方），并记进
/// agent 日志。没有它，事后无法区分"依据证据的调整"和"随手乱试"。
///
/// 写入前置条件，按顺序拒绝：未知 key → 被锁 → 预算耗尽 → 取值非法。
/// 顺序有意义：先报最根本的原因，避免模型对着"预算用尽"去调一个根本不存在的 key。
pub fn set(
    key: &str,
    value: &str,
    persist: bool,
    rationale: Option<&str>,
) -> Result<String, String> {
    let knob = Knob::by_key(key).ok_or_else(|| unknown_key(key))?;

    if is_locked(knob.key) {
        return Err(format!(
            "`{}` is locked by {} and cannot be self-tuned. An operator pinned it; leave it alone. \
             {}",
            knob.key,
            ENV_LOCKED,
            knob.summary
        ));
    }

    let normalized = knob.validate(value)?;

    // 预算在合法性校验之后才扣：一次被拒的写入不该消耗预算，否则模型对着
    // 一个永远改不对的值会很快把预算烧光。
    if remaining_budget() == 0 {
        return Err(format!(
            "Self-tuning budget for this session is exhausted ({} successful changes). \
             The current value of `{}` stays as it is. You can still use the `reset` action to \
             undo earlier changes, and you can describe the situation to the user instead of \
             tuning further.",
            budget(),
            knob.key
        ));
    }

    if let Ok(mut map) = table().write() {
        map.insert(knob.key.to_string(), normalized.clone());
    } else {
        return Err(format!(
            "Self-tuning table is poisoned; cannot set `{}`.",
            knob.key
        ));
    }
    consume_budget();

    crate::utils::logging::append_agent_log_line(&format!(
        "[SelfTuning] `{}` -> {} (persist={}, budget {}/{}){}",
        knob.key,
        normalized,
        persist,
        budget().saturating_sub(remaining_budget()),
        budget(),
        rationale
            .map(|reason| format!(" reason={:?}", reason.trim()))
            .unwrap_or_default()
    ));

    if persist {
        save_persisted()?;
    }
    Ok(normalized)
}

/// 撤销一个覆盖值，回到 env / 默认值。
///
/// `key` 为 `None` 表示清空全部覆盖。
pub fn reset(key: Option<&str>) -> Result<Vec<String>, String> {
    let cleared = match key {
        Some(key) => {
            let knob = Knob::by_key(key).ok_or_else(|| unknown_key(key))?;
            let mut map = table()
                .write()
                .map_err(|_| "Self-tuning table is poisoned.".to_string())?;
            map.remove(knob.key);
            vec![knob.key.to_string()]
        }
        None => {
            let mut map = table()
                .write()
                .map_err(|_| "Self-tuning table is poisoned.".to_string())?;
            let keys: Vec<String> = map.keys().cloned().collect();
            map.clear();
            keys
        }
    };
    save_persisted()?;
    Ok(cleared)
}

/// 某个旋钮是否被自调覆盖（无论当前有效值是什么）。
pub fn is_overridden(key: &str) -> bool {
    table()
        .read()
        .map(|map| map.contains_key(key))
        .unwrap_or(false)
}

/// 全部覆盖值，供 `inspect` 渲染。
pub fn snapshot() -> BTreeMap<String, String> {
    table().read().map(|map| map.clone()).unwrap_or_default()
}

/// 覆盖层的原始字符串，未设置则为 `None`。
fn override_value(key: &str) -> Option<String> {
    table().read().ok().and_then(|map| map.get(key).cloned())
}

/// 一个旋钮的有效值与来源。
///
/// env 路径**不做**区间校验，与既有 `from_env` 行为保持一致；只有模型写入的
/// 覆盖值走 [`Knob::validate`]。因此 `ValueSource::Environment` 可能是越界值，
/// 这是用户自己摆进去的，不该被静默改写。
pub fn resolve(key: &str) -> (Option<String>, ValueSource) {
    if let Some(value) = override_value(key) {
        return (Some(value), ValueSource::SelfTuned);
    }
    let Some(knob) = Knob::by_key(key) else {
        return (None, ValueSource::Default);
    };
    match std::env::var(knob.env) {
        Ok(value) if !value.trim().is_empty() => {
            (Some(value.trim().to_string()), ValueSource::Environment)
        }
        _ => (None, ValueSource::Default),
    }
}

/// 取覆盖层的字符串值，未覆盖则为 `None`。不查 env。
pub fn override_str(key: &str) -> Option<String> {
    override_value(key)
}

/// 一个旋钮的**有效值**，把静态默认值也算进来。
///
/// [`resolve`] 只回答"覆盖层或 env 有没有给出值"，所以对只有静态默认值的旋钮
/// 它返回 `None`。但 `set` 的 before→after 报告需要真正生效的那个值：
/// 告诉模型 `(unset) → 80000` 等于没说，它看不出自己到底改动了什么。
pub fn effective_str(key: &str) -> (String, ValueSource) {
    match resolve(key) {
        (Some(value), source) => (value, source),
        (None, _) => match Knob::by_key(key).and_then(|knob| knob.default) {
            Some(default) => (default.to_string(), ValueSource::Default),
            // 无静态默认值：默认由调用点按工具类型分档，如实说明没有单一值。
            None => (String::new(), ValueSource::Default),
        },
    }
}

/// 仅测试使用：清空内存覆盖表但**不**落盘。
///
/// 模拟进程重启时需要这个 —— `reset` 会把清空结果也写盘，那正好抹掉刚写的
/// 文件，测到的就不是"重启"而是"重启后又 reset 了一次"。
#[cfg(test)]
pub(crate) fn clear_in_memory_for_test() {
    if let Ok(mut map) = table().write() {
        map.clear();
    }
}

pub fn usize_value(key: &str) -> Option<usize> {
    resolve(key).0.and_then(|v| v.parse().ok())
}

pub fn u64_value(key: &str) -> Option<u64> {
    resolve(key).0.and_then(|v| v.parse().ok())
}

pub fn f64_value(key: &str) -> Option<f64> {
    resolve(key).0.and_then(|v| v.parse().ok())
}

/// 取布尔值。`None` 表示"未设置"，由调用点决定默认值 —— 这与压缩配置里
/// `unwrap_or(true)` 的既有写法对应。
pub fn bool_value(key: &str) -> Option<bool> {
    match resolve(key)
        .0
        .as_deref()
        .map(|v| v.trim().to_ascii_lowercase())
    {
        Some(value) => match value.as_str() {
            "false" | "0" | "off" | "no" | "n" => Some(false),
            // 与 from_env 的历史语义一致：只有 "false"/"0" 为假，其余为真。
            _ => Some(true),
        },
        None => None,
    }
}

/// 正整数语义：非正数视为未设置。这是压缩配置 `positive_env_tokens` 的等价物。
pub fn positive_usize(key: &str) -> Option<usize> {
    usize_value(key).filter(|value| *value > 0)
}

/// `KnobType` 的通用解析，供持久化回读校验。
pub fn coerce(knob: &Knob, raw: &str) -> Option<String> {
    knob.validate(raw).ok()
}

pub fn knob_type_name(value_type: KnobType) -> &'static str {
    match value_type {
        KnobType::Usize => "integer",
        KnobType::U64 => "integer",
        KnobType::F64 => "number",
        KnobType::Bool => "boolean",
    }
}

fn unknown_key(key: &str) -> String {
    let known: Vec<&str> = super::knobs::knobs().iter().map(|k| k.key).collect();
    format!(
        "Unknown self-tuning knob `{key}`. Known knobs: {}.",
        known.join(", ")
    )
}

/// 覆盖表快照的磁盘形态。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PersistedOverrides {
    /// key -> value。键不在目录里的条目会被丢弃。
    #[serde(default)]
    values: BTreeMap<String, String>,
}

/// 持久化路径覆盖，供测试隔离真实 `~/.star/self-tuning.json`。
///
/// 与 `utils::checkpoint_manager` 的 `STAR_FILE_HISTORY_DIR_OVERRIDE` 同一思路：
/// 覆盖表是进程级状态，它的持久化文件如果跟着跑测试，测试就会写进用户真实的
///  home 目录。
pub const ENV_PATH_OVERRIDE: &str = "STAR_SELFTUNING_PATH";

fn persisted_path() -> std::path::PathBuf {
    if let Ok(path) = std::env::var(ENV_PATH_OVERRIDE) {
        if !path.trim().is_empty() {
            return std::path::PathBuf::from(path);
        }
    }
    crate::core::config::storage::Storage::self_tuning_path()
}

/// 把当前覆盖表写盘。写失败不 panic：自调是尽力而为的能力，
/// 不该因为磁盘只读就让模型的一次 `set` 变成工具错误。
fn save_persisted() -> Result<(), String> {
    let path = persisted_path();
    let payload = PersistedOverrides { values: snapshot() };
    let serialized = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("Failed to serialize self-tuning overrides: {error}"))?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, serialized)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

/// 启动时把持久化的覆盖读回覆盖表。
///
/// 未知键、类型不符、区间越界的条目被跳过并记日志，而不是让整个文件失败 ——
/// 目录演进后旧文件里的过期旋钮不该阻塞启动。
pub fn load_persisted() -> usize {
    let path = persisted_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(parsed) = serde_json::from_str::<PersistedOverrides>(&raw) else {
        crate::utils::logging::append_agent_log_line(&format!(
            "[SelfTuning] ignoring unreadable overrides at {}",
            path.display()
        ));
        return 0;
    };

    let mut applied = 0usize;
    let mut rejected: Vec<String> = Vec::new();
    if let Ok(mut map) = table().write() {
        for (key, value) in parsed.values {
            match Knob::by_key(&key).and_then(|knob| coerce(knob, &value)) {
                Some(normalized) => {
                    map.insert(key, normalized);
                    applied += 1;
                }
                None => rejected.push(key),
            }
        }
    }
    if !rejected.is_empty() {
        crate::utils::logging::append_agent_log_line(&format!(
            "[SelfTuning] dropped {} unusable persisted override(s): {}",
            rejected.len(),
            rejected.join(", ")
        ));
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::self_tuning::knobs::knobs;

    /// 覆盖表是进程级状态，触碰它的测试必须串行，否则互相看到对方的值。
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 串行 + 清空覆盖表 + 清零预算 + 把持久化重定向到临时文件。
    /// 让每个测试都从干净状态开始，且不碰用户真实的 `~/.star/self-tuning.json`。
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let held = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var(
            ENV_PATH_OVERRIDE,
            std::env::temp_dir().join("starcode-self-tuning-test.json"),
        );
        reset(None).expect("clear overrides");
        reset_budget_for_test();
        held
    }

    #[test]
    fn set_then_reset_returns_to_environment_or_default() {
        let _serial = guard();
        let key = "compact.max_tokens";
        reset(Some(key)).expect("reset");

        // 默认态：没有 env 时应落到 Default。
        let (value, source) = resolve(key);
        assert!(matches!(
            source,
            ValueSource::Default | ValueSource::Environment
        ));

        set(key, "80000", false, None).expect("set");
        assert_eq!(usize_value(key), Some(80_000));
        assert_eq!(resolve(key).1, ValueSource::SelfTuned);
        assert!(is_overridden(key));

        reset(Some(key)).expect("reset");
        assert!(!is_overridden(key));
        assert_eq!(resolve(key).0, value);
    }

    #[test]
    fn override_wins_over_environment() {
        let _serial = guard();
        let key = "compact.max_tokens";
        reset(Some(key)).expect("reset");

        // 造一个 env 冲突：覆盖层必须赢。
        let env_name = Knob::by_key(key).expect("knob").env;
        std::env::set_var(env_name, "123456");
        let _env = EnvGuard::new(env_name);

        assert_eq!(resolve(key).1, ValueSource::Environment);
        set(key, "64000", false, None).expect("set");
        assert_eq!(
            resolve(key).1,
            ValueSource::SelfTuned,
            "override must beat env"
        );
        assert_eq!(usize_value(key), Some(64_000));
    }

    #[test]
    fn invalid_value_is_rejected_and_leaves_state_untouched() {
        let _serial = guard();
        let key = "compact.weight.recency";
        reset(Some(key)).expect("reset");

        assert!(
            set(key, "5.0", false, None).is_err(),
            "out of range must fail"
        );
        assert!(
            !is_overridden(key),
            "a rejected set must not write anything"
        );
        assert!(
            set(key, "banana", false, None).is_err(),
            "wrong type must fail"
        );
    }

    #[test]
    fn unknown_key_reports_the_known_knobs() {
        let _serial = guard();
        let err = set("compact.nonexistent", "1", false, None).expect_err("unknown key");
        assert!(
            err.contains("compact.max_tokens"),
            "should list known knobs: {err}"
        );
        assert!(reset(Some("compact.nonexistent")).is_err());
    }

    #[test]
    fn locked_knob_rejects_writes_even_with_a_valid_value() {
        let _serial = guard();
        let key = "compact.max_tokens";
        reset(Some(key)).expect("reset");
        std::env::set_var(ENV_LOCKED, key);
        let _lock = EnvGuard::new(ENV_LOCKED);

        assert!(is_locked(key));
        assert!(locked_knob_keys().contains(&key.to_string()));

        let err = set(key, "60000", false, None).expect_err("locked knob must refuse");
        assert!(err.contains("locked"), "error should say why: {err}");
        assert!(!is_overridden(key), "a locked knob must not be written");
    }

    #[test]
    fn lock_list_is_comma_separated_and_tolerates_whitespace() {
        let _serial = guard();
        std::env::set_var(ENV_LOCKED, " compact.max_tokens , compact.auto_enabled ,, ");
        let _lock = EnvGuard::new(ENV_LOCKED);

        let locked = locked_knob_keys();
        assert_eq!(locked.len(), 2, "blank entries must be skipped: {locked:?}");
        assert!(is_locked("compact.max_tokens"));
        assert!(is_locked("compact.auto_enabled"));
        assert!(!is_locked("compact.target_tokens"));
    }

    #[test]
    fn unknown_key_is_reported_before_a_lock_or_budget_refusal() {
        let _serial = guard();
        std::env::set_var(ENV_LOCKED, "compact.nonexistent");
        let _lock = EnvGuard::new(ENV_LOCKED);

        let err = set("compact.nonexistent", "1", false, None).expect_err("unknown key");
        assert!(
            err.contains("Unknown self-tuning knob"),
            "the root cause must come first: {err}"
        );
    }

    #[test]
    fn budget_stops_further_writes_but_reset_still_works() {
        let _serial = guard();
        reset(None).expect("reset");
        std::env::set_var(ENV_MAX_SETS, "2");
        let _cap = EnvGuard::new(ENV_MAX_SETS);

        set("compact.max_tokens", "70000", false, None).expect("first set");
        set("compact.target_tokens", "50000", false, None).expect("second set");
        assert_eq!(remaining_budget(), 0);

        let err = set("compact.micro_threshold", "50", false, None).expect_err("budget spent");
        assert!(
            err.contains("budget"),
            "error should name the budget: {err}"
        );
        assert!(!is_overridden("compact.micro_threshold"));

        // 撤销永远可用：预算不该把模型锁在自己调坏的配置上。
        assert!(reset(Some("compact.max_tokens")).is_ok());
        assert!(!is_overridden("compact.max_tokens"));
    }

    #[test]
    fn a_rejected_write_does_not_consume_budget() {
        let _serial = guard();
        reset(None).expect("reset");
        std::env::set_var(ENV_MAX_SETS, "1");
        let _cap = EnvGuard::new(ENV_MAX_SETS);

        // 非法取值不扣预算，否则模型会对着一个改不对的值把预算烧光。
        assert!(set("compact.max_tokens", "not-a-number", false, None).is_err());
        assert_eq!(
            remaining_budget(),
            1,
            "rejected write must not spend budget"
        );
        set("compact.max_tokens", "70000", false, None).expect("valid write still allowed");
    }

    #[test]
    fn reset_all_clears_every_override() {
        let _serial = guard();
        set("compact.max_tokens", "70000", false, None).expect("set");
        set("compact.auto_enabled", "false", false, None).expect("set");

        let cleared = reset(None).expect("reset all");
        assert!(cleared.contains(&"compact.max_tokens".to_string()));
        assert!(cleared.contains(&"compact.auto_enabled".to_string()));
        assert!(snapshot().is_empty());
    }

    #[test]
    fn positive_usize_treats_zero_as_unset() {
        let _serial = guard();
        set("compact.max_tokens", "5000", false, None).expect("set");
        assert_eq!(positive_usize("compact.max_tokens"), Some(5000));
        // 覆盖层能写入 0，但正整数语义把它当作未设置。
        table()
            .write()
            .expect("table")
            .insert("compact.max_tokens".into(), "0".into());
        assert_eq!(positive_usize("compact.max_tokens"), None);
    }

    #[test]
    fn bool_value_only_treats_false_and_zero_as_false() {
        let _serial = guard();
        let key = "compact.auto_enabled";
        for (raw, expected) in [
            ("false", false),
            ("0", false),
            ("FALSE", false),
            ("true", true),
            ("1", true),
            ("yes", true),
        ] {
            set(key, raw, false, None).expect("set");
            assert_eq!(bool_value(key), Some(expected), "input {raw}");
        }
    }

    #[test]
    fn persisted_roundtrip_only_keeps_catalogued_keys() {
        let _serial = guard();
        set("compact.max_tokens", "90000", false, None).expect("set");
        // 塞一个目录里没有的键，模拟旧版本残留。
        table()
            .write()
            .expect("table")
            .insert("compact.retired_knob".into(), "1".into());
        save_persisted().expect("save");

        // 模拟重启：清空内存覆盖表但**不**走 `reset` —— reset 会把清空结果也
        // 落盘，那正好抹掉刚写的文件，测的就不是重启而是"重启后又 reset 了一次"。
        table().write().expect("table").clear();
        assert!(!is_overridden("compact.max_tokens"), "in-memory cleared");

        let applied = load_persisted();
        assert!(applied >= 1, "catalogued override should survive");
        assert_eq!(usize_value("compact.max_tokens"), Some(90_000));
        assert!(
            !is_overridden("compact.retired_knob"),
            "unknown persisted keys must be dropped"
        );
    }

    /// 临时环境变量，析构时恢复。
    struct EnvGuard(&'static str);

    impl EnvGuard {
        fn new(name: &'static str) -> Self {
            Self(name)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    /// 目录里的每个旋钮都能被 set/reset 往返一次。
    #[test]
    fn every_catalogued_knob_roundtrips() {
        let _serial = guard();
        // 目录会增长，预算不能成为这个测试的隐形失败原因。
        std::env::set_var(ENV_MAX_SETS, "999");
        let _cap = EnvGuard::new(ENV_MAX_SETS);

        for knob in knobs() {
            // 无静态默认值的旋钮（默认值由调用点分档）也要能往返，
            // 给一个落在区间内的具体值。
            let raw = match (knob.value_type, knob.default) {
                (crate::core::self_tuning::knobs::KnobType::Bool, _) => "false",
                (crate::core::self_tuning::knobs::KnobType::F64, None) => "0.5",
                (_, None) => "1000",
                (_, Some(default)) => default,
            };
            set(knob.key, raw, false, None)
                .unwrap_or_else(|e| panic!("knob {} with value {raw}: {e}", knob.key));
            assert!(is_overridden(knob.key), "knob {} not overridden", knob.key);
            reset(Some(knob.key)).expect("reset");
            assert!(!is_overridden(knob.key), "knob {} survived reset", knob.key);
        }
    }
}
