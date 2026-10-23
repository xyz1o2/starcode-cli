use crate::agent::loop_engineering::StructuredError;
use crate::core::tools::ToolRegistry;
use crate::llm::client::StarClient;
use crate::types::{StarTool, StarToolCall, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::config::Config;
use crate::core::confirmation_bus::types::{
    Message, MessageBusType, ToolConfirmationRequest, ToolConfirmationResponse, ToolFinished,
    ToolPolicyRejection, ToolStarted,
};
use crate::core::confirmation_bus::MessageBus;
use crate::core::permissions::{PermissionHit, SessionPermissionManager};
use crate::core::policy::{FunctionCall, PolicyDecision};
use crate::core::state::CachedToolResult;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

// 注：确认等待没有超时。等多久是用户的事，不是计时器的事——见
// `MessageBus::await_tool_decision`。
//
// 注：工具执行兜底时限不在本结构上持有（见 `with_watchdog`）。

/// 工具自己跑多久算卡死。
///
/// 这道兜底量的是"工具跑了多久"，**不是**"这次调用过了多久"。门禁等用户的时间
/// 算在 `pass_gate` 里、在这道兜底之外 —— 用户思考十分钟不该把工具判成卡死。
///
/// 默认值给得很宽（30 分钟）：工具大多自带更紧的时限（Shell 有
/// `STAR_SHELL_TIMEOUT` 和不活动超时），这里只接"内置工具集体失联"这种情况。
const DEFAULT_TOOL_WATCHDOG_SECS: u64 = 1800;

fn tool_watchdog() -> std::time::Duration {
    let secs = std::env::var("STAR_TOOL_WATCHDOG_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_TOOL_WATCHDOG_SECS);
    std::time::Duration::from_secs(secs)
}

/// 从 panic payload 里取出可读消息
///
/// `panic!("...")` 的 payload 是 `&str` 或 `String`（`format!` 参数化的那种）；
/// 切片越界这类由 std 抛的也是 `String`。其它类型只能给个占位。
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string())
}

fn should_run_invocation_confirmation(
    approved: bool,
    requires_invocation_confirmation: bool,
) -> bool {
    !approved || requires_invocation_confirmation
}

/// 门禁的结论。`Closed` 一定带着理由 —— 模型需要知道是被规则拒了、被用户拒了，
/// 还是这一轮被中断了，这三者的下一步完全不同。
enum Gate {
    Open,
    Closed(String),
}

/// [`ToolExecutor::pass_gate`] 的入参。参数多到不适合逐个传递，
/// 但它们全是这次调用的既有事实，不该重新计算。
struct GateInput<'a> {
    tool_name: &'a str,
    args: &'a Value,
    tool: &'a dyn crate::core::tools::tools::BaseDeclarativeTool,
    invocation: &'a dyn crate::core::tools::tools::ToolInvocation,
    permission_identity: Option<&'a str>,
    requires_invocation_confirmation: bool,
    /// 模型侧的 `tool_call.id`，UI 靠它把确认卡和工具结果对上。
    originating_tool_call_id: &'a str,
    abort_signal: Option<&'a tokio_util::sync::CancellationToken>,
}

/// `ask` 规则命中、但工具自身没有确认要求时补出来的确认卡片。
///
/// 用户在 settings.json 里写 `ask` 就是想要一次"我看看再决定"的机会，
/// 工具层返回 `Ok(None)` 只是说它没有额外顾虑，不能把这个机会吞掉。
fn policy_confirmation_details(
    tool_name: &str,
    args: &serde_json::Value,
) -> crate::core::tools::tools::ToolCallConfirmationDetails {
    crate::core::tools::tools::ToolCallConfirmationDetails {
        confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
        title: format!("Permission required: {}", tool_name),
        prompt: format!(
            "`{}` matches an `ask` rule in your permission settings.{}",
            tool_name,
            confirmation_summary(args)
        ),
        on_confirm: std::sync::Arc::new(|_, _feedback| {}),
    }
}

/// 确认卡片里附带的命令或路径摘要。
fn confirmation_summary(args: &serde_json::Value) -> String {
    args.get("command")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
        .map(|v| format!("\n\n{}", v))
        .unwrap_or_default()
}

/// 没有命中任何规则、只是兜底 `AskUser` 时的确认卡片。
///
/// 与 `policy_confirmation_details` 的区别只在文案：这里**没有** ask 规则命中，
/// 说"matches an `ask` rule in your permission settings"是凭空捏造的 —— 用户会去
/// settings 里找一条根本不存在的规则。兜底弹框是保守行为，不是用户配置的结果。
fn default_confirmation_details(
    tool_name: &str,
    args: &serde_json::Value,
) -> crate::core::tools::tools::ToolCallConfirmationDetails {
    crate::core::tools::tools::ToolCallConfirmationDetails {
        confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
        title: format!("Permission required: {}", tool_name),
        prompt: format!(
            "`{}` needs approval before it can run. No allow rule covers it yet.{}",
            tool_name,
            confirmation_summary(args)
        ),
        on_confirm: std::sync::Arc::new(|_, _feedback| {}),
    }
}

pub struct ToolExecutor {
    tool_registry: Arc<ToolRegistry>,
    config: Arc<Config>,
    message_bus: Option<Arc<MessageBus>>,
    cached_tool_definitions: RwLock<Option<(u64, Vec<StarTool>)>>,
    permission_manager: Arc<SessionPermissionManager>,
}

impl ToolExecutor {
    /// Cache TTL in milliseconds (5 minutes)
    const CACHE_TTL_MS: u128 = 300_000;
    /// Maximum number of cached entries before eviction
    const MAX_CACHE_ENTRIES: usize = 200;

    pub fn new(
        _client: StarClient,
        tool_registry: Arc<ToolRegistry>,
        message_bus: Option<Arc<MessageBus>>,
        config: Arc<Config>,
    ) -> Self {
        let grants_path = config.storage().project_permission_grants_path();
        // 有运行时共享状态时复用它：`Agent::run_stream` 每条消息都会重建
        // `ToolExecutor`，会话级授予不能跟着 executor 一起重建。
        let permission_manager = match config.runtime_global_state() {
            Some(global_state) => global_state.session_permission_manager(grants_path),
            None => Arc::new(SessionPermissionManager::with_persistence(grants_path)),
        };

        Self {
            tool_registry,
            config,
            message_bus,
            cached_tool_definitions: RwLock::new(None),
            permission_manager,
        }
    }

    pub fn get_tool_definitions(&self) -> Vec<StarTool> {
        let generation = self.tool_registry.generation();
        if let Ok(cache) = self.cached_tool_definitions.read() {
            if let Some((cached_generation, tools)) = cache.as_ref() {
                if *cached_generation == generation {
                    return tools.clone();
                }
            }
        }

        let mut tools = Vec::new();
        let hide_skill = self.config.recursion_depth > 0;

        // Add tools from registry
        for f in self.tool_registry.get_function_declarations() {
            // Tool description single source of truth: tool-description-*.md
            // (external dir → embedded). Falls back to the tool's default
            // description when no .md file is registered for it.
            let description =
                crate::core::prompts::tool_descriptions::resolve_tool_description(&f.name)
                    .unwrap_or_else(|| f.description.clone());
            tools.push(StarTool {
                tool_type: "function".to_string(),
                function: crate::types::StarToolFunction {
                    name: f.name,
                    description,
                    parameters: crate::types::StarToolParameters {
                        param_type: "object".to_string(),
                        properties: f
                            .parameters
                            .get("properties")
                            .cloned()
                            .unwrap_or(serde_json::json!({}))
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                        required: f
                            .parameters
                            .get("required")
                            .cloned()
                            .unwrap_or(serde_json::json!([]))
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .iter()
                            .map(|v| v.as_str().unwrap_or_default().to_string())
                            .collect(),
                    },
                },
            });
        }

        if hide_skill {
            tools.retain(|t| t.function.name != "skill");
        }

        // MCP management tools are injected here so the agent can always orchestrate MCP safely.
        tools.extend(mcp_management_tools());

        let compacted = tools
            .into_iter()
            .map(compact_tool_definition)
            .collect::<Vec<_>>();

        if let Ok(mut cache) = self.cached_tool_definitions.write() {
            *cache = Some((generation, compacted.clone()));
        }

        compacted
    }

