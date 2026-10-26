//! `SelfConfig`：模型调节自身运行期设置的唯一入口。
//!
//! # 为什么需要它
//!
//! 这个 agent 的运行期行为高度依赖 `STAR_*` 环境变量（342 个），但模型**看不到**
//! 它们（系统提示词里没有任何一处渲染当前设置），更**改不了**它们 —— 唯一能写
//! 设置的入口在 UI 侧。于是模型观察到"上下文快满了""工具输出被截断了"，只能把
//! 这件事写进给用户的解释里，让用户自己去查文档、改环境变量、重启。
//!
//! 这个工具让模型把这件事自己做掉：它能看到当前有效值和它们的来源，能在运行中
//! 改其中**确实会生效**的那些，改错了能撤销。
//!
//! # 边界：只暴露真正生效的旋钮
//!
//! `core::self_tuning::knobs` 只收**读取点在使用时解析**的设置。构造期捕获的设置
//! （`STAR_LLM_TIMEOUT` 已烤进 `reqwest::Client`）不在表里 —— 给了模型一个写了不
//! 生效的旋钮，比不给更糟：那会让它以为自己调过了。
//!
//! 模型与上下文窗口确实想调，但它们在 `agent_run.rs` 里被构造期冻结进 per-message
//! 的 fresh `Agent`，运行中的回合改不了。`inspect` 把它们作为**只读**项列出并指向
//! `/model`、`/context`，而不是假装它们可调。
//!
//! # 三道防抖
//!
//! 对标 ToolSelf 的 reconfiguration budget，以及 Claude Code 的 managed-settings
//! `lock`。一个发现旋钮能翻的模型会每轮乱翻，所以：
//!
//! 1. **锁**：运维可以用 `STAR_SELFTUNING_LOCKED` 钉住旋钮，模型写入被拒。
//! 2. **预算**：每次会话的成功覆盖次数有限（默认 12），且在动手前就告知。
//! 3. **理由**：`set` 强制要求 `reason`。没有它，事后无法区分"依据证据的调整"和
//!    "随手乱试"，抖动也无从审计。
//!
//! `reset` 永远不消耗预算 —— 撤销必须一直可用，否则模型一旦把值调坏就再也回不去。

use std::sync::Arc;

use serde::Deserialize;

use crate::core::config::Config;
use crate::core::self_tuning::overrides;
use crate::core::self_tuning::report::{self, SessionContext};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};

/// 面向模型的简短描述。完整指引在 `tool-description-selfconfig.md`，
/// 它会被注入系统提示词 bundle（见 `core::prompts::tool_descriptions`）。
const DESCRIPTION: &str = "Read and change your own runtime settings — the knobs that govern \
                           context compaction, tool output budget and tool call timeouts. Use \
                           `inspect` to see current values, their source and your remaining \
                           change budget; `set` to change one; `reset` to undo. Changes take \
                           effect in the running session without a restart.";

#[derive(Clone)]
pub struct SelfConfigTool {
    config: Arc<Config>,
}

impl SelfConfigTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    fn session_context(&self) -> SessionContext {
        let config = self.config.as_ref();
        SessionContext {
            model: config.model().to_string(),
            provider_id: None,
            context_window: config.context_window(),
            context_window_source: context_window_source(config),
            thinking_effort: crate::llm::thinking::current_effort().as_str().to_string(),
            registered_tools: config
                .runtime_tool_registry()
                .map(|registry| registry.get_all_tool_entries().len())
                .unwrap_or(0),
        }
    }
}

