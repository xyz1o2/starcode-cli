//! 自调旋钮目录：模型可以读写的运行期设置的唯一真相源。
//!
//! 这张表同时驱动四件事，改一处即可：
//! 1. **取值**：`resolve_*` 按 `override > env > default` 解析（见 `super`）。
//! 2. **校验**：`set` 只接受通过类型与区间检查的值。
//! 3. **来源**：`inspect` 报告每个旋钮的有效值来自 self-tuned / env / default。
//! 4. **生效时机**：`EffectTiming` 告诉模型"改完什么时候起作用"。
//!
//! ## 为什么单独建表，而不是就地 `std::env::set_var`
//!
//! tokio 多线程运行时里写进程环境变量会与其它线程的 `getenv` 形成 libc 级
//! 数据竞争。更重要的是 `env` 无法区分"用户设的"和"模型上一轮设的"，
//! 而这正是 `inspect` 要回答的问题。覆盖层与 `env` 并存且优先级更高，
//! 因此**用户已有的环境变量配置继续有效**，模型只是多了一层可写、可溯源、
//! 可撤销的覆盖。

use std::sync::OnceLock;

/// 旋钮值的类型。决定 `set` 的解析方式与 `inspect` 的呈现。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnobType {
    Usize,
    U64,
    F64,
    Bool,
}

impl KnobType {
    pub fn parse(self, raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        match self {
            KnobType::Usize => trimmed.parse::<usize>().ok().map(|_| self),
            KnobType::U64 => trimmed.parse::<u64>().ok().map(|_| self),
            KnobType::F64 => trimmed.parse::<f64>().ok().map(|_| self),
            // 接受 true/false/1/0/on/off/yes/no，忽略大小写。
            // env 路径历史上只认 "false"/"0"，这里是有意的超集：这是给模型用的
            // 写入面，模型更容易写对，且不影响 env 路径的既有语义。
            KnobType::Bool => match trimmed.to_ascii_lowercase().as_str() {
                "true" | "1" | "on" | "yes" | "y" => Some(self),
                "false" | "0" | "off" | "no" | "n" => Some(self),
                _ => None,
            },
        }
    }
}

/// 覆盖值何时对模型生效。见 [`EffectTiming`] 的各变体注释。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectTiming {
    /// 读取点就在使用时解析，运行中的调用下一次读取即生效。
    Immediate,
    /// 每轮重建一次配置，运行中的回合在下一轮迭代生效。
    NextLoopIteration,
    /// 构造期捕获，改了要重启进程才生效。这类旋钮不应出现在可写表里，
    /// 保留变体是为了让 `inspect` 能如实标注只读项的时机。
    NextSession,
}

impl EffectTiming {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectTiming::Immediate => "immediate",
            EffectTiming::NextLoopIteration => "next_loop_iteration",
            EffectTiming::NextSession => "next_session",
        }
    }

    /// 面向模型的说明。`inspect` 直接输出这句。
    pub fn guidance(self) -> &'static str {
        match self {
            EffectTiming::Immediate => {
                "takes effect on the next read — running calls are unaffected, nothing to restart"
            }
            EffectTiming::NextLoopIteration => {
                "rebuilt every turn — takes effect on the next loop iteration of the current run"
            }
            EffectTiming::NextSession => "captured at startup — only applies after a restart",
        }
    }
}

/// 一个可自调旋钮。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob {
    /// 模型使用的点分小写键名，如 `compact.max_tokens`。
    pub key: &'static str,
    /// 对应的环境变量。覆盖层优先于它，用户既有配置不受影响。
    pub env: &'static str,
    pub value_type: KnobType,
    /// 静态默认值。`None` 表示"默认值依赖调用点"（例如按工具类型分档），
    /// 此时未设置即表示调用点自选默认值。
    pub default: Option<&'static str>,
    /// 模型可写入的取值区间。`env` 路径**不做**区间校验，以保持既有语义不变。
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub effect: EffectTiming,
    /// 面向模型的一句话说明：这个旋钮调了会发生什么。
    pub summary: &'static str,
}