    /// 执行一个工具调用。
    ///
    /// # 为什么套一层 `catch_unwind`
    ///
    /// 工具是在 agent worker 那个 tokio 任务里**直接** await 的（`execute_batch` 用
    /// `join_all`，没有 per-tool spawn）。于是任何一个工具 panic，展开的是 worker 自己：
    /// UI→Agent 通道从此静默失效，用户看到的是"消息发出去了，没有任何反应"，而且没有
    /// 任何错误提示。
    ///
    /// 已知触发过这条路径的：`WebSearch` 按字节切中文网页正文（`&content[..2000]` 落在
    /// 汉字中间）。那些切片已经逐一改成按字符，但工具还会继续加，第三方 crate 也会自己
    /// panic（`readability-rust` 就是现成的例子），所以这里留一道兜底 —— panic 变成一条
    /// 普通的工具错误：模型看得到、能换个做法，worker 活着。
    ///
    /// panic hook 仍会先跑一遍，但 `ui::app::runtime` 的 hook 只在渲染线程上拆终端，
    /// 后台线程的 panic 只落日志，不会花屏。
    pub async fn execute(
        &self,
        tool_call: &StarToolCall,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> ToolResult {
        let guarded = std::panic::AssertUnwindSafe(self.execute_inner(
            tool_call,
            update_output,
            abort_signal,
        ));

        match futures::FutureExt::catch_unwind(guarded).await {
            Ok(result) => {
                if result.success {
                    Self::mark_edited_file_dirty(tool_call);
                }
                result
            }
            Err(payload) => {
                let detail = panic_payload_message(payload.as_ref());
                let tool_name = tool_call.function.name.as_str();

                crate::utils::logging::append_debug_log_line(&format!(
                    "[ToolExecutor] tool '{}' panicked ({}) — surfaced as a tool error instead of \
                     unwinding the agent worker",
                    tool_name, detail
                ));

                // 正常路径末尾会发 ToolFinished；panic 跳过了那里，这里补一条，
                // 否则 UI 上这个工具会一直转圈。
                if let Some(bus) = &self.message_bus {
                    let _ = bus
                        .publish(Message::ToolFinished(ToolFinished {
                            message_type: MessageBusType::ToolFinished,
                            tool_call_id: tool_call.id.clone(),
                            tool_name: tool_name.to_string(),
                            success: false,
                        }))
                        .await;
                }

                ToolResult {
                    success: false,
                    output: Some(format!(
                        "Tool `{}` crashed while running ({}). This is a defect in the tool itself, \
                         not in the request. The session is unaffected — try a different approach \
                         or a different tool.",
                        tool_name, detail
                    )),
                    error: Some(format!("tool panicked: {}", detail)),
                    data: None,
                    llm_content: None,
                }
            }
        }
    }

    /// 编辑类工具成功执行后，把目标文件标记为脏 —— 后台索引协调器
    /// （`context::watcher`）会在防抖窗口后做增量重建，语义搜索保持新鲜。
    fn mark_edited_file_dirty(tool_call: &StarToolCall) {
        // 走统一归一：覆盖 Edit/multi_edit/smart_edit/Write/NotebookEdit 及其别名
        if !crate::core::tools::constants::is_edit_tool_name(&tool_call.function.name) {
            return;
        }
        let Ok(args) = serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments)
        else {
            return;
        };
        let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) else {
            return;
        };
        crate::core::context::watcher::mark_files_dirty(std::iter::once(file_path.to_string()));
    }

    /// 给工具的执行加一道兜底时限，超时后转成一条普通的工具错误。
    ///
    /// 必须由调用方自己收窄作用域：这道计时器只该套在 `invocation.execute(...)`
    /// 上。套在更外层（比如整个 `execute_batch`）就会把用户思考的时间也算成
    /// 工具的执行时间，于是用户离开一会儿回来，看到的是一句"执行超时"，
    /// 真正的原因 —— 没人点同意 —— 被彻底吞掉。
    async fn with_watchdog<F>(
        tool_name: &str,
        exec: F,
    ) -> Result<crate::core::tools::tools::ToolResult, Box<dyn std::error::Error>>
    where
        F: std::future::Future<
            Output = Result<crate::core::tools::tools::ToolResult, Box<dyn std::error::Error>>,
        >,
    {
        let limit = tool_watchdog();
        match tokio::time::timeout(limit, exec).await {
            Ok(result) => result,
            Err(_) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "⏱ Tool {} exceeded the {}s watchdog",
                    tool_name,
                    limit.as_secs()
                ));
                Err(format!(
                    "Tool `{}` did not finish within {}s and was stopped. This is a defect or an \
                     unreasonably long-running tool, not a permission decision — narrow the work \
                     and try again.",
                    tool_name,
                    limit.as_secs()
                )
                .into())
            }
        }
    }

    /// 门禁：这次工具调用能不能往下走。
    ///
    /// 只有两种结局，**没有第三种**。尤其没有"等太久了就当拒绝"：等待时长由
    /// 用户决定，不由计时器决定（真正的等待在 [`MessageBus::await_tool_decision`]，
    /// 它没有超时）。
    async fn pass_gate(&self, input: GateInput<'_>) -> Gate {
        let GateInput {
            tool_name,
            args,
            tool,
            invocation,
            permission_identity,
            requires_invocation_confirmation,
            originating_tool_call_id,
            abort_signal,
        } = input;

        // 策略引擎要求弹窗时（`ask` 规则命中），必须真的走到用户面前。
        // 工具自己的 `should_confirm_execute` 返回 `Ok(None)` 只表示
        // "这个工具没有额外顾虑"，不能用来否决一条显式的 `ask` 规则。
        // `AskUser` 有两个来源，语义不同，必须分开：
        // - `requires_prompt`：不管怎样都要弹一次框
        // - `policy_requires_confirmation`：**显式 ask 规则**命中。会话里的
        //   「本会话允许」不该把它降级 —— 用户写 `ask: [...]` 的语义就是
        //   「每次都问我」。
        // 兜底 default（默认 AskUser）只置前者。之前两者混为一谈，导致
        // 没有任何 settings 的用户点「本会话允许」后下一次照样弹框。
        let mut requires_prompt = false;
        let mut policy_requires_confirmation = false;
        let mut approved = false;
        // 卡片是谁要求弹的，只用于面包屑。
        let mut card_source = "unknown";

        if let Some(bus) = &self.message_bus {
            let check = bus
                .check_tool_policy(
                    &FunctionCall {
                        name: tool_name.to_string(),
                        args: Some(args.clone()),
                    },
                    None,
                )
                .await;

            match check.decision {
                PolicyDecision::Deny => {
                    Self::publish_policy_rejection(bus, tool_name, args).await;
                    return Gate::Closed("Tool execution denied by policy.".to_string());
                }
                PolicyDecision::DenyWithReason(reason) => {
                    Self::publish_policy_rejection(bus, tool_name, args).await;
                    return Gate::Closed(format!("Tool execution denied: {}", reason));
                }
                PolicyDecision::Allow => approved = true,
                PolicyDecision::AskUser => {
                    requires_prompt = true;
                    if check.explicit_rule {
                        policy_requires_confirmation = true;
                    }
                }
            }
        }

        // 会话/持久授权：用户按下「本会话允许 / 总是允许」时给**这个工具**的授权，
        // 必须读回来。
        //
        // 曾经这里挂着 `&& !requires_invocation_confirmation`：工具只要自带逐次确认
        // （`Edit` 在 `folder_trust` 打开时就是），整块就被跳过 —— 授权写进了
        // `SessionPermissionManager` 却永远读不回来，用户点完「允许本会话」，下一次
        // 编辑照样弹框。授权必须是**读的**，写了不读等于没写。
        //
        // 仍然压得住的只有一件事：用户自己写下的 `ask` 规则，语义就是"每次都问我"。
        if !policy_requires_confirmation {
            let permission_hit = self.permission_manager.check_allowed_with_identity(
                tool_name,
                args,
                permission_identity,
            );
            if permission_hit != PermissionHit::None {
                let source = match permission_hit {
                    PermissionHit::ToolSession => "tool_session",
                    PermissionHit::ToolPersisted => "tool_persisted",
                    PermissionHit::SignatureSession => "signature_session",
                    PermissionHit::SignaturePersisted => "signature_persisted",
                    PermissionHit::None => "none",
                };
                crate::utils::logging::append_debug_log_line(&format!(
                    "✅ Permission: Allowing {} without prompt (source={})",
                    tool_name, source
                ));
                approved = true;
            }
        }

        // 工具自己的逐次检查：它知道参数级/路径级的顾虑（`Edit` 关心未信任目录），
        // 通用授权不该替它做主，所以这条路径不受上面的 `approved` 影响。
        if !should_run_invocation_confirmation(approved, requires_invocation_confirmation) {
            return Gate::Open;
        }

        let Some(bus) = &self.message_bus else {
            // 没有总线（legacy/test 模式）：没人能问，也没人能拒。
            return Gate::Open;
        };

        let should_confirm = invocation.should_confirm_execute(abort_signal).await;
        let details = match should_confirm {
            Ok(Some(details)) => {
                card_source = "tool";
                details
            }
            // 工具自己没话说。兜底卡片只在**没有人授权过**的时候才需要：一旦策略
            // 放行、或用户已经给过会话/持久授权，就不该再弹。
            //
            // 卡片文案两种要分开：显式 ask 规则才配说 "matches an `ask` rule"，
            // 兜底 default 没有规则可指。
            Ok(None) if requires_prompt && !approved => {
                card_source = if policy_requires_confirmation {
                    "ask_rule"
                } else {
                    "fallback_default"
                };
                if policy_requires_confirmation {
                    policy_confirmation_details(tool_name, args)
                } else {
                    default_confirmation_details(tool_name, args)
                }
            }
            Ok(None) => return Gate::Open,
            Err(e) => return Gate::Closed(e.to_string()),
        };
        let on_confirm = details.on_confirm.clone();

        // 每次弹框都留一条面包屑。
        //
        // 这条缺失让"允许本会话为什么没生效"根本查不下去：授权命中时日志里有
        // "Allowing X without prompt"，可**弹框本身一声不吭**，于是日志里出现
        // "刚放行过，下一个调用却又弹了"这种自相矛盾的现象，却拿不到任何证据
        // 说明第二次到底是谁要求弹的。卡片来源（工具自己 / ask 规则 / 兜底）正是
        // 判断依据，缺了它就只能靠猜。
        crate::utils::logging::append_debug_log_line(&format!(
            "🔔 Asking for approval: {} (source={}, granted={}, policy_ask={}, explicit_rule={})",
            tool_name, card_source, approved, requires_prompt, policy_requires_confirmation
        ));

        let request = ToolConfirmationRequest {
            message_type: MessageBusType::ToolConfirmationRequest,
            tool_call: FunctionCall {
                name: tool_name.to_string(),
                args: Some(args.clone()),
            },
            // correlation_id 由 bus 在发出去之前填。
            correlation_id: String::new(),
            server_name: None,
            title: Some(details.title),
            prompt: Some(details.prompt),
            // UI 手上只有模型侧的 tool_call.id，靠它把"这张卡"和稍后到达的
            // ToolResult 对上；不上的话卡片收不到自己那次调用的结局。
            originating_tool_call_id: Some(originating_tool_call_id.to_string()),
        };

        // 唯一的等待点。没有超时：用户没回答就一直等，用户中断就立刻收手。
        let response = match bus.await_tool_decision(request, abort_signal).await {
            Ok(response) => response,
            Err(e) => return Gate::Closed(e.to_string()),
        };

        if !response.confirmed {
            crate::utils::logging::append_debug_log_line(&format!(
                "🚫 Approval declined for {} (source={})",
                tool_name, card_source
            ));
            // 用户明确拒绝（含 ESC 取消）。这是一次决定，不是错误、更不是超时。
            return Gate::Closed(format!(
                "The user declined to run `{}`. Do not retry the same call — say what you wanted \
                 to do, or ask the user how to proceed.",
                tool_name
            ));
        }

        self.record_approval(
            tool,
            tool_name,
            args,
            permission_identity,
            &response,
            &on_confirm,
        );
        Gate::Open
    }

    /// 记下"用户批准了这一次"，并把「本会话允许 / 总是允许」落到权限缓存上。
    fn record_approval(
        &self,
        tool: &dyn crate::core::tools::tools::BaseDeclarativeTool,
        tool_name: &str,
        args: &Value,
        permission_identity: Option<&str>,
        response: &ToolConfirmationResponse,
        on_confirm: &Arc<
            dyn Fn(crate::types::ToolConfirmationOutcome, Option<String>) + Send + Sync,
        >,
    ) {
        let requested_outcome = response
            .outcome
            .clone()
            .unwrap_or(crate::types::ToolConfirmationOutcome::ProceedOnce);
        let outcome = tool.normalize_confirmation_outcome(requested_outcome.clone());
        if outcome != requested_outcome {
            crate::utils::logging::append_debug_log_line(&format!(
                "🔐 Permission outcome normalized for {}: {:?} -> {:?}",
                tool_name, requested_outcome, outcome
            ));
        }
        on_confirm(outcome.clone(), response.feedback.clone());

        match outcome {
            crate::types::ToolConfirmationOutcome::AllowSession => {
                self.permission_manager
                    .allow_tool_session_with_identity(tool_name, permission_identity);
                crate::utils::logging::append_debug_log_line(&format!(
                    "🔐 Permission Granted: Allowing tool {} for session",
                    tool_name
                ));
            }
            crate::types::ToolConfirmationOutcome::ProceedAlways => {
                self.permission_manager.allow_action_with_identity(
                    tool_name,
                    args,
                    permission_identity,
                );
                crate::utils::logging::append_debug_log_line(&format!(
                    "🔐 Permission Granted: Allowing {} for session (signature)",
                    tool_name
                ));
            }
            crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave => {
                self.permission_manager
                    .allow_tool_persisted_with_identity(tool_name, permission_identity);
                crate::utils::logging::append_debug_log_line(&format!(
                    "🔐 Permission Granted: Persisted allow for tool {}",
                    tool_name
                ));
            }
            _ => {}
        }
    }

    /// 策略拒绝也要让 UI 画得出来：只回一条给模型的错误，界面上看不出是规则拦的。
    async fn publish_policy_rejection(bus: &MessageBus, tool_name: &str, args: &Value) {
        let _ = bus
            .publish(Message::ToolPolicyRejection(ToolPolicyRejection {
                message_type: MessageBusType::ToolPolicyRejection,
                tool_call: FunctionCall {
                    name: tool_name.to_string(),
                    args: Some(args.clone()),
                },
            }))
            .await;
    }

    async fn execute_inner(
        &self,
        tool_call: &StarToolCall,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> ToolResult {
        let original_tool_name = tool_call.function.name.as_str();
        let resolved_tool_name = Self::canonical_tool_name(original_tool_name);
        let name = resolved_tool_name.as_str();
        if self.config.recursion_depth > 0 && name == "skill" {
            return Self::error_result(
                "Skill tool is disabled for sub-agents to prevent recursion."
                    .to_string()
                    .into(),
            );
        }
        let mut args: Value =
            serde_json::from_str(&tool_call.function.arguments).unwrap_or(Value::Null);
        // LLM may send null or empty arguments; convert to empty object so
        // deserialization into tool params structs doesn't fail.
        if args.is_null() {
            args = Value::Object(serde_json::Map::new());
        }
        // 部分 OpenAI-compatible 提供方把整段 arguments 再套一层 JSON 字符串
        // （`"arguments": "{\"todos\": […]}"`）。第一层 from_str 会把它解成
        // `Value::String`，原样下发的话每个工具都收到一个字符串，报
        // `invalid type: string, expected struct …`——模型重试多少次都一样。
        // 工具入参的顶层一律是对象，所以只剥「字符串化的 JSON 对象」这一层，
        // 且必须是合法 JSON；剥不动的原样下发，交给各工具的严格解析报错
        // （`tasks.rs` 的 strictObject 约定保持不变）。
        args = Self::unwrap_stringified_args(args);
        let args = Self::normalize_tool_args(original_tool_name, name, args);

        // Check for cancellation before starting
        if let Some(ref token) = abort_signal {
            if token.is_cancelled() {
                return Self::error_result("Tool execution cancelled before start.".into());
            }
        }

        // --- Cache Logic: Read ---
        let is_readonly = self.is_tool_read_only(name);
        let global_state = self.tool_registry.get_config().runtime_global_state();
        let cache_key = if is_readonly {
            Some(format!("{}:{}", name, args.to_string()))
        } else {
            None
        };

        if let Some(key) = &cache_key {
            if let Some(state) = &global_state {
                let cache = state.tool_cache.read().await;
                if let Some(cached) = cache.get(key) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let age_ms = now.saturating_sub(cached.timestamp);
                    if age_ms < Self::CACHE_TTL_MS {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "🚀 Cache Hit: {} (age: {}ms)",
                            name, age_ms
                        ));

                        if let Some(bus) = &self.message_bus {
                            let _ = bus
                                .publish(Message::ToolStarted(ToolStarted {
                                    message_type: MessageBusType::ToolStarted,
                                    tool_call_id: tool_call.id.clone(),
                                    tool_name: name.to_string(),
                                }))
                                .await;
                        }

                        if let Some(bus) = &self.message_bus {
                            let _ = bus
                                .publish(Message::ToolFinished(ToolFinished {
                                    message_type: MessageBusType::ToolFinished,
                                    tool_call_id: tool_call.id.clone(),
                                    tool_name: name.to_string(),
                                    success: cached.result.success,
                                }))
                                .await;
                        }

                        // 命中缓存时 `execute()` 被整个跳过了，而"读过的文件要登记
                        // 进 read state"正是写在工具 `execute()` 里的副作用。于是同
                        // 一个 `Read` 第二次命中缓存时：内容返回给模型了，登记却
                        // 没有发生。配合 5 分钟 TTL，`Read` → `Edit` → `Read` →
                        // `Edit` 可以无限循环在 `[edit_file_not_read]` 上。
                        //
                        // 缓存省的是重复读盘，不是这个登记，所以命中时把它补回来。
                        // 先 clone 再释放读锁：`rerecord_read_paths` 自己也要拿锁。
                        let cached_result = cached.result.clone();
                        drop(cache);
                        self.rerecord_read_paths(name, &args).await;

                        return cached_result;
                    }
                    // else: stale — don't use, fall through to re-execute
                }
                drop(cache);
            }
        }
        // -------------------------

        // Emit ToolStarted
        if let Some(bus) = &self.message_bus {
            let _ = bus
                .publish(Message::ToolStarted(ToolStarted {
                    message_type: MessageBusType::ToolStarted,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: name.to_string(),
                }))
                .await;
        }

        // Try registry
        let result = if let Some(tool) = self.tool_registry.get_tool(name) {
            let permission_identity = tool.permission_cache_identity();
            let requires_invocation_confirmation = tool.requires_invocation_confirmation();
            match tool.create_invocation(args.clone()) {
                Ok(invocation) => {
                    // 门禁先行：没批准就一步都不许往下走。
                    match self
                        .pass_gate(GateInput {
                            tool_name: name,
                            args: &args,
                            tool: tool.as_ref(),
                            invocation: invocation.as_ref(),
                            permission_identity: permission_identity.as_deref(),
                            requires_invocation_confirmation,
                            originating_tool_call_id: &tool_call.id,
                            abort_signal: abort_signal.as_ref(),
                        })
                        .await
                    {
                        Gate::Open => {}
                        Gate::Closed(reason) => return Self::error_result(reason.into()),
                    }

                    {
                        // First execution. The watchdog wraps **only** the tool run:
                        // it bounds a hung tool, not the time a human spends deciding.
                        let first_result = match Self::with_watchdog(
                            name,
                            invocation.execute(abort_signal.as_ref(), update_output.clone()),
                        )
                        .await
                        {
                            Ok(core_result) => {
                                let success = core_result.error.is_none();
                                let error_msg =
                                    core_result.error.clone().map(|e| e.message.clone());
                                let output = core_result
                                    .return_display
                                    .clone()
                                    .unwrap_or_else(|| core_result.output.clone());

                                // 给模型的正文：工具自己的 `llm_content` 优先，
                                // 没有就退回 `output`。
                                //
                                // 曾经这里只取 `output`，而 `llm_content` 被整个
                                // 丢掉 —— `tool_search` 那种"摘要一行、正文另存"的
                                // 工具，模型收到的只有 `"Found 3 tool(s) for 'exa'"`，
                                // 工具名和 schema 全丢，于是只能反复瞎猜名字。
                                // `shell.rs` 里那段注释（"llm_content 根本没人读"）
                                // 描述的就是这个状态。
                                let llm_content = core_result
                                    .llm_content
                                    .clone()
                                    .filter(|s| !s.trim().is_empty())
                                    .or_else(|| Some(output.clone()))
                                    .filter(|s| !s.trim().is_empty());

                                // Enrich error with structured error info
                                if !success {
                                    if let Some(ref err) = error_msg {
                                        let structured =
                                            StructuredError::from_tool_output(name, &output, err);
                                        let mut data = core_result
                                            .data
                                            .clone()
                                            .unwrap_or_else(|| serde_json::json!({}));
                                        if let Some(obj) = data.as_object_mut() {
                                            obj.insert(
                                                "structured_error".to_string(),
                                                structured.to_json_value(),
                                            );
                                        }
                                        ToolResult {
                                            success: false,
                                            output: Some(structured.format_display()),
                                            error: error_msg,
                                            data: Some(data),
                                            llm_content: None,
                                        }
                                    } else {
                                        ToolResult {
                                            success: false,
                                            output: Some(output),
                                            error: error_msg,
                                            data: core_result.data.clone(),
                                            llm_content: None,
                                        }
                                    }
                                } else {
                                    ToolResult {
                                        success: true,
                                        output: Some(output),
                                        error: None,
                                        data: core_result.data.clone(),
                                        llm_content,
                                    }
                                }
                            }
                            Err(e) => return Self::error_result(e),
                        };

                        // Check for sandbox restriction and auto-retry without sandbox
                        if !first_result.success {
                            if let Some(ref data) = first_result.data {
                                if let Some(should_retry) = data.get("should_retry_without_sandbox")
                                {
                                    if should_retry.as_bool().unwrap_or(false) {
                                        crate::utils::logging::append_debug_log_line(
                                             &format!("🔄 Sandbox restriction detected, retrying without sandbox for {}", name)
                                         );

                                        // Re-execute with sandbox disabled
                                        let retry_args = if name == "Bash" {
                                            let mut map =
                                                args.as_object().cloned().unwrap_or_default();
                                            map.insert(
                                                "dangerously_disable_sandbox".to_string(),
                                                serde_json::json!(true),
                                            );
                                            serde_json::Value::Object(map)
                                        } else {
                                            args.clone()
                                        };

                                        // Create new invocation with modified args
                                        if let Some(tool) = self.tool_registry.get_tool(name) {
                                            if let Ok(retry_invocation) =
                                                tool.create_invocation(retry_args)
                                            {
                                                match Self::with_watchdog(
                                                    name,
                                                    retry_invocation.execute(
                                                        abort_signal.as_ref(),
                                                        update_output,
                                                    ),
                                                )
                                                .await
                                                {
                                                    Ok(retry_result) => {
                                                        return ToolResult {
                                                            success: retry_result.error.is_none(),
                                                            output: Some(
                                                                retry_result
                                                                    .return_display
                                                                    .unwrap_or(retry_result.output),
                                                            ),
                                                            error: retry_result
                                                                .error
                                                                .map(|e| e.message),
                                                            data: retry_result.data,
                                                            llm_content: None,
                                                        };
                                                    }
                                                    Err(e) => {
                                                        return Self::error_result(e);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        first_result
                    }
                }
                Err(e) => Self::error_result(e),
            }
        } else if is_mcp_management_tool(name) {
            execute_mcp_management_tool(&self.tool_registry, name, &args).await
        } else if name.starts_with("mcp__") {
            execute_mcp_dynamic_tool(&self.tool_registry, name, &args).await
        } else {
            Self::error_result(format!("Unknown tool: {}", name).into())
        };

        // --- Cache Logic: Write/Invalidate ---
        if let Some(state) = &global_state {
            if let Some(key) = cache_key {
                // Is read-only, save result if successful
                if result.success {
                    let mut cache = state.tool_cache.write().await;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    cache.insert(
                        key,
                        CachedToolResult {
                            result: result.clone(),
                            timestamp: now,
                        },
                    );
                    // Evict oldest entries if over limit
                    if cache.len() > Self::MAX_CACHE_ENTRIES {
                        let to_remove: usize = cache.len() - Self::MAX_CACHE_ENTRIES;
                        let mut entries: Vec<_> = cache.iter().collect();
                        entries.sort_by_key(|(_, v)| v.timestamp);
                        let keys_to_remove: Vec<String> = entries
                            .into_iter()
                            .take(to_remove)
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in &keys_to_remove {
                            cache.remove(k);
                        }
                    }
                }
            } else {
                // Write operation -> Invalidate
                let mut cache = state.tool_cache.write().await;
                if !cache.is_empty() {
                    cache.clear();
                    crate::utils::logging::append_debug_log_line(
                        "🧹 Cache invalidated due to write operation",
                    );
                }
            }
        }
        // -------------------------------------

        // Emit ToolFinished
        if let Some(bus) = &self.message_bus {
            let _ = bus
                .publish(Message::ToolFinished(ToolFinished {
                    message_type: MessageBusType::ToolFinished,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: name.to_string(),
                    success: result.success,
                }))
                .await;
        }

        result
    }
    fn error_result(e: Box<dyn std::error::Error>) -> ToolResult {
        let error_msg = e.to_string();
        let structured = StructuredError::from_tool_output("unknown", "", &error_msg);
        let mut data = serde_json::Map::new();
        data.insert("structured_error".to_string(), structured.to_json_value());
        ToolResult {
            success: false,
            output: Some(structured.format_display()),
            error: Some(error_msg),
            data: Some(serde_json::Value::Object(data)),
            llm_content: None,
        }
    }

    /// 缓存命中后补做"读过的文件要登记"这一步。
    ///
    /// 只处理两个"读"类工具，因为只有它们把路径交给模型、也只有它们的登记会被
    /// `Edit`/`Write`/`multi_edit`/`notebook_edit` 的前置检查消费。登记的是**当前
    /// 磁盘内容**，不是缓存里那份：缓存里的内容可能已经过时，而登记过期内容等于
    /// 放行一次基于旧版本的编辑。
    async fn rerecord_read_paths(&self, name: &str, args: &Value) {
        let paths: Vec<String> = match Self::canonical_tool_name(name).as_str() {
            "Read" | "read_file" => args
                .get("file_path")
                .and_then(Value::as_str)
                .map(|p| vec![p.to_string()])
                .or_else(|| {
                    args.get("file_paths")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                })
                .unwrap_or_default(),
            "notebook_read" => args
                .get("file_path")
                .and_then(Value::as_str)
                .map(|p| vec![p.to_string()])
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if paths.is_empty() {
            return;
        }
        let Some(global_state) = self.tool_registry.get_config().runtime_global_state() else {
            return;
        };
        let config = self.tool_registry.get_config();
        let base_dir = config.target_dir();
        // 显式成 `&Path`：`resolve_tool_path` 要的是 `&Path`，传 `&PathBuf` 会
        // 触发 clippy 的 needless_borrow 误报（它看不出这是解引用强制转换）。
        let base: &std::path::Path = base_dir.as_path();
        for raw in &paths {
            let resolved = crate::core::utils::paths::resolve_tool_path(base, raw);
            let Ok(content) =
                crate::core::utils::file_utils::read_file_with_encoding_async(&resolved).await
            else {
                continue;
            };
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let file_system_timestamp = tokio::fs::metadata(&resolved)
                .await
                .and_then(|m| m.modified())
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis())
                .unwrap_or(timestamp);
            crate::core::tools::read_file::record_file_read(
                &global_state,
                &crate::core::utils::paths::read_state_key(&resolved),
                crate::core::tools::edit::normalize_line_endings(&content),
                timestamp,
                file_system_timestamp,
            )
            .await;
        }
    }

    pub fn is_tool_read_only(&self, name: &str) -> bool {
        let canonical_name = Self::canonical_tool_name(name);
        let name = canonical_name.as_str();
        if matches!(
            name,
            "mcp_list_servers" | "mcp_list_tools" | "mcp_tool_info" | "mcp_search_tools"
        ) {
            return true;
        }
        if let Some(tool) = self.tool_registry.get_tool(name) {
            tool.is_read_only()
        } else {
            false
        }
    }

    fn canonical_tool_name(name: &str) -> String {
        crate::core::tools::constants::canonical_tool_name(name)
    }

    /// 剥掉「字符串化的 JSON」一层外壳（见上面的调用点注释）。
    ///
    /// 只剥一层、且只在解出来是对象时收手：工具入参顶层永远是对象，解出数字/
    /// 布尔/裸串说明模型发的本来就不是参数对象，那种情况留给严格解析去报错。
    fn unwrap_stringified_args(args: Value) -> Value {
        let Value::String(s) = args else {
            return args;
        };
        match serde_json::from_str::<Value>(&s) {
            Ok(inner) if inner.is_object() => inner,
            // 字符串里装的还是字符串（套了两层）或别的——不再猜，原样退回。
            _ => Value::String(s),
        }
    }

    fn normalize_tool_args(original_name: &str, canonical_name: &str, args: Value) -> Value {
        // 先对所有工具做通用修复：
        // 1. 把字符串数字转为真正的数字
        // 2. 把 camelCase 参数名转为 snake_case
        let args = Self::fix_string_numbers(args);
        let args = Self::camel_to_snake_keys(args);
        match canonical_name {
            "Edit" => Self::normalize_replace_args(args),
            "multi_edit" => Self::normalize_multi_edit_args(args),
            "Grep" => Self::normalize_search_args(original_name, args),
            "Read" => Self::normalize_read_file_args(args),
            "Agent" => Self::normalize_agent_args(args),
            _ => args,
        }
    }

    /// 递归遍历 JSON args，把 camelCase 的 key 转为 snake_case
    /// 例如: filePath → file_path, oldString → old_string
    fn camel_to_snake_keys(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    let snake_key = Self::camel_to_snake(&k);
                    new_map.insert(snake_key, Self::camel_to_snake_keys(v));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Self::camel_to_snake_keys).collect())
            }
            other => other,
        }
    }

    /// 把 camelCase 字符串转为 snake_case
    fn camel_to_snake(s: &str) -> String {
        // 如果已经是 snake_case 或全小写，直接返回
        if !s.chars().any(|c| c.is_uppercase()) {
            return s.to_string();
        }
        let mut result = String::with_capacity(s.len() + 4);
        for (i, ch) in s.chars().enumerate() {
            if ch.is_uppercase() {
                if i > 0 {
                    result.push('_');
                }
                result.push(ch.to_lowercase().next().unwrap_or(ch));
            } else {
                result.push(ch);
            }
        }
        result
    }

    /// 归一化 read_file 参数，处理 LLM 常见的参数格式错误
    fn normalize_read_file_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };
        // 兼容各种参数名
        Self::copy_string_arg(
            obj,
            "file_path",
            &["file_path", "path", "filePath", "filename", "file"],
        );
        args
    }

    /// 归一化 Agent 参数，处理 subagent_type 大小写问题
    fn normalize_agent_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };
        // subagent_type: 兼容 camelCase 和 PascalCase
        if let Some(val) = obj
            .get("subagent_type")
            .or(obj.get("subagentType"))
            .cloned()
        {
            if let Some(s) = val.as_str() {
                let lower = s.to_lowercase();
                let normalized = match lower.as_str() {
                    "general_purpose" | "generalpurpose" | "general" => "general_purpose",
                    "explorer" | "explore" => "explorer",
                    "analyzer" | "analyze" => "analyzer",
                    "editor" | "edit" => "editor",
                    "code_reviewer" | "codereviewer" | "reviewer" => "code_reviewer",
                    other => other,
                };
                obj.insert(
                    "subagent_type".to_string(),
                    Value::String(normalized.to_string()),
                );
                obj.remove("subagentType");
            }
        }
        args
    }

    /// 递归遍历 JSON args，把字符串数字转为真正的数字类型
    fn fix_string_numbers(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (k, v) in map {
                    new_map.insert(k, Self::fix_string_numbers(v));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(Self::fix_string_numbers).collect())
            }
            Value::String(s) => {
                // 尝试解析为 u64/i64/f64，成功后返回数字类型
                if let Ok(n) = s.parse::<i64>() {
                    Value::Number(serde_json::Number::from(n))
                } else if let Ok(n) = s.parse::<u64>() {
                    Value::Number(serde_json::Number::from(n))
                } else if let Ok(n) = s.parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(n) {
                        Value::Number(num)
                    } else {
                        Value::String(s)
                    }
                } else {
                    Value::String(s)
                }
            }
            other => other,
        }
    }

    fn normalize_replace_args(mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        Self::copy_string_arg(
            obj,
            "file_path",
            &["file_path", "path", "target_file", "file"],
        );
        Self::copy_string_arg(
            obj,
            "old_string",
            &["old_string", "old_str", "old", "oldString", "old_text"],
        );
        Self::copy_string_arg(
            obj,
            "new_string",
            &["new_string", "new_str", "new", "newString", "new_text"],
        );

        args
    }

    /// 归一化 multi_edit 参数，处理 LLM 常见的参数格式错误
    fn normalize_multi_edit_args(mut args: Value) -> Value {
        // 如果 LLM 错误地使用了 flat 格式（跟 replace 一样），自动包装成 edits 数组
        if args.as_object().map_or(false, |obj| {
            obj.contains_key("file_path") && !obj.contains_key("edits")
        }) {
            let obj = args.as_object_mut().unwrap();
            // 提取 file_path, old_string, new_string
            let file_path =
                Self::first_string_arg(obj, &["file_path", "path", "target_file", "file"]);
            let old_string = Self::first_string_arg(
                obj,
                &["old_string", "old_str", "old", "oldString", "old_text"],
            );
            let new_string = Self::first_string_arg(
                obj,
                &["new_string", "new_str", "new", "newString", "new_text"],
            );

            if let (Some(file_path), Some(old_string), Some(new_string)) =
                (file_path, old_string, new_string)
            {
                let edit = serde_json::json!({
                    "file_path": file_path,
                    "old_string": old_string,
                    "new_string": new_string,
                });
                args = serde_json::json!({ "edits": [edit] });
            }
            return args;
        }

        // 归一化 edits 数组中每个元素的字段名
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        if let Some(Value::Array(edits)) = obj.get_mut("edits") {
            for edit in edits.iter_mut() {
                if let Some(edit_obj) = edit.as_object_mut() {
                    Self::copy_string_arg(
                        edit_obj,
                        "file_path",
                        &["file_path", "path", "target_file", "file"],
                    );
                    Self::copy_string_arg(
                        edit_obj,
                        "old_string",
                        &["old_string", "old_str", "old", "oldString", "old_text"],
                    );
                    Self::copy_string_arg(
                        edit_obj,
                        "new_string",
                        &["new_string", "new_str", "new", "newString", "new_text"],
                    );
                }
            }
        }

        args
    }

    fn normalize_search_args(original_name: &str, mut args: Value) -> Value {
        let Some(obj) = args.as_object_mut() else {
            return args;
        };

        Self::copy_string_arg(obj, "query", &["query", "pattern"]);

        if obj.get("search_type").is_none() && matches!(original_name, "Grep") {
            obj.insert("search_type".to_string(), Value::String("text".to_string()));
        }

        if obj.get("regex").is_none() && matches!(original_name, "Grep") {
            obj.insert("regex".to_string(), Value::Bool(true));
        }

        if obj.get("include_pattern").is_none() {
            let legacy_path = Self::first_string_arg(obj, &["path", "dir_path"]);
            let legacy_file_pattern =
                Self::first_string_arg(obj, &["file_pattern", "include", "Glob"]);
            if let Some(include_pattern) = Self::build_legacy_search_include_pattern(
                legacy_path.as_deref(),
                legacy_file_pattern.as_deref(),
            ) {
                obj.insert(
                    "include_pattern".to_string(),
                    Value::String(include_pattern),
                );
            }
        }

        args
    }

    fn copy_string_arg(
        obj: &mut serde_json::Map<String, Value>,
        target_key: &str,
        candidate_keys: &[&str],
    ) {
        if obj.get(target_key).and_then(|v| v.as_str()).is_some() {
            return;
        }

        if let Some(value) = Self::first_string_arg(obj, candidate_keys) {
            obj.insert(target_key.to_string(), Value::String(value));
        }
    }

    fn first_string_arg(
        obj: &serde_json::Map<String, Value>,
        candidate_keys: &[&str],
    ) -> Option<String> {
        candidate_keys.iter().find_map(|key| {
            obj.get(*key)
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty())
        })
    }

    fn build_legacy_search_include_pattern(
        legacy_path: Option<&str>,
        legacy_file_pattern: Option<&str>,
    ) -> Option<String> {
        let normalized_path = legacy_path
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != ".")
            .map(Self::normalize_search_scope);
        let normalized_file_pattern = legacy_file_pattern
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.replace('\\', "/"));

        match (normalized_path, normalized_file_pattern) {
            (None, None) => None,
            (None, Some(pattern)) => Some(pattern),
            (Some(path), None) => {
                if Self::looks_like_glob(&path) || std::path::Path::new(&path).extension().is_some()
                {
                    Some(path)
                } else {
                    Some(format!("{}/**", path.trim_end_matches('/')))
                }
            }
            (Some(path), Some(pattern)) => {
                let path = path.trim_end_matches('/');
                let pattern = pattern.trim_start_matches('/');
                if Self::looks_like_glob(pattern) && pattern.starts_with(path) {
                    Some(pattern.to_string())
                } else if pattern.contains('/') {
                    Some(format!("{}/{}", path, pattern))
                } else {
                    Some(format!("{}/**/{}", path, pattern))
                }
            }
        }
    }

    fn normalize_search_scope(raw: &str) -> String {
        let path = std::path::Path::new(raw);
        if path.is_absolute() {
            let cwd = crate::core::utils::paths::current_dir_cached();
            if let Ok(relative) = path.strip_prefix(cwd) {
                return relative.to_string_lossy().replace('\\', "/");
            }
        }

        raw.trim_start_matches("./").replace('\\', "/")
    }

    fn looks_like_glob(value: &str) -> bool {
        value.contains('*') || value.contains('?') || value.contains('[') || value.contains('{')
    }

    /// 获取最大工具并发数（从环境变量读取，默认10）
    fn max_tool_concurrency() -> usize {
        std::env::var("STAR_MAX_TOOL_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10)
            .max(1) // 至少1个并发
    }

    pub async fn execute_batch(
        &self,
        tool_calls: Vec<StarToolCall>,
        update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        abort_signal: Option<tokio_util::sync::CancellationToken>,
    ) -> Vec<ToolResult> {
        let mut tool_calls_with_index: Vec<(usize, StarToolCall)> =
            tool_calls.into_iter().enumerate().collect();
        let mut groups: Vec<Vec<(usize, StarToolCall)>> = Vec::new();

        if tool_calls_with_index.is_empty() {
            return Vec::new();
        }

        let first = tool_calls_with_index.remove(0);
        let mut current_group = vec![first];
        let mut is_current_group_readonly =
            self.is_tool_read_only(&current_group[0].1.function.name);

        for (index, call) in tool_calls_with_index {
            let is_readonly = self.is_tool_read_only(&call.function.name);

            if is_readonly == is_current_group_readonly {
                current_group.push((index, call));
            } else {
                groups.push(current_group);
                current_group = vec![(index, call)];
                is_current_group_readonly = is_readonly;
            }
        }
        groups.push(current_group);

        let max_concurrency = Self::max_tool_concurrency();
        let mut final_results: Vec<(usize, ToolResult)> = Vec::new();

        for group in groups {
            if group.is_empty() {
                continue;
            }
            let is_readonly = self.is_tool_read_only(&group[0].1.function.name);

            if is_readonly && group.len() > 1 {
                // Parallel execution for read-only tools with concurrency limit
                use futures::future::join_all;

                // Split into chunks of max_concurrency size
                for chunk in group.chunks(max_concurrency) {
                    let futures: Vec<_> = chunk
                        .iter()
                        .map(|(index, call)| {
                            let uo = update_output.clone();
                            let sig = abort_signal.clone();
                            let call_clone = call.clone();
                            let index_clone = *index;
                            async move {
                                let result = self.execute(&call_clone, uo, sig).await;
                                (index_clone, result)
                            }
                        })
                        .collect();
                    let results = join_all(futures).await;
                    final_results.extend(results);
                }
            } else {
                // Sequential execution for write tools
                for (index, call) in group {
                    let result = self
                        .execute(&call, update_output.clone(), abort_signal.clone())
                        .await;
                    final_results.push((index, result));
                }
            }
        }

        final_results.sort_by_key(|k| k.0);
        final_results.into_iter().map(|(_, v)| v).collect()
    }
}