/// 如实描述当前上下文容量的来源。
///
/// 报告里的 `source` 列是这个工具的核心价值 —— 模型必须能区分"用户配的"和
/// "我自己上次调的"。这里只能看到 `Config` 上真正可观测的事实，所以就说事实：
/// 显式选择、环境变量、还是 Auto（由 provider/模型能力推导）。
fn context_window_source(config: &Config) -> String {
    if config
        .configured_context_window_selection()
        .as_fixed()
        .is_some()
    {
        return "explicit selection".to_string();
    }
    match std::env::var("STAR_CONTEXT_WINDOW") {
        Ok(value) if !value.trim().is_empty() => format!("STAR_CONTEXT_WINDOW={}", value.trim()),
        _ => "Auto (provider/model capability, built-in fallback)".to_string(),
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SelfConfigParams {
    pub action: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    /// `set` 必填：为什么现在要调这个。强制填写是为了让调整可审计 —— 一个说不清
    /// 理由的调整通常是在瞎试。
    #[serde(default)]
    pub reason: Option<String>,
    /// 是否写入 `~/.star/self-tuning.json` 跨会话存活。默认否：一次会话内的临时
    /// 决策不该变成用户的长期设置。
    #[serde(default)]
    pub persist: Option<bool>,
}

pub struct SelfConfigInvocation {
    config: Arc<Config>,
    params: SelfConfigParams,
}

impl SelfConfigInvocation {
    fn session_context(&self) -> SessionContext {
        SelfConfigTool::new(self.config.clone()).session_context()
    }

    /// 成功结果。报告本身就是给模型读的正文，同时回显给用户看。
    fn ok_with(text: String) -> ToolResult {
        ToolResult {
            llm_content: Some(text.clone()),
            return_display: Some(text.clone()),
            output: text,
            error: None,
            data: None,
        }
    }

    /// 拒绝结果。
    ///
    /// 锁、预算耗尽、取值非法、缺参数都是模型可以据此改变策略的反馈，不是崩溃，
    /// 因此走 `ToolResult.error` 而不是 `Err` —— 模型能读到 `message` 并换个做法，
    /// 而 `Err` 只会变成一条"工具崩了"。
    fn refuse(message: impl Into<String>) -> ToolResult {
        let message = message.into();
        ToolResult {
            llm_content: None,
            return_display: None,
            output: String::new(),
            error: Some(ToolError {
                error_type: "self_tuning_refused".to_string(),
                message,
            }),
            data: None,
        }
    }

    fn run(&self) -> ToolResult {
        match self.params.action.as_str() {
            "inspect" => SelfConfigInvocation::ok_with(report::render(&self.session_context())),
            "set" => self.run_set(),
            "reset" => self.run_reset(),
            other => SelfConfigInvocation::refuse(format!(
                "Unknown action `{other}`. Valid actions: inspect, set, reset."
            )),
        }
    }

    fn run_set(&self) -> ToolResult {
        let Some(key) = self
            .params
            .key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        else {
            return SelfConfigInvocation::refuse(
                "`set` requires `key`. Run the `inspect` action to list the available knobs.",
            );
        };
        let Some(value) = self.params.value.as_deref() else {
            return SelfConfigInvocation::refuse(format!("`set` requires `value` for `{key}`."));
        };
        let Some(reason) = self
            .params
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
        else {
            // 不给理由就拒绝，但不算预算 —— 模型补一个理由再来即可。
            return SelfConfigInvocation::refuse(format!(
                "`set` requires a non-empty `reason` explaining what you observed that makes \
                 `{key}` worth changing now. Run the `inspect` action first if you have not \
                 looked at the current values."
            ));
        };

        let before = Some(overrides::effective_str(key).0);
        let persist = self.params.persist.unwrap_or(false);
        match overrides::set(key, value, persist, Some(reason)) {
            Ok(after) => SelfConfigInvocation::ok_with(format!(
                "{}\nRationale recorded: {reason}\nRemaining budget: {} change(s).",
                report::render_change(
                    "set",
                    &[key.to_string()],
                    &[(key.to_string(), before, Some(after))]
                ),
                overrides::remaining_budget()
            )),
            Err(reason_for_refusal) => SelfConfigInvocation::refuse(reason_for_refusal),
        }
    }

    fn run_reset(&self) -> ToolResult {
        let key = self
            .params
            .key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());

        // 先取改前值，这样报告能告诉模型"撤回了什么"。
        let targets: Vec<String> = match key {
            Some(key) => vec![key.to_string()],
            None => overrides::snapshot().keys().cloned().collect(),
        };
        let before: Vec<(String, Option<String>, Option<String>)> = targets
            .iter()
            .map(|key| {
                let (value, _) = overrides::effective_str(key);
                (key.clone(), Some(value), None)
            })
            .collect();

        match overrides::reset(key) {
            Ok(cleared) => SelfConfigInvocation::ok_with(format!(
                "{}\nCleared {} override(s). Reset does not use the change budget; remaining \
                 budget: {} change(s).",
                report::render_change("reset", &cleared, &before),
                cleared.len(),
                overrides::remaining_budget()
            )),
            Err(reason_for_refusal) => SelfConfigInvocation::refuse(reason_for_refusal),
        }
    }
}

impl ToolInvocation for SelfConfigInvocation {
    fn get_description(&self) -> String {
        match self.params.action.as_str() {
            "inspect" => "Inspect self-tuning knobs and session context".to_string(),
            "set" => format!(
                "Set self-tuning knob {}",
                self.params.key.as_deref().unwrap_or("(unspecified)")
            ),
            "reset" => format!(
                "Reset self-tuning knob {}",
                self.params.key.as_deref().unwrap_or("(all)")
            ),
            other => format!("Self-config action: {other}"),
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { Ok(self.run()) })
    }
}

impl BaseDeclarativeTool for SelfConfigTool {
    fn name(&self) -> &str {
        "SelfConfig"
    }

    fn display_name(&self) -> &str {
        "Self Config"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn kind(&self) -> Kind {
        // Read: 它改的是自己的运行期参数，不碰用户工作区里的任何文件。
        Kind::Read
    }

    fn is_read_only(&self) -> bool {
        // 不能声明只读：`set` 确实改了进程状态，权限系统需要知道这一点。
        false
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["inspect", "set", "reset"],
                    "description": "inspect = read current values, sources and remaining budget; \
                                    set = change one knob; reset = undo overrides (all when key is \
                                    omitted). Reset never consumes budget."
                },
                "key": {
                    "type": "string",
                    "description": "Knob key, exactly as listed by inspect (e.g. \
                                    'compact.max_tokens'). Required for set; optional for reset, \
                                    where omitting it clears every override."
                },
                "value": {
                    "type": "string",
                    "description": "New value as a string. Type and allowed range are per-knob and \
                                    listed by inspect; an out-of-range value is rejected with the \
                                    valid range in the error."
                },
                "reason": {
                    "type": "string",
                    "description": "REQUIRED for set. What you observed that makes this change \
                                    worth doing now. Recorded so the change is auditable."
                },
                "persist": {
                    "type": "boolean",
                    "description": "Write the change to ~/.star/self-tuning.json so it survives \
                                    restarts. Defaults to false — a within-session adjustment \
                                    should not silently become a permanent setting."
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SelfConfigParams = serde_json::from_value(params)?;
        Ok(Box::new(SelfConfigInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Config, ConfigParameters};

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 覆盖表、预算计数器、持久化文件都是进程级状态，测试必须串行且隔离。
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let held = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var(
            overrides::ENV_PATH_OVERRIDE,
            std::env::temp_dir().join("starcode-self-config-tool-test.json"),
        );
        overrides::reset(None).expect("clear overrides");
        overrides::reset_budget_for_test();
        held
    }

    fn tool() -> SelfConfigTool {
        let config = Config::new(ConfigParameters::default());
        SelfConfigTool::new(Arc::new(config))
    }

    fn invoke(tool: &SelfConfigTool, params: serde_json::Value) -> ToolResult {
        let invocation = tool.create_invocation(params).expect("invocation");
        // `execute` 是 async，但这些用例全是纯同步的表层逻辑，用一个
        // current-thread runtime 阻塞跑完即可。
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(invocation.execute(None, None))
            .expect("execute")
    }

    #[test]
    fn inspect_lists_knobs_and_the_change_budget() {
        let _serial = guard();
        let result = invoke(&tool(), serde_json::json!({"action": "inspect"}));

        assert!(result.error.is_none(), "{:?}", result.error);
        let text = result.llm_content.unwrap_or_default();
        assert!(text.contains("compact.max_tokens"), "{text}");
        assert!(text.contains("Change budget for this session"), "{text}");
        assert!(text.contains("Session context (read-only)"), "{text}");
    }

    #[test]
    fn set_applies_and_reports_before_and_after() {
        let _serial = guard();
        let result = invoke(
            &tool(),
            serde_json::json!({
                "action": "set",
                "key": "compact.max_tokens",
                "value": "80000",
                "reason": "context filled up twice this turn"
            }),
        );

        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(overrides::usize_value("compact.max_tokens"), Some(80_000));
        let text = result.llm_content.unwrap_or_default();
        assert!(text.contains("150000") && text.contains("80000"), "{text}");
        assert!(text.contains("Rationale recorded"), "{text}");
    }

    #[test]
    fn set_without_a_reason_is_refused_and_costs_nothing() {
        let _serial = guard();
        let result = invoke(
            &tool(),
            serde_json::json!({"action": "set", "key": "compact.max_tokens", "value": "80000"}),
        );

        assert!(result.error.is_some(), "missing reason must be refused");
        assert!(!overrides::is_overridden("compact.max_tokens"));
        assert_eq!(
            overrides::remaining_budget(),
            overrides::DEFAULT_SESSION_BUDGET,
            "a refusal must not spend budget"
        );
    }

    #[test]
    fn set_reports_the_allowed_range_when_the_value_is_invalid() {
        let _serial = guard();
        let result = invoke(
            &tool(),
            serde_json::json!({
                "action": "set",
                "key": "compact.weight.recency",
                "value": "5.0",
                "reason": "weight does not look high enough"
            }),
        );

        let message = result.error.expect("out of range must fail").message;
        assert!(
            message.contains("0..=1"),
            "error should state the range: {message}"
        );
    }

    #[test]
    fn set_survives_a_restart_only_when_persisted() {
        let _serial = guard();
        let transient = invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.max_tokens", "value": "80000",
                "reason": "transient spike", "persist": false
            }),
        );
        assert!(transient.error.is_none(), "{:?}", transient.error);

        // 模拟重启：清空内存覆盖表（不落盘），再读回持久化文件。
        overrides::clear_in_memory_for_test();
        overrides::load_persisted();
        assert_ne!(
            overrides::usize_value("compact.max_tokens"),
            Some(80_000),
            "a non-persisted change must not survive a restart"
        );

        // 对照：显式 persist 的同一路径必须活下来，否则上面那句可能只是因为
        // 持久化整体坏了。
        let persisted_result = invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.max_tokens", "value": "80000",
                "reason": "worth keeping across sessions", "persist": true
            }),
        );
        assert!(
            persisted_result.error.is_none(),
            "{:?}",
            persisted_result.error
        );

        overrides::clear_in_memory_for_test();
        overrides::load_persisted();
        assert_eq!(
            overrides::usize_value("compact.max_tokens"),
            Some(80_000),
            "a persisted change must survive a restart"
        );
    }

    #[test]
    fn reset_undoes_a_previous_set_and_stays_free() {
        let _serial = guard();
        invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.max_tokens", "value": "80000",
                "reason": "context filling up"
            }),
        );
        let budget_before = overrides::remaining_budget();

        let result = invoke(
            &tool(),
            serde_json::json!({"action": "reset", "key": "compact.max_tokens"}),
        );

        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(!overrides::is_overridden("compact.max_tokens"));
        assert_eq!(
            overrides::remaining_budget(),
            budget_before,
            "reset must not spend budget"
        );
    }

    #[test]
    fn reset_without_a_key_clears_every_override() {
        let _serial = guard();
        for key in ["compact.max_tokens", "compact.auto_enabled"] {
            invoke(
                &tool(),
                serde_json::json!({
                    "action": "set", "key": key, "value": "false", "reason": "test setup"
                }),
            );
        }
        let result = invoke(&tool(), serde_json::json!({"action": "reset"}));
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(overrides::snapshot().is_empty());
    }

    #[test]
    fn locked_knob_is_refused_with_an_explanation() {
        let _serial = guard();
        std::env::set_var(overrides::ENV_LOCKED, "compact.max_tokens");
        let _lock = EnvGuard(overrides::ENV_LOCKED);

        let result = invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.max_tokens", "value": "80000",
                "reason": "context filling up"
            }),
        );

        let message = result.error.expect("locked knob must be refused").message;
        assert!(message.contains("locked"), "{message}");
        assert!(!overrides::is_overridden("compact.max_tokens"));
    }

    #[test]
    fn budget_refusal_still_points_at_reset() {
        let _serial = guard();
        std::env::set_var(overrides::ENV_MAX_SETS, "1");
        let _cap = EnvGuard(overrides::ENV_MAX_SETS);

        invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.max_tokens", "value": "80000",
                "reason": "first"
            }),
        );
        let result = invoke(
            &tool(),
            serde_json::json!({
                "action": "set", "key": "compact.micro_threshold", "value": "50",
                "reason": "second"
            }),
        );

        let message = result.error.expect("budget must be enforced").message;
        assert!(message.contains("budget"), "{message}");
        assert!(
            message.contains("reset"),
            "must advertise the undo path: {message}"
        );
    }

    #[test]
    fn unknown_action_is_refused_with_the_valid_set() {
        let _serial = guard();
        let result = invoke(&tool(), serde_json::json!({"action": "reconfigure"}));
        let message = result.error.expect("unknown action must fail").message;
        assert!(message.contains("inspect, set, reset"), "{message}");
    }

    #[test]
    fn description_and_schema_stay_in_sync_with_the_actions() {
        let _serial = guard();
        let tool = tool();
        let schema = tool.parameter_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("enum in schema")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();

        for action in ["inspect", "set", "reset"] {
            assert!(
                actions.contains(&action),
                "{action} missing from schema: {actions:?}"
            );
        }
        // `reason` 是 set 的硬要求，必须出现在 schema 的描述里，否则模型不会知道。
        assert!(
            schema["properties"]["reason"]["description"]
                .as_str()
                .expect("reason description")
                .contains("REQUIRED"),
            "schema must mark reason as required for set"
        );
    }

    struct EnvGuard(&'static str);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }
}
