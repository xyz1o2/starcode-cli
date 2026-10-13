//! 自调覆盖层：让模型在运行中调节智能体自身，而不是让用户去调环境变量。
//!
//! # 动机
//!
//! 这个仓库的运行期行为高度依赖 `STAR_*` 环境变量 —— 342 个，几乎都在使用点
//! 内联读取（`std::env::var("STAR_…").ok().and_then(…).unwrap_or(default)`）。
//! 这套机制对运维友好，对模型不友好：模型**看不到**这些值（系统提示词里没有任何
//! 一处渲染当前设置），更**改不了**它们（唯一能写设置的入口是 UI 侧的
//! `/effort`、`/fast`；`/config set` 至今只是个回显占位符）。
//!
//! 结果是：模型观察到"上下文快满了""工具输出被截断了"，只能把这件事写进给用户的
//! 解释里，让用户自己去查文档、改环境变量、重启。
//!
//! # 做法
//!
//! 增加一层位于 env 之上的**进程级覆盖表**，并把可自调的设置收进
//! [`knobs`] 目录。模型的入口是 `self_config` 工具（`inspect` / `set` / `reset`）。
//!
//! ```text
//!   override（模型自调，可溯源、可撤销）
//!     ↓ 未命中
//!   env（用户既有配置，行为不变）
//!     ↓ 未命中
//!   调用点默认值
//! ```
//!
//! # 为什么不是 `std::env::set_var`
//!
//! 1. **安全**：tokio 多线程运行时里，`setenv` 与其它线程的 `getenv` 并发是 libc
//!    级数据竞争。
//! 2. **可溯源**：env 分不出"用户设的"和"模型上一轮设的"，而 `inspect` 必须回答
//!    "这个值是谁定的"，否则模型会对着自己上一轮的修改反复叠加。
//!
//! # 只收真正生效的旋钮
//!
//! 收录标准是**读取点在使用时解析**（或每轮重建）。构造期捕获的设置收进来只会
//! 让模型以为改成功了 —— `STAR_LLM_TIMEOUT` 已经被烤进 `reqwest::Client` 的
//! `.timeout()`，`Config` 字段里的 `context_window` 也在 `Config::new` 时定格。
//! 这类项在 [`knobs::EffectTiming::NextSession`] 上有对应变体，`inspect` 会如实
//! 标注时机，但不会给模型一个写了不生效的写入面。
//!
//! # 生效时机分两档
//!
//! - [`knobs::EffectTiming::Immediate`] / [`knobs::EffectTiming::NextLoopIteration`]：
//!   **运行中**改完就作用于同一进程内的后续读取，无需重启。这就是"运行中调节自身"。
//! - 模型与上下文窗口仍然走既有的 `RuntimeSettingsMutation` 通道，在逻辑回合边界
//!   安装，对下一条用户消息生效 —— 因为它们在 `agent_run.rs` 里是构造期冻结进
//!   per-message fresh `Agent` 的，运行中的回合改不了。

pub mod knobs;
pub mod overrides;
pub mod report;

pub use knobs::{EffectTiming, Knob, KnobType};
pub use overrides::{
    bool_value, f64_value, is_locked, is_overridden, locked_knob_keys, override_str,
    positive_usize, remaining_budget, snapshot, u64_value, usize_value, ValueSource,
    DEFAULT_SESSION_BUDGET, ENV_LOCKED, ENV_MAX_SETS,
};
// `set` / `reset` 不在这里再导出：它们要显式传 `rationale` / `persist`，
// 调用点写 `overrides::set(..)` 比短别名更能提醒这几个参数不是随便传的。