// ── Schema pruning helpers ───────────────────────────────────────────

const DEFAULT_TOOL_DESCRIPTION_MAX_CHARS: usize = 160;
const DEFAULT_TOOL_PARAM_DESCRIPTION_MAX_CHARS: usize = 120;
const DEFAULT_TOOL_SCHEMA_DEPTH: usize = 4;

fn normalize_tool_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }

    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn tool_description_max_chars() -> usize {
    std::env::var("STAR_TOOL_DESCRIPTION_MAX_CHARS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_TOOL_DESCRIPTION_MAX_CHARS)
        .max(48)
}

fn tool_param_description_max_chars() -> usize {
    std::env::var("STAR_TOOL_PARAM_DESCRIPTION_MAX_CHARS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_TOOL_PARAM_DESCRIPTION_MAX_CHARS)
        .max(32)
}

fn prune_schema_value(value: &Value, depth: usize, max_desc_chars: usize) -> Value {
    if depth == 0 {
        return match value {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                if let Some(value_type) = map.get("type") {
                    out.insert("type".to_string(), value_type.clone());
                }
                if let Some(description) = map.get("description").and_then(|v| v.as_str()) {
                    out.insert(
                        "description".to_string(),
                        Value::String(normalize_tool_text(description, max_desc_chars)),
                    );
                }
                Value::Object(out)
            }
            _ => value.clone(),
        };
    }

    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();

            if let Some(value_type) = map.get("type") {
                out.insert("type".to_string(), value_type.clone());
            }

            if let Some(description) = map.get("description").and_then(|v| v.as_str()) {
                let normalized = normalize_tool_text(description, max_desc_chars);
                if !normalized.is_empty() {
                    out.insert("description".to_string(), Value::String(normalized));
                }
            }

            if let Some(format) = map.get("format").and_then(|v| v.as_str()) {
                out.insert("format".to_string(), Value::String(format.to_string()));
            }

            if let Some(enum_values) = map.get("enum").and_then(|v| v.as_array()) {
                out.insert(
                    "enum".to_string(),
                    Value::Array(enum_values.iter().take(16).cloned().collect()),
                );
            }

            if let Some(items) = map.get("items") {
                out.insert(
                    "items".to_string(),
                    prune_schema_value(items, depth.saturating_sub(1), max_desc_chars),
                );
            }

            if let Some(properties) = map.get("properties").and_then(|v| v.as_object()) {
                let mut pruned_properties = serde_json::Map::new();
                for (key, property_schema) in properties {
                    pruned_properties.insert(
                        key.clone(),
                        prune_schema_value(
                            property_schema,
                            depth.saturating_sub(1),
                            max_desc_chars,
                        ),
                    );
                }
                if !pruned_properties.is_empty() {
                    out.insert("properties".to_string(), Value::Object(pruned_properties));
                }
            }

            if let Some(required) = map.get("required").and_then(|v| v.as_array()) {
                out.insert("required".to_string(), Value::Array(required.clone()));
            }

            if let Some(additional_properties) = map.get("additionalProperties") {
                let pruned = match additional_properties {
                    Value::Bool(value) => Value::Bool(*value),
                    Value::Object(_) => prune_schema_value(
                        additional_properties,
                        depth.saturating_sub(1),
                        max_desc_chars,
                    ),
                    _ => Value::Bool(true),
                };
                out.insert("additionalProperties".to_string(), pruned);
            }

            for key in ["anyOf", "oneOf", "allOf"] {
                if let Some(variants) = map.get(key).and_then(|v| v.as_array()) {
                    out.insert(
                        key.to_string(),
                        Value::Array(
                            variants
                                .iter()
                                .take(4)
                                .map(|variant| {
                                    prune_schema_value(
                                        variant,
                                        depth.saturating_sub(1),
                                        max_desc_chars,
                                    )
                                })
                                .collect(),
                        ),
                    );
                }
            }

            Value::Object(out)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(16)
                .map(|value| prune_schema_value(value, depth.saturating_sub(1), max_desc_chars))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn compact_tool_definition(mut tool: StarTool) -> StarTool {
    let tool_description_limit = tool_description_max_chars();
    let param_description_limit = tool_param_description_max_chars();

    tool.function.description =
        normalize_tool_text(&tool.function.description, tool_description_limit);
    tool.function.parameters.properties = tool
        .function
        .parameters
        .properties
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                prune_schema_value(&value, DEFAULT_TOOL_SCHEMA_DEPTH, param_description_limit),
            )
        })
        .collect();
    tool
}