impl Knob {
    pub fn by_key(key: &str) -> Option<&'static Knob> {
        static MAP: OnceLock<std::collections::HashMap<&'static str, &'static Knob>> =
            OnceLock::new();
        MAP.get_or_init(|| knobs().iter().map(|knob| (knob.key, knob)).collect())
            .get(key)
            .copied()
    }

    pub fn by_env(env: &str) -> Option<&'static Knob> {
        static MAP: OnceLock<std::collections::HashMap<&'static str, &'static Knob>> =
            OnceLock::new();
        MAP.get_or_init(|| knobs().iter().map(|knob| (knob.env, knob)).collect())
            .get(env)
            .copied()
    }

    /// 校验一个模型提交的值，返回规范化后的字符串表示。
    ///
    /// 失败时给出模型能照着改的错误信息：区间的两端值必须出现在提示里，
    /// 否则模型只能反复猜。
    pub fn validate(&self, raw: &str) -> Result<String, String> {
        if self.value_type.parse(raw).is_none() {
            return Err(format!(
                "`{}` expects a {} value, got `{}`.",
                self.key,
                match self.value_type {
                    KnobType::Usize => "non-negative integer",
                    KnobType::U64 => "non-negative integer",
                    KnobType::F64 => "number",
                    KnobType::Bool => "boolean (true/false, 1/0, on/off, yes/no)",
                },
                raw.trim()
            ));
        }
        let normalized = normalize(self.value_type, raw);
        if self.value_type == KnobType::F64 || self.value_type == KnobType::Usize {
            let value: f64 = normalized.parse().map_err(|_| {
                format!(
                    "`{}` value `{}` is not comparable to the allowed range.",
                    self.key, normalized
                )
            })?;
            if let Some(min) = self.min {
                if value < min {
                    return Err(self.range_error(normalized.clone(), Some(min), self.max));
                }
            }
            if let Some(max) = self.max {
                if value > max {
                    return Err(self.range_error(normalized.clone(), self.min, Some(max)));
                }
            }
        }
        Ok(normalized)
    }

    fn range_error(&self, got: String, min: Option<f64>, max: Option<f64>) -> String {
        let allowed = match (min, max) {
            (Some(lo), Some(hi)) => format!("{lo}..={hi}"),
            (Some(lo), None) => format!("at least {lo}"),
            (None, Some(hi)) => format!("at most {hi}"),
            (None, None) => "unbounded".to_string(),
        };
        format!(
            "`{}` is out of range: got `{}`, allowed {}. {}",
            self.key, got, allowed, self.summary
        )
    }
}

/// 把值规范化成稳定字符串，让 `inspect` 的输出和覆盖表的存储都可回显。
fn normalize(value_type: KnobType, raw: &str) -> String {
    let trimmed = raw.trim();
    match value_type {
        KnobType::Bool => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "1" | "on" | "yes" | "y" => "true".to_string(),
            _ => "false".to_string(),
        },
        KnobType::F64 => trimmed
            .parse::<f64>()
            .map(|v| format!("{v}"))
            .unwrap_or_else(|_| trimmed.to_string()),
        _ => trimmed.to_string(),
    }
}

/// 全部可自调旋钮。
///
/// 收录标准：**读取点就在使用时解析**（或每轮重建），因此覆盖立刻生效。
/// 构造期捕获的设置（如 `STAR_LLM_TIMEOUT` 已烤进 `reqwest::Client`）
/// 不在这里 —— 收了也只会让模型以为自己改成功了。
pub fn knobs() -> &'static [Knob] {
    static KNOBS: OnceLock<Vec<Knob>> = OnceLock::new();
    KNOBS.get_or_init(build_knobs).as_slice()
}