// ── MCP management tools ─────────────────────────────────────────────

fn mcp_management_tools() -> [StarTool; 10] {
    [
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_servers".to_string(),
                description: "List MCP servers currently configured".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: HashMap::new(),
                    required: vec![],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_tools".to_string(),
                description: "List tools for an MCP server (returns mcp__<server>__<tool>)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_tool_info".to_string(),
                description: "Get schema/description for a tool in an MCP server".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "tool".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Tool name in server (without mcp__ prefix)"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "tool".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_search_tools".to_string(),
                description: "Search MCP tools by keyword across servers".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "query".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Keyword"
                            }),
                        );
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Optional server filter"
                            }),
                        );
                        props
                    },
                    required: vec!["query".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_restart_server".to_string(),
                description: "Restart one MCP server and refresh tools".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_refresh".to_string(),
                description: "Re-discover MCP servers and refresh tool cache".to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: HashMap::new(),
                    required: vec![],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_resources".to_string(),
                description: "列出 MCP 服务器的资源列表 (List resources for an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_read_resource".to_string(),
                description: "读取 MCP 服务器上的资源内容 (Read a resource from an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "uri".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Resource URI to read"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "uri".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_list_prompts".to_string(),
                description: "列出 MCP 服务器的提示模板列表 (List prompt templates for an MCP server)"
                    .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string()],
                },
            },
        },
        StarTool {
            tool_type: "function".to_string(),
            function: crate::types::StarToolFunction {
                name: "mcp_get_prompt".to_string(),
                description:
                    "获取 MCP 服务器的提示模板 (Get a prompt template from an MCP server with optional arguments)"
                        .to_string(),
                parameters: crate::types::StarToolParameters {
                    param_type: "object".to_string(),
                    properties: {
                        let mut props = HashMap::new();
                        props.insert(
                            "server".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "MCP server name"
                            }),
                        );
                        props.insert(
                            "prompt".to_string(),
                            serde_json::json!({
                                "type": "string",
                                "description": "Prompt template name"
                            }),
                        );
                        props.insert(
                            "arguments".to_string(),
                            serde_json::json!({
                                "type": "object",
                                "description": "Optional arguments for the prompt template"
                            }),
                        );
                        props
                    },
                    required: vec!["server".to_string(), "prompt".to_string()],
                },
            },
        },
    ]
}

fn is_mcp_management_tool(name: &str) -> bool {
    matches!(
        name,
        "mcp_list_servers"
            | "mcp_list_tools"
            | "mcp_tool_info"
            | "mcp_search_tools"
            | "mcp_restart_server"
            | "mcp_refresh"
            | "mcp_list_resources"
            | "mcp_read_resource"
            | "mcp_list_prompts"
            | "mcp_get_prompt"
    )
}

async fn execute_mcp_management_tool(
    tool_registry: &Arc<ToolRegistry>,
    name: &str,
    args: &Value,
) -> ToolResult {
    let config = tool_registry.get_config();
    let Some(mcp_manager) = config.runtime_mcp_manager() else {
        return ToolResult {
            success: false,
            output: None,
            error: Some("MCP not initialized".to_string()),
            data: None,
            llm_content: None,
        };
    };

    let require_str = |key: &str| -> Result<String, ToolResult> {
        let value = args
            .get(key)
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        if value.is_empty() {
            Err(ToolResult {
                success: false,
                output: None,
                error: Some(format!("Missing required argument: {}", key)),
                data: None,
                llm_content: None,
            })
        } else {
            Ok(value)
        }
    };

    match name {
        "mcp_list_servers" => {
            let servers = mcp_manager.list_server_names().await;
            ToolResult {
                success: true,
                output: Some(servers.join("\n")),
                error: None,
                data: Some(serde_json::json!(servers)),
                llm_content: None,
            }
        }
        "mcp_list_tools" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_tools(&server).await {
                Ok(tools) => {
                    let names: Vec<String> = tools
                        .iter()
                        .map(|t| format!("mcp__{}__{}", server, t.name))
                        .collect();
                    ToolResult {
                        success: true,
                        output: Some(names.join("\n")),
                        error: None,
                        data: Some(serde_json::json!(tools)),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_tool_info" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let tool = match require_str("tool") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_tools(&server).await {
                Ok(tools) => match tools.into_iter().find(|t| t.name == tool) {
                    Some(t) => {
                        let payload = serde_json::json!({
                            "server": server,
                            "tool": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema
                        });
                        ToolResult {
                            success: true,
                            output: Some(
                                serde_json::to_string_pretty(&payload)
                                    .unwrap_or_else(|_| payload.to_string()),
                            ),
                            error: None,
                            data: Some(payload),
                            llm_content: None,
                        }
                    }
                    None => ToolResult {
                        success: false,
                        output: None,
                        error: Some(format!("Tool {} not found in server {}", tool, server)),
                        data: None,
                        llm_content: None,
                    },
                },
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_search_tools" => {
            let query = match require_str("query") {
                Ok(v) => v.to_lowercase(),
                Err(err) => return err,
            };
            let server_filter = args
                .get("server")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut hits = Vec::new();
            let mut errors = Vec::new();
            let servers = mcp_manager.list_server_names().await;
            for server in servers {
                if let Some(ref filter) = server_filter {
                    if !server.contains(filter) {
                        continue;
                    }
                }

                match mcp_manager.list_tools(&server).await {
                    Ok(tools) => {
                        for tool in tools {
                            let n = tool.name.to_lowercase();
                            let d = tool.description.to_lowercase();
                            if n.contains(&query) || d.contains(&query) {
                                hits.push(serde_json::json!({
                                    "server": server,
                                    "tool": tool.name,
                                    "description": tool.description,
                                    "fullName": format!("mcp__{}__{}", server, tool.name),
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(serde_json::json!({
                            "server": server,
                            "error": e.to_string(),
                        }));
                    }
                }
            }

            let payload = serde_json::json!({
                "query": query,
                "hits": hits,
                "errors": errors,
            });
            ToolResult {
                success: true,
                output: Some(
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
                ),
                error: None,
                data: Some(payload),
                llm_content: None,
            }
        }
        "mcp_restart_server" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.restart_server(&server).await {
                Ok(()) => {
                    let tools = mcp_manager
                        .get_cached_tools(&server)
                        .await
                        .unwrap_or_default();
                    let names: Vec<String> = tools
                        .iter()
                        .map(|t| format!("mcp__{}__{}", server, t.name))
                        .collect();
                    let payload = serde_json::json!({
                        "server": server,
                        "toolCount": names.len(),
                        "tools": names,
                    });
                    ToolResult {
                        success: true,
                        output: Some(
                            serde_json::to_string_pretty(&payload)
                                .unwrap_or_else(|_| payload.to_string()),
                        ),
                        error: None,
                        data: Some(payload),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_refresh" => {
            if let Err(e) = mcp_manager.initialize_mcp_servers().await {
                return ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                };
            }

            let errors = mcp_manager.discover_all().await;
            let server_count = mcp_manager.list_server_names().await.len();
            let payload = serde_json::json!({
                "serverCount": server_count,
                "errors": errors,
            });
            ToolResult {
                success: payload["errors"]
                    .as_object()
                    .map(|m| m.is_empty())
                    .unwrap_or(true),
                output: Some(
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
                ),
                error: None,
                data: Some(payload),
                llm_content: None,
            }
        }
        "mcp_list_resources" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_resources(&server).await {
                Ok(resources) => {
                    let text = resources
                        .iter()
                        .map(|r| format!("{} ({})", r.uri, r.name))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(if text.is_empty() {
                            "(no resources)".to_string()
                        } else {
                            text
                        }),
                        error: None,
                        data: Some(serde_json::json!(resources)),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_read_resource" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let uri = match require_str("uri") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.read_resource(&server, &uri).await {
                Ok(contents) => {
                    let text = contents
                        .iter()
                        .filter_map(|c| c.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(text),
                        error: None,
                        data: Some(serde_json::json!(contents)),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_list_prompts" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            match mcp_manager.list_prompts(&server).await {
                Ok(prompts) => {
                    let text = prompts
                        .iter()
                        .map(|p| {
                            let desc = p.description.as_deref().unwrap_or("");
                            if desc.is_empty() {
                                p.name.clone()
                            } else {
                                format!("{}: {}", p.name, desc)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolResult {
                        success: true,
                        output: Some(if text.is_empty() {
                            "(no prompts)".to_string()
                        } else {
                            text
                        }),
                        error: None,
                        data: Some(serde_json::json!(prompts)),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        "mcp_get_prompt" => {
            let server = match require_str("server") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let prompt = match require_str("prompt") {
                Ok(v) => v,
                Err(err) => return err,
            };
            let prompt_args = args.get("arguments").cloned();
            match mcp_manager.get_prompt(&server, &prompt, prompt_args).await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| result.to_string());
                    ToolResult {
                        success: true,
                        output: Some(text),
                        error: None,
                        data: Some(result),
                        llm_content: None,
                    }
                }
                Err(e) => ToolResult {
                    success: false,
                    output: None,
                    error: Some(e.to_string()),
                    data: None,
                    llm_content: None,
                },
            }
        }
        _ => ToolResult {
            success: false,
            output: None,
            error: Some(format!("Unknown MCP management tool: {}", name)),
            data: None,
            llm_content: None,
        },
    }
}

async fn execute_mcp_dynamic_tool(
    tool_registry: &Arc<ToolRegistry>,
    name: &str,
    args: &Value,
) -> ToolResult {
    let suffix = name.strip_prefix("mcp__").unwrap_or(name);
    let mut parts = suffix.splitn(2, "__");
    let server = parts.next().unwrap_or("").trim();
    let tool = parts.next().unwrap_or("").trim();

    if server.is_empty() || tool.is_empty() {
        return ToolResult {
            success: false,
            output: None,
            error: Some(format!("Invalid MCP tool name: {}", name)),
            data: None,
            llm_content: None,
        };
    }

    let config = tool_registry.get_config();
    let Some(mcp_manager) = config.runtime_mcp_manager() else {
        return ToolResult {
            success: false,
            output: None,
            error: Some("MCP not initialized".to_string()),
            data: None,
            llm_content: None,
        };
    };

    let arg_payload = if args.is_null() {
        serde_json::json!({})
    } else {
        args.clone()
    };

    match mcp_manager.call_tool(server, tool, arg_payload).await {
        Ok(value) => ToolResult {
            success: true,
            output: Some(
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            ),
            error: None,
            data: Some(value),
            llm_content: None,
        },
        Err(e) => ToolResult {
            success: false,
            output: None,
            error: Some(e.to_string()),
            data: None,
            llm_content: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 静音 panic hook 跑一段代码：被捕获的 panic 不该往测试输出里打噪音。
    fn without_panic_output<T>(f: impl FnOnce() -> T) -> T {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let out = f();
        std::panic::set_hook(hook);
        out
    }

    /// 一个"摘要一行、正文另存"的工具 —— `tool_search` 就是这个形状。
    struct TwoChannelTool;

    impl crate::core::tools::tools::BaseDeclarativeTool for TwoChannelTool {
        fn name(&self) -> &str {
            "two_channel"
        }

        fn display_name(&self) -> &str {
            "Two Channel"
        }

        fn description(&self) -> &str {
            "Reports a summary to the user and the details to the model."
        }

        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Read
        }

        fn is_read_only(&self) -> bool {
            true
        }

        fn parameter_schema(&self) -> serde_json::Value {
            json!({"type": "object", "properties": {}})
        }

        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(TwoChannelInvocation))
        }
    }

    struct TwoChannelInvocation;

    impl crate::core::tools::tools::ToolInvocation for TwoChannelInvocation {
        fn get_description(&self) -> String {
            "Two channel".to_string()
        }

        fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
            Vec::new()
        }

        fn execute(
            &self,
            _signal: Option<&tokio_util::sync::CancellationToken>,
            _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::core::tools::tools::ToolResult,
                            Box<dyn std::error::Error>,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                Ok(crate::core::tools::tools::ToolResult {
                    // 给 UI 的一行摘要。
                    llm_content: Some("the full body the model needs".to_string()),
                    return_display: Some("Found 1 tool(s)".to_string()),
                    output: "Found 1 tool(s)".to_string(),
                    error: None,
                    data: None,
                })
            })
        }
    }

    /// 回归：工具的 `llm_content` 曾经被整个丢掉 —— executor 只取
    /// `return_display`，于是模型收到的是 "Found 1 tool(s)"，工具名和 schema
    /// 全不在里面。`tool_search` 靠这条通道把发现结果交给模型，通道一断，
    /// 模型就只能反复瞎猜工具名。
    #[tokio::test]
    async fn llm_content_reaches_the_agent_level_result() {
        use crate::core::config::ConfigParameters;
        use crate::types::StarToolCallFunction;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(TwoChannelTool));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            None,
            config,
        );

        let call = crate::types::StarToolCall {
            id: "call-1".to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "two_channel".to_string(),
                arguments: "{}".to_string(),
            },
        };

        let result = executor.execute(&call, None, None).await;

        // 给 UI 的摘要不变。
        assert_eq!(result.output.as_deref(), Some("Found 1 tool(s)"));
        // 给模型的正文必须到位。
        assert_eq!(
            result.llm_content.as_deref(),
            Some("the full body the model needs"),
            "llm_content 必须在 executor 里被保留下来"
        );
    }

    #[test]
    fn panic_messages_are_extracted_from_both_payload_shapes() {
        // `panic!("literal")` → &str
        assert_eq!(panic_payload_message(&"boom"), "boom");
        // `panic!("{}", x)` 和 std 自己抛的（比如切片越界）→ String
        assert_eq!(panic_payload_message(&"boom".to_string()), "boom");
        // 其它类型只能给占位，但不能崩
        assert_eq!(panic_payload_message(&42u8), "<non-string panic payload>");
    }

    /// `execute` 用的就是这套组合子。这里验证机制本身：一个会 panic 的 async 块被
    /// 捕获成 `Err(payload)`，而不是把调用方（真实场景里就是 agent worker）带走。
    #[test]
    fn a_panicking_future_is_contained_by_the_same_guard_execute_uses() {
        let caught = without_panic_output(|| {
            futures::executor::block_on(async {
                let boom = async {
                    // 复现真实成因：按字节切一段中文。
                    let content = "上网找的知识点";
                    let _ = &content[..2];
                    "unreachable"
                };
                futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(boom)).await
            })
        });

        let payload = caught.expect_err("byte-slicing CJK should panic");
        let message = panic_payload_message(payload.as_ref());
        assert!(
            message.contains("byte index") || message.contains("char boundary"),
            "unexpected panic message: {}",
            message
        );
    }

    #[test]
    fn path_specific_confirmation_runs_despite_generic_approval() {
        assert!(should_run_invocation_confirmation(false, false));
        assert!(!should_run_invocation_confirmation(true, false));
        assert!(should_run_invocation_confirmation(true, true));
    }

    /// 回归：部分 OpenAI-compatible 提供方把整段 arguments 套一层 JSON 字符串。
    /// 不剥的话每个工具都收到 `Value::String`，TodoWrite 报
    /// `invalid type: string, expected struct TodoWriteParams`，模型重试无效。
    #[test]
    fn stringified_json_args_are_unwrapped_to_object() {
        // `"arguments": "{\"todos\": [{…}]}"` → 第一层 from_str 得到 String
        let inner = r#"{"todos":[{"content":"Run tests","status":"in_progress","activeForm":"Running tests"}]}"#;
        let args = ToolExecutor::unwrap_stringified_args(Value::String(inner.to_string()));
        assert!(
            args.is_object(),
            "stringified JSON object must be unwrapped"
        );
        assert!(args["todos"][0]["activeForm"].as_str() == Some("Running tests"));
    }

    /// 只剥一层：套了两层字符串的不再往下猜，保持原样交给严格解析。
    #[test]
    fn double_stringified_args_left_alone() {
        // 一个 JSON 字符串，里面又装了一个 JSON 字符串
        let doubly = r#""{\"todos\":[]}"#;
        let args = ToolExecutor::unwrap_stringified_args(Value::String(doubly.to_string()));
        assert!(args.is_string(), "do not unwrap past one layer");
    }

    /// 字符串里装的不是对象（数字/裸串/数组）——工具入参顶层永远是对象，
    /// 解不出对象就原样退回，由各工具的严格解析报错。
    #[test]
    fn non_object_stringified_args_left_alone() {
        let args = ToolExecutor::unwrap_stringified_args(Value::String("123".to_string()));
        assert!(args.is_string());
        let args =
            ToolExecutor::unwrap_stringified_args(Value::String("just some text".to_string()));
        assert!(args.is_string());
        let args = ToolExecutor::unwrap_stringified_args(Value::String("[1, 2]".to_string()));
        assert!(args.is_string());
    }

    /// 对象/数组/裸值径直通过，不做任何变形。
    #[test]
    fn already_decoded_args_pass_through() {
        let obj = json!({ "todos": [] });
        assert_eq!(ToolExecutor::unwrap_stringified_args(obj.clone()), obj);
        assert_eq!(ToolExecutor::unwrap_stringified_args(json!(42)), json!(42));
    }

    /// 端到端：`execute()` 的完整解码链（from_str → null→{} → 剥字符串壳 → 工具严格解析）
    /// 对"字符串化的 arguments"必须走通，否则 TodoWrite 永远报 invalid type: string。
    #[test]
    fn decode_chain_parses_stringified_todo_write_payload() {
        use crate::core::tools::tasks::parse_todo_write_params;

        // 提供方发来的是 `{"todos": [{…}]}` 的**字符串字面量**
        let raw = r#"{"todos":[{"content":"Run tests","status":"in_progress","activeForm":"Running tests"}]}"#;
        let mut args = serde_json::from_str::<Value>(raw).expect("outer parse");
        if args.is_null() {
            args = Value::Object(serde_json::Map::new());
        }
        args = ToolExecutor::unwrap_stringified_args(args);

        let parsed = parse_todo_write_params(&args).expect("TodoWrite must accept unwrapped args");
        assert_eq!(parsed.todos.len(), 1);
        assert_eq!(parsed.todos[0].content, "Run tests");
    }

    type ObservedConfirmation =
        Arc<std::sync::Mutex<Option<(crate::types::ToolConfirmationOutcome, Option<String>)>>>;

    struct FeedbackTool {
        observed: ObservedConfirmation,
    }

    struct FeedbackInvocation {
        observed: ObservedConfirmation,
    }

    impl crate::core::tools::tools::BaseDeclarativeTool for FeedbackTool {
        fn name(&self) -> &str {
            "FeedbackTest"
        }

        fn display_name(&self) -> &str {
            "Feedback test"
        }

        fn description(&self) -> &str {
            "test-only confirmation tool"
        }

        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Other
        }

        fn parameter_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(FeedbackInvocation {
                observed: self.observed.clone(),
            }))
        }

        fn normalize_confirmation_outcome(
            &self,
            _outcome: crate::types::ToolConfirmationOutcome,
        ) -> crate::types::ToolConfirmationOutcome {
            crate::types::ToolConfirmationOutcome::ProceedOnce
        }
    }

    impl crate::core::tools::tools::ToolInvocation for FeedbackInvocation {
        fn get_description(&self) -> String {
            "Feedback test".to_string()
        }

        fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
            Vec::new()
        }

        fn should_confirm_execute(
            &self,
            _abort_signal: Option<&tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                            Box<dyn std::error::Error + Send + Sync>,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            let observed = self.observed.clone();
            Box::pin(async move {
                Ok(Some(
                    crate::core::tools::tools::ToolCallConfirmationDetails {
                        confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                        title: "Confirm".to_string(),
                        prompt: "Continue?".to_string(),
                        on_confirm: Arc::new(move |outcome, feedback| {
                            *observed.lock().expect("callback state poisoned") =
                                Some((outcome, feedback));
                        }),
                    },
                ))
            })
        }

        fn execute(
            &self,
            _signal: Option<&tokio_util::sync::CancellationToken>,
            _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::core::tools::tools::ToolResult,
                            Box<dyn std::error::Error>,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Ok(crate::core::tools::tools::ToolResult::default()) })
        }
    }

    #[tokio::test]
    async fn approved_confirmation_passes_feedback_to_normalized_callback() {
        use crate::core::config::ConfigParameters;
        use crate::types::StarToolCallFunction;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));
        let bus = Arc::new(MessageBus::new(
            crate::core::policy::PolicyEngine::new(Default::default()),
            false,
        ));
        let observed = Arc::new(std::sync::Mutex::new(None));
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(FeedbackTool {
            observed: observed.clone(),
        }));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let mut requests = bus.subscribe();
        let responder_bus = bus.clone();
        let responder = tokio::spawn(async move {
            let request = loop {
                if let Message::ToolConfirmationRequest(request) =
                    requests.recv().await.expect("confirmation channel closed")
                {
                    break request;
                }
            };
            responder_bus
                .publish(Message::ToolConfirmationResponse(
                    ToolConfirmationResponse::from_user_decision(
                        request.correlation_id,
                        crate::types::ToolConfirmationOutcome::AllowSession,
                        Some("take the safe route".to_string()),
                    ),
                ))
                .await
                .expect("publish confirmation response");
        });

        let result = executor
            .execute(
                &StarToolCall {
                    id: "feedback-call".to_string(),
                    call_type: "function".to_string(),
                    function: StarToolCallFunction {
                        name: "FeedbackTest".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                None,
                None,
            )
            .await;
        assert!(result.success);
        responder.await.expect("response task failed");
        assert_eq!(
            *observed.lock().expect("callback state poisoned"),
            Some((
                crate::types::ToolConfirmationOutcome::ProceedOnce,
                Some("take the safe route".to_string()),
            ))
        );
    }

    #[tokio::test]
    async fn cached_edit_permission_does_not_skip_path_specific_confirmation() {
        use crate::core::config::trusted_folders::TrustedFolders;
        use crate::core::config::ConfigParameters;
        use crate::core::state::GlobalState;
        use crate::core::tools::edit::EditTool;
        use crate::types::StarToolCallFunction;
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let mut config_params = ConfigParameters::default();
        config_params.target_dir = temp.path().to_path_buf();
        config_params.cwd = temp.path().to_path_buf();
        config_params.folder_trust = Some(true);
        let mut config = Config::new(config_params);
        config.trusted_folders_manager = Some(
            TrustedFolders::new_for_test(temp.path().join("trustedFolders.json"))
                .expect("创建隔离 trust 配置失败"),
        );
        let config = Arc::new(config);
        let bus = Arc::new(MessageBus::new(
            crate::core::policy::PolicyEngine::new(Default::default()),
            false,
        ));
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(EditTool::new(
            config.clone(),
            bus.clone(),
            Arc::new(GlobalState::new()),
        )));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let mut requests = bus.subscribe();
        let responder_bus = bus.clone();
        let responder = tokio::spawn(async move {
            let mut file_paths = Vec::new();
            while file_paths.len() < 2 {
                if let Message::ToolConfirmationRequest(request) =
                    requests.recv().await.expect("确认消息通道关闭")
                {
                    file_paths.push(
                        request
                            .tool_call
                            .args
                            .as_ref()
                            .and_then(|args| args.get("file_path"))
                            .and_then(serde_json::Value::as_str)
                            .expect("确认请求缺少 file_path")
                            .to_string(),
                    );
                    responder_bus
                        .publish(Message::ToolConfirmationResponse(
                            ToolConfirmationResponse {
                                message_type: MessageBusType::ToolConfirmationResponse,
                                correlation_id: request.correlation_id,
                                confirmed: true,
                                outcome: Some(crate::types::ToolConfirmationOutcome::AllowSession),
                                requires_user_confirmation: None,
                                feedback: None,
                            },
                        ))
                        .await
                        .expect("发送确认结果失败");
                }
            }
            file_paths
        });

        let make_call = |id: &str, file_path: &str| StarToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "Edit".to_string(),
                arguments: serde_json::json!({
                    "file_path": file_path,
                    "old_string": "",
                    "new_string": id,
                    "expected_replacements": 1,
                })
                .to_string(),
            },
        };

        assert!(
            executor
                .execute(&make_call("first", "first.txt"), None, None)
                .await
                .success
        );
        assert!(
            executor
                .execute(&make_call("second", "second.txt"), None, None)
                .await
                .success
        );

        let file_paths = tokio::time::timeout(Duration::from_secs(2), responder)
            .await
            .expect("第二个未受信任 Edit 必须再次请求确认")
            .expect("确认响应任务失败");
        assert_eq!(file_paths, ["first.txt", "second.txt"]);
    }

    /// 端到端复现用户截图里的场景：**真的 `EditTool`** + 「允许本会话」。
    ///
    /// `folder_trust` 关闭（默认值），所以 `EditTool::requires_invocation_confirmation()`
    /// 为假、`should_confirm_execute()` 返回 `Ok(None)` —— 弹出来的正是截图里那张
    /// "`Edit` needs approval before it can run. No allow rule covers it yet."，
    /// 也就是兜底卡片。用了合成工具的回归测试锁不住这一条：真实的 `Edit` 还带着
    /// 参数校验、read-state 检查、`permission_cache_identity` 这些附加条件。
    #[tokio::test]
    async fn the_real_edit_tool_honours_allow_for_session() {
        use crate::core::config::trusted_folders::TrustedFolders;
        use crate::core::config::ConfigParameters;
        use crate::core::state::GlobalState;
        use crate::core::tools::edit::EditTool;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        std::fs::create_dir_all(temp.path().join(".star")).expect("seed project .star dir");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        params.folder_trust = Some(false);
        let mut config = Config::new(params);
        config.trusted_folders_manager = Some(
            TrustedFolders::new_for_test(temp.path().join("trustedFolders.json"))
                .expect("创建隔离 trust 配置失败"),
        );
        let config = Arc::new(config);

        let file = temp.path().join("main.py");
        std::fs::write(&file, "END\n").expect("seed target file");

        let bus = Arc::new(MessageBus::new(
            crate::core::policy::PolicyEngine::new(Default::default()),
            false,
        ));
        let global_state = Arc::new(GlobalState::new());
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        // `Edit` 要求先读过同一个文件，所以这里也得装上真的 `Read`。
        registry.register_tool(Arc::new(crate::core::tools::read_file::ReadFileTool::new(
            config.clone(),
            bus.clone(),
            global_state.clone(),
        )));
        registry.register_tool(Arc::new(EditTool::new(
            config.clone(),
            bus.clone(),
            global_state,
        )));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        // 先读一次：只读工具在 `default` 档下自动放行，不会打扰计数。
        let read = executor
            .execute(
                &StarToolCall {
                    id: "read-1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::types::StarToolCallFunction {
                        name: "Read".to_string(),
                        arguments: serde_json::json!({"file_path": file.to_string_lossy()})
                            .to_string(),
                    },
                },
                None,
                None,
            )
            .await;
        assert!(read.success, "预读失败：{:?}", read.error);

        let prompts = Arc::new(AtomicUsize::new(0));
        let _ = &prompts;
        let responder = tokio::spawn(respond_to_each_confirmation(
            bus.clone(),
            vec![crate::types::ToolConfirmationOutcome::AllowSession],
        ));
        let counting = tokio::spawn(count_each_confirmation(bus.clone(), prompts.clone()));

        let call = |id: &str| StarToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Edit".to_string(),
                arguments: serde_json::json!({
                    "file_path": file.to_string_lossy(),
                    "old_string": "END",
                    "new_string": format!("END\nline {id}"),
                    "expected_replacements": 1,
                })
                .to_string(),
            },
        };

        for i in 1..=3 {
            // 每次编辑前重读一次：`Edit` 的 read-state 记的是内容，文件被自己改过
            // 之后就失效了。只读工具自动放行，不会计进确认框。
            let read = executor
                .execute(
                    &StarToolCall {
                        id: format!("read-{i}"),
                        call_type: "function".to_string(),
                        function: crate::types::StarToolCallFunction {
                            name: "Read".to_string(),
                            arguments: serde_json::json!({"file_path": file.to_string_lossy()})
                                .to_string(),
                        },
                    },
                    None,
                    None,
                )
                .await;
            assert!(read.success, "第 {i} 次预读失败：{:?}", read.error);

            let result = tokio::time::timeout(
                Duration::from_secs(5),
                executor.execute(&call(&format!("one{i}")), None, None),
            )
            .await
            .unwrap_or_else(|_| panic!("第 {i} 次调用挂住了"));
            assert!(
                result.success,
                "第 {i} 次 Edit 应当成功：{:?}",
                result.error
            );
        }
        settle().await;
        responder.abort();
        counting.abort();

        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "点了「允许本会话」之后，Edit 不该再弹第二次"
        );
    }

    /// 端到端复现用户截图：`folder_trust` 打开、目录已受信任时的 `Edit`。
    ///
    /// 这才是那张截图的形状：`folder_trust` 打开 → `EditTool::
    /// requires_invocation_confirmation()` 为真；目录已信任 →
    /// `should_confirm_execute()` 返回 `Ok(None)`。于是弹出来的是兜底卡片
    /// （"No allow rule covers it yet."），而不是"未信任目录"那张 ——
    /// 看起来像通用批准失效了，其实是授权压根没被读。
    ///
    /// 修复前门禁里的 `!requires_invocation_confirmation` 把整块读授权跳过了，
    /// 三次编辑弹三次框。
    #[tokio::test]
    async fn a_trusted_folder_edit_honours_allow_for_session() {
        use crate::core::config::trusted_folders::{TrustLevel, TrustedFolders};
        use crate::core::config::ConfigParameters;
        use crate::core::state::GlobalState;
        use crate::core::tools::edit::EditTool;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        std::fs::create_dir_all(temp.path().join(".star")).expect("seed project .star dir");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        params.folder_trust = Some(true);
        let mut config = Config::new(params);
        let trusted = TrustedFolders::new_for_test(temp.path().join("trustedFolders.json"))
            .expect("创建隔离 trust 配置失败");
        trusted
            .set_trust_level(temp.path(), TrustLevel::TrustFolder)
            .expect("把临时目录标为受信任");
        config.trusted_folders_manager = Some(trusted);
        let config = Arc::new(config);

        let file = temp.path().join("main.py");
        std::fs::write(&file, "END\n").expect("seed target file");

        let bus = Arc::new(MessageBus::new(
            crate::core::policy::PolicyEngine::new(Default::default()),
            false,
        ));
        let global_state = Arc::new(GlobalState::new());
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(crate::core::tools::read_file::ReadFileTool::new(
            config.clone(),
            bus.clone(),
            global_state.clone(),
        )));
        registry.register_tool(Arc::new(EditTool::new(
            config.clone(),
            bus.clone(),
            global_state,
        )));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let prompts = Arc::new(AtomicUsize::new(0));
        let responder = tokio::spawn(respond_to_each_confirmation(
            bus.clone(),
            vec![crate::types::ToolConfirmationOutcome::AllowSession],
        ));
        let counting = tokio::spawn(count_each_confirmation(bus.clone(), prompts.clone()));

        let call = |id: &str| StarToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Edit".to_string(),
                arguments: serde_json::json!({
                    "file_path": file.to_string_lossy(),
                    "old_string": "END",
                    "new_string": format!("END\nline {id}"),
                    "expected_replacements": 1,
                })
                .to_string(),
            },
        };

        for i in 1..=3 {
            let read = executor
                .execute(
                    &StarToolCall {
                        id: format!("read-{i}"),
                        call_type: "function".to_string(),
                        function: crate::types::StarToolCallFunction {
                            name: "Read".to_string(),
                            arguments: serde_json::json!({"file_path": file.to_string_lossy()})
                                .to_string(),
                        },
                    },
                    None,
                    None,
                )
                .await;
            assert!(read.success, "第 {i} 次预读失败：{:?}", read.error);

            let result = tokio::time::timeout(
                Duration::from_secs(5),
                executor.execute(&call(&format!("one{i}")), None, None),
            )
            .await
            .unwrap_or_else(|_| panic!("第 {i} 次调用挂住了"));
            assert!(
                result.success,
                "第 {i} 次 Edit 应当成功：{:?}",
                result.error
            );
        }
        settle().await;
        responder.abort();
        counting.abort();

        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "受信任目录里点了「允许本会话」，Edit 不该再弹第二次"
        );
    }

    /// 回归：`ask` 规则必须真的弹窗。
    ///
    /// 此前 `PolicyDecision::AskUser` 只把 `approved` 置为 false，随后工具自身的
    /// `should_confirm_execute()` 返回 `Ok(None)` 又把它翻回 true —— 于是
    /// `ask: ["Bash(git push:*)"]` 对普通 `git push` 静默放行。
    /// `SilentTool` 刻意不要求任何确认，正是这个 bug 的触发条件。
    struct SilentTool;

    impl crate::core::tools::tools::BaseDeclarativeTool for SilentTool {
        fn name(&self) -> &str {
            "SilentTool"
        }
        fn display_name(&self) -> &str {
            "Silent tool"
        }
        fn description(&self) -> &str {
            "test-only tool that never asks for confirmation"
        }
        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Other
        }
        fn parameter_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(SilentInvocation))
        }
    }

    struct SilentInvocation;

    impl crate::core::tools::tools::ToolInvocation for SilentInvocation {
        fn get_description(&self) -> String {
            "Silent".to_string()
        }
        fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
            Vec::new()
        }
        // 刻意不实现 should_confirm_execute：走默认的 Ok(None)。
        fn execute(
            &self,
            _signal: Option<&tokio_util::sync::CancellationToken>,
            _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::core::tools::tools::ToolResult,
                            Box<dyn std::error::Error>,
                        >,
                    > + Send
                    + 'static,
            >,
        > {
            Box::pin(async {
                Ok(crate::core::tools::tools::ToolResult {
                    output: "ran".to_string(),
                    ..Default::default()
                })
            })
        }
    }

    #[tokio::test]
    async fn ask_rule_forces_a_prompt_even_when_the_tool_asks_for_nothing() {
        use crate::core::config::ConfigParameters;
        use crate::core::policy::settings_rules::{PermissionRuleSpec, SettingsPermissions};
        use crate::core::policy::PolicyEngine;
        use crate::types::StarToolCallFunction;
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        let mut engine = PolicyEngine::new(Default::default());
        let mut perms = SettingsPermissions::default();
        perms
            .ask
            .push(PermissionRuleSpec::parse("SilentTool").expect("parse rule"));
        engine.set_settings_permissions(perms);
        let bus = Arc::new(MessageBus::new(engine, false));

        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(SilentTool));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let mut requests = bus.subscribe();
        let responder_bus = bus.clone();
        let responder = tokio::spawn(async move {
            let request = loop {
                if let Message::ToolConfirmationRequest(request) =
                    requests.recv().await.expect("confirmation channel closed")
                {
                    break request;
                }
            };
            responder_bus
                .publish(Message::ToolConfirmationResponse(
                    ToolConfirmationResponse::from_user_decision(
                        request.correlation_id,
                        crate::types::ToolConfirmationOutcome::ProceedOnce,
                        None,
                    ),
                ))
                .await
                .expect("publish confirmation response");
        });

        let result = executor
            .execute(
                &StarToolCall {
                    id: "silent-call".to_string(),
                    call_type: "function".to_string(),
                    function: StarToolCallFunction {
                        name: "SilentTool".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                None,
                None,
            )
            .await;

        tokio::time::timeout(Duration::from_secs(5), responder)
            .await
            .expect("`ask` rule must have produced a confirmation prompt")
            .expect("confirmation responder task failed");
        assert!(result.success);
    }

    /// 确认请求必须带上模型侧的 tool_call id。
    ///
    /// UI 手上只有 correlation id（bus 生成的 UUID）和工具结果里的 `tool_call.id`，
    /// 两者不是同一个命名空间。没有 originating id，UI 就分不清"这张卡对应的
    /// 工具已经结束了"，卡片永远停在等待态，后续每次确认请求都会被顶掉。
    #[tokio::test]
    async fn confirmation_request_carries_the_originating_tool_call_id() {
        use crate::core::config::ConfigParameters;
        use crate::core::policy::PolicyEngine;
        use crate::types::StarToolCallFunction;
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        let bus = Arc::new(MessageBus::new(
            PolicyEngine::new(Default::default()),
            false,
        ));
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(SilentTool));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let mut requests = bus.subscribe();
        let responder_bus = bus.clone();
        let responder = tokio::spawn(async move {
            let request = loop {
                match requests.recv().await.expect("confirmation channel closed") {
                    Message::ToolConfirmationRequest(request) => break request,
                    _ => continue,
                }
            };
            let originating_tool_call_id = request.originating_tool_call_id.clone();
            let _ = responder_bus
                .publish(Message::ToolConfirmationResponse(
                    ToolConfirmationResponse::from_user_decision(
                        request.correlation_id,
                        crate::types::ToolConfirmationOutcome::ProceedOnce,
                        None,
                    ),
                ))
                .await;
            originating_tool_call_id
        });

        let result = executor
            .execute(
                &StarToolCall {
                    id: "call-42".to_string(),
                    call_type: "function".to_string(),
                    function: StarToolCallFunction {
                        name: "SilentTool".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                None,
                None,
            )
            .await;

        let originating_tool_call_id = tokio::time::timeout(Duration::from_secs(5), responder)
            .await
            .expect("应产生一次确认请求")
            .expect("confirmation responder task failed");
        assert!(result.success);
        assert_eq!(
            originating_tool_call_id.as_deref(),
            Some("call-42"),
            "确认请求必须带上模型侧的 tool_call id，否则 UI 的卡片收不到结局"
        );
    }

    /// 「本会话允许」必须真的生效：选过一次之后，同一工具不该再弹框。
    ///
    /// 这条锁的是一个真实 bug：兜底 `default_decision` 是 `AskUser`，而 `AskUser`
    /// 此前被无条件当成"策略点名要求确认"，于是 executor 里
    /// `!policy_requires_confirmation` 这个闸门永远为假，会话授权**根本不会被读**。
    /// 结果是任何没有 settings 文件的用户点「本会话允许」都白点。
    ///
    /// 断言方式：统计确认请求的条数。修复前是 2，修复后必须是 1。
    #[tokio::test]
    async fn allow_for_session_prevents_the_second_prompt() {
        use crate::core::config::ConfigParameters;
        use crate::core::policy::PolicyEngine;
        use crate::types::StarToolCallFunction;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        // 刻意不装任何规则 —— 复现"用户根本没配过 settings"的真实场景
        let bus = Arc::new(MessageBus::new(
            PolicyEngine::new(Default::default()),
            false,
        ));

        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(SilentTool));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let prompts = Arc::new(AtomicUsize::new(0));
        let mut requests = bus.subscribe();
        let responder_bus = bus.clone();
        let responder_prompts = prompts.clone();
        let responder = tokio::spawn(async move {
            // 注意用 `if let` 而不是 `while let`：总线上还有别的消息类型，
            // 遇到非确认消息就退出循环会让后续 execute 一直等到确认超时。
            while let Ok(message) = requests.recv().await {
                if let Message::ToolConfirmationRequest(request) = message {
                    responder_prompts.fetch_add(1, Ordering::SeqCst);
                    let _ = responder_bus
                        .publish(Message::ToolConfirmationResponse(
                            ToolConfirmationResponse::from_user_decision(
                                request.correlation_id,
                                crate::types::ToolConfirmationOutcome::AllowSession,
                                None,
                            ),
                        ))
                        .await;
                }
            }
        });

        let call = || StarToolCall {
            id: "silent-call".to_string(),
            call_type: "function".to_string(),
            function: StarToolCallFunction {
                name: "SilentTool".to_string(),
                arguments: "{}".to_string(),
            },
        };

        let first = executor.execute(&call(), None, None).await;
        assert!(
            first.success,
            "第一次调用应成功，实际输出：{:?}",
            first.output
        );
        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "没有任何规则时也应弹一次框（兜底 AskUser 是保守行为），实际弹了 {} 次",
            prompts.load(Ordering::SeqCst)
        );

        let second = executor.execute(&call(), None, None).await;
        responder.abort();

        assert!(
            second.success,
            "第二次调用应成功，实际输出：{:?}",
            second.output
        );
        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "「本会话允许」后不该再弹第二次框"
        );
    }

    /// 反向保证：这个修复只加固"显式 `ask` 规则"，不会把默认路径变成"每次都弹窗"。
    ///
    /// 默认决策本来就是 `AskUser`，但它**可以**被工具自身的 `Ok(None)` 放行；
    /// 只有用户在 settings.json 里写下的 `ask` 规则才是不可否决的。
    /// 这里给引擎装一个 `allow` 规则，让决策变成 Allow，验证确认卡片不会发出。
    #[tokio::test]
    async fn an_allow_rule_short_circuits_without_prompting() {
        use crate::core::config::ConfigParameters;
        use crate::core::policy::settings_rules::{PermissionRuleSpec, SettingsPermissions};
        use crate::core::policy::PolicyEngine;
        use crate::types::StarToolCallFunction;
        use std::time::Duration;

        let temp = tempfile::tempdir().expect("create temporary directory");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        let mut engine = PolicyEngine::new(Default::default());
        let mut perms = SettingsPermissions::default();
        perms
            .allow
            .push(PermissionRuleSpec::parse("SilentTool").expect("parse rule"));
        engine.set_settings_permissions(perms);
        let bus = Arc::new(MessageBus::new(engine, false));

        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(Arc::new(SilentTool));
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );

        let mut requests = bus.subscribe();
        let result = executor
            .execute(
                &StarToolCall {
                    id: "silent-call".to_string(),
                    call_type: "function".to_string(),
                    function: StarToolCallFunction {
                        name: "SilentTool".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                None,
                None,
            )
            .await;

        assert!(result.success);
        // ToolStarted / ToolFinished 也会走这条广播，只断言"没有确认请求"。
        let mut saw_confirmation = false;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
        while let Ok(Ok(msg)) = tokio::time::timeout_at(deadline, requests.recv()).await {
            if matches!(msg, Message::ToolConfirmationRequest(_)) {
                saw_confirmation = true;
                break;
            }
        }
        assert!(
            !saw_confirmation,
            "an `allow` rule must not produce a confirmation prompt"
        );
    }

    // ── 门禁：没批准就不许动 ──────────────────────────────────────────────

    /// 门禁夹具：一枚只在 `execute` 里记账的计数器，用来证明"没批准时工具没跑"。
    struct CountingTool {
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::core::tools::tools::BaseDeclarativeTool for CountingTool {
        fn name(&self) -> &str {
            "CountingTool"
        }
        fn display_name(&self) -> &str {
            "Counting tool"
        }
        fn description(&self) -> &str {
            "test-only tool that counts how many times it actually ran"
        }
        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Other
        }
        fn parameter_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(CountingInvocation {
                runs: self.runs.clone(),
            }))
        }
    }

    struct CountingInvocation {
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::core::tools::tools::ToolInvocation for CountingInvocation {
        fn get_description(&self) -> String {
            "counting invocation".to_string()
        }
        fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
            Vec::new()
        }
        fn execute(
            &self,
            _signal: Option<&tokio_util::sync::CancellationToken>,
            _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::core::tools::tools::ToolResult,
                            Box<dyn std::error::Error>,
                        >,
                    > + Send,
            >,
        > {
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Ok(crate::core::tools::tools::ToolResult {
                    output: "ran".to_string(),
                    return_display: Some("ran".to_string()),
                    error: None,
                    data: None,
                    llm_content: None,
                })
            })
        }
    }

    /// 门禁夹具：装配一个必然会被兜底 `AskUser` 拦下的 executor。
    ///
    /// 返回 executor、总线和"工具实际跑了多少次"的计数器。临时目录被 executor
    /// 的 `Arc<Config>` 持有，生命周期与测试同在。
    fn counting_gate() -> (
        ToolExecutor,
        Arc<MessageBus>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tool = Arc::new(CountingTool { runs: runs.clone() });
        gate_with(tool, runs)
    }

    /// 通用门禁夹具：`tool` 决定工具本身要不要说话，策略一律"没规则 → AskUser"。
    ///
    /// `runs` 由调用方建好并交给工具，夹具只把它原样还回去。夹具自己再造一个
    /// 计数器的话，返回的就不是工具在写的那一个，断言会读到恒为 0 的值。
    fn gate_with(
        tool: Arc<dyn crate::core::tools::tools::BaseDeclarativeTool>,
        runs: Arc<std::sync::atomic::AtomicUsize>,
    ) -> (
        ToolExecutor,
        Arc<MessageBus>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use crate::core::config::ConfigParameters;
        use crate::core::policy::PolicyEngine;

        let temp = tempfile::tempdir().expect("create temporary directory");
        // 先把 `.star` 建好，`Storage` 的上溯就会停在临时目录里。少了这一步，
        // 临时目录（通常在 home 之下）会一路找到用户真实的 `~/.star`：测试点一次
        // 「总是允许」就写进用户目录，还会反过来把后续调用的第一次也放行。
        std::fs::create_dir_all(temp.path().join(".star")).expect("seed project .star dir");
        let mut params = ConfigParameters::default();
        params.target_dir = temp.path().to_path_buf();
        params.cwd = temp.path().to_path_buf();
        let config = Arc::new(Config::new(params));

        // 刻意不装任何规则：兜底 `AskUser` 一定会弹一次框。
        let bus = Arc::new(MessageBus::new(
            PolicyEngine::new(Default::default()),
            false,
        ));
        let registry = Arc::new(ToolRegistry::new(config.clone()));
        registry.register_tool(tool);
        let executor = ToolExecutor::new(
            StarClient::new("", None, None, None, None),
            registry,
            Some(bus.clone()),
            config,
        );
        (executor, bus, runs)
    }

    /// `EditTool` 在 `folder_trust` 打开时的形状：工具要求"每次都再确认一次"，
    /// 但它自己那次检查在目录已被信任之后是 `Ok(None)`。
    ///
    /// 这正是"允许本会话 / 总是允许"失效的现场：授权写进了
    /// `SessionPermissionManager`，却因为 `!requires_invocation_confirmation` 那一句
    /// 永远读不回来，于是每次编辑都重新弹框。
    struct ReconfirmingTool {
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::core::tools::tools::BaseDeclarativeTool for ReconfirmingTool {
        fn name(&self) -> &str {
            "ReconfirmingTool"
        }
        fn display_name(&self) -> &str {
            "Reconfirming tool"
        }
        fn description(&self) -> &str {
            "test-only tool that demands a per-invocation check"
        }
        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Other
        }
        fn parameter_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn requires_invocation_confirmation(&self) -> bool {
            true
        }
        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(CountingInvocation {
                runs: self.runs.clone(),
            }))
        }
    }

    fn reconfirming_call() -> StarToolCall {
        StarToolCall {
            id: "reconfirming-call".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "ReconfirmingTool".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    fn reconfirming_gate() -> (
        ToolExecutor,
        Arc<MessageBus>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tool = Arc::new(ReconfirmingTool { runs: runs.clone() });
        gate_with(tool, runs)
    }

    /// `requires_invocation_confirmation()` 为真、**并且**自己真的要说话的工具。
    ///
    /// 对照 `ReconfirmingTool`（会说话但 check 返回 `Ok(None)`）：这条要锁的是
    /// 「用户给了通用授权之后，工具自己的那条意见仍然算数」。
    struct SelfPromptingTool {
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::core::tools::tools::BaseDeclarativeTool for SelfPromptingTool {
        fn name(&self) -> &str {
            "SelfPromptingTool"
        }
        fn display_name(&self) -> &str {
            "Self-prompting tool"
        }
        fn description(&self) -> &str {
            "test-only tool that always speaks up"
        }
        fn kind(&self) -> crate::core::tools::tools::Kind {
            crate::core::tools::tools::Kind::Other
        }
        fn parameter_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn requires_invocation_confirmation(&self) -> bool {
            true
        }
        fn create_invocation(
            &self,
            _params: serde_json::Value,
        ) -> Result<
            Box<dyn crate::core::tools::tools::ToolInvocation>,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            Ok(Box::new(SelfPromptingInvocation {
                runs: self.runs.clone(),
            }))
        }
    }

    struct SelfPromptingInvocation {
        runs: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::core::tools::tools::ToolInvocation for SelfPromptingInvocation {
        fn get_description(&self) -> String {
            "self-prompting invocation".to_string()
        }
        fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
            Vec::new()
        }
        fn should_confirm_execute(
            &self,
            _abort_signal: Option<&tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                            Box<dyn std::error::Error + Send + Sync>,
                        >,
                    > + Send,
            >,
        > {
            Box::pin(async {
                Ok(Some(
                    crate::core::tools::tools::ToolCallConfirmationDetails {
                        confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                        title: "Permission required: SelfPromptingTool".to_string(),
                        prompt: "the tool always has something to say".to_string(),
                        on_confirm: std::sync::Arc::new(|_outcome, _feedback| {}),
                    },
                ))
            })
        }
        fn execute(
            &self,
            _signal: Option<&tokio_util::sync::CancellationToken>,
            _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::core::tools::tools::ToolResult,
                            Box<dyn std::error::Error>,
                        >,
                    > + Send,
            >,
        > {
            self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Ok(crate::core::tools::tools::ToolResult {
                    output: "ran".to_string(),
                    return_display: Some("ran".to_string()),
                    error: None,
                    data: None,
                    llm_content: None,
                })
            })
        }
    }

    fn self_prompting_call() -> StarToolCall {
        StarToolCall {
            id: "self-prompting-call".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "SelfPromptingTool".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    /// 数一数这轮里弹了几次确认框。
    async fn count_prompts(bus: &MessageBus, within: std::time::Duration) -> usize {
        let mut rx = bus.subscribe();
        let deadline = tokio::time::Instant::now() + within;
        let mut count = 0;
        while let Ok(Ok(msg)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if matches!(msg, Message::ToolConfirmationRequest(_)) {
                count += 1;
            }
        }
        count
    }

    /// 对每一次确认按 `plan` 依次作答，plan 用完后一律重复最后一项。
    ///
    /// 订阅必须在**调用方线程**上完成再 spawn：`broadcast::Receiver` 只缓冲订阅
    /// 之后的消息，在任务里现订阅会漏掉已经发出去的请求，于是测试会挂住。
    fn respond_to_each_confirmation(
        bus: Arc<MessageBus>,
        plan: Vec<crate::types::ToolConfirmationOutcome>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut rx = bus.subscribe();
        async move {
            let mut seen = 0usize;
            loop {
                match rx.recv().await {
                    Ok(Message::ToolConfirmationRequest(req)) => {
                        let outcome = plan
                            .get(seen)
                            .or_else(|| plan.last())
                            .cloned()
                            .unwrap_or(crate::types::ToolConfirmationOutcome::ProceedOnce);
                        seen += 1;
                        let _ = bus
                            .publish(Message::ToolConfirmationResponse(
                                ToolConfirmationResponse::from_user_decision(
                                    req.correlation_id,
                                    outcome,
                                    None,
                                ),
                            ))
                            .await;
                    }
                    Ok(_) => continue,
                    Err(_) => return,
                }
            }
        }
    }

    /// 给挂在总线上的后台任务一次把缓冲区排空的机会。
    ///
    /// `broadcast::Receiver` 收到的东西先进缓冲区，被 poll 一次才算数。计数任务
    /// 如果在断言前就被 abort，统计出来的是"还没排队的条数"，不是"弹了几次框"。
    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    /// 统计弹了几次确认框。订阅同样在调用方线程上完成 —— 否则会漏掉早于它发出的那些。
    fn count_each_confirmation(
        bus: Arc<MessageBus>,
        prompts: Arc<std::sync::atomic::AtomicUsize>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        use std::sync::atomic::Ordering;
        let mut rx = bus.subscribe();
        async move {
            loop {
                match rx.recv().await {
                    Ok(Message::ToolConfirmationRequest(_)) => {
                        prompts.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(_) => continue,
                    Err(_) => return,
                }
            }
        }
    }

    fn counting_call() -> StarToolCall {
        StarToolCall {
            id: "counting-call".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "CountingTool".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    /// 订阅总线，拿到下一次确认请求的 correlation_id。
    ///
    /// 订阅必须在**调用方线程上**先建好再 spawn：broadcast 只缓冲订阅之后的消息，
    /// 在任务里现订阅会漏掉已经发出去的请求，于是应答永远等不到。
    fn watch_for_confirmation(
        bus: Arc<MessageBus>,
        outcome: crate::types::ToolConfirmationOutcome,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut rx = bus.subscribe();
        async move {
            let id = loop {
                match rx.recv().await.expect("应收到确认请求") {
                    Message::ToolConfirmationRequest(req) => break req.correlation_id,
                    _ => continue,
                }
            };
            bus.publish(Message::ToolConfirmationResponse(
                ToolConfirmationResponse::from_user_decision(id, outcome, None),
            ))
            .await
            .ok();
        }
    }

    /// 回归（用户报的那个现象）：没人批准时，等待**必须继续挂着**，工具也一步不许跑。
    ///
    /// 修复前这里是 `timeout(600s, …)`：用户只是离开了一会儿，回来就看到
    /// `Confirmation timeout or error: Confirmation timed out`，模型据此宣布这个工具
    /// 不可用 —— 一次没人否决的请求，在用户眼前自己变成了失败。
    #[tokio::test]
    async fn an_unanswered_confirmation_never_turns_into_a_failure() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let (executor, _bus, runs) = counting_gate();
        let call = counting_call();
        let waiting = executor.execute(&call, None, None);
        tokio::pin!(waiting);

        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            early = &mut waiting => panic!(
                "没人回答，等待就结束了：{early:?} —— 计时器把'还没决定'写成了'被拒绝'"
            ),
        }

        assert_eq!(runs.load(Ordering::SeqCst), 0, "没批准就执行了工具");
    }

    /// 用户按 ESC 取消之后：不再等，工具也绝不能跑，报错里不许出现"超时"。
    #[tokio::test]
    async fn a_cancelled_confirmation_never_runs_the_tool() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let (executor, bus, runs) = counting_gate();
        let responder = tokio::spawn(watch_for_confirmation(
            bus.clone(),
            crate::types::ToolConfirmationOutcome::Cancel,
        ));

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            executor.execute(&counting_call(), None, None),
        )
        .await
        .expect("用户取消后必须立刻返回");
        responder.await.expect("responder task");

        assert!(!result.success, "被拒绝的调用不能报成功");
        assert_eq!(runs.load(Ordering::SeqCst), 0, "用户没批准，工具却跑了");
        let error = result.error.unwrap_or_default();
        assert!(
            !error.contains("timed out"),
            "取消不是超时，报错里不该出现 timed out：{error}"
        );
        assert!(
            error.contains("declined"),
            "报错要说明是用户拒绝了，而不是笼统的失败：{error}"
        );
    }

    /// 用户批准之后照常执行 —— 门禁没有把路堵死。
    #[tokio::test]
    async fn an_approved_confirmation_runs_the_tool() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let (executor, bus, runs) = counting_gate();
        let responder = tokio::spawn(watch_for_confirmation(
            bus.clone(),
            crate::types::ToolConfirmationOutcome::ProceedOnce,
        ));

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            executor.execute(&counting_call(), None, None),
        )
        .await
        .expect("批准之后必须立刻返回");
        responder.await.expect("responder task");

        assert!(result.success, "批准后应当执行成功：{:?}", result.error);
        assert_eq!(runs.load(Ordering::SeqCst), 1, "批准后工具应当恰好跑一次");
    }

    /// 回归（用户报的现象）：「允许本会话」必须真的生效。
    ///
    /// `ReconfirmingTool` 是 `EditTool` 在 `folder_trust` 打开时的形状 ——
    /// `requires_invocation_confirmation() == true`。修复前门禁里有这么一句：
    ///
    /// ```ignore
    /// if !approved && !requires_invocation_confirmation && … { /* 查授权 */ }
    /// ```
    ///
    /// 工具只要自带逐次确认，这块就被整块跳过；于是「允许本会话」把授权**写进去
    /// 了却永远读不回来**，用户点完还是每次弹框。
    #[tokio::test]
    async fn allow_for_session_survives_a_tool_that_demands_a_per_invocation_check() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let (executor, bus, runs) = reconfirming_gate();
        let prompts = Arc::new(AtomicUsize::new(0));
        let responder = tokio::spawn(respond_to_each_confirmation(
            bus.clone(),
            vec![crate::types::ToolConfirmationOutcome::AllowSession],
        ));
        let counting = tokio::spawn(count_each_confirmation(bus.clone(), prompts.clone()));

        for _ in 0..3 {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                executor.execute(&reconfirming_call(), None, None),
            )
            .await
            .expect("批准之后必须立刻返回");
            assert!(result.success, "每次调用都应成功：{:?}", result.error);
        }
        // 让计数任务把缓冲区里最后几条确认请求排空再收工，否则读到的数字偏小。
        settle().await;
        responder.abort();
        counting.abort();

        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "点了「允许本会话」之后不该再弹第二次"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 3, "三次调用都应真正执行");
    }

    /// 回归：「总是允许」也一样要生效 —— 它落到磁盘，跨会话读回来。
    #[tokio::test]
    async fn always_allow_survives_a_tool_that_demands_a_per_invocation_check() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let (executor, bus, _runs) = reconfirming_gate();
        let prompts = Arc::new(AtomicUsize::new(0));
        let responder = tokio::spawn(respond_to_each_confirmation(
            bus.clone(),
            vec![crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave],
        ));
        let counting = tokio::spawn(count_each_confirmation(bus.clone(), prompts.clone()));

        for _ in 0..3 {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                executor.execute(&reconfirming_call(), None, None),
            )
            .await
            .expect("批准之后必须立刻返回");
            assert!(result.success, "每次调用都应成功：{:?}", result.error);
        }
        settle().await;
        responder.abort();
        counting.abort();

        assert_eq!(
            prompts.load(Ordering::SeqCst),
            1,
            "点了「总是允许」之后不该再弹第二次"
        );
    }

    /// 反向保证：工具自己的逐次检查仍然有话语权 —— 通用授权不能让它闭嘴。
    ///
    /// 这条锁的是 `requires_invocation_confirmation` 真正的用途：工具知道
    /// 参数级/路径级的顾虑（Edit 关心未信任目录），通用授权不该替它做主。
    /// 放宽的只是"它能否否决用户的授权"，不是"它能否说话"。
    #[tokio::test]
    async fn a_per_invocation_check_still_outranks_a_session_grant() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let (executor, bus, _runs) = gate_with(
            Arc::new(SelfPromptingTool {
                runs: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }),
            Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        let prompts = Arc::new(AtomicUsize::new(0));
        // 第一次只批准这一次；第二次给一个「本会话」授权。
        let responder = tokio::spawn(respond_to_each_confirmation(
            bus.clone(),
            vec![
                crate::types::ToolConfirmationOutcome::ProceedOnce,
                crate::types::ToolConfirmationOutcome::AllowSession,
            ],
        ));
        let counting = tokio::spawn(count_each_confirmation(bus.clone(), prompts.clone()));

        for _ in 0..3 {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                executor.execute(&self_prompting_call(), None, None),
            )
            .await
            .expect("批准之后必须立刻返回");
            assert!(result.success, "每次调用都应成功：{:?}", result.error);
        }
        settle().await;
        responder.abort();
        counting.abort();

        assert_eq!(
            prompts.load(Ordering::SeqCst),
            3,
            "工具自己提出的确认不能被「本会话允许」吞掉"
        );
    }

    /// 中断这一轮必须让等待立刻收手：用户不想等了，剩下的就不该继续挂着。
    #[tokio::test]
    async fn interrupting_the_turn_unblocks_the_gate_and_skips_the_tool() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        let (executor, _bus, runs) = counting_gate();
        let token = tokio_util::sync::CancellationToken::new();

        let call = counting_call();
        let waiting = executor.execute(&call, None, Some(token.clone()));
        tokio::pin!(waiting);

        let interrupter = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            token.cancel();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(waiting, interrupter)
        })
        .await
        .expect("中断后必须立刻返回");

        assert!(!result.success, "被中断的调用不能报成功");
        assert_eq!(runs.load(Ordering::SeqCst), 0, "中断时工具不该跑");
        let error = result.error.unwrap_or_default();
        assert!(
            error.contains("interrupted"),
            "报错要如实说是被中断，别让模型以为换个写法就能过：{error}"
        );
    }
}