fn build_knobs() -> Vec<Knob> {
    vec![
        // ---- 上下文压缩：全部在 compact::CompactConfig::from_env 里每轮重建 ----
        Knob {
            key: "compact.max_tokens",
            env: "STAR_COMPACT_MAX_TOKENS",
            value_type: KnobType::Usize,
            default: Some("150000"),
            min: Some(1.0),
            max: None,
            effect: EffectTiming::NextLoopIteration,
            summary:
                "Token count that triggers context compaction. Lower it to compact earlier and \
                      keep more room; raise it to let context grow before summarizing.",
        },
        Knob {
            key: "compact.target_tokens",
            env: "STAR_COMPACT_TARGET_TOKENS",
            value_type: KnobType::Usize,
            default: Some("100000"),
            min: Some(1.0),
            max: None,
            effect: EffectTiming::NextLoopIteration,
            summary: "Token count compaction shrinks the context to. Lower it to free more \
                      context after a compaction.",
        },
        Knob {
            key: "compact.auto_enabled",
            env: "STAR_AUTO_COMPACT_ENABLED",
            value_type: KnobType::Bool,
            default: Some("true"),
            min: None,
            max: None,
            effect: EffectTiming::NextLoopIteration,
            summary: "Automatic compaction on overflow. Turn off only when you are deliberately \
                      inspecting raw context; turning it off risks overflowing the model.",
        },
        Knob {
            key: "compact.micro_threshold",
            env: "STAR_MICRO_COMPACT_THRESHOLD",
            value_type: KnobType::Usize,
            default: Some("100"),
            min: Some(1.0),
            max: None,
            effect: EffectTiming::NextLoopIteration,
            summary: "Message count at which cheap micro-compaction runs to trim older turns \
                      without a full summarize pass.",
        },
        Knob {
            key: "compact.semantic_relevance",
            env: "STAR_SEMANTIC_RELEVANCE_ENABLED",
            value_type: KnobType::Bool,
            default: Some("true"),
            min: None,
            max: None,
            effect: EffectTiming::NextLoopIteration,
            summary: "Score messages by semantic relevance when compacting instead of by recency \
                      alone. Turn off only if compaction results look incoherent.",
        },
        Knob {
            key: "compact.weight.recency",
            env: "STAR_COMPACT_RECENCY_WEIGHT",
            value_type: KnobType::F64,
            default: Some("0.4"),
            min: Some(0.0),
            max: Some(1.0),
            effect: EffectTiming::NextLoopIteration,
            summary: "How much compaction favours recently seen messages.",
        },
        Knob {
            key: "compact.weight.relevance",
            env: "STAR_COMPACT_RELEVANCE_WEIGHT",
            value_type: KnobType::F64,
            default: Some("0.4"),
            min: Some(0.0),
            max: Some(1.0),
            effect: EffectTiming::NextLoopIteration,
            summary: "How much compaction favours messages relevant to the current task.",
        },
        Knob {
            key: "compact.weight.tool_result",
            env: "STAR_COMPACT_TOOL_RESULT_WEIGHT",
            value_type: KnobType::F64,
            default: Some("0.2"),
            min: Some(0.0),
            max: Some(1.0),
            effect: EffectTiming::NextLoopIteration,
            summary: "How much compaction favours keeping tool output.",
        },
        // ---- 工具输出预算：每次产出结果时解析 ----
        Knob {
            key: "tools.max_result_chars",
            env: "STAR_MAX_TOOL_RESULT_SIZE",
            value_type: KnobType::Usize,
            // 默认值按工具类型分档（见 tools::result_budget::default_max_result_size），
            // 没有单一静态值，因此留 None。
            default: None,
            min: Some(1.0),
            max: None,
            effect: EffectTiming::Immediate,
            summary: "Character cap on a single tool result before it is truncated and spilled to \
                      disk. Raise it when results are being cut off mid-content; lower it if tool \
                      output is crowding out the conversation.",
        },
        // ---- 超时：注意它并不守护主工具循环 ----
        Knob {
            key: "tools.confirmed_call_timeout_secs",
            env: "STAR_TOOL_TIMEOUT_SECS",
            value_type: KnobType::U64,
            default: None,
            min: Some(1.0),
            max: None,
            effect: EffectTiming::Immediate,
            summary:
                "Timeout for a tool call that resumes after an approval prompt (default 180s, \
                      240s for smart_edit/skill, 120s for Bash). NOTE: this does not bound the \
                      main tool loop, which is currently unguarded.",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_keys_are_unique() {
        let mut keys: Vec<&str> = knobs().iter().map(|k| k.key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "duplicate knob key in the catalogue");
    }

    #[test]
    fn knob_envs_are_unique() {
        let mut envs: Vec<&str> = knobs().iter().map(|k| k.env).collect();
        envs.sort_unstable();
        let before = envs.len();
        envs.dedup();
        assert_eq!(before, envs.len(), "duplicate env var in the catalogue");
    }

    #[test]
    fn declared_defaults_parse_as_their_own_type() {
        for knob in knobs() {
            let Some(default) = knob.default else {
                continue;
            };
            assert!(
                knob.value_type.parse(default).is_some(),
                "knob {} default {} does not parse as {:?}",
                knob.key,
                default,
                knob.value_type
            );
        }
    }

    #[test]
    fn lookup_works_by_key_and_by_env() {
        let knob = Knob::by_key("compact.max_tokens").expect("knob exists");
        assert_eq!(knob.env, "STAR_COMPACT_MAX_TOKENS");
        assert_eq!(Knob::by_env("STAR_COMPACT_MAX_TOKENS"), Some(knob));
        assert!(Knob::by_key("compact.nope").is_none());
    }

    #[test]
    fn validate_rejects_out_of_range_and_names_the_bounds() {
        let knob = Knob::by_key("compact.weight.recency").expect("knob exists");
        let err = knob.validate("1.5").expect_err("out of range must fail");
        assert!(err.contains("0..=1"), "error should state the range: {err}");
        assert!(knob.validate("0.75").is_ok());
    }

    #[test]
    fn validate_normalizes_bools_and_accepts_common_spellings() {
        let knob = Knob::by_key("compact.auto_enabled").expect("knob exists");
        assert_eq!(knob.validate("ON").expect("on").as_str(), "true");
        assert_eq!(knob.validate(" 0 ").expect("zero").as_str(), "false");
        assert_eq!(knob.validate("yes").expect("yes").as_str(), "true");
        assert!(knob.validate("maybe").is_err());
    }

    #[test]
    fn validate_rejects_wrong_type_with_the_expected_shape() {
        let knob = Knob::by_key("compact.max_tokens").expect("knob exists");
        let err = knob.validate("lots").expect_err("non-numeric must fail");
        assert!(err.contains("non-negative integer"), "{err}");
        assert!(knob.validate("0").is_err(), "min bound is 1");
    }
}
