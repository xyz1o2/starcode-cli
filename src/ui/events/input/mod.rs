//! 键盘输入的总入口。
//!
//! 本文件只留**主分发**（handle_key_event）与跨模式通用的那几块；
//! 独立成子模块的是各自成体系的状态机：
//!
//! - [provider_form] —— provider 配置表单
//! - [palette_action] —— 命令面板动作执行
//! - [sk_user_question] —— 问答工具的交互输入
//! - [chat_search] —— 会话内搜索
//! - [g_agent] —— 后台代理选择器 / 各类模态框按键
//! - [	ool_approval] —— 工具调用审批

mod ask_user_question;
mod bg_agent;
mod chat_search;
mod palette_action;
mod provider_form;
mod tool_approval;

use ask_user_question::*;
use bg_agent::*;
use chat_search::*;
use palette_action::*;
use provider_form::*;
use tool_approval::*;

// 下列函数以前就在 input 模块上可见（pub(crate)），搬进子模块后在这里
// 重新导出，调用方的路径不用改。
pub(crate) use bg_agent::{
    handle_log_selector_key, handle_status_modal_key, handle_theme_picker_key,
    retry_last_user_message,
};
pub(crate) use palette_action::execute_palette_action;
pub(crate) use tool_approval::{handle_input_modal, handle_task_panel_key};

pub(crate) use super::clipboard_paste::push_cursor_off_sentinel_pub;
use super::clipboard_paste::{
    collect_modal_input, detect_file_paths, insert_file_paste_block, insert_image_paste_block,
    insert_paste_block, maybe_auto_fold_input, normalize_modal_api_key, normalize_modal_base_url,
    push_cursor_off_sentinel, reset_main_textarea, save_clipboard_image, sync_input_from_textarea,
};
use super::clipboard_paste::{PASTE_ENTER_GUARD_MS, RAPID_PASTE_KEY_INTERVAL_MS};
use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tui_textarea::TextArea;

use crate::runtime::messages::AgentRequest;
use crate::types::{ApprovalMode, ChatEntryType};
use crate::ui::events::key_context::KeyContext;
use crate::ui::events::keymap::GlobalBinding;
use crate::ui::state::palette::{PaletteAction, PaletteMode};
use crate::ui::state::ChatState;

/// 尝试从剪贴板读取图片并保存到 .star/images/，返回 (相对路径, 宽, 高)

fn clear_suggestion_overlays(state: &mut ChatState) {
    state.show_command_hints = false;
    state.command_hints.clear();
    state.show_mention_hints = false;
    state.mention_hints.clear();
}

/// 关闭 provider / session 两个快捷菜单（薄封装，实现见 `ChatState`）。
fn close_quick_menus(state: &mut ChatState) {
    state.close_quick_menus();
}

/// 压入 kill ring（容量 10，Claude Code readline 风格）
fn push_kill(state: &mut ChatState, text: String) {
    if text.is_empty() {
        return;
    }
    state.kill_ring.push_back(text);
    while state.kill_ring.len() > 10 {
        state.kill_ring.pop_front();
    }
    state.kill_ring_pos = None;
}

/// 从 kill ring 指定索引 yank：先删除上次 yank 的内容（yank-pop），再插入
fn yank_from_ring(state: &mut ChatState, idx: usize) {
    if state.last_yank_len > 0 {
        for _ in 0..state.last_yank_len {
            state.textarea.move_cursor(tui_textarea::CursorMove::Back);
            state.textarea.delete_char(); // 删除光标前刚 yank 的内容
        }
    }
    if let Some(text) = state.kill_ring.get(idx) {
        let text = text.clone();
        state.textarea.insert_str(&text);
        state.last_yank_len = text.chars().count();
        state.kill_ring_pos = Some(idx);
    } else {
        state.last_yank_len = 0;
    }
    sync_input_from_textarea(state);
    crate::ui::services::suggestions::on_input_changed(state);
}

/// 把排队中的输入全部取回输入框 —— 对标 Claude Code 的 `popAllEditable`
/// （`src/utils/messageQueueManager.ts`）：多条按入队顺序用换行拼接，输入框里
/// 已有的内容接在最后，光标留在末尾。返回是否真的取回了内容。
///
/// 排队面板只负责把队列显示出来，「改词 / 撤掉」这条退路靠这个函数：
/// 参考实现把它绑在 ↑ 和 Esc 上，这里保持一致。
pub(crate) fn restore_queued_messages_to_input(state: &mut ChatState) -> bool {
    if state.pending_user_messages.is_empty() {
        return false;
    }
    let mut inputs: Vec<_> = state.pending_user_messages.drain(..).collect();
    // 取回队列会把当前输入重组为新草稿，不能让旧历史导航状态在下一次
    // ↓ 时把这个新内容覆盖回历史进入前的快照。
    state.history_index = None;
    state.history_input_snapshot = None;
    // 图片路径可能已经失效；即使展开失败，也必须把当前草稿和它的
    // paste segment 原样保住，不能因为取回队列而静默丢掉附件引用。
    let current_raw = state.textarea.lines().join("\n");
    let current_segments = state.paste_segments.clone();
    let current = state
        .prepared_input()
        .unwrap_or_else(|_| crate::types::UserInput {
            text: current_raw.clone(),
            images: Vec::new(),
            intent: state.input_intent,
        });
    if !current.text.is_empty() || !current.images.is_empty() {
        inputs.push(current);
    }
    let intent = if inputs.len() == 1 {
        inputs[0].intent
    } else {
        crate::types::InputIntent::Literal
    };
    let text = inputs
        .iter()
        .map(|input| input.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let images = inputs.into_iter().flat_map(|input| input.images).collect();
    state.set_user_input(crate::types::UserInput {
        text,
        images,
        intent,
    });
    if !current_segments.is_empty() && state.paste_segments.is_empty() {
        state.paste_segments = current_segments;
        sync_input_from_textarea(state);
    }
    crate::ui::services::suggestions::on_input_changed(state);
    true
}

/// Build context breakdown from current state for the context visualization.
fn build_context_breakdown(
    state: &ChatState,
) -> crate::ui::components::highlight::context_viz::TokenBreakdown {
    let total = state.token_count;
    // Heuristic breakdown
    crate::ui::components::highlight::context_viz::TokenBreakdown {
        system_prompt: (total as f64 * 0.20) as u32,
        conversation: (total as f64 * 0.50) as u32,
        tool_outputs: (total as f64 * 0.25) as u32,
        context_files: (total as f64 * 0.05) as u32,
        total,
        max_context: get_max_context_for_model(&state.current_model),
    }
}

fn get_max_context_for_model(model: &str) -> u32 {
    let lower = model.to_lowercase();
    if lower.contains("claude-opus") || lower.contains("claude-sonnet") {
        200_000
    } else if lower.contains("claude-haiku") {
        200_000
    } else if lower.contains("gpt-4o") || lower.contains("gpt-4-turbo") {
        128_000
    } else if lower.contains("deepseek") {
        128_000
    } else if lower.contains("gemini") {
        1_000_000
    } else {
        128_000
    }
}

fn open_provider_selection_menu(
    state: &mut ChatState,
    back: Option<crate::ui::state::QuickMenuKind>,
    origin_palette: bool,
) {
    clear_suggestion_overlays(state);
    state.close_palette();
    state.exit_input_modal();
    state.quick_menu_back = back;
    state.quick_menu_origin_palette = origin_palette;
    // open_provider_menu 内部保证与 session 菜单互斥
    state.open_provider_menu();
}

fn navigate_back_from_quick_menu(state: &mut ChatState) {
    if matches!(
        state.quick_menu_back,
        Some(crate::ui::state::QuickMenuKind::Provider)
    ) {
        if state.quick_menu_origin_palette {
            // Came from palette Provider mode – return there instead of quick overlay
            close_quick_menus(state);
            show_palette_mode(state, crate::ui::state::PaletteMode::Provider);
        } else {
            open_provider_selection_menu(state, None, false);
        }
        return;
    }

    close_quick_menus(state);
    if state.quick_menu_origin_palette {
        show_palette_mode(state, state.palette_mode.clone());
    } else {
        state.quick_menu_back = None;
        state.quick_menu_origin_palette = false;
    }
}

pub(crate) async fn request_runtime_settings(
    state: &mut ChatState,
    model: Option<crate::runtime::messages::RuntimeModelSelection>,
    context_window: Option<crate::core::context_policy::ContextWindowSelection>,
    thinking_effort: Option<crate::types::ThinkingEffort>,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> bool {
    request_runtime_settings_with_persistence(
        state,
        model,
        context_window,
        thinking_effort,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent,
        agent_tx,
    )
    .await
}

/// 发出具备明确持久化语义的 runtime mutation。所有变更均取得 FIFO revision；只有
/// persistent mutation 才在 UI 上显示设置文件正在保存。
pub(crate) async fn request_runtime_settings_with_persistence(
    state: &mut ChatState,
    model: Option<crate::runtime::messages::RuntimeModelSelection>,
    context_window: Option<crate::core::context_policy::ContextWindowSelection>,
    thinking_effort: Option<crate::types::ThinkingEffort>,
    persistence: crate::runtime::messages::RuntimeSettingsPersistencePolicy,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> bool {
    request_runtime_settings_with_persistence_revision(
        state,
        model,
        context_window,
        thinking_effort,
        persistence,
        agent_tx,
    )
    .await
    .is_some()
}

/// 与 [`request_runtime_settings_with_persistence`] 相同，但在 worker 收到请求后返回
/// UI revision，供必须将本地 UI 状态绑定到 acknowledgement 的会话级操作使用。
pub(crate) async fn request_runtime_settings_with_persistence_revision(
    state: &mut ChatState,
    model: Option<crate::runtime::messages::RuntimeModelSelection>,
    context_window: Option<crate::core::context_policy::ContextWindowSelection>,
    thinking_effort: Option<crate::types::ThinkingEffort>,
    persistence: crate::runtime::messages::RuntimeSettingsPersistencePolicy,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Option<u64> {
    let mutation = state.begin_runtime_settings_update_with_persistence(
        model,
        context_window,
        thinking_effort,
        persistence,
    );
    let ui_revision = mutation.ui_revision;
    if persistence == crate::runtime::messages::RuntimeSettingsPersistencePolicy::Persistent {
        state.runtime_settings_persistence =
            Some(crate::ui::state::store::RuntimeSettingsPersistence::Pending { ui_revision });
    }
    match agent_tx
        .send(AgentRequest::UpdateRuntimeSettings(mutation))
        .await
    {
        Ok(()) => Some(ui_revision),
        Err(_) => {
            state.discard_runtime_settings_update(ui_revision);
            if matches!(
                state.runtime_settings_persistence.as_ref(),
                Some(crate::ui::state::store::RuntimeSettingsPersistence::Pending {
                    ui_revision: pending_revision,
                }) if *pending_revision == ui_revision
            ) {
                state.runtime_settings_persistence = None;
            }
            state.current_status_line =
                Some("Runtime settings could not reach the agent worker.".to_string());
            None
        }
    }
}

/// 决定一次模型切换要用哪个 provider。
///
/// 顺序：**当前列表给出的归属 → 磁盘缓存的跨桶归属 → 当前 provider**，每一级都
/// 只考虑**还活着的** provider（`ChatState::provider_is_live` / `live_providers`）。
///
/// # 为什么每级都要过滤
///
/// 磁盘缓存的桶和 provider 的寿命是脱钩的：删掉 provider 时程序不会顺手删桶，
/// 外部工具直接改配置同样留残留，全桶扫描就会把模型名认领给一个已经不存在、
/// 也从未配置过的 provider。`apply_model_selection` 随后以
/// `Provider '<x>' is not configured.` 拒掉，而那个 reason 只落到状态栏、不写
/// 日志 —— 用户看到的就只是"选了模型但没反应"，而且**刚改完 provider 时必现**
/// （此时内存列表刚被 `clear()` 过，兜底全都掉到磁盘扫描那一步）。
///
/// # 中间那一步仍然不能删
///
/// `model_provider_map` 只装**当前列表**的映射（`apply_models_to_state` 每次都
/// `clear()` 再重建）。于是一个属于别的网关的名字（OpenRouter 的
/// `xiaomi/mimo-v2.6-flash`）必然查不到，直接兜底发给当前 provider —— Zen 收到
/// OpenRouter 的模型 id，回 `Model is unavailable`；反过来则是
/// `is not a valid model ID`。两种都表现为"选了模型但没切成功"，而用户无从
/// 知道模型被发去了另一家。
///
/// 兜底到 `current_provider_id` 本身没有错 —— 用户切到一个谁都没有的模型名时，
/// 仍该留在当前 provider 上试一次（中转站的 `/models` 常年不全，拉不到不等于
/// 不能用）。关键是不能**在能查到的情况下**还去兜底。
pub(crate) fn resolve_provider_for_model(state: &ChatState, model: &str) -> Option<String> {
    if let Some(provider_id) = state.model_provider_map.get(model) {
        if state.provider_is_live(provider_id) {
            return Some(provider_id.clone());
        }
    }
    if let Some(provider_id) = crate::agent::model_cache::find_provider_for_model(
        model,
        state.current_provider_id.as_deref(),
        Some(&state.live_providers()),
    ) {
        return Some(provider_id);
    }
    state.current_provider_id.clone()
}

/// 从 UI 发起模型切换时只记录 requested intent；实际 active model 只能由 worker
/// acknowledgement 写回，避免 provider 失败时界面提前谎报成功。
pub(crate) async fn request_runtime_model_change(
    state: &mut ChatState,
    model: String,
    provider_id: Option<String>,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> bool {
    state.pending_model_change = None;
    // 豁免是一次性的：无论这条切换走没走成，都不影响之后的普通确认门
    state.waive_next_model_confirmation = false;
    request_runtime_settings(
        state,
        Some(crate::runtime::messages::RuntimeModelSelection { model, provider_id }),
        None,
        None,
        agent_tx,
    )
    .await
}

pub(crate) fn show_palette_mode(state: &mut ChatState, mode: PaletteMode) {
    close_quick_menus(state);
    state.quick_menu_back = None;
    state.quick_menu_origin_palette = false;
    state.exit_input_modal();
    state.open_palette(mode);
}

/// 打开 provider 单面板表单。
///
/// `editing = Some(id)` 是编辑模式：五个字段预填该 provider 的现值（API Key
/// 例外，密钥不往屏幕上画，留空表示"保持原 key"），提交时 upsert 回同一个 id。
/// `None` 是新增，id 由名称 / URL host 派生。
///
/// 五个字段共用一个 textarea：切字段时由 `switch_provider_form_field` 把内容
/// 在 `ProviderFormState::values` 和 textarea 之间搬。初始焦点给 Type（下标 0），
/// 让用户先定协议类型；Type 不渲染 textarea，←/→ 选完 Tab 就到 Name。
pub(crate) async fn open_provider_form(state: &mut ChatState, editing: Option<String>) {
    state.quick_menu_back = Some(crate::ui::state::QuickMenuKind::Provider);
    state.quick_menu_origin_palette = true;
    state.close_palette();
    state.enter_input_modal();

    let mut form = crate::ui::state::palette::ProviderFormState::new();
    form.editing_id = editing.clone();

    if let Some(id) = editing {
        prefill_form_from_provider(&mut form, &id).await;
    }

    state.provider_form = form;
    state.provider_form.active_field = 0;

    sync_textarea_to_active_field(state);

    state.input_context = Some(crate::ui::state::palette::InputContext::ProviderForm);
}

/// 用 provider 的现值预填表单。
///
/// 类型优先读已保存的设置，没有就按内置 provider 的协议推断；名称 / Base URL /

/// 建议浮层（provider / session 快捷菜单、命令与 @ 补全）：↑↓ 选择、Enter 采纳、
/// Tab 采纳补全。
///
/// 按键表来自 `keymap::map_key`（只对浮层可见的键有意义）。返回 `false` 表示这个键
/// 不属于浮层 —— 例如 `AcceptCompletion` 在没有补全项时会「处理失败」，此时应当
/// 继续往下落到输入框。
async fn handle_suggestion_overlay(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    use crate::ui::events::keymap::UiAction;

    let Some(action) = crate::ui::events::keymap::map_key(state, &key) else {
        return Ok(false);
    };
    match action {
        UiAction::SelectPrev => {
            crate::ui::services::suggestions::handle_up(state);
            Ok(true)
        }
        UiAction::SelectNext => {
            crate::ui::services::suggestions::handle_down(state);
            Ok(true)
        }
        UiAction::AcceptSuggestion => {
            if !crate::ui::services::suggestions::handle_enter(state) {
                return Ok(false);
            }
            if let Some(action) = state.pending_palette_action.take() {
                execute_palette_action(state, action, agent_tx).await?;
            }
            if let Some(pending) = state.pending_model_change.take() {
                // 归属已经在确认框里存下来了，这里不再重新推导。
                let provider_id = pending
                    .provider_id
                    .or_else(|| resolve_provider_for_model(state, &pending.model));
                if request_runtime_model_change(state, pending.model.clone(), provider_id, agent_tx)
                    .await
                {
                    state.current_status_line =
                        Some(format!("Switching to model {}…", pending.model));
                }
            }
            Ok(true)
        }
        UiAction::AcceptCompletion => Ok(crate::ui::services::suggestions::handle_tab(state)),
    }
}

/// 行内确认：`/clear` 与大段粘贴的「确定吗」。
///
/// 两个确认都吃掉**所有**键（`_ => {}`），所以它们必须在上下文分发表里独占一个
/// 分支 —— 排在别处的话，任何新加的层都可能从它们手里漏走按键。
async fn handle_inline_confirm(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    if state.show_clear_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                state.show_clear_confirmation = false;
                let saved =
                    match crate::commands::chat::save_conversation_for_clear(state, agent_tx).await
                    {
                        Ok(saved) => saved,
                        Err(error) => {
                            state.current_status_line = Some(format!(
                                "Conversation kept: it could not be saved before clearing ({})",
                                error
                            ));
                            return Ok(());
                        }
                    };
                state.pending_clear_name = None;
                state.active_session_id = None;
                state.session_schedules.change_session();
                state.current_session_title = None;
                state.chat_history.clear();
                // 会话已重置，上下文读数必须一起归零：否则新会话首轮会顶着一个
                // 与空历史矛盾的旧基线，直到下一次 provider usage 到来。
                state.token_usage = None;
                state.token_count = 0;
                state.turn_tokens = 0;
                state.cache_read_tokens = 0;
                state.cache_creation_tokens = 0;
                let _ = agent_tx
                    .send(crate::runtime::messages::AgentRequest::ResetSession)
                    .await;
                state.current_status_line = Some(match saved {
                    Some(id) => format!("Conversation cleared. Previous saved as `{}`.", id),
                    None => crate::core::i18n::t1("ui.status.cleared").to_string(),
                });
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.show_clear_confirmation = false;
                state.pending_clear_name = None;
            }
            _ => {}
        }
        return Ok(());
    }

    if state.show_paste_confirmation {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                state.show_paste_confirmation = false;
                if let Some(text) = state.pending_paste.take() {
                    super::clipboard_paste::insert_paste_block_confirmed(state, text);
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.show_paste_confirmation = false;
                state.pending_paste = None;
            }
            _ => {}
        }
    }
    Ok(())
}

/// 模型切换 y/n 确认。确认外的键一律吞掉 —— 换模型是破坏性的，不能被别的键顶掉。
///
/// 吞键的前提是**必须能退出**：`pending_model_confirmation` 一旦为 true，
/// 分发阶梯就把整个键盘交给了这里。所以无论走到哪个分支，标志都必须被清掉 ——
/// 留在 true 就等于把输入框永久锁死（`y` 键若无 `pending_model_change` 可用而
/// 不清标志，就会制造出这样一个死局）。
async fn handle_model_confirmation(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            // 先取再清：`pending_model_change` 就是这次要提交的模型及其归属。
            let pending = state.pending_model_change.take();
            state.pending_model_confirmation = false;
            let Some(pending) = pending else {
                // 有确认框却没有待切换的模型 = 状态已经不一致。不要卡在这里。
                state.current_status_line = None;
                return Ok(());
            };
            // 归属来自当初按下 Enter 时那个选项自带的值，不是现在重新推的 ——
            // 确认框可能有延迟，磁盘缓存在这期间已经变了。
            let provider_id = pending
                .provider_id
                .or_else(|| resolve_provider_for_model(state, &pending.model));
            if request_runtime_model_change(
                state,
                pending.model.clone(),
                provider_id.clone(),
                agent_tx,
            )
            .await
            {
                crate::ui::app::logic::emit_status_text(
                    state,
                    0,
                    &format!(
                        "Requesting model {} (provider {})…",
                        pending.model,
                        provider_id.as_deref().unwrap_or("default")
                    ),
                );
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.pending_model_confirmation = false;
            state.pending_model_change = None;
            state.current_status_line = None;
        }
        _ => {}
    }
    Ok(())
}

/// Shift+Tab（`TogglePlanMode`）：Auto → Default → AcceptEdits → Plan → BypassPermissions → Auto
/// （对标 app:cycleMode / CC 的 shift+tab）。
///
/// `handle_key_event` 里有一个**排在它前面**的调用点（必须抢在两个行内确认之前），
/// 注册表分发器里也有一份 —— 两者共用这个函数，避免同一套模式循环写两遍。
async fn cycle_approval_mode(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>) {
    state.approval_mode = match state.approval_mode {
        ApprovalMode::Auto => ApprovalMode::Default,
        ApprovalMode::Default => ApprovalMode::AcceptEdits,
        ApprovalMode::AcceptEdits => ApprovalMode::Plan,
        ApprovalMode::Plan => ApprovalMode::BypassPermissions,
        ApprovalMode::BypassPermissions => ApprovalMode::DontAsk,
        // dontAsk 不进 Shift+Tab 循环 —— 它是 CI/headless 的档位；交互终端里
        // 「不问直接拒」只会让人以为程序卡死了。再次 Shift+Tab 回到 Default。
        ApprovalMode::DontAsk => ApprovalMode::Default,
    };

    let _ = agent_tx
        .send(AgentRequest::SetApprovalMode(state.approval_mode.clone()))
        .await;
}

/// 分发一个全局按键绑定。
///
/// 返回 `false` 表示「该绑定在当前模式下不适用」（守卫不满足），调用方继续走后续
/// 处理层 —— 与重构前 `if ... { return Ok(()) }` 守卫不满足就落下去的行为一致。
///
/// **只处理命令类全局键**，且只在 `handle_overlay_input` 之前的那一段生效：
///
/// - [`GlobalBinding::Paste`]（`Ctrl+V`）必须在 `handle_input_modal` **之后**：
///   输入框自己处理 `Ctrl+V`（把剪贴板填进 modal textarea，填 API Key 靠它）。
/// - [`GlobalBinding::ToggleHelp`]（`F1`）仍在主 `match` 里，与 `?` 的帮助开关
///   放在一起，保持原顺序。
///
/// 这两个在这里返回 `Ok(false)` 而不是被"顺手处理掉"，是为了不改动既有的键序。
pub async fn dispatch_global_binding(
    state: &mut ChatState,
    binding: GlobalBinding,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match binding {
        GlobalBinding::StashPrompt => {
            let context = KeyContext::of(state);
            let inline_completion = context == KeyContext::SuggestionOverlay
                && !state.show_provider_menu
                && !state.show_session_menu;
            if !matches!(context, KeyContext::TextInput | KeyContext::Vim) && !inline_completion {
                return Ok(true);
            }
            let has_input = state.textarea.lines().iter().any(|line| !line.is_empty())
                || !state.paste_segments.is_empty();
            if !has_input && state.stashed_prompt.is_none() {
                return Ok(true);
            }
            let previous = state.stashed_prompt.take();
            let had_stash = previous.is_some();
            let current = has_input.then(|| crate::ui::state::StashedPrompt {
                textarea: std::mem::take(&mut state.textarea),
                paste_segments: std::mem::take(&mut state.paste_segments),
                intent: state.input_intent,
                folded: state.input_folded,
            });
            reset_main_textarea(state);
            if let Some(previous) = previous {
                state.textarea = previous.textarea;
                state.paste_segments = previous.paste_segments;
                state.input_intent = previous.intent;
                state.input_folded = previous.folded;
            }
            sync_input_from_textarea(state);
            state.stashed_prompt = current;
            state.history_index = None;
            state.history_input_snapshot = None;
            crate::ui::services::suggestions::on_input_changed(state);
            state.save_draft();
            let message = match (has_input, had_stash) {
                (true, false) => crate::core::i18n::t1("ui.stash.saved"),
                (true, true) => crate::core::i18n::t1("ui.stash.swapped"),
                _ => crate::core::i18n::t1("ui.stash.restored"),
            };
            state.push_toast(&message, crate::ui::state::ToastKind::Info);
            Ok(true)
        }
        GlobalBinding::WithdrawMessage => {
            if KeyContext::of(state).is_exclusive() {
                return Ok(false);
            }
            if restore_queued_messages_to_input(state) {
                return Ok(true);
            }
            state.steered_user_messages.retain(|(id, _)| {
                let tag = format!("user-message:{id}");
                state.chat_history.iter().any(|entry| {
                    entry.entry_type == ChatEntryType::User
                        && entry.group_id.as_deref() == Some(tag.as_str())
                })
            });
            if let Some((message_id, _)) = state.steered_user_messages.last() {
                agent_tx
                    .send(AgentRequest::WithdrawMessage {
                        message_id: *message_id,
                    })
                    .await?;
            } else {
                let message = crate::core::i18n::t1("ui.withdraw.unavailable");
                state.push_toast(&message, crate::ui::state::store::ToastKind::Info);
            }
            Ok(true)
        }
        // 已经在 `handle_key_event` 开头处理过（要抢在行内确认之前），这里只是
        // 保证注册表里每个绑定都有唯一实现。
        GlobalBinding::TogglePlanMode => {
            if state.is_awaiting_confirmation || state.is_input_modal_open() {
                return Ok(false);
            }
            cycle_approval_mode(state, agent_tx).await;
            Ok(true)
        }
        // Ctrl+P：开关命令面板
        GlobalBinding::TogglePalette => {
            if state.is_palette_open() {
                state.close_palette();
            } else {
                state.show_help = false;
                state.exit_input_modal();
                close_quick_menus(state);
                state.quick_menu_back = None;
                state.quick_menu_origin_palette = false;
                state.palette_history.clear();
                state.open_palette(PaletteMode::Main);
            }
            Ok(true)
        }
        // Ctrl+O：全局切换 transcript / verbose 输出（对标 Claude Code 的
        // app:toggleTranscript，同为 Global 作用域）。必须早于 handle_overlay_input，
        // 否则又会退回到「只有任务面板可见时才生效」的老行为。
        GlobalBinding::ToggleTranscript => {
            let on = state.toggle_transcript_mode();
            // 走 toast，不走 `current_status_line`。
            //
            // `current_status_line` 是**实时状态**槽，渲染在活动 spinner 行里；
            // "Verbose output: ON/OFF" 是一条用过即弃的通知，塞进去会占用一个
            // `·` 分隔位、与回合计时并排显示成 `18s · Verbose output: OFF`
            // ——一条过期通知伪装成了"现在正在发生的事"。
            //
            // （此前写这里是因为 `emit_status_text` 只放行
            // `i18n::status_prefixes()` 允许的前缀、"Verbose output: …" 会被它
            // 静默丢掉；那是个绕过白名单的 workaround，仓里 `Auto-compact on/off`
            // 早就是用 toast 的。）
            state.push_toast(
                if on {
                    "Verbose output: ON"
                } else {
                    "Verbose output: OFF"
                },
                crate::ui::state::store::ToastKind::Info,
            );
            Ok(true)
        }
        // Ctrl+T：任务面板（对标 app:toggleTodos）。Ctrl+B 不在此列 —— Claude Code
        // 把它留给 task:background（仅前台有任务可转后台时才拦截），空闲时归输入框。
        GlobalBinding::ToggleTasks => {
            state.task_panel.toggle_visibility();
            if state.task_panel.is_visible {
                // 打开时重读任务文件，和 /tasks 一样避免显示旧快照。
                state.task_panel.reload();
            }
            Ok(true)
        }
        // Ctrl+Shift+F：全局搜索（对标 Claude Code app:globalSearch）
        GlobalBinding::GlobalSearch => {
            state.open_global_search();
            Ok(true)
        }
        // Ctrl+F：对话内搜索（对标 Claude Code conversation search）。
        // 与 Ctrl+Shift+F 的区别：那个搜仓库里的文件，这个搜已经发生过的对话 ——
        // 长会话里"刚才那条命令到底写的什么"只能靠它。
        GlobalBinding::SearchChat => {
            if state.is_awaiting_confirmation || state.is_input_modal_open() {
                return Ok(false);
            }
            if state.chat_search.active {
                close_chat_search(state);
            } else {
                open_chat_search(state);
            }
            Ok(true)
        }
        // Ctrl+Shift+P：快速打开文件（对标 Claude Code app:quickOpen）
        GlobalBinding::QuickOpen => {
            state.open_quick_open();
            Ok(true)
        }
        // Ctrl+R：历史搜索（对标 Claude Code history:search）
        GlobalBinding::HistorySearch => {
            if state.is_awaiting_confirmation || state.is_palette_open() {
                return Ok(false);
            }
            state.open_history_search();
            Ok(true)
        }
        // Alt 系绑定的共同前置：审批中 / 面板开着 / 输入模态打开时不抢键，
        // 与重构前那个包住三个分支的 `if` 完全一致。
        GlobalBinding::FoldInput | GlobalBinding::CycleThinking | GlobalBinding::CycleTheme => {
            if state.is_awaiting_confirmation
                || state.is_palette_open()
                || state.is_input_modal_open()
            {
                return Ok(false);
            }
            match binding {
                GlobalBinding::FoldInput => {
                    let line_count = state.input_line_count;
                    if line_count >= crate::ui::state::INPUT_FOLD_MIN_LINES {
                        state.input_folded = !state.input_folded;
                        state.current_status_line = Some(if state.input_folded {
                            format!("Input folded ({} lines), press Alt+P to expand", line_count)
                        } else {
                            "Input expanded, press Alt+P to fold again".to_string()
                        });
                    } else {
                        state.input_folded = false;
                        state.current_status_line =
                            Some("Content is too short to fold".to_string());
                    }
                }
                GlobalBinding::CycleTheme => next_theme(state),
                // Alt+T：循环思考深度。
                //
                // 兼容不上报 SHIFT 的终端：它们把 Alt+Shift+T 送成大写 'T'。注册表
                // 按 shift 位判定（见 `keymap` 的同名测试，因为字符大小写对非字母键和
                // 部分终端并不可靠），这里再兜一次，否则这类终端上再也切不了主题。
                GlobalBinding::CycleThinking => {
                    if matches!(key.code, KeyCode::Char(c) if c.is_uppercase()) {
                        next_theme(state);
                    } else {
                        cycle_thinking_effort(state, agent_tx).await?;
                    }
                }
                _ => unreachable!("三个 Alt 绑定已在上一层收拢"),
            }
            Ok(true)
        }
        // Ctrl+C：复制选区 / 中断生成 / 连按两次退出
        GlobalBinding::CopyOrInterrupt => {
            copy_selection_or_interrupt(state, agent_tx).await;
            Ok(true)
        }
        // Ctrl+D：退出。
        //
        // 命中即 return：此前只在「任务面板关闭 + 输入框为空」这条路径上 return，
        // 其余情况会继续往下落到 handle_overlay_input 的权限调试面板
        // （见本文件 toggle_permission_debug）和任务描述编辑分支，
        // 同一个键被处理三次。
        GlobalBinding::Exit => {
            if !state.task_panel.is_visible && state.input.is_empty() {
                state.should_exit = true;
            }
            Ok(true)
        }
        // Ctrl+L：清屏
        GlobalBinding::ClearScreen => {
            state.request_clear_screen = true;
            Ok(true)
        }
        // 见函数文档：这两个键必须留在各自原来的处理层。
        GlobalBinding::Paste | GlobalBinding::ToggleHelp => Ok(false),
    }
}

/// Alt+Shift+T：切到下一个主题（`CycleTheme` 与大写-T 兜底共用）。
fn next_theme(state: &mut ChatState) {
    state.theme_manager.next_theme();
    let theme_name = state.theme_manager.current().name.clone();
    state.current_status_line = Some(format!("Theme: {}", theme_name));
}

/// Alt+T：循环思考力度档位，并把请求发给 runtime。
async fn cycle_thinking_effort(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cap = crate::core::config::models::thinking_capability(&state.current_model);
    let effort = match cap {
        crate::core::config::models::ThinkingCapability::Binary => {
            // Binary models: Off ↔ Medium (On)
            match state.thinking_effort {
                crate::types::ThinkingEffort::Off => crate::types::ThinkingEffort::Medium,
                _ => crate::types::ThinkingEffort::Off,
            }
        }
        _ => state.thinking_effort.next(),
    };
    // 当前模型根本没有思考开关时说清楚，否则用户会以为档位没生效。
    let unsupported = matches!(cap, crate::core::config::models::ThinkingCapability::None);
    if request_runtime_settings(state, None, None, Some(effort.clone()), agent_tx).await {
        state.current_status_line = Some(format!(
            "Requesting {}{}",
            effort.notification_text(),
            if unsupported {
                " · current model has no thinking mode"
            } else {
                ""
            }
        ));
    }
    Ok(())
}

/// Ctrl+C：复制选区 → 中断生成 → 连按两次退出，按这个优先级依次尝试。
///
/// 有选区时**只**是复制，绝不往下落到取消生成/双击退出：此前 `get_selected_text()`
/// 返回 None 时会静默继续往下走，用户想复制结果把正在输出的内容 Abort 掉了。
async fn copy_selection_or_interrupt(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>) {
    if state.text_selection.has_selection() {
        state.last_ctrl_c = None;
        // 复制走 `ui::selection::copy_selection` —— 和鼠标松开同一个入口，
        // 系统剪贴板优先、失败自动退回 OSC 52（SSH/WSL 下 arboard 写的是远端
        // 剪贴板，那条环境里能用的正是 OSC 52）。
        match crate::ui::selection::copy_selection(state) {
            Ok(()) => {
                state.text_selection.clear();
                state.current_status_line = Some("Copied".to_string());
            }
            Err(e) => {
                // 剪贴板写不进去（WSL/SSH/headless、被其他进程占用）时必须说真话，
                // 并且**保留选区** —— 此前这里无条件宣告 "Copied" 并清掉选区，
                // 用户粘出来的是上次的旧内容，且再也无法重试。
                state.current_status_line =
                    Some(format!("Copy failed: {e} — press Ctrl+C to retry"));
                crate::utils::logging::append_debug_log_line(&format!("[COPY] failed: {}", e));
            }
        }
        return;
    }

    let now = Instant::now();
    let within_double_press = state
        .last_ctrl_c
        .is_some_and(|previous| now.duration_since(previous) <= Duration::from_millis(1500));
    if within_double_press {
        state.last_ctrl_c = None;
        state.should_exit = true;
        return;
    }
    state.last_ctrl_c = Some(now);

    // 查看子代理时，Ctrl+C 先向当前代理发送定向停止请求并回到主会话；它不应
    // 错误地取消 lead turn。下一次 Ctrl+C 仍可按双击语义退出整个 TUI。
    if let Some(task_id) = state.viewing_agent_task_id.clone() {
        let stop_task = state.active_agent_tasks.get(&task_id).is_some_and(|info| {
            info.is_async
                && matches!(
                    info.status,
                    crate::types::AgentTaskStatus::Running
                        | crate::types::AgentTaskStatus::Background
                )
        });
        if stop_task {
            match agent_tx.try_send(AgentRequest::StopBackgroundAgent {
                task_id: task_id.clone(),
            }) {
                Ok(()) => {
                    // 不阻塞键盘事件等待 worker；实际取消结果由后续流消息反馈。
                    let message =
                        crate::core::i18n::t1("agent.stop.queued").replace("{task}", &task_id);
                    state.push_toast(&message, crate::ui::state::store::ToastKind::Info);
                }
                Err(error) => {
                    let message = crate::core::i18n::t1("agent.stop.queue_failed")
                        .replace("{task}", &task_id)
                        .replace("{error}", &error.to_string());
                    state.push_toast(&message, crate::ui::state::store::ToastKind::Error);
                }
            }
        } else {
            state.push_toast(
                &crate::core::i18n::t1("agent.returned_main"),
                crate::ui::state::store::ToastKind::Info,
            );
        }
        state.exit_teammate_view();
        state.auto_follow = true;
        return;
    }

    // 流输出"僵住"：取消宽限期已过但 is_streaming 没复位（例如 Done 落在宽限窗口里）。
    let cancelling_stale = state.is_streaming
        && state
            .cancelling_since
            .map(|started| started.elapsed() > Duration::from_millis(1500))
            .unwrap_or(false);
    if cancelling_stale {
        state.is_streaming = false;
        state.is_processing = false;
        state.cancelling_since = None;
    }

    if state.is_streaming || state.is_processing {
        // try_send 避免满队列把键盘事件循环卡死。失败时保留真实运行状态，
        // 用户仍可再按 Ctrl+C 退出，而不能伪报“已取消”。
        match agent_tx.try_send(AgentRequest::Abort) {
            Ok(()) => {
                state.current_status_line = Some(crate::core::i18n::t1("ui.thinking.cancelling"));
                state.is_processing = false;
                state.cancelling_since = Some(now);
            }
            Err(error) => {
                state.current_status_line = Some(
                    crate::core::i18n::t1("ui.status.interrupt_failed")
                        .replace("{error}", &error.to_string()),
                );
            }
        }
        return;
    }

    if !state.input.is_empty() {
        // First press: clear input
        reset_main_textarea(state);
        crate::ui::services::suggestions::on_input_changed(state);
    }
    state.current_status_line = Some(crate::core::i18n::t1("ui.status.ctrl_c_exit"));
}

pub async fn handle_key_event(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
    last_key_time: Option<Instant>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Ctrl+C 是跨面板的复制/中断/退出键，必须在搜索、行内确认和 modal 处理前
    // 分发；其他全局键仍保留原优先级，避免改写各自的面板语义。
    if crate::ui::events::keymap::binding_for(&key) == Some(GlobalBinding::CopyOrInterrupt) {
        // 长按产生的 Repeat 不是第二次主动按下，不能触发退出。
        if key.kind != crossterm::event::KeyEventKind::Press {
            return Ok(());
        }
        dispatch_global_binding(state, GlobalBinding::CopyOrInterrupt, key, agent_tx).await?;
        return Ok(());
    }
    if key.kind == crossterm::event::KeyEventKind::Press {
        // 提示要求连续两次 Ctrl+C；中间发生其他按键时重新开始计次。
        state.last_ctrl_c = None;
    }

    // 确认卡的粘贴先于旧模态处理，避免隐藏的输入模态截走图片。
    if KeyContext::of(state) == KeyContext::ToolConfirmation && handle_paste(state, key).await? {
        return Ok(());
    }
    // ── Unified modal stack takes priority over everything else ──
    // Palette / MCP / Market modals consume keys first; Esc pops one level.
    if super::modal_input::handle_modal_key(state, key, agent_tx).await? {
        return Ok(());
    }

    // ── 对话内搜索面板（Ctrl+F）──────────────────────────────────────────
    //
    // 位置在 modal 栈**之后**、全局键注册表**之前**：
    // - 必须在注册表之前 —— 面板打开期间输入框不该拿到这些键；
    // - 不能进 modal 栈 —— 它是一个常驻在聊天区顶部的横条，不是居中对话框。
    //
    // 例外交给 `handle_chat_search_key` 自己决定：它返回 false 时继续往下走
    // （`Ctrl+Shift+F` 先收面板再开全局文件搜索），其余键一律被面板吞掉。
    if state.chat_search.active && handle_chat_search_key(state, key) {
        return Ok(());
    }

    // Handle Shift+Tab for mode cycle: Auto → Default → AcceptEdits → Plan → BypassPermissions → Auto
    // (Shift+Tab is normalized to BackTab by the runtime event loop before reaching here)
    //
    // 位置是刻意的：它是**唯一**排在两个行内确认之前的全局绑定。`/clear` 与大段
    // 粘贴的确认分支会吞掉所有键（`_ => return Ok(())`），放在它们之后，模式切换
    // 在确认框开着时就按不到了。其余全局键统一交给下面的注册表分发。
    // 输入模态打开时 Shift+Tab 不能触发模式切换（会吞掉输入框里的按键）
    if key.code == KeyCode::BackTab
        && !state.is_input_modal_open()
        && !state.is_awaiting_confirmation
    {
        cycle_approval_mode(state, agent_tx).await;
        return Ok(());
    }

    // Handle /clear confirmation (y/n/Enter/Esc) 与大段粘贴确认
    if state.show_clear_confirmation || state.show_paste_confirmation {
        handle_inline_confirm(state, key, agent_tx).await?;
        return Ok(());
    }

    // 模型切换 y/n 确认。
    //
    // 位置必须和上面的 `handle_inline_confirm` 一样在**全局键注册表之前**：这个确认
    // 要吃掉所有键（换模型是破坏性的，不能被 F1/Ctrl+P 顶掉）。此前它只在下方的
    // `KeyContext::of` 分发表里出现过，而那个分支是
    //
    //     KeyContext::ModelConfirm => return Ok(())
    //
    // —— 位于"这些上下文已经在阶梯更早处被独占处理"那一组里，只吞键、不处理。
    // `handle_model_confirmation` 因此**没有任何调用点**：一旦选模型时触发了 y/n
    // 提示，`y` 被吞、`n` 被吞、`Esc` 也被吞（Esc 的清零在 `ModelConfirm` 分支之后
    // 才执行），`pending_model_confirmation` 永远为 true，输入框彻底死掉 ——
    // 表现为"选完模型之后界面按什么都不动，也切不了第二次"。
    //
    // 门槛用 `KeyContext::of(state) == ModelConfirm` 而不是裸读标志：`of()` 里的
    // 判断顺序就是优先级表（模态 > 工具审批 > help > 行内确认 > 模型确认），
    // 直接拿它当条件，两处就不会各自漂移。裸读标志会把"工具审批和模型确认同时
    // 挂着"的情况判给模型确认，与 `of()` 声明的顺序相反。
    if KeyContext::of(state) == KeyContext::ModelConfirm {
        handle_model_confirmation(state, key, agent_tx).await?;
        return Ok(());
    }

    // ── 全局键分发 ──
    //
    // 「哪个键对应哪个绑定」只有 `keymap::binding_for` 一处定义，帮助文案与 modal
    // 的放行名单都由它派生。这里不再手写 `if CONTROL && Char('p')` —— 注册表与实际
    // 行为一旦各写一份就会漂移（此前 `Ctrl+R` 的守卫、`Shift+Tab` 的例外就各自漂
    // 过一次），而且没有任何测试能发现。
    if let Some(binding) = crate::ui::events::keymap::binding_for(&key) {
        if dispatch_global_binding(state, binding, key, agent_tx).await? {
            return Ok(());
        }
    }

    // Ctrl+B 有意不绑定：Claude Code 把它留给 `task:background`（Task 作用域），且只在
    // 真有前台任务可转后台时才吃掉这个键，空闲时让它落回 readline 的 backward-char。
    // starcode 目前没有「把正在跑的前台任务转后台」的运行时能力（run_in_background 只在
    // 工具描述里许诺过，Rust 侧没有实现），所以这里同样不拦截，交给 textarea 当左移光标用。
    // 任务面板改由 Ctrl+T 切换（对标 app:toggleTodos）。

    // ── 上下文查表分发 ──
    //
    // 阶梯剩下的每一层现在都由 `KeyContext::of` 决定归属：守卫不再写在调用点，
    // 而是「这个键此刻处在哪个上下文」的唯一推导结果（`events/key_context.rs`）。
    // 换来的不只是可读性 —— 此前「选择器 vs 任务面板」这类优先级只能靠书写顺序
    // 表达，现在由 `of()` 里的判断顺序显式给出。
    match KeyContext::of(state) {
        // 工具审批 / 权限说明 / 权限调试面板
        KeyContext::ToolConfirmation => {
            handle_tool_approval(state, key, agent_tx).await?;
            return Ok(());
        }
        // 任务面板（常驻面板，不独占键盘）：未命中的组合键继续往下走。
        KeyContext::TaskPanel => {
            if handle_task_panel_key(state, key) {
                return Ok(());
            }
        }
        // 后台代理选择器：焦点在选择器上时独占 ↑/↓/Enter/Esc。
        // `BgAgentSelector` 已蕴含「没有模态、没有行内确认」，不需要额外守卫。
        KeyContext::BgAgentSelector => {
            if handle_bg_agent_selector_key(state, key) {
                return Ok(());
            }
        }
        // 建议浮层：↑↓ 选择、Enter 采纳、Tab 采纳补全。
        KeyContext::SuggestionOverlay => {
            if handle_suggestion_overlay(state, key, agent_tx).await? {
                return Ok(());
            }
        }
        // 输入框：`Ctrl+V` 是全局键，但注册表里刻意把它留给这一层（填 API Key、
        // Base URL 靠它粘剪贴板）。其余全局键已经在上面分发过了。
        KeyContext::InputModal => {
            if crate::ui::events::keymap::binding_for(&key) == Some(GlobalBinding::Paste) {
                crate::ui::events::input::handle_input_modal(state, key, agent_tx).await?;
            }
            return Ok(());
        }
        // 帮助浮层：ESC 关闭它（弹窗上明写着「Press ? or ESC to close」）。
        //
        // 之前它和下面那批「吃掉所有键」的上下文混在一起，于是 ESC 在这里
        // 就被吞掉了 —— 用户按 ESC 没反应，弹窗关不掉。
        //
        // 它必须**先于**下面那批处理：主 ESC 处理器在更下面（`KeyCode::Esc`
        // 那一支），而这里 return 掉了，键根本走不到那儿。
        //
        // F1 / ? 也关（与打开时对称）。
        KeyContext::Help => {
            match key.code {
                KeyCode::Esc => state.show_help = false,
                KeyCode::F(1) => state.show_help = false,
                KeyCode::Char('?') => state.show_help = false,
                KeyCode::Up => state.help_scroll = state.help_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::PageDown | KeyCode::End => {
                    let height = state.last_help_terminal_height;
                    let max = crate::ui::components::help_popup::max_help_scroll(
                        height,
                        state.last_terminal_width,
                    );
                    state.help_scroll = match key.code {
                        KeyCode::End => max,
                        KeyCode::PageDown => state.help_scroll.saturating_add(10).min(max),
                        _ => state.help_scroll.saturating_add(1).min(max),
                    };
                }
                KeyCode::PageUp => state.help_scroll = state.help_scroll.saturating_sub(10),
                KeyCode::Home => state.help_scroll = 0,
                _ => {}
            }
            return Ok(());
        }
        // 这些上下文已经在阶梯更早处被独占处理（它们吃掉所有键），走到这里
        // 说明处理完了但键没被消费 —— 直接结束，别漏进主输入框。
        KeyContext::InlineConfirm
        | KeyContext::ModelConfirm
        | KeyContext::Modal
        | KeyContext::ThemePicker
        | KeyContext::ReadOnlyDialog
        | KeyContext::Palette => return Ok(()),
        KeyContext::Vim | KeyContext::TextInput => {}
    }

    // Code block copy: press 'c' when no overlay is active to copy last code block
    if key.code == KeyCode::Char('c') && key.modifiers.is_empty() {
        if state.input.is_empty() {
            if let Some(ref code) = state.last_code_block_content {
                // Copy to clipboard（含 OSC 52 兜底）
                match crate::ui::utils::clipboard::copy_to_clipboard(code) {
                    Ok(()) => state.push_toast(
                        &crate::core::i18n::t1("ui.status.copied_code_block"),
                        crate::ui::state::store::ToastKind::Success,
                    ),
                    Err(e) => {
                        // 此前这里只 `if let Ok(...)`，失败时什么都不发生 ——
                        // 用户按了 'c' 却毫无反馈。失败要说出来。
                        state.current_status_line =
                            Some(format!("Copy failed: {e} — press 'c' to retry"));
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[COPY] code block failed: {}",
                            e
                        ));
                    }
                }
                return Ok(());
            }
        }
    }

    // Vim mode intercept
    if state.vim_enabled && KeyContext::of(state).vim_enabled() {
        use crate::ui::vim::VimMode;
        if let KeyCode::Char(c) = key.code {
            match state.vim_state.mode {
                VimMode::Normal => {
                    state.vim_state.pending_keys.push(c);
                    let pending_keys = state.vim_state.pending_keys.clone();

                    // Try motion first
                    if let Some(motion) =
                        crate::ui::vim::motions::Motion::from_key(c, &pending_keys)
                    {
                        apply_vim_motion(state, &motion);
                        state.vim_state.reset();
                        return Ok(());
                    }

                    // Check if we need more keys (e.g., 'g', 'f', 'F', 't', 'T')
                    let needs_more = matches!(c, 'g' | 'f' | 'F' | 't' | 'T');
                    if needs_more {
                        return Ok(()); // Wait for next key
                    }

                    // Mode switches
                    match c {
                        'i' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'a' => {
                            state.vim_state.mode = VimMode::Insert;
                            state
                                .textarea
                                .move_cursor(tui_textarea::CursorMove::Forward);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'A' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::End);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'I' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'o' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::End);
                            state.textarea.insert_newline();
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'O' => {
                            state.vim_state.mode = VimMode::Insert;
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.textarea.insert_newline();
                            state.textarea.move_cursor(tui_textarea::CursorMove::Up);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'v' => {
                            state.vim_state.mode = VimMode::Visual;
                            state.vim_state.reset();
                            return Ok(());
                        }
                        'x' => {
                            // Delete character under cursor
                            state.textarea.delete_next_char();
                            sync_input_from_textarea(state);
                            return Ok(());
                        }
                        'd' if pending_keys == "dd" => {
                            // Delete entire line
                            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                            state.textarea.delete_line_by_end();
                            state.textarea.delete_next_char();
                            sync_input_from_textarea(state);
                            state.vim_state.reset();
                            return Ok(());
                        }
                        _ => {
                            // Unknown key in normal mode, reset
                            state.vim_state.reset();
                            return Ok(());
                        }
                    }
                }
                VimMode::Insert => {
                    // In insert mode, fall through to normal input handling
                }
                VimMode::Visual => {
                    // Visual mode: Escape returns to Normal
                    if key.code == KeyCode::Esc {
                        state.vim_state.mode = VimMode::Normal;
                        state.vim_state.reset();
                        return Ok(());
                    }
                    // Other visual mode keys handled by motions
                    let pending_keys = state.vim_state.pending_keys.clone();
                    if let Some(motion) =
                        crate::ui::vim::motions::Motion::from_key(c, &pending_keys)
                    {
                        apply_vim_motion(state, &motion);
                        state.vim_state.reset();
                        return Ok(());
                    }
                    state.vim_state.reset();
                    return Ok(());
                }
            }
        } else {
            // Non-char keys in vim modes
            match state.vim_state.mode {
                VimMode::Normal => match key.code {
                    KeyCode::Esc => {
                        state.vim_state.reset();
                        return Ok(());
                    }
                    _ => {}
                },
                VimMode::Insert => {
                    // Fall through to normal input handling for non-char keys
                }
                VimMode::Visual => {
                    if key.code == KeyCode::Esc {
                        state.vim_state.mode = VimMode::Normal;
                        state.vim_state.reset();
                        return Ok(());
                    }
                }
            }
        }
    }

    // 建议浮层（provider / session 菜单、命令与 @ 补全）的按键由上下文分发表接管；
    // 粘贴与主输入区的键在这里处理。

    if handle_paste(state, key).await? {
        return Ok(());
    }

    // 模态栈上的窗口一律由 `modal_input::handle_modal_key` 分发（LogSelector、
    // StatusModal、ThemePicker、InputModal 都在那张表里），
    // 非模态的上下文也已在上文的查表分发里处理完。这里只剩主输入区。

    match key.code {
        KeyCode::Enter => {
            // 如果正在粘贴，忽略 Enter 键（防止换行被解释为发送）
            if state.paste_in_progress {
                return Ok(());
            }

            // 粘贴结束后的短保护窗口：吞掉残留 Enter，避免误发送
            if let Some(end_time) = state.paste_end_time {
                if end_time.elapsed() < Duration::from_millis(PASTE_ENTER_GUARD_MS) {
                    return Ok(());
                }
            }

            // 启发式粘贴检测：如果按键间隔极短 (<20ms)，认为是粘贴行为，不发送消息
            // 这主要解决了不支持 Bracketed Paste 的终端（如部分 Windows 环境）粘贴多行文本时会自动发送的问题
            if let Some(last_time) = last_key_time {
                if last_time.elapsed() < Duration::from_millis(RAPID_PASTE_KEY_INTERVAL_MS) {
                    state.textarea.insert_newline();
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                    return Ok(());
                }
            }

            if !state.show_help {
                // Check for Alt+Enter or Ctrl+Enter for newline
                if key.modifiers.contains(KeyModifiers::ALT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    state.textarea.insert_newline();
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                    return Ok(());
                }

                if crate::ui::services::session_restore::block_submission(state) {
                    return Ok(());
                }
                let raw_input = state.textarea.lines().join("\n");
                let input =
                    crate::ui::state::expand_paste_segments(&raw_input, &state.paste_segments);

                let commands_allowed =
                    state.input_intent == crate::types::InputIntent::CommandEligible;
                let trimmed_raw = raw_input.trim();
                let has_images = state.paste_segments.iter().any(|segment| {
                    matches!(
                        segment.kind,
                        crate::ui::state::PasteKind::Image { .. }
                            | crate::ui::state::PasteKind::AttachedImage(_)
                    ) && raw_input.contains(&crate::ui::state::format_image_paste_ref(segment.id))
                });
                if commands_allowed && has_images && trimmed_raw.starts_with(['/', '!']) {
                    state.push_toast(
                        &crate::core::i18n::t1("ui.image.send_unavailable"),
                        crate::ui::state::ToastKind::Warning,
                    );
                    return Ok(());
                }
                if commands_allowed && (trimmed_raw == "/model" || trimmed_raw == "/models") {
                    reset_main_textarea(state);

                    if state.available_models.is_empty() {
                        // 先吃磁盘缓存：面板直接显示真实列表，不闪假 loading。
                        let pid = state.current_provider_id.clone().unwrap_or_else(|| {
                            crate::agent::model_cache::DEFAULT_BUCKET.to_string()
                        });
                        let seeded =
                            crate::ui::services::stream::seed_models_from_disk_cache(state, &pid);
                        state.awaiting_models = !seeded;
                        state.awaiting_models_since = if seeded {
                            None
                        } else {
                            Some(std::time::Instant::now())
                        };
                        // Trigger model list fetch
                        if let Err(_e) = agent_tx
                            .send(AgentRequest::ListModels { force: false })
                            .await
                        {
                            // Handle error silently or log
                        }
                    }

                    state.palette_history.clear();
                    state.open_palette(PaletteMode::Model);
                    crate::ui::services::suggestions::on_input_changed(state);
                    return Ok(());
                }

                if let Some(model_name) = trimmed_raw
                    .strip_prefix("/model ")
                    .filter(|_| commands_allowed)
                {
                    let model_name = model_name.trim();
                    if !model_name.is_empty() {
                        reset_main_textarea(state);
                        // 与 palette 路径同口径：映射 → 跨桶查找 → 当前 provider。
                        let provider_id = resolve_provider_for_model(state, model_name);
                        // 已获取过模型列表时做校验提示（仍允许自定义名称）。
                        let known = state.available_models.is_empty()
                            || state.available_models.iter().any(|m| m == model_name);
                        let note = if known {
                            String::new()
                        } else {
                            format!(" {}", crate::core::i18n::t1("ui.model.not_in_list"))
                        };
                        if request_runtime_model_change(
                            state,
                            model_name.to_string(),
                            provider_id,
                            agent_tx,
                        )
                        .await
                        {
                            crate::ui::app::logic::emit_status_text(
                                state,
                                0,
                                &format!("Requesting model {}{}…", model_name, note),
                            );
                        }
                        return Ok(());
                    }
                }

                if !input.trim().is_empty() {
                    let prepared = match state.prepared_input() {
                        Ok(input) => input,
                        Err(error) => {
                            state.push_toast(&error, crate::ui::state::store::ToastKind::Error);
                            return Ok(());
                        }
                    };
                    if !prepared.images.is_empty()
                        && (state.network_offline
                            || crate::core::offline::is_offline()
                            || state.current_model.trim().is_empty()
                            || (commands_allowed
                                && prepared.text.trim_start().starts_with(['/', '!'])))
                    {
                        state.push_toast(
                            &crate::core::i18n::t1("ui.image.send_unavailable"),
                            crate::ui::state::store::ToastKind::Warning,
                        );
                        return Ok(());
                    }
                    // Important: Clear textarea BEFORE enqueueing message to prevent UI glitch
                    // where Enter keeps triggering and input remains.
                    reset_main_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);

                    crate::ui::app::logic::enqueue_user_input(state, prepared, agent_tx).await?;
                } else {
                    // Clear empty input if any (e.g. just newlines)
                    reset_main_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                }
            } else {
                state.show_help = false;
            }
        }
        KeyCode::Esc => {
            // 选择器可能在一轮刚开始时仍然打开。Esc 先关选择器，不中断模型回合。
            if state.show_session_menu {
                navigate_back_from_quick_menu(state);
                return Ok(());
            }
            // 详情页的 Esc 先返回主会话，不能被运行中的取消分支抢走。
            if state.viewing_agent_task_id.is_some() {
                state.exit_teammate_view();
                return Ok(());
            }

            // Handle Streaming Cancellation
            if state.is_streaming || state.is_processing {
                let _ = agent_tx.send(AgentRequest::Abort).await;
                state.current_status_line = Some("Cancelling...".to_string());
                state.is_processing = false;
                state.cancelling_since = Some(Instant::now());
                return Ok(());
            }

            state.show_help = false;
            state.show_command_hints = false;
            state.command_hints.clear();
            state.show_mention_hints = false;
            state.mention_hints.clear();
            // 关闭 modal stack 栈顶（如果有）
            if let Some(closed) = state.pop_modal() {
                match closed {
                    crate::ui::state::modal::Modal::ThemePicker { prev_theme } => {
                        if let Some(prev) = prev_theme {
                            state.theme_manager.set_theme(&prev);
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            state.pending_model_confirmation = false;
            if state.show_provider_menu || state.show_session_menu {
                navigate_back_from_quick_menu(state);
            } else {
                close_quick_menus(state);
            }

            // Claude Code 风格：无任何覆盖层时，双击 Esc 清空输入并存入历史。
            //
            // 「有没有覆盖层」由 `KeyContext` 判定（取代手写的 `has_overlay_active()`
            // 清单）：任务面板与 vim 不算覆盖层，其余都算。
            let overlay_open = KeyContext::of(state).is_exclusive();

            // 本轮以错误收场且没有别的遮挡时，Esc 重发上一条用户消息。
            //
            // 放在排队消息和双击清空之前：那两条的前提都是"上一轮已经结束"，而
            // 失败回合恰好是最需要"按一下就好"的那一刻 —— 用户刚在 `/provider`
            // 里换好 key，界面上却没有任何键能把刚才那条再发一遍，Esc 只能走到
            // 最下面的 `else { state.last_esc_at = None; }` 变成彻底的 no-op。
            //
            // 但**这次重发注定会被"没选模型"的保护挡下**时不重发。那个保护会**代为
            // 打开模型面板**，于是 Esc → 重发 → 开面板 → Esc → 重发 → 又一条同样的
            // 报错，循环往复，屏幕上一排相同的提示。真实事故里转了 6 次。
            //
            // 判据必须和 `enqueue_user_message` 里那道保护**逐字一致**：模型为空
            // **且**有 provider。只看"模型为空"是不够的 —— 没有 provider 时那条
            // 消息仍然发得出去，agent 能自己解析默认模型，Esc 重发照样有意义。
            let blocked_by_no_model_guard = state.current_model.trim().is_empty()
                && state
                    .current_provider_id
                    .as_deref()
                    .map(|provider_id| !provider_id.trim().is_empty())
                    .unwrap_or(false);
            if !overlay_open
                && state.last_turn_failed
                && !blocked_by_no_model_guard
                && state.input.trim().is_empty()
            {
                if retry_last_user_message(state, agent_tx).await? {
                    state.last_esc_at = None;
                    state.current_status_line = Some("Retrying last message…".to_string());
                    return Ok(());
                }
            }

            // 本轮已经停了但队列还压着东西：Esc 先把它取回输入框，
            // 让用户能改词或直接删掉（对标 popAllEditable 绑在 Esc 上）
            if !overlay_open && !state.pending_user_messages.is_empty() {
                let n = state.pending_user_messages.len();
                restore_queued_messages_to_input(state);
                state.last_esc_at = None;
                state.current_status_line = Some(format!(
                    "{} queued message{} moved back to the prompt",
                    n,
                    if n == 1 { "" } else { "s" }
                ));
                return Ok(());
            }

            if !overlay_open && !state.input.trim().is_empty() {
                let now = Instant::now();
                let double = state
                    .last_esc_at
                    .map(|t| now.duration_since(t) <= Duration::from_millis(1000))
                    .unwrap_or(false);
                if double {
                    if let Ok(input) = state.prepared_input() {
                        let is_dup = state.command_history.front() == Some(&input);
                        if !is_dup {
                            state.command_history.push_front(input);
                            state.command_history.truncate(100);
                            state.save_input_history();
                        }
                    }
                    reset_main_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                    state.last_esc_at = None;
                    state.current_status_line =
                        Some("Input cleared (saved to history)".to_string());
                } else {
                    state.last_esc_at = Some(now);
                    state.current_status_line = Some("Press Esc again to clear input".to_string());
                }
            } else {
                state.last_esc_at = None;
            }
        }
        KeyCode::F(1) => {
            state.show_help = !state.show_help;
        }
        KeyCode::Char('?') if !state.is_palette_open() && !state.is_input_modal_open() => {
            // ? when input is empty: show help (like Claude Code)
            if state.textarea.lines().iter().all(|l| l.is_empty()) {
                state.show_help = !state.show_help;
            } else {
                // Otherwise insert ? normally
                push_cursor_off_sentinel(state);
                state.textarea.input(key);
                sync_input_from_textarea(state);
            }
        }
        KeyCode::Tab => {
            // Tab: toggle expand/collapse of the tool block under the viewport
            if !state.is_palette_open() && !state.is_awaiting_confirmation {
                let focused_idx = find_focused_tool_entry(state);
                if let Some(idx) = focused_idx {
                    // 与鼠标单击工具头走同一条路径（ChatState::toggle_tool_expansion）：
                    // 切换、失效缓存、锚定视口都在里面做完了。
                    state.toggle_tool_expansion(idx);
                }
            }
        }
        KeyCode::PageUp => {
            let page = state.last_chat_height.saturating_sub(2) as i32;
            crate::ui::state::scroll_chat(state, -page.max(1));
        }
        KeyCode::PageDown => {
            let page = state.last_chat_height.saturating_sub(2) as i32;
            crate::ui::state::scroll_chat(state, page.max(1));
        }
        KeyCode::Home => {
            // Ctrl+Home: scroll to top of chat
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                state.scroll = 0;
                state.auto_follow = false;
            } else {
                // Home: move cursor to start of line
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                }
            }
        }
        KeyCode::End => {
            // Ctrl+End: scroll to bottom of chat (auto-follow)
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                state.auto_follow = true;
                state.show_scroll_to_bottom = false;
            } else {
                // End: move cursor to end of line
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                }
            }
        }
        KeyCode::Up => {
            let cursor_row = state.textarea.cursor().0;

            if cursor_row == 0 && !state.pending_user_messages.is_empty() {
                // 队列优先于命令历史（对标 PromptInput 的 handleHistoryUp）：
                // 光标在首行时 ↑ 先把排队的输入取回来改，而不是翻历史
                let n = state.pending_user_messages.len();
                restore_queued_messages_to_input(state);
                state.current_status_line = Some(format!(
                    "{} queued message{} moved back to the prompt",
                    n,
                    if n == 1 { "" } else { "s" }
                ));
            } else if cursor_row == 0 {
                // 首行继续向上进入历史；先保留完整草稿，向下退出时恢复正文和附件。
                if !state.command_history.is_empty() {
                    let idx = match state.history_index {
                        Some(i) => i.saturating_add(1),
                        None => {
                            let Ok(snapshot) = state.prepared_input() else {
                                // 附件或粘贴内容无法还原时，不能覆盖尚未保存的草稿。
                                return Ok(());
                            };
                            state.history_input_snapshot = Some(snapshot);
                            0
                        }
                    };
                    if idx < state.command_history.len() {
                        state.history_index = Some(idx);
                        let hist_entry = state.command_history[idx].clone();
                        state.set_user_input(hist_entry);
                        sync_input_from_textarea(state);
                        crate::ui::services::suggestions::on_input_changed(state);
                    }
                }
            } else if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        KeyCode::Down => {
            let cursor_row = state.textarea.cursor().0;
            let line_count = state.input_line_count;

            if cursor_row + 1 >= line_count && state.history_index.is_some() {
                // Command history navigation: Down when cursor is on last line and in history mode
                let idx = state.history_index.unwrap();
                if idx == 0 {
                    state.history_index = None;
                    let snapshot = state.history_input_snapshot.take().unwrap_or_default();
                    state.set_user_input(snapshot);
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                } else {
                    let new_idx = idx.saturating_sub(1);
                    state.history_index = Some(new_idx);
                    let hist_entry = state.command_history[new_idx].clone();
                    state.set_user_input(hist_entry);
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                }
            } else if cursor_row + 1 >= line_count
                && state.bg_agent_selection.is_none()
                && !state.background_agent_rows().is_empty()
            {
                // ↓ 从输入框末行溢出 → 落进后台代理选择器（对标 background agent selector：
                // 「↓ manage」那句提示指的就是这个动作）。没有后台代理时保持原样，
                // 让 textarea 自己吞掉按键。
                state.bg_agent_selection = Some(0);
            } else if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+Z: Undo
            if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+Y: Kill ring 为空时退化为 Redo（textarea 默认行为）
            if state.kill_ring.is_empty() {
                if state.textarea.input(key) {
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                }
            } else {
                let idx = state.kill_ring.len() - 1;
                yank_from_ring(state, idx);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::ALT) => {
            // Alt+Y: yank-pop — 轮换 kill ring 中更早的条目
            if !state.kill_ring.is_empty() {
                let len = state.kill_ring.len();
                let next = match state.kill_ring_pos {
                    Some(p) if p > 0 => p - 1,
                    Some(_) => len - 1,
                    None => len.saturating_sub(1),
                };
                yank_from_ring(state, next);
            }
        }
        KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+X: Cut selected text
            if state.textarea.input(key) {
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+U: Delete from cursor to beginning of line (Unix readline)
            let (row, col) = state.textarea.cursor();
            if let Some(line) = state.textarea.lines().get(row) {
                let killed = line.chars().take(col).collect::<String>();
                push_kill(state, killed);
            }
            state.textarea.delete_line_by_head();
            sync_input_from_textarea(state);
            crate::ui::services::suggestions::on_input_changed(state);
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+K: Delete from cursor to end of line (Unix readline)
            let (row, col) = state.textarea.cursor();
            if let Some(line) = state.textarea.lines().get(row) {
                let killed = line.chars().skip(col).collect::<String>();
                push_kill(state, killed);
            }
            state.textarea.delete_line_by_end();
            sync_input_from_textarea(state);
            crate::ui::services::suggestions::on_input_changed(state);
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+W: Delete word backward (Unix readline)
            let (row, col) = state.textarea.cursor();
            let line_owned = state.textarea.lines().get(row).cloned();
            if let Some(line) = line_owned {
                let chars: Vec<char> = line.chars().collect();
                let cursor = col.min(chars.len());
                let before = &chars[..cursor];
                let trimmed_len = before
                    .iter()
                    .rposition(|ch| !ch.is_whitespace())
                    .map_or(0, |idx| idx + 1);
                let new_col = if trimmed_len == 0 {
                    0
                } else {
                    let last_is_word =
                        before[trimmed_len - 1].is_alphanumeric() || before[trimmed_len - 1] == '_';
                    let mut start = trimmed_len - 1;
                    while start > 0 {
                        let previous = before[start - 1];
                        if (previous.is_alphanumeric() || previous == '_') != last_is_word {
                            break;
                        }
                        start -= 1;
                    }
                    start
                };
                let killed: String = chars[new_col..cursor].iter().collect();
                push_kill(state, killed);
                let new_line: String = chars[..new_col]
                    .iter()
                    .chain(chars[cursor..].iter())
                    .collect();
                state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                state.textarea.delete_line_by_end();
                state.textarea.insert_str(&new_line);
                state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                for _ in 0..new_col {
                    state
                        .textarea
                        .move_cursor(tui_textarea::CursorMove::Forward);
                }
                sync_input_from_textarea(state);
                crate::ui::services::suggestions::on_input_changed(state);
            }
        }
        _ => {
            if !state.show_help {
                push_cursor_off_sentinel(state);
                let mut handled = state.textarea.input(key);
                // Fallback: if textarea.input() didn't handle a regular Char, insert it directly
                if !handled {
                    if let KeyCode::Char(c) = key.code {
                        if !c.is_control()
                            && (key.modifiers.is_empty()
                                || key.modifiers == crossterm::event::KeyModifiers::SHIFT)
                        {
                            state.textarea.insert_char(c);
                            handled = true;
                        }
                    }
                }
                if handled {
                    // 仅在无粘贴块时解除旧式折叠；有粘贴块时保持块内嵌显示
                    if state.input_folded && state.paste_segments.is_empty() {
                        state.input_folded = false;
                    }
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                }
            }
        }
    }

    Ok(())
}

async fn handle_paste(
    state: &mut ChatState,
    key: KeyEvent,
) -> Result<bool, Box<dyn std::error::Error>> {
    if crate::ui::events::keymap::binding_for(&key) != Some(GlobalBinding::Paste) {
        return Ok(false);
    }

    let Some(target) = super::clipboard_paste::clipboard_target(state) else {
        return Ok(false);
    };
    if state.pending_clipboard_read.is_some() {
        return Ok(true);
    }
    let (tx, result) = tokio::sync::oneshot::channel();
    state.pending_clipboard_read = Some(crate::ui::state::PendingClipboardRead { target, result });
    state.paste_in_progress = true;
    state.paste_end_time = None;
    tokio::spawn(async move {
        use crate::ui::state::ClipboardPayload;
        let read = tokio::task::spawn_blocking(|| {
            if let Some((path, width, height)) = save_clipboard_image() {
                return Ok(ClipboardPayload::Image(path, width, height));
            }
            Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .map(ClipboardPayload::Text)
                .map_err(|error| error.to_string())
        });
        let value = match timeout(Duration::from_secs(15), read).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("Clipboard read timed out".to_string()),
        };
        let _ = tx.send(value);
    });
    Ok(true)
}

/// 测试用的入口：暴露"视口下那一块工具"的查找结果。
#[cfg(test)]
pub(crate) fn find_focused_tool_entry_for_test(state: &ChatState) -> Option<usize> {
    find_focused_tool_entry(state)
}

/// Find the tool block under the viewport, for keyboard toggle.
/// Returns the index of the tool **header** entry, or None if no tool block
/// is visible in the viewport.
///
/// 找的是**整块**（工具头 + 工具体），不是只找工具头那条 entry。
///
/// 一个工具块在 `chat_history` 里是两条 entry：工具头（ToolCall，一行工具名）
/// 和工具体（ToolResult，展开后能有两百行）。只按工具头找的话，视口中心落在
/// 工具体里时永远匹配不上 —— 而"展开之后正好在正文中间"恰恰是最常见的场景，
/// 于是 Tab 在那儿按了没反应。
///
/// 只认**与视口有重叠**的块：视口里根本没有工具块时，Tab 什么都不做 ——
/// 去切一个屏幕外的块只会把视口拽到别处。
fn find_focused_tool_entry(state: &ChatState) -> Option<usize> {
    use crate::types::ChatEntryType;

    let viewport_height = state.last_chat_height as usize;
    if viewport_height == 0 {
        return None;
    }
    // 用本帧算出来的 scroll，而不是 state.scroll：后者要到渲染完才回写，
    // auto_follow 时每帧都在变（与 app/mod.rs 里 chat 区的取法同一个理由）。
    let max_scroll = state.total_rendered_lines.saturating_sub(viewport_height);
    let scroll = if state.auto_follow || state.scroll > max_scroll {
        max_scroll
    } else {
        state.scroll
    };
    let viewport_center = scroll + viewport_height / 2;
    let viewport_bottom = scroll + viewport_height;

    // 关键：条目的**屏幕行区间**由上一次渲染记录的真实行高累加得到。
    //
    // 原实现拿 entry 下标跟 `scroll / 2` 比距离 —— entry 下标是序号不是行号，
    // 一个 30 行的工具块和一个 1 行的回复在下标空间里等价，于是 Tab 经常展开
    // 视口里根本看不见的那一块，用户看到的是"什么都没发生"。
    //
    // 用 `last_item_heights` 而不是 `virtual_list`：鼠标命中测试
    // （`map_mouse_to_chat_position`）用的也是它 —— 两条路径必须用同一份
    // 行高，否则"点到的"和"Tab 切的"会是不同的块。
    //
    // 候选是「同一个工具块的头 + 身」：工具体有多高都算进这一块的区间里，
    // 视口中心落在正文里同样能命中这一块。
    let mut best_idx: Option<usize> = None;
    let mut best_distance: usize = usize::MAX;
    let mut top = 0usize;
    for (idx, entry) in state.chat_history.iter().enumerate() {
        let height = state.last_item_heights.get(idx).copied().unwrap_or(0) as usize;
        let bottom = top + height;
        // 工具体（ToolResult）也是候选：它才是展开后占满屏幕的那一条。
        // 命中后仍然切**工具头**那条 entry —— 折叠位按 tool_call.id 记，
        // 两条切的是同一份状态，但切头那条能保证"同一块只切一次"。
        let is_tool_part = entry.tool_call.is_some()
            && matches!(
                entry.entry_type,
                ChatEntryType::ToolCall | ChatEntryType::ToolResult
            )
            && height > 0;
        // 与视口有重叠才算"视口下的那一块"
        if is_tool_part && top < viewport_bottom && bottom > scroll {
            // 中心落在这一块里 → 距离 0（最优）；否则取中心到这一块区间的距离。
            let distance = if viewport_center >= top && viewport_center < bottom {
                0
            } else if viewport_center < top {
                top - viewport_center
            } else {
                viewport_center - bottom + 1
            };
            if distance < best_distance {
                best_distance = distance;
                best_idx = Some(idx);
            }
        }
        top = bottom;
    }

    // 命中的可能是工具体那条；统一换成同一块的工具头 —— 折叠位与视口锚定
    // 都以头为准。
    best_idx.and_then(|idx| state.tool_header_index(idx).or(Some(idx)))
}

/// Apply a vim motion to the textarea cursor.
fn apply_vim_motion(state: &mut ChatState, motion: &crate::ui::vim::motions::Motion) {
    use crate::ui::vim::motions::Motion;
    match motion {
        Motion::Left => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Back);
        }
        Motion::Down => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Down);
        }
        Motion::Up => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Up);
        }
        Motion::Right => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::Forward);
        }
        Motion::WordForward => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordForward);
        }
        Motion::WordBackward => {
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordBack);
        }
        Motion::WordEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        Motion::LineStart => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Head);
        }
        Motion::LineEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        Motion::FileStart => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Top);
        }
        Motion::FileEnd => {
            state.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        }
        _ => {
            // Paragraph, matching bracket, find char — approximate with word movements
            state
                .textarea
                .move_cursor(tui_textarea::CursorMove::WordForward);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::store::AgentTaskInfo;

    #[tokio::test]
    async fn pasted_markdown_hash_is_sent_in_full_instead_of_saved_as_memory() {
        let project = crate::ui::test_support::TestProject::new();
        let text = std::iter::once("# 粘贴的任务要求".to_string())
            .chain((1..53).map(|i| format!("第 {i:02} 行：{}", "保留完整正文与缩进 ".repeat(6))))
            .collect::<Vec<_>>()
            .join("\n");
        for with_image in [false, true] {
            for collapsed in [false, true] {
                let mut state = ChatState::test_fixture();
                state.chat_history.clear();
                state.draft_path = None;
                state.current_model = "test-model".into();
                if collapsed {
                    super::super::clipboard_paste::insert_paste_block_confirmed(
                        &mut state,
                        text.clone(),
                    );
                    assert_eq!(state.paste_segments.len(), 1);
                } else {
                    state.textarea.insert_str("# 短标题\n正文");
                }
                let mut png = std::io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgb8(1, 1)
                    .write_to(&mut png, image::ImageFormat::Png)
                    .unwrap();
                let image = crate::types::ImageAttachment::from_bytes(png.get_ref()).unwrap();
                if with_image {
                    let id = state.paste_segments.len();
                    state.paste_segments.push(crate::ui::state::PasteSegment {
                        id,
                        content: "[Image]".into(),
                        line_count: 1,
                        kind: crate::ui::state::PasteKind::AttachedImage(image.clone()),
                    });
                    state
                        .textarea
                        .insert_str(&crate::ui::state::format_image_paste_ref(id));
                }
                sync_input_from_textarea(&mut state);
                let raw = state.textarea.lines().join("\n");
                let expected = crate::ui::state::prepare_user_input(&raw, &state.paste_segments)
                    .expect("prepare test input");
                state
                    .command_history
                    .push_front(crate::types::UserInput::from(expected.text.trim()));
                let (tx, mut rx) = mpsc::channel(8);
                handle_key_event(
                    &mut state,
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    &tx,
                    None,
                )
                .await
                .expect("submit pasted Markdown");
                match rx.try_recv().expect("pasted Markdown must reach the agent") {
                    AgentRequest::SendMessage { message, .. } if !with_image => {
                        assert_eq!(message, expected.text);
                    }
                    AgentRequest::SendUserInput { input, .. } if with_image => {
                        assert_eq!(input.text, expected.text);
                        assert_eq!(input.images, vec![image]);
                    }
                    other => panic!("unexpected Markdown request: {other:?}"),
                }
                assert!(rx.try_recv().is_err());
                assert!(state.pending_user_messages.is_empty());
                assert!(!project.path().join(".star/memory.md").exists());
                assert!(!state
                    .chat_history
                    .iter()
                    .any(|entry| entry.content.contains("Saved in")));
            }
        }
    }

    #[tokio::test]
    async fn command_palette_fills_textarea_and_clears_stale_completion() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("existing prompt");
        state.open_palette(PaletteMode::Command);
        state.palette_filter = "/resume".into();
        let (tx, mut rx) = mpsc::channel(4);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "/resume ");
        assert_eq!(state.textarea.lines().join("\n"), state.input);
        assert!(!state.is_palette_open());
        assert!(state.command_hints.is_empty());
        assert!(rx.try_recv().is_err(), "filling a command is not execution");
    }

    #[tokio::test]
    async fn dynamic_plugin_completion_accepts_real_enter_and_tab_without_execution() {
        for key in [KeyCode::Enter, KeyCode::Tab] {
            let mut state = ChatState::test_fixture();
            let (tx, mut rx) = mpsc::channel(16);
            state.plugin_commands.seed_for_tests(vec![
                crate::commands::system::PluginCommandCandidate {
                    name: "fixture-check".into(),
                    description: "Inspect local metadata".into(),
                },
            ]);
            state.set_input_text("/fixture-c");
            crate::ui::services::suggestions::on_input_changed(&mut state);
            handle_key_event(
                &mut state,
                KeyEvent::new(key, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.input.trim(), "/fixture-check");
            assert_eq!(state.textarea.lines().join("\n"), state.input);
            assert!(state.command_hints.is_empty());
            assert!(
                rx.try_recv().is_err(),
                "completion must not invoke the agent"
            );
            assert!(
                !crate::ui::services::suggestions::handle_enter(&mut state),
                "a second Enter must fall through to normal command dispatch"
            );
            state.set_input_text("/fixture-check --path \"a b\"");
            crate::ui::services::suggestions::on_input_changed(&mut state);
            assert!(!crate::ui::services::suggestions::handle_tab(&mut state));
            assert_eq!(state.input, "/fixture-check --path \"a b\"");
        }
    }

    /// 用真实按键覆盖清空后的同步和命令补全，不直接给 input 赋期望值。
    #[tokio::test]
    async fn slash_completion_recovers_after_deleting_long_input() {
        for (after_paste, panel_visible, use_tab) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, true, true),
        ] {
            let mut state = ChatState::test_fixture();
            state.task_panel.is_visible = panel_visible;
            let (tx, _rx) = mpsc::channel(16);
            if after_paste {
                state.paste_in_progress = true;
                state.paste_end_time = Some(Instant::now());
            }
            let text = "长文本🙂abcdefghijklmnopqrstuvwx".repeat(3);
            for ch in text.chars() {
                handle_key_event(
                    &mut state,
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    &tx,
                    None,
                )
                .await
                .unwrap();
            }
            for _ in text.chars() {
                handle_key_event(
                    &mut state,
                    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                    &tx,
                    None,
                )
                .await
                .unwrap();
            }
            assert!(state.input.is_empty());
            assert_eq!(state.textarea.lines(), [""]);
            assert_eq!(
                crate::ui::components::chat_input::calc_input_height(&mut state, 20),
                3
            );
            if after_paste {
                state.paste_end_time = Some(Instant::now());
            }
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            if after_paste {
                assert!(!state.show_command_hints);
                assert!(!crate::ui::services::suggestions::expire_paste_guard(
                    &mut state
                ));
                state.paste_end_time = Some(Instant::now() - Duration::from_secs(1));
                assert!(crate::ui::services::suggestions::expire_paste_guard(
                    &mut state
                ));
                assert!(!state.paste_in_progress);
            }
            assert_eq!(state.input, "/");
            assert!(state.show_command_hints);
            assert!(!state.command_hints.is_empty());
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.selected_hint, 1);
            let chosen = state.command_hints[1]
                .split(" - ")
                .next()
                .unwrap()
                .to_owned();
            handle_key_event(
                &mut state,
                KeyEvent::new(
                    if use_tab {
                        KeyCode::Tab
                    } else {
                        KeyCode::Enter
                    },
                    KeyModifiers::NONE,
                ),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.input.trim(), chosen);
            assert_eq!(state.input, state.textarea.lines().join("\n"));
            assert_eq!(
                state.textarea.wrap_mode(),
                tui_textarea::WrapMode::WordOrGlyph
            );
        }
    }

    #[tokio::test]
    async fn task_editing_keeps_keys_when_prompt_suggestions_exist() {
        let mut state = ChatState::test_fixture();
        state.task_panel.is_visible = true;
        state.task_panel.editing_task_id = Some("task-test".into());
        state.task_panel.edit_input.insert_str("task");
        state.set_input_text("/");
        crate::ui::services::suggestions::on_input_changed(&mut state);
        let (tx, _rx) = mpsc::channel(16);
        for code in [KeyCode::Char('x'), KeyCode::Left, KeyCode::Char('y')] {
            handle_key_event(
                &mut state,
                KeyEvent::new(code, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
        }
        assert_eq!(state.task_panel.edit_input.lines(), ["taskyx"]);
        assert_eq!(state.input, "/");
        assert_eq!(state.selected_hint, 0);
    }

    /// 删除引用不能让已失效的粘贴记录继续隐藏编辑器和原生光标。
    #[tokio::test]
    async fn deleting_paste_reference_restores_wrapped_editor() {
        use ratatui::{backend::TestBackend, Terminal};

        let mut state = ChatState::test_fixture();
        let (tx, _rx) = mpsc::channel(16);
        insert_paste_block(&mut state, "pasted text ".repeat(300));
        sync_input_from_textarea(&mut state);
        assert_eq!(state.paste_segments.len(), 1);
        while !state.input.is_empty() {
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
        }
        assert_eq!(state.paste_segments.len(), 1);
        for ch in "abcdefghijklmnopqrstuvwx".chars() {
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
        }
        let height = crate::ui::components::chat_input::calc_input_height(&mut state, 12);
        assert!(height > 3);
        let mut terminal = Terminal::new(TestBackend::new(12, height)).unwrap();
        terminal
            .draw(|f| crate::ui::components::chat_input::render_input(f, &state, f.area()))
            .unwrap();
        assert!(terminal.backend().cursor_visible());
        assert!(state.textarea.rendered_cursor_position().unwrap().y > 1);
    }

    #[tokio::test]
    async fn help_navigation_reaches_the_end_without_editing_input() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("draft");
        state.show_help = true;
        state.last_help_terminal_height = 24;
        state.last_terminal_width = 100;
        let (tx, _rx) = mpsc::channel(16);
        let max = crate::ui::components::help_popup::max_help_scroll(24, 100);
        assert!(max > 0);
        for (key, expected) in [(KeyCode::End, max), (KeyCode::Home, 0)] {
            handle_key_event(
                &mut state,
                KeyEvent::new(key, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.help_scroll, expected);
            assert!(state.show_help);
            assert_eq!(state.input, "draft");
        }
    }

    #[tokio::test]
    async fn shift_tab_does_not_approve_a_pending_tool() {
        let mut state = ChatState::test_fixture();
        state.is_awaiting_confirmation = true;
        state.pending_tool_call_id = Some("approval-regression".into());
        state.pending_confirmation_choice = 1;
        let mode = state.approval_mode.clone();
        let (tx, mut rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            &tx,
            None,
        )
        .await
        .unwrap();

        assert!(rx.try_recv().is_err());
        assert!(state.is_awaiting_confirmation);
        assert_eq!(
            state.pending_tool_call_id.as_deref(),
            Some("approval-regression")
        );
        assert_eq!(state.approval_mode, mode);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentRequest::ConfirmTool {
                outcome: crate::types::ToolConfirmationOutcome::ProceedOnce,
                ..
            }
        ));
        assert!(!state.is_awaiting_confirmation);
    }

    #[tokio::test]
    async fn rapid_arrow_keys_navigate_history_without_scrolling_chat() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("");
        state.command_history = [
            crate::types::UserInput::from("newest"),
            crate::types::UserInput::from("older"),
            crate::types::UserInput::from("oldest"),
        ]
        .into();
        state.history_index = None;
        let scroll_before = (state.scroll, state.scroll_velocity);
        let (tx, _rx) = mpsc::channel(16);
        for (key, expected) in [
            (KeyCode::Up, "newest"),
            (KeyCode::Up, "older"),
            (KeyCode::Up, "oldest"),
            (KeyCode::Down, "older"),
            (KeyCode::Down, "newest"),
            (KeyCode::Down, ""),
        ] {
            handle_key_event(
                &mut state,
                KeyEvent::new(key, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.input, expected);
            assert_eq!((state.scroll, state.scroll_velocity), scroll_before);
        }
    }

    #[tokio::test]
    async fn history_navigation_restores_nonempty_multiline_draft() {
        let mut state = ChatState::test_fixture();
        let draft = "first draft line\nsecond draft line";
        state.set_input_text(draft);
        state.textarea.move_cursor(tui_textarea::CursorMove::Top);
        state.command_history = [crate::types::UserInput::from("previous question")].into();
        let scroll_before = (state.scroll, state.scroll_velocity);
        let (tx, mut rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            state.input, "previous question",
            "必须确实进入历史，而非原地上下移动"
        );
        assert_eq!(state.history_index, Some(0));
        assert_eq!(state.textarea.lines().join("\n"), "previous question");
        assert_eq!(state.input_line_count, 1);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, draft);
        assert_eq!(state.textarea.lines().join("\n"), draft);
        assert_eq!(state.input_line_count, 2);
        assert_eq!(state.history_index, None);
        assert_eq!((state.scroll, state.scroll_velocity), scroll_before);
        assert!(rx.try_recv().is_err(), "历史浏览不得提交消息");
    }

    #[tokio::test]
    async fn history_navigation_restores_literal_draft_and_image() {
        let mut state = ChatState::test_fixture();
        state.set_user_input(crate::types::UserInput {
            text: "/keep this literal [Image]".into(),
            intent: crate::types::InputIntent::Literal,
            images: vec![crate::types::ImageAttachment {
                media_type: "image/png".into(),
                data: "Zml4dHVyZQ==".into(),
                width: 1,
                height: 1,
            }],
        });
        let before = state.prepared_input().unwrap();
        state.command_history = [crate::types::UserInput::from("previous question")].into();
        let (tx, mut rx) = mpsc::channel(16);
        for (key, expected) in [
            (KeyCode::Up, "previous question"),
            (KeyCode::Down, before.text.as_str()),
        ] {
            handle_key_event(
                &mut state,
                KeyEvent::new(key, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
            assert_eq!(state.prepared_input().unwrap().text, expected);
        }
        let restored = state.prepared_input().unwrap();
        assert_eq!(restored.intent, before.intent);
        assert_eq!(restored.images.len(), 1);
        assert_eq!(restored.images[0].data, before.images[0].data);
        assert_eq!(state.paste_segments.len(), 1);
        assert!(state.history_input_snapshot.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn history_up_keeps_multiline_cursor_motion_and_empty_history_safe() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("first draft line\nsecond draft line");
        state.command_history = [crate::types::UserInput::from("previous question")].into();
        assert_eq!(state.textarea.cursor().0, 1);
        let (tx, _rx) = mpsc::channel(16);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.textarea.cursor().0, 0);
        assert_eq!(state.input, "first draft line\nsecond draft line");
        assert_eq!(state.history_index, None);
        state.command_history.clear();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "first draft line\nsecond draft line");
        assert!(state.history_input_snapshot.is_none());
    }

    #[test]
    fn task_panel_does_not_consume_clipboard_fallback_key() {
        let mut state = ChatState::test_fixture();
        state.task_panel.is_visible = true;
        let key = KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert!(!handle_task_panel_key(&mut state, key));
        assert_eq!(
            crate::ui::events::keymap::binding_for(&key),
            Some(GlobalBinding::Paste)
        );
    }

    #[test]
    fn restoring_structured_input_replaces_image_markers_without_duplicates() {
        let mut state = ChatState::test_fixture();
        let image = crate::types::ImageAttachment {
            media_type: "image/png".into(),
            data: "aGVsbG8=".into(),
            width: 1,
            height: 1,
        };
        state.set_user_input(crate::types::UserInput {
            text: "look at [Image] please".into(),
            intent: crate::types::InputIntent::Literal,
            images: vec![image],
        });

        assert_eq!(state.paste_segments.len(), 1);
        assert_eq!(state.input, "look at [Image #0] please");
        let prepared = state.prepared_input().expect("restored attachment");
        assert_eq!(prepared.images.len(), 1);
        assert_eq!(prepared.text, "look at [Image] please");
    }

    #[test]
    fn restoring_queued_messages_preserves_image_draft() {
        let mut state = ChatState::test_fixture();
        insert_image_paste_block(&mut state, "/tmp/draft-image.png".into(), 10, 10);
        sync_input_from_textarea(&mut state);
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::from("queued requirement"));

        assert!(restore_queued_messages_to_input(&mut state));
        assert!(state.input.starts_with("queued requirement\n"));
        assert_eq!(state.paste_segments.len(), 1);
        let expanded = crate::ui::state::expand_paste_segments(&state.input, &state.paste_segments);
        assert!(expanded.contains("![Image](/tmp/draft-image.png)"));
        assert!(state.pending_user_messages.is_empty());
    }

    /// **帮助浮层必须能被 ESC 关掉。**
    ///
    /// 弹窗上明写着「Press ? or ESC to close」，但 ESC 按了没反应。
    ///
    /// 原因：`KeyContext::Help` 和其它「独占上下文」一起
    /// `return Ok(())` —— 所有键都被吞掉，包括 ESC。而真正关 help 的
    /// ESC 处理在**更下面**（主 `KeyCode::Esc` 那一支），键根本走不到。
    ///
    /// 修法：`KeyContext::Help` 单独处理 ESC / F1 / `?`，先于那批
    /// 「吃掉所有键」的上下文。
    #[tokio::test]
    async fn esc_closes_the_help_overlay() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);

        state.show_help = true;
        assert_eq!(
            crate::ui::events::key_context::KeyContext::of(&state),
            crate::ui::events::key_context::KeyContext::Help,
            "help 打开时上下文必须是 Help"
        );

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();

        assert!(
            !state.show_help,
            "ESC 必须关掉帮助浮层（弹窗上写着「Press ? or ESC to close」）"
        );
    }

    /// F1 与 `?` 也关（与打开时对称）。
    #[tokio::test]
    async fn f1_closes_the_help_overlay() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);

        state.show_help = true;
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();

        assert!(!state.show_help, "F1 必须关掉帮助浮层");
    }

    #[tokio::test]
    async fn ctrl_c_repeat_does_not_count_as_second_press() {
        let mut state = ChatState::test_fixture();
        let (tx, _rx) = mpsc::channel(4);
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        let mut repeat = key;
        repeat.kind = crossterm::event::KeyEventKind::Repeat;
        handle_key_event(&mut state, repeat, &tx, None)
            .await
            .unwrap();
        assert!(!state.should_exit);
    }

    #[tokio::test]
    async fn another_key_breaks_the_double_ctrl_c_sequence() {
        let mut state = ChatState::test_fixture();
        let (tx, _rx) = mpsc::channel(4);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, ctrl_c, &tx, None)
            .await
            .unwrap();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        handle_key_event(&mut state, ctrl_c, &tx, None)
            .await
            .unwrap();
        assert!(!state.should_exit);
    }

    #[tokio::test]
    async fn ctrl_c_expired_window_and_closed_worker_do_not_fake_cancellation() {
        let mut state = ChatState::test_fixture();
        state.last_ctrl_c = Some(Instant::now() - Duration::from_secs(2));
        state.is_processing = true;
        state.is_streaming = true;
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(!state.should_exit);
        assert!(state.is_processing);
        assert!(state.cancelling_since.is_none());
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.should_exit);
    }

    #[tokio::test]
    async fn ctrl_c_twice_during_cancellation_requests_abort_once_then_exits() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(4);
        state.is_streaming = true;
        state.is_processing = true;

        for _ in 0..2 {
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &tx,
                None,
            )
            .await
            .unwrap();
        }

        assert!(state.should_exit, "第二次 Ctrl+C 应退出，而非再次取消");
        assert!(matches!(rx.try_recv(), Ok(AgentRequest::Abort)));
        assert!(rx.try_recv().is_err(), "同一回合只应发送一次 Abort");
    }

    #[tokio::test]
    async fn ctrl_c_copy_failure_clears_exit_window_without_aborting() {
        let mut state = ChatState::test_fixture();
        state.chat_history.clear();
        state.is_processing = true;
        state.last_ctrl_c = Some(Instant::now());
        state.text_selection.start = Some((0, 0));
        state.text_selection.end = Some((0, 1));
        state.text_selection.start_entry_idx = Some(99);
        state.text_selection.end_entry_idx = Some(99);
        let (tx, mut rx) = mpsc::channel(4);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(!state.should_exit);
        assert!(state.last_ctrl_c.is_none());
        assert!(state.text_selection.has_selection());
        assert!(state.is_processing);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_c_done_preserves_exit_window_and_unsubmitted_draft() {
        let mut state = ChatState::test_fixture();
        state.is_processing = true;
        state.is_streaming = true;
        state.set_input_text("unfinished draft");
        state.paste_segments.push(crate::ui::state::PasteSegment {
            id: 0,
            content: "pasted text".into(),
            line_count: 1,
            kind: crate::ui::state::PasteKind::Text,
        });
        let (tx, mut rx) = mpsc::channel(4);
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(AgentRequest::Abort)));
        crate::ui::services::stream::handle_stream_update(
            &mut state,
            crate::runtime::messages::StreamMessage::Done { message_id: 1 },
            &tx,
        )
        .await
        .unwrap();
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.should_exit);
        assert_eq!(state.input, "unfinished draft");
        assert_eq!(state.paste_segments.len(), 1);
    }

    #[tokio::test]
    async fn ctrl_c_in_background_agent_view_requests_its_stop_and_returns_to_main() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(4);
        crate::ui::services::stream::handle_stream_update(
            &mut state,
            crate::runtime::messages::StreamMessage::AgentTaskUpdate {
                message_id: 1,
                task_id: "bg-task".into(),
                agent_type: "Explore".into(),
                description: "inspect".into(),
                status: crate::types::AgentTaskStatus::Running,
                tool_use_count: 0,
                tokens: 0,
                is_async: true,
                is_resolved: false,
                is_error: false,
                last_tool_info: None,
                name: None,
                task_description: None,
                new_sub_entries: Vec::new(),
            },
            &tx,
        )
        .await
        .unwrap();
        state.enter_teammate_view("bg-task");

        let responder = tokio::spawn(async move {
            if let Some(AgentRequest::StopBackgroundAgent { task_id }) = rx.recv().await {
                assert_eq!(task_id, "bg-task");
            } else {
                panic!("Ctrl+C should send a targeted background-task stop");
            }
        });
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        responder.await.unwrap();

        assert!(state.viewing_agent_task_id.is_none());
        assert!(state.auto_follow);
        assert!(
            state.last_ctrl_c.is_some(),
            "the return press starts the exit window"
        );
    }

    #[tokio::test]
    async fn ctrl_c_when_agent_queue_is_full_stays_responsive_and_can_still_exit() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(AgentRequest::McpRefresh)
            .expect("fill the request queue");
        state.is_streaming = true;
        state.is_processing = true;

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            state.is_processing,
            "failed enqueue must not fake cancellation"
        );
        assert!(state.cancelling_since.is_none());

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(state.should_exit, "the second press must remain responsive");
        assert!(matches!(rx.try_recv(), Ok(AgentRequest::McpRefresh)));
        assert!(
            rx.try_recv().is_err(),
            "no duplicate Abort should be queued"
        );
    }

    #[tokio::test]
    async fn ctrl_c_double_press_is_not_swallowed_by_search_or_inline_confirmations() {
        for context in [
            "chat_search",
            "clear_confirmation",
            "paste_confirmation",
            "model_confirmation",
            "tool_confirmation",
            "palette",
        ] {
            let mut state = ChatState::test_fixture();
            match context {
                "chat_search" => state.chat_search.active = true,
                "clear_confirmation" => state.show_clear_confirmation = true,
                "paste_confirmation" => state.show_paste_confirmation = true,
                "model_confirmation" => state.pending_model_confirmation = true,
                "tool_confirmation" => state.is_awaiting_confirmation = true,
                "palette" => state.push_modal(crate::ui::state::modal::Modal::Palette),
                _ => unreachable!(),
            }
            let (tx, _rx) = mpsc::channel(4);
            for _ in 0..2 {
                handle_key_event(
                    &mut state,
                    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                    &tx,
                    None,
                )
                .await
                .unwrap();
            }
            assert!(state.should_exit, "Ctrl+C must remain global in {context}");
        }
    }

    /// 取消流式输出的宽限期内，用户输入新需求不应被排队。
    ///
    /// 问题：Esc 取消后 is_streaming 仍保留 1.5 秒（用于 UI 动画收尾），导致
    /// enqueue_user_message 的保护拦住下一条输入 → 进入 pending_user_messages
    /// → 用户按一次 Enter 看起来没反应，要等宽限期过或再按一次才发出去。
    ///
    /// 修复：enqueue_user_message 判断「真正在流式中」时排除取消宽限期。
    #[tokio::test]
    async fn enter_during_cancel_grace_period_does_not_queue() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);

        state.is_streaming = true;
        state.is_processing = true;
        state
            .chat_history
            .push(crate::types::ChatEntry::user("first"));
        state
            .chat_history
            .push(crate::types::ChatEntry::assistant("").with_streaming(true));

        // 按 Esc 取消
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            state.is_streaming,
            "宽限期内 is_streaming 仍是 true（用于动画）"
        );
        assert_eq!(state.is_processing, false);
        assert!(state.cancelling_since.is_some());
        let _ = rx.try_recv(); // 吃掉 Abort 请求

        // 输入新需求 + 回车
        state.textarea.insert_str("second");
        sync_input_from_textarea(&mut state);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();

        // 验证：应该立即发送，不应进排队
        assert_eq!(
            state.pending_user_messages.len(),
            0,
            "取消宽限期内的输入应立即发送，不应排队"
        );
        let sent = rx.try_recv().expect("应该收到 SendMessage 请求");
        assert!(
            matches!(sent, AgentRequest::SendMessage { .. }),
            "应该立即发送，不是排队"
        );
    }

    /// 对话内搜索：命中行号必须与 `render_chat_lines` 的输出一致。
    ///
    /// 这一致性是全部功能的前提 —— 跳到"第 N 行"而高亮画在第 M 行，用户就再也
    /// 找不到那句话了。所以这里用真实渲染结果当 haystack，不自己拼一份假数据。
    #[test]
    fn chat_search_matches_rendered_lines_and_jumps_there() {
        let mut state = ChatState::test_fixture();
        state.chat_history = vec![
            crate::types::ChatEntry::user("cargo test --lib"),
            crate::types::ChatEntry::assistant("跑一遍测试"),
            crate::types::ChatEntry::user("再跑一次 cargo test"),
        ];
        state.rendered_cache.clear();

        // 走真实渲染，让缓存与行高与线上一致
        let width = 80u16;
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, width);
        assert!(!lines.is_empty());
        state.total_rendered_lines = lines.len();
        state.last_chat_height = 5;

        open_chat_search(&mut state);
        for c in "cargo".chars() {
            assert!(handle_chat_search_key(
                &mut state,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            ));
        }
        assert!(!state.chat_search.matches.is_empty(), "应当命中");

        let first = state.chat_search.current_match().expect("first match");
        let first_text: String = lines[first]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            first_text.to_lowercase().contains("cargo"),
            "跳到的行必须真的含有查询词，实际 {first_text:?}"
        );

        // Enter 换到下一处命中，行号必须真的变大（升序）
        assert!(handle_chat_search_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        ));
        let second = state.chat_search.current_match().expect("second match");
        assert!(second > first, "第二个命中应排在更后面");
        let second_text: String = lines[second]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(second_text.to_lowercase().contains("cargo"));

        // 无匹配时要如实报告 0，而不是沿用上一次的命中
        state.chat_search.query = "zzz-not-present".to_string();
        state.chat_search.run(&chat_search_haystack(&state));
        assert!(state.chat_search.matches.is_empty());
        assert_eq!(state.chat_search.current_match(), None);

        // 面板吞掉所有键：漏一个就会表现为"打一个字进了主输入框"
        for code in [
            KeyCode::Char('q'),
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Tab,
            KeyCode::PageUp,
        ] {
            assert!(
                handle_chat_search_key(&mut state, KeyEvent::new(code, KeyModifiers::NONE)),
                "{code:?} 应被面板吞掉"
            );
        }
        // Ctrl+Shift+F 放行给全局文件搜索（先收面板），Ctrl+F 只收面板
        assert!(!handle_chat_search_key(
            &mut state,
            KeyEvent::new(
                KeyCode::Char('f'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ));
        assert!(
            !state.chat_search.active,
            "放行之前要先收掉面板，否则全局搜索叠在上面"
        );

        // Esc 关闭并还原打开前的滚动位置
        state.scroll = 42;
        state.chat_search.active = true;
        state.chat_search.restore_scroll = 42;
        assert!(handle_chat_search_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));
        assert!(!state.chat_search.active);
        assert_eq!(state.chat_search.restore_scroll, 42);
    }

    /// 注册表里的每个绑定都必须有实现。
    ///
    /// 这条测试是「键位注册表」这件事成立的前提：新增一行 `GLOBAL_BINDINGS` 却忘了
    /// 在 [`dispatch_global_binding`] 里加 arm 时，帮助文案会列出这个键，按下去却
    /// 没有任何反应 —— 改动前没有任何测试能发现这种漂移（`Ctrl+R` 与 `Shift+Tab`
    /// 各漂过一次）。
    ///
    /// 刻意留在分发器之外的只有 `Ctrl+V` 与 `F1`（见分发器文档：前者必须排在
    /// `handle_input_modal` 之后，后者仍由主 `match` 处理）。
    #[tokio::test]
    async fn every_registered_binding_is_either_dispatched_or_documented_as_deferred() {
        use crate::ui::events::keymap::GLOBAL_BINDINGS;

        for entry in GLOBAL_BINDINGS {
            let mut state = ChatState::new();
            let (tx, _rx) = mpsc::channel(16);
            let handled = dispatch_global_binding(
                &mut state,
                entry.binding,
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                &tx,
            )
            .await
            .expect("dispatch must not fail");
            let deferred = matches!(
                entry.binding,
                GlobalBinding::Paste | GlobalBinding::ToggleHelp
            );
            assert!(
                handled || deferred,
                "{:?} 既没被分发也没被登记为「留给后面的层」",
                entry.binding
            );
        }
    }

    /// `Ctrl+P` 走注册表分发：开 → 关，两次都真的落到 palette 上。
    #[tokio::test]
    async fn ctrl_p_toggles_palette_through_the_registry() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);

        handle_key_event(&mut state, ctrl_p, &tx, None)
            .await
            .unwrap();
        assert!(state.is_palette_open(), "Ctrl+P 应当打开命令面板");

        handle_key_event(&mut state, ctrl_p, &tx, None)
            .await
            .unwrap();
        assert!(!state.is_palette_open(), "再按一次 Ctrl+P 应当关闭");
    }

    // ══════════════════════════════════════════════════════════════════════
    // Tab 键：切换视口下的工具块
    // ══════════════════════════════════════════════════════════════════════

    fn tool_block_state() -> ChatState {
        let mut state = ChatState::new();
        let tool_call = crate::types::StarToolCall {
            id: "t1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "Read".to_string(),
                arguments: r#"{"file_path":"app/main.py"}"#.to_string(),
            },
        };
        let tool_result = crate::types::ToolResult {
            success: true,
            output: Some("line A\nline B\nline C".to_string()),
            error: None,
            data: None,
            llm_content: None,
        };
        state.chat_history = vec![
            crate::types::ChatEntry::user("看看文件"),
            crate::types::ChatEntry::assistant("好"),
            crate::types::ChatEntry::tool_call("", tool_call.clone()),
            crate::types::ChatEntry::tool_result("", tool_call, tool_result),
        ];
        state
    }

    /// 回归：视口落在**工具体内部**时，Tab 也必须命中那一块。
    ///
    /// 旧实现只认工具头那条 entry（ToolCall）。展开后的正文能有两百行，
    /// 视口中心落在正文里是常态 —— 那时 Tab 匹配不上任何东西，
    /// 用户看到的是"按了没反应"。
    #[test]
    fn tab_finds_the_tool_block_when_the_viewport_is_inside_the_body() {
        let mut state = tool_block_state();
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        state.total_rendered_lines = lines.len();
        state.last_chat_height = 4;

        // 视口中心落在工具体那条 entry 上（工具头下面那条）
        let body_entry = 3usize;
        let body_top: usize = state.last_item_heights[..body_entry]
            .iter()
            .map(|h| *h as usize)
            .sum();
        state.scroll = body_top;
        state.auto_follow = false;

        assert_eq!(
            find_focused_tool_entry_for_test(&state),
            Some(2),
            "视口在正文里时，Tab 必须命中这一块的工具头"
        );
    }

    /// 回归：Tab 切的是**工具头**那条 entry。
    ///
    /// 工具头与工具体共用同一个 `tool_call.id`，折叠位是同一份 ——
    /// 但视口锚定以头为准，切头那条才能保证"同一块只切一次"。
    #[test]
    fn tab_targets_the_tool_header_of_the_block() {
        let mut state = tool_block_state();
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        state.total_rendered_lines = lines.len();
        state.last_chat_height = 24;
        state.auto_follow = false;

        assert_eq!(find_focused_tool_entry_for_test(&state), Some(2));
    }

    /// 回归：Tab 真的能切换展开/折叠。
    #[tokio::test]
    async fn tab_toggles_the_expansion_of_the_block_under_the_viewport() {
        let mut state = tool_block_state();
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        state.total_rendered_lines = lines.len();
        state.last_chat_height = 24;
        state.auto_follow = false;
        let (tx, _rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            state.expanded_tool_call_ids.contains("t1"),
            "Tab 必须能展开视口下的那一块"
        );

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            !state.expanded_tool_call_ids.contains("t1"),
            "再按一次 Tab 必须能收起"
        );
    }

    /// 回归：视口里没有工具块时，Tab 什么都不做。
    ///
    /// 去切一个屏幕外的块只会把视口拽到别处 —— 比"按了没反应"更糟。
    #[tokio::test]
    async fn tab_does_nothing_when_no_tool_block_is_visible() {
        let mut state = tool_block_state();
        // 工具块后面再补一条助手消息，让文档末尾不是工具块
        state
            .chat_history
            .push(crate::types::ChatEntry::assistant("好了"));
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        state.total_rendered_lines = lines.len();
        // 视口只有 2 行，滚到文档最末尾 —— 那里只有助手消息，没有工具块
        state.last_chat_height = 2;
        state.scroll = state.total_rendered_lines.saturating_sub(2);
        state.auto_follow = false;
        let (tx, _rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            state.expanded_tool_call_ids.is_empty(),
            "视口里没有工具块时，Tab 不得去切屏幕外的块"
        );
    }

    /// 回归：TodoWrite 的任务清单不可折叠 —— Tab 按它什么也不发生。
    #[tokio::test]
    async fn tab_does_nothing_on_a_todowrite_block() {
        let mut state = ChatState::new();
        let tool_call = crate::types::StarToolCall {
            id: "t2".to_string(),
            call_type: "function".to_string(),
            function: crate::types::StarToolCallFunction {
                name: "TodoWrite".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let tool_result = crate::types::ToolResult {
            success: true,
            output: Some("ok".to_string()),
            error: None,
            data: None,
            llm_content: None,
        };
        state.chat_history = vec![
            crate::types::ChatEntry::user("列个计划"),
            crate::types::ChatEntry::tool_call("", tool_call.clone()),
            crate::types::ChatEntry::tool_result("", tool_call, tool_result),
        ];
        let lines = crate::ui::components::chat_history::render_chat_lines(&mut state, 80);
        state.total_rendered_lines = lines.len();
        state.last_chat_height = 24;
        state.auto_follow = false;
        let (tx, _rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(
            !state.expanded_tool_call_ids.contains("t2"),
            "TodoWrite 不得被折叠/展开"
        );
    }

    /// 回归：已删除 provider 的残留缓存桶不得劫持模型路由。
    ///
    /// 真实事故：用户在 provider 面板删掉 `magpie`，随后去模型面板切模型 ——
    /// 切不动。日志里能看到切换请求到了 worker，但没有 `[SetModel]`。
    ///
    /// 成因链条：删 provider 时内存列表被 `clear()` 了，磁盘上 `magpie` 那个桶
    /// 却留着 → 面板选中的模型不在当前列表里 → `resolve_provider_for_model`
    /// 掉到全桶扫描 → 认领走 `magpie` → worker 以
    /// `Provider 'magpie' is not configured.` 静默拒绝。
    ///
    /// 这个测试必须真的走磁盘（`find_provider_for_model` 读文件），所以用
    /// `STAR_MODEL_CACHE_PATH` 把缓存指向临时文件 —— 否则它会去读用户真实的
    /// `~/.star/model-cache.json`，既测不到东西又会污染用户环境。
    #[test]
    fn a_removed_providers_cached_bucket_cannot_hijack_model_routing() {
        let cache =
            std::env::temp_dir().join(format!("star-resolve-provider-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&cache);
        crate::agent::model_cache::save_to_path(
            &cache,
            "magpie",
            &[crate::types::ModelInfo::new(
                "deepseek-v4.1-flash",
                "magpie",
            )],
        );
        std::env::set_var("STAR_MODEL_CACHE_PATH", &cache);

        let mut state = ChatState::new();
        // magpie 已从配置里删除；当前 provider 是 yunjunet。
        state.configured_providers = vec!["yunjunet".to_string()];
        state.current_provider_id = Some("yunjunet".to_string());
        // 刚删完 provider，内存列表被清空了 —— 这正是必现场景。
        state.model_provider_map.clear();

        assert_eq!(
            resolve_provider_for_model(&state, "deepseek-v4.1-flash").as_deref(),
            Some("yunjunet"),
            "已删除 provider 的残留桶不得把模型劫持走；应留在当前 provider 上"
        );

        std::env::remove_var("STAR_MODEL_CACHE_PATH");
        let _ = std::fs::remove_file(&cache);
    }

    /// 回归：`model_provider_map` 本身也可能指向已删除的 provider
    /// （外部工具直接改配置时它不会跟着刷新），同样不得被采信。
    ///
    /// 必须隔离磁盘缓存：`find_provider_for_model` 会读
    /// `~/.star/model-cache.json`，用户真实配置里若恰好有同名模型（哪怕挂在
    /// 另一个 provider 下），全桶扫描就会先命中它，断言随之失败 —— 那是用户
    /// 配置，不是这个测试要测的东西。上一条测试已经这么做了，这里跟上。
    #[test]
    fn a_stale_model_provider_map_entry_is_not_trusted() {
        let cache =
            std::env::temp_dir().join(format!("star-stale-map-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&cache);
        crate::agent::model_cache::save_to_path(
            &cache,
            "magpie",
            &[crate::types::ModelInfo::new(
                "deepseek-v4.1-flash",
                "magpie",
            )],
        );
        std::env::set_var("STAR_MODEL_CACHE_PATH", &cache);

        let mut state = ChatState::new();
        state.configured_providers = vec!["yunjunet".to_string()];
        state.current_provider_id = Some("yunjunet".to_string());
        state
            .model_provider_map
            .insert("deepseek-v4.1-flash".to_string(), "magpie".to_string());

        assert_eq!(
            resolve_provider_for_model(&state, "deepseek-v4.1-flash").as_deref(),
            Some("yunjunet"),
            "内存映射指向已删除 provider 时必须回退，不能照单全收"
        );

        std::env::remove_var("STAR_MODEL_CACHE_PATH");
        let _ = std::fs::remove_file(&cache);
    }

    /// 正常路径不能被这个修复误伤：还活着的 provider 必须照旧生效。
    #[test]
    fn a_live_provider_in_the_map_is_still_used() {
        let mut state = ChatState::new();
        state.configured_providers = vec!["yunjunet".to_string()];
        state.current_provider_id = Some("magpie".to_string());
        state
            .model_provider_map
            .insert("deepseek-v4.1-pro".to_string(), "yunjunet".to_string());

        assert_eq!(
            resolve_provider_for_model(&state, "deepseek-v4.1-pro").as_deref(),
            Some("yunjunet"),
            "活着的映射必须优先于当前 provider"
        );
    }

    /// 内建 provider 不在 `configured_providers` 里，但它们永远"活着" ——
    /// 否则活该被认成已死，模型永远切不到 Zen / OpenAI。
    ///
    /// 直接遍历 `ALL_PROVIDERS` 而不是点名某一个 id，免得以后新增内建 provider 时
    /// 这个测试还守着某个已不存在的 id 绿着。
    #[test]
    fn builtin_providers_count_as_live() {
        let state = ChatState::new();
        assert!(
            !crate::core::config::providers::ALL_PROVIDERS.is_empty(),
            "内建表不该是空的，否则这个测试等于没测"
        );
        for p in crate::core::config::providers::ALL_PROVIDERS {
            assert!(
                state.provider_is_live(p.id),
                "内建 provider {} 必须始终算数，不能因为没进 configured_providers 就被当成已死",
                p.id
            );
        }
    }

    /// 回归：`y` 之后发出去的切换请求必须带着确认框存下来的那个 provider，
    /// 而不是按 `y` 的那一刻重新推导出来的另一个。
    ///
    /// 确认框是有延迟的：弹出 → 用户看一眼 → 按 `y`。这段时间里磁盘缓存可能已经
    /// 变了，重新推导就可能给出一个不一样的（甚至是已删除的）provider。
    #[tokio::test]
    async fn pressing_y_reuses_the_provider_stored_with_the_confirmation() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "deepseek-v4.1-flash".to_string(),
            provider_id: Some("yunjunet".to_string()),
        });
        state.pending_model_confirmation = true;
        // 当前 provider 是另一个 —— 如果按 `y` 时重新推导，请求就会发去 magpie。
        state.current_provider_id = Some("magpie".to_string());
        state.configured_providers = vec!["yunjunet".to_string(), "magpie".to_string()];
        state
            .model_provider_map
            .insert("deepseek-v4.1-flash".to_string(), "magpie".to_string());

        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key_event(&mut state, y, &tx, None).await.unwrap();

        let sent = rx.try_recv().expect("必须发出切换请求");
        match sent {
            AgentRequest::UpdateRuntimeSettings(mutation) => {
                let selected = mutation.model.expect("请求必须带模型");
                assert_eq!(selected.model, "deepseek-v4.1-flash");
                assert_eq!(
                    selected.provider_id.as_deref(),
                    Some("yunjunet"),
                    "必须用确认框存下来的 provider，不能按 `y` 时重新推导"
                );
            }
            other => panic!("预期 UpdateRuntimeSettings，实际 {:?}", other),
        }
    }

    /// 回归：`y/n` 确认必须真的能被回答，否则整个键盘会被锁死。
    ///
    /// 真实事故：在有历史的会话里从模型面板选一个模型 → 弹出 "切换模型到 X? (y/n)"
    /// → 按 `y` **没有任何反应**，之后输入框彻底失灵，连 Esc 都退不出去（只能
    /// Ctrl+C 两次退出）。
    ///
    /// 根因：`handle_model_confirmation` 在整个 crate 里**没有调用点**。它本该像
    /// `handle_inline_confirm` 那样在分发阶梯前段被调用，但分发表里
    /// `KeyContext::ModelConfirm` 被归进了"这些上下文已经在阶梯更早处被独占处理"
    /// 那一组，只 `return Ok(())` 吞键。而 Esc 的清零逻辑排在那个分支**之后**，
    /// 于是 `pending_model_confirmation` 永远为 true。
    /// 回归：Esc 不得把用户送进"重发 → 开模型面板 → 再重发"的死循环。
    ///
    /// 真实事故：apinex 余额耗尽报错 → Esc 重发 → `current_model` 为空 →
    /// `enqueue_user_message` 的保护代为打开模型面板并再推一条 "No model
    /// selected…" → 再按 Esc → 再重发 → 再推一条 …… 屏幕上 6 行一模一样的报错，
    /// 而真正的原因（模型为什么是空的）被完全淹没。
    ///
    /// 这种状态下 Esc **不该**重发：这次重发注定撞回同一道墙。让错误浮层或模型面板
    /// 说话，比反复撞墙有用。
    #[tokio::test]
    async fn esc_does_not_resend_into_the_no_model_guard() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.current_provider_id = Some("apinex".to_string());
        state.current_model = String::new();
        state.last_turn_failed = true;
        state.chat_history.push(crate::types::ChatEntry::user("hi"));

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(
            rx.try_recv().is_err(),
            "注定被'没选模型'挡下的重发不该发出去 —— 那正是死循环的来源"
        );
        assert!(
            !state.is_palette_open(),
            "Esc 不该替用户打开模型面板；该由错误浮层说话"
        );
    }

    /// 反向：没有 provider 时，"模型为空"不代表重发没意义 —— agent 仍能自己解析
    /// 默认模型。判据必须和那道保护**逐字一致**，不能只看模型名是否为空。
    #[tokio::test]
    async fn esc_still_resends_when_there_is_no_provider_to_block_it() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.current_provider_id = None;
        state.current_model = String::new();
        state.last_turn_failed = true;
        state.chat_history.push(crate::types::ChatEntry::user("hi"));

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(
            matches!(rx.try_recv(), Ok(AgentRequest::SendMessage { .. })),
            "没有 provider 时重发仍然有意义，必须照发"
        );
    }

    #[tokio::test]
    async fn a_pending_model_confirmation_is_answered_by_pressing_y() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "some-other-model".to_string(),
            provider_id: None,
        });
        state.pending_model_confirmation = true;

        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key_event(&mut state, y, &tx, None).await.unwrap();

        assert!(
            !state.pending_model_confirmation,
            "`y` 之后确认标志必须清掉，否则键盘继续被锁"
        );
        assert!(
            state.pending_model_change.is_none(),
            "`y` 之后待切换的模型应当已经被消费"
        );
        assert!(
            matches!(rx.try_recv(), Ok(AgentRequest::UpdateRuntimeSettings(_))),
            "`y` 必须真的把模型切换请求发给 worker"
        );
    }

    /// Esc 是这个确认的逃生口：它排在 `KeyContext::of` 分发**之后**，所以标志
    /// 一旦卡住就再也走不到那段清零代码。
    #[tokio::test]
    async fn a_pending_model_confirmation_is_dismissed_by_escape() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "some-other-model".to_string(),
            provider_id: None,
        });
        state.pending_model_confirmation = true;

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(!state.pending_model_confirmation, "Esc 必须能取消这次切换");
        assert!(state.pending_model_change.is_none());
        assert!(rx.try_recv().is_err(), "取消不应该发出任何切换请求");
    }

    /// 完整复现用户报告的症状：切完模型之后输入框还能不能打字。
    ///
    /// 修复前这条会挂 —— `y` 被吞掉，`pending_model_confirmation` 留在 true，
    /// 后面每一个字符都落进 `ModelConfirm => return Ok(())`。
    #[tokio::test]
    async fn the_keyboard_still_works_after_a_model_switch() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);

        // 面板选中模型：有历史 → 走 y/n 确认
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "some-other-model".to_string(),
            provider_id: None,
        });
        state.pending_model_confirmation = true;

        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key_event(&mut state, y, &tx, None).await.unwrap();

        // 确认之后随便打一个字符，必须真的进输入框
        let h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        handle_key_event(&mut state, h, &tx, None).await.unwrap();
        assert_eq!(
            state.input, "h",
            "模型确认处理完之后输入框必须恢复可用，而不是继续吞键"
        );
    }

    /// 防御：确认框开着却没有待切换的模型（状态不一致）时，按 `y` 也必须把标志
    /// 清掉。留着 true 就等于把输入框永久锁死，且没有任何键能解开。
    #[tokio::test]
    async fn a_model_confirmation_with_nothing_pending_still_clears_itself() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.pending_model_confirmation = true;
        // 刻意不设 pending_model_change

        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key_event(&mut state, y, &tx, None).await.unwrap();

        assert!(
            !state.pending_model_confirmation,
            "没有待切换模型时也必须解除确认，否则键盘永久锁死"
        );
        assert!(rx.try_recv().is_err());

        let h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        handle_key_event(&mut state, h, &tx, None).await.unwrap();
        assert_eq!(state.input, "h");
    }

    /// 确认开着的时候，其他键必须被吞掉 —— 换模型是破坏性的，不能被顺手敲的
    /// 字符顶掉（这是设计意图，锁住它以免有人"顺手"放行）。
    #[tokio::test]
    async fn unrelated_keys_are_swallowed_while_a_model_confirmation_is_pending() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "some-other-model".to_string(),
            provider_id: None,
        });
        state.pending_model_confirmation = true;

        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        handle_key_event(&mut state, q, &tx, None).await.unwrap();

        assert!(state.input.is_empty(), "确认期间别的字符不能进输入框");
        assert!(state.pending_model_confirmation, "也不能被无关键清掉");
        assert!(rx.try_recv().is_err());
    }

    /// `KeyContext::ModelConfirm` 必须在阶梯**前段**就被真正处理，不能只落到
    /// 分发表底部那个"只吞键"的兜底分支。
    ///
    /// `every_registered_binding_is_either_dispatched_or_documented_as_deferred`
    /// 守着全局键注册表，但没有任何测试守着 `KeyContext` —— "声明自己独占、却没人
    /// 分发"的变体就是靠这条漏出去的。修复前 `ModelConfirm` 正是后者。
    #[tokio::test]
    async fn the_model_confirm_context_is_dispatched_early_not_swallowed() {
        use crate::ui::events::key_context::KeyContext;

        let mut state = ChatState::new();
        state.pending_model_confirmation = true;
        assert_eq!(
            KeyContext::of(&state),
            KeyContext::ModelConfirm,
            "pending_model_confirmation 必须让 KeyContext 落到 ModelConfirm"
        );

        let (tx, mut rx) = mpsc::channel(16);
        let z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        handle_key_event(&mut state, z, &tx, None).await.unwrap();
        assert!(state.input.is_empty(), "无关键应当被吞掉");
        assert!(rx.try_recv().is_err(), "无关键不应当触发任何请求");
    }

    /// 切换模型时 `waive_next_model_confirmation` 的一次性豁免必须被消费。
    /// 不消费的话它会一直留着，让之后每次切模型都跳过 y/n 确认。
    #[tokio::test]
    async fn the_provider_switch_waiver_is_consumed_by_the_next_model_switch() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.waive_next_model_confirmation = true;
        state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
            model: "some-other-model".to_string(),
            provider_id: None,
        });
        state.pending_model_confirmation = true;

        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        handle_key_event(&mut state, y, &tx, None).await.unwrap();

        assert!(
            !state.waive_next_model_confirmation,
            "一次性豁免必须在下一次切换时被消费"
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentRequest::UpdateRuntimeSettings(_))
        ));
    }

    /// 守卫不满足时必须**让位**（返回 false），不能吞掉：此前 modal 层放行全局键
    /// 之后落进一个不满足的守卫，用户看到的就是「键没反应」。
    #[tokio::test]
    async fn ctrl_r_defers_while_waiting_for_confirmation() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);
        state.is_awaiting_confirmation = true;

        let handled = dispatch_global_binding(
            &mut state,
            GlobalBinding::HistorySearch,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();

        assert!(!handled, "审批中 Ctrl+R 应当让位给后续层");
        assert!(!state.is_modal_open(), "让位时不该打开历史搜索");
    }

    /// `Alt+Shift+T` 在不上报 SHIFT 的终端上表现为大写 `'T'` —— 注册表按 shift 位判定
    /// 会把它当成 `CycleThinking`，分发器必须再兜一次，否则这类终端再也切不了主题。
    #[tokio::test]
    async fn alt_shift_t_falls_back_to_uppercase_char() {
        let (tx, _rx) = mpsc::channel(16);

        let mut with_shift = ChatState::new();
        let before = with_shift.theme_manager.current().name.clone();
        dispatch_global_binding(
            &mut with_shift,
            GlobalBinding::CycleTheme,
            KeyEvent::new(KeyCode::Char('T'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            &tx,
        )
        .await
        .unwrap();
        assert_ne!(with_shift.theme_manager.current().name, before);

        let mut without_shift = ChatState::new();
        let before = without_shift.theme_manager.current().name.clone();
        let handled = dispatch_global_binding(
            &mut without_shift,
            GlobalBinding::CycleThinking,
            KeyEvent::new(KeyCode::Char('T'), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert!(handled);
        assert_ne!(
            without_shift.theme_manager.current().name,
            before,
            "大写 T（终端没上报 SHIFT）应当仍然切主题，而不是循环思考深度"
        );
    }

    /// `Ctrl+V` / `F1` 必须留在原来的处理层：输入框自己消费 `Ctrl+V`（填 API Key 靠它）。
    #[tokio::test]
    async fn paste_and_help_are_left_to_their_original_layers() {
        let (tx, _rx) = mpsc::channel(16);
        for binding in [GlobalBinding::Paste, GlobalBinding::ToggleHelp] {
            let mut state = ChatState::new();
            let handled = dispatch_global_binding(
                &mut state,
                binding,
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                &tx,
            )
            .await
            .unwrap();
            assert!(!handled, "{binding:?} 不该在这一层被消费");
        }
    }

    /// 任务面板的 modal 守卫由 `KeyContext::is_dialog()` 给出：输入框打开时面板不得
    /// 截走按键（API Key 表单里的 Ctrl+S / Alt+←→），任务面板自己开着时仍然工作。
    #[tokio::test]
    async fn task_panel_is_blocked_while_a_dialog_owns_the_keyboard() {
        let mut state = ChatState::new();
        state.task_panel.is_visible = true;
        state.enter_input_modal();
        assert!(KeyContext::of(&state).is_dialog(), "输入框必须是独占对话框");

        state.exit_input_modal();
        assert_eq!(KeyContext::of(&state), KeyContext::TaskPanel);
    }

    /// 任务面板的自动显隐由 `tick_task_panel` 推进（事件循环的 1Hz 节拍），不再靠
    /// 「画一帧就重算一次」。这条测试同时锁住新入口的行为等价性。
    #[test]
    fn task_panel_tick_shows_new_tasks_without_any_render() {
        use crate::core::tasks::models::TaskNode;

        let mut state = ChatState::new();
        state.task_panel.mark_modified();
        let node = TaskNode::new("写代码".into());
        let id = node.id.clone();
        state
            .task_panel
            .task_manager
            .graph
            .root_ids
            .push(id.clone());
        state.task_panel.task_manager.graph.nodes.insert(id, node);

        assert!(!state.task_panel.is_visible, "初始不应自动弹出");
        state.tick_task_panel();
        assert!(
            state.task_panel.is_visible,
            "TodoWrite 之后由节拍弹出面板，不依赖是否渲染过"
        );
    }

    /// 填 API Key 时全局键仍然放行：Ctrl+P 要能跳去命令面板（这是用户的逃生口），
    /// Ctrl+C 仍能中断生成。输入框的 handler 只该吃普通字符与 Enter/Esc。
    #[tokio::test]
    async fn global_keys_still_pass_through_the_input_modal() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);
        state.enter_input_modal();
        state.input_modal_title = "API Key".to_string();

        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(
            !super::super::modal_input::handle_modal_key(&mut state, ctrl_p, &tx)
                .await
                .unwrap(),
            "全局键不应被输入框消费"
        );
    }

    /// 输入框开着时，普通字符**不得**漏进主输入框。
    ///
    /// 锁的是上下文分发表：`InputModal` 那一臂只放行 `Ctrl+V`（注册表里刻意
    /// deferred 的那一个），其余键一律结束。输入框此前由阶梯中段的独立分支处理，
    /// 改成分发表时若把 `InputModal` 混进「已处理」的兜底臂，字符就会掉进主框。
    #[tokio::test]
    async fn input_modal_swallows_typing_instead_of_leaking_to_the_main_input() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);
        state.enter_input_modal();
        state.input_modal_title = "API Key".to_string();

        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();

        assert!(
            state.input.is_empty(),
            "输入框开着时普通字符不得落到主输入框（实际：{:?}）",
            state.input
        );
    }

    /// 建议浮层开着时 ↑↓ 必须作用在浮层上：分发表把 `SuggestionOverlay` 交给
    /// `handle_suggestion_overlay`，不能落进主输入框。
    #[tokio::test]
    async fn suggestion_overlay_arrows_are_handled_by_the_overlay() {
        let mut state = ChatState::new();
        let (tx, _rx) = mpsc::channel(16);
        state.show_command_hints = true;
        state.command_hints = vec!["/help".to_string(), "/clear".to_string()];
        state.selected_hint = 0;

        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        handle_key_event(&mut state, down, &tx, None).await.unwrap();

        assert_eq!(
            state.selected_hint, 1,
            "↓ 应当移动浮层选中项，而不是被主输入框吃掉"
        );
    }

    /// `resolve_provider_for_model` 的可测两步：UI 映射优先，其次当前 provider。
    ///
    /// 跨桶查找那一步要读磁盘缓存，`ui` 这层不注入路径，所以它的行为由
    /// `agent::model_cache::tests::finds_the_owning_provider_across_buckets_not_just_the_current_one`
    /// 逐条覆盖；这里只锁定"查得到就听映射、查不到才兜底"这个顺序不被改回去。
    #[test]
    fn provider_resolution_prefers_the_map_then_falls_back_to_the_current_provider() {
        let mut state = ChatState::new();
        state.current_provider_id = Some("opencode".to_string());
        state
            .model_provider_map
            .insert("mimo-v2.6-flash-free".to_string(), "opencode".to_string());

        // 映射命中时，行为不变。
        assert_eq!(
            resolve_provider_for_model(&state, "mimo-v2.6-flash-free").as_deref(),
            Some("opencode")
        );
        // 谁都没有的名字才兜底到当前 provider（手敲自定义模型名的既有行为）。
        assert_eq!(
            resolve_provider_for_model(&state, "some-unlisted-model").as_deref(),
            Some("opencode")
        );
        // 连当前 provider 都没有时是 None，交给下游按"沿用"处理。
        let bare = ChatState::new();
        assert_eq!(resolve_provider_for_model(&bare, "any-model"), None);
    }

    /// 失败回合之后 Esc 必须重发上一条，而不是静默 no-op。
    ///
    /// 真实场景：provider 配额耗尽 → 用户去 `/provider` 换 key → 回到输入框按
    /// Esc。此前 Esc 落到 `else { state.last_esc_at = None; }`，什么都不发生。
    #[tokio::test]
    async fn esc_retries_the_last_message_after_a_failed_turn() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.chat_history.push(crate::types::ChatEntry::user("hi"));
        state.last_turn_failed = true;

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(
            !state.last_turn_failed,
            "重发已经发起，失败标记就该清掉，否则连按两次 Esc 会重发两条"
        );
        let sent = rx.try_recv().expect("Esc 应当把上一条消息重新发给 worker");
        assert!(
            matches!(sent, AgentRequest::SendMessage { .. }),
            "重发走的是正常的 SendMessage 通道，不是某个旁路"
        );
    }

    /// 正常收尾的会话里按 Esc 不能凭空多出一条消息。
    #[tokio::test]
    async fn esc_does_not_resend_when_the_last_turn_succeeded() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.chat_history.push(crate::types::ChatEntry::user("hi"));
        state.last_turn_failed = false;

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(
            rx.try_recv().is_err(),
            "没有失败回合时 Esc 不该发出任何请求"
        );
    }

    /// 失败回合之前根本没有用户消息（恢复出来的半截会话）时，Esc 必须把标志
    /// 清掉，否则它会永远卡在这个分支里，后面再也走不到正常逻辑。
    #[tokio::test]
    async fn esc_clears_the_failure_flag_when_there_is_nothing_to_resend() {
        let mut state = ChatState::new();
        let (tx, mut rx) = mpsc::channel(16);
        state.last_turn_failed = true;

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        handle_key_event(&mut state, esc, &tx, None).await.unwrap();

        assert!(!state.last_turn_failed);
        assert!(rx.try_recv().is_err());
    }

    fn bg_task(id: &str) -> AgentTaskInfo {
        AgentTaskInfo {
            task_id: id.to_string(),
            agent_type: "Explore".to_string(),
            description: "研究 src/hooks 目录".to_string(),
            status: crate::types::AgentTaskStatus::Running,
            tool_use_count: 3,
            tokens: 1200,
            is_async: true,
            is_resolved: false,
            is_error: false,
            last_tool_info: None,
            name: None,
            task_description: None,
            started_at: Instant::now(),
            finished_at: None,
            sub_entries: Vec::new(),
            entry_idx: 0,
        }
    }

    fn press(state: &mut ChatState, code: KeyCode) -> bool {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        handle_bg_agent_selector_key(state, key)
    }

    /// ↑/↓ 在 main 行与各代理行之间移动；在 main 行继续 ↑ 把焦点交回输入框。
    #[test]
    fn selector_keys_move_focus_and_release_to_input() {
        let mut state = ChatState::new();
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a"));
        state
            .active_agent_tasks
            .insert("b".to_string(), bg_task("b"));
        state.bg_agent_selection = Some(0);

        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(1));
        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(2));
        // 到底了不再往下
        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(state.bg_agent_selection, Some(2));

        for expected in [Some(1), Some(0), None] {
            assert!(press(&mut state, KeyCode::Up));
            assert_eq!(state.bg_agent_selection, expected);
        }
    }

    /// Enter 在「看主会话」与「看某个代理」之间切换，且不移动焦点行；
    /// Esc 只收起选择器，不改变正在看的详情。
    #[test]
    fn selector_enter_toggles_view_and_esc_keeps_it() {
        let mut state = ChatState::new();
        state
            .active_agent_tasks
            .insert("a".to_string(), bg_task("a"));
        state.bg_agent_selection = Some(1);

        assert!(press(&mut state, KeyCode::Enter));
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
        assert_eq!(state.bg_agent_selection, Some(1));

        // 同一行再按 Enter → 收回主会话
        assert!(press(&mut state, KeyCode::Enter));
        assert!(state.viewing_agent_task_id.is_none());

        // 进详情后按 Esc：选择器收起，详情保留（详情自己的 Esc 在主 Esc 分支里处理）
        assert!(press(&mut state, KeyCode::Enter));
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
        assert!(press(&mut state, KeyCode::Esc));
        assert!(state.bg_agent_selection.is_none());
        assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn escape_from_running_agent_detail_returns_without_aborting() {
        let mut state = ChatState::test_fixture();
        state.active_agent_tasks.insert("a".into(), bg_task("a"));
        state.viewing_agent_task_id = Some("a".into());
        state.is_processing = true;
        state.is_streaming = true;
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert!(state.viewing_agent_task_id.is_none());
        assert!(state.is_processing);
        assert!(state.is_streaming);
        assert!(rx.try_recv().is_err(), "returning must not send Abort");
    }

    /// 没有后台代理时选择器不该抢键，并顺手把残留焦点清掉。
    #[test]
    fn selector_yields_when_no_background_agents() {
        let mut state = ChatState::new();
        state.bg_agent_selection = Some(0);
        assert!(!press(&mut state, KeyCode::Down));
        assert!(state.bg_agent_selection.is_none());
    }

    /// Tab/↑↓ 在五个字段和底部操作行之间走：末字段往下进操作行，操作行往下
    /// 循环回 Type；Type 顶头往上不动，操作行往上回末字段。
    #[tokio::test]
    async fn provider_form_navigation_cycles_through_fields_and_action_row() {
        let mut state = ChatState::new();
        open_provider_form(&mut state, None).await;
        assert_eq!(state.provider_form.active_field, 0);

        for expected in 1..=4 {
            switch_provider_form_field(&mut state, true);
            assert_eq!(state.provider_form.active_field, expected);
            assert!(!state.provider_form.on_actions);
        }
        // 末字段再往下：进操作行（默认选中 Save）
        switch_provider_form_field(&mut state, true);
        assert!(state.provider_form.on_actions);
        assert_eq!(state.provider_form.action_index, 1);
        // 操作行再往下：循环回 Type
        switch_provider_form_field(&mut state, true);
        assert_eq!(state.provider_form.active_field, 0);
        assert!(!state.provider_form.on_actions);

        // Type 顶头往上：不动
        switch_provider_form_field(&mut state, false);
        assert_eq!(state.provider_form.active_field, 0);

        // 操作行往上：回末字段
        state.provider_form.on_actions = true;
        switch_provider_form_field(&mut state, false);
        assert!(!state.provider_form.on_actions);
        assert_eq!(state.provider_form.active_field, 4);
    }

    /// 离开字段时 textarea 里的内容得存进 values——包括切到操作行的那一步
    /// （Model 的值就是这时候存走的），否则填了也白填。
    #[tokio::test]
    async fn provider_form_saves_field_value_when_moving_on() {
        let mut state = ChatState::new();
        open_provider_form(&mut state, None).await;
        switch_provider_form_field(&mut state, true); // → Name
        state.modal_textarea.insert_str("My Provider");
        switch_provider_form_field(&mut state, true); // → Base URL
        assert_eq!(state.provider_form.values[1], "My Provider");

        // 直接站到 Model 上填值，然后进操作行
        state.provider_form.active_field = 4;
        state.modal_textarea = TextArea::default();
        state.modal_textarea.insert_str("gpt-5");
        switch_provider_form_field(&mut state, true);
        assert!(state.provider_form.on_actions);
        assert_eq!(state.provider_form.values[4], "gpt-5");
    }

    /// 提交失败（Base URL 为空）时表单必须留在原地：模态不关、context 不丢。
    /// context 一旦被拿走没还回来，`is_provider_form_active` 变 false，多字段
    /// 面板就退化成通用单字段输入框（还带着别的弹窗残留的 title）。
    #[tokio::test]
    async fn enter_on_incomplete_form_keeps_the_panel_rendered() {
        let mut state = ChatState::new();
        open_provider_form(&mut state, None).await;
        switch_provider_form_field(&mut state, true); // → Name，Base URL 留空
        let (agent_tx, _rx) = mpsc::channel(1);

        handle_input_modal(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
        )
        .await
        .expect("enter should be handled");

        assert!(state.is_input_modal_open(), "modal must stay open");
        assert!(matches!(
            state.input_context,
            Some(crate::ui::state::palette::InputContext::ProviderForm)
        ));
        assert_eq!(
            state.provider_form.error.as_deref(),
            Some("Base URL is required.")
        );
    }

    /// 操作行上 ←/→ 在 Back / Save 之间选，Enter 走选中项：Back 取消回上一层，
    /// 不碰文件系统、不发请求。
    #[tokio::test]
    async fn action_row_enter_runs_the_selected_option() {
        let mut state = ChatState::new();
        open_provider_form(&mut state, None).await;
        for _ in 0..5 {
            switch_provider_form_field(&mut state, true);
        }
        assert!(state.provider_form.on_actions);
        assert_eq!(state.provider_form.action_index, 1);

        let (agent_tx, _rx) = mpsc::channel(1);
        // ← 选到 Back
        handle_input_modal(
            &mut state,
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            &agent_tx,
        )
        .await
        .expect("left should choose Back");
        assert_eq!(state.provider_form.action_index, 0);
        // 回车：取消，模态关、context 清空
        handle_input_modal(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
        )
        .await
        .expect("enter should cancel");
        assert!(!state.is_input_modal_open());
        assert!(state.input_context.is_none());
    }

    /// Session Browser 选择项必须经统一命令入口排队，不能发送已断开的 ResumeSession 请求。
    #[tokio::test]
    async fn log_selector_queues_resume_command_while_streaming() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.open_log_selector();
        state.log_selector_state.set_sessions(vec![
            crate::ui::components::log_selector::LogSessionEntry {
                id: "saved-session".to_string(),
                title: "Saved".to_string(),
                created_at: "today".to_string(),
                message_count: 1,
                preview: "preview".to_string(),
            },
        ]);
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
            None,
        )
        .await
        .expect("selector Enter should be handled");

        assert_ne!(
            state.top_modal(),
            Some(&crate::ui::state::modal::Modal::LogSelector)
        );
        assert_eq!(
            state
                .pending_user_messages
                .iter()
                .map(|input| input.text.as_str())
                .collect::<Vec<_>>(),
            vec!["/chat resume saved-session"]
        );
        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn readline_kill_keys_use_character_columns_for_unicode_text() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("你abc好");
        let (tx, _rx) = mpsc::channel(16);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "abc好");

        state.set_input_text("你abc好");
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "你");

        state.set_input_text("甲 乙");
        for _ in 0..4 {
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                &tx,
                None,
            )
            .await
            .unwrap();
        }
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "甲 ");
    }

    #[tokio::test]
    async fn recalling_queue_exits_history_without_losing_recalled_input() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("original draft");
        state.command_history = [crate::types::UserInput::from("previous question")].into();
        let (tx, mut rx) = mpsc::channel(16);
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "previous question");
        assert_eq!(state.history_index, Some(0));

        state
            .pending_user_messages
            .push_back(crate::types::UserInput::from("queued"));
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "queued\nprevious question");
        assert!(state.pending_user_messages.is_empty());
        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            state.input, "queued\nprevious question",
            "Down 不得用旧历史草稿覆盖取回的队列"
        );
        assert_eq!(state.textarea.lines().join("\n"), state.input);
        assert_eq!(state.input_line_count, 2);
        assert!(state.history_index.is_none());
        assert!(state.history_input_snapshot.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_stashes_and_restores_the_prompt_without_submitting() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("first draft line\nsecond draft line");
        state.textarea.move_cursor(tui_textarea::CursorMove::Back);
        let cursor = state.textarea.cursor();
        let (tx, mut rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, "", "Ctrl+S 应暂存而不是忽略当前草稿");
        assert_eq!(state.textarea.lines().join("\n"), "");
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, "first draft line\nsecond draft line");
        assert_eq!(state.textarea.cursor(), cursor);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_swaps_drafts_and_preserves_literal_image_input() {
        let mut state = ChatState::test_fixture();
        state.set_user_input(crate::types::UserInput {
            text: "/literal [Image]".into(),
            intent: crate::types::InputIntent::Literal,
            images: vec![crate::types::ImageAttachment {
                media_type: "image/png".into(),
                data: "fixture-image-data".into(),
                width: 1,
                height: 1,
            }],
        });
        let before = state.prepared_input().unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.paste_segments.is_empty());
        state.set_input_text("temporary question");
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        let restored = state.prepared_input().unwrap();
        assert_eq!(restored.text, before.text);
        assert_eq!(restored.intent, before.intent);
        assert_eq!(restored.images[0].data, before.images[0].data);
        assert_eq!(state.paste_segments.len(), 1);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, "temporary question");
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.prepared_input().unwrap().text, before.text);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_does_not_stash_through_modal_or_confirmation() {
        let (tx, mut rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        for mode in 0..4 {
            let mut state = ChatState::test_fixture();
            state.set_input_text("private draft");
            match mode {
                0 => state.open_palette(PaletteMode::Main),
                1 => {
                    state.show_help = true;
                }
                2 => {
                    state.is_awaiting_confirmation = true;
                }
                _ => {
                    state.show_paste_confirmation = true;
                }
            }
            handle_key_event(&mut state, key, &tx, None).await.unwrap();
            assert_eq!(state.input, "private draft", "context {mode}");
            assert!(state.stashed_prompt.is_none());
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_handles_empty_prompt_and_exits_history_on_stash() {
        let mut state = ChatState::test_fixture();
        let (tx, mut rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.stashed_prompt.is_none());
        assert!(state.toast_queue.is_empty());
        state.set_input_text("/resum");
        crate::ui::services::suggestions::on_input_changed(&mut state);
        assert!(state.show_command_hints);
        state.history_index = Some(0);
        state.history_input_snapshot = Some(crate::types::UserInput::from("old draft"));
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.input.is_empty());
        assert!(!state.show_command_hints);
        assert!(state.history_index.is_none());
        assert!(state.history_input_snapshot.is_none());
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, "/resum");
        assert!(state.show_command_hints);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_keeps_unreadable_image_reference_and_cancels_late_clipboard() {
        let mut state = ChatState::test_fixture();
        insert_image_paste_block(&mut state, "/missing/stashed-image.png".into(), 10, 10);
        state.input_intent = crate::types::InputIntent::Literal;
        sync_input_from_textarea(&mut state);
        let before = state.input.clone();
        let (sender, result) = tokio::sync::oneshot::channel();
        state.pending_clipboard_read = Some(crate::ui::state::PendingClipboardRead {
            target: crate::ui::state::ClipboardTarget::Prompt {
                session_id: state.active_session_id.clone(),
                session_started_at: state.session_started_at,
                next_message_id: state.next_message_id,
            },
            result,
        });
        let (tx, mut rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert!(state.input.is_empty());
        assert!(state.pending_clipboard_read.is_none());
        assert!(sender
            .send(Ok(crate::ui::state::ClipboardPayload::Text("late".into())))
            .is_err());
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, before);
        assert_eq!(state.paste_segments.len(), 1);
        assert_eq!(state.input_intent, crate::types::InputIntent::Literal);
        assert!(
            state.prepared_input().is_err(),
            "失效附件仍由发送校验，暂存不得删除引用"
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ctrl_s_restores_editor_undo_history() {
        let mut state = ChatState::test_fixture();
        state.set_input_text("draft");
        state.textarea.insert_str(" changed");
        sync_input_from_textarea(&mut state);
        let (tx, _rx) = mpsc::channel(16);
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        handle_key_event(&mut state, key, &tx, None).await.unwrap();
        assert_eq!(state.input, "draft changed");
        assert!(state.textarea.undo());
        sync_input_from_textarea(&mut state);
        assert_eq!(state.input, "draft");
    }

    /// 排队的输入按入队顺序拼回输入框，队列随之清空。
    #[test]
    fn restoring_the_queue_joins_messages_in_order() {
        let mut state = ChatState::new();
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::from("first"));
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::from("second"));

        assert!(restore_queued_messages_to_input(&mut state));
        assert_eq!(state.input, "first\nsecond");
        assert!(state.pending_user_messages.is_empty());
    }

    /// 输入框里已经敲了一半的内容不能被队列覆盖：它接在队列后面。
    #[test]
    fn restoring_the_queue_keeps_the_half_typed_draft() {
        let mut state = ChatState::new();
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::from("queued"));
        state.textarea.insert_str("half typed");
        sync_input_from_textarea(&mut state);

        assert!(restore_queued_messages_to_input(&mut state));
        assert_eq!(state.input, "queued\nhalf typed");
    }

    /// 队列为空时是 no-op，输入框内容不动（否则 ↑ 会莫名清掉草稿）。
    #[test]
    fn restoring_an_empty_queue_is_a_noop() {
        let mut state = ChatState::new();
        state.textarea.insert_str("draft");
        sync_input_from_textarea(&mut state);

        assert!(!restore_queued_messages_to_input(&mut state));
        assert_eq!(state.input, "draft");
    }

    /// 会话运行中输入的需求必须**插进当前这一轮**，不能被丢掉、也不能排到
    /// 本轮结束才发。
    #[tokio::test]
    async fn requirement_typed_while_streaming_is_steered_into_the_running_turn() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.is_processing = true;
        state.active_message_id = Some(7);

        let (agent_tx, mut agent_rx) = mpsc::channel(4);

        for ch in "fix the bug".chars() {
            handle_key_event(
                &mut state,
                KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                &agent_tx,
                None,
            )
            .await
            .expect("typing should be handled");
        }

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent_tx,
            None,
        )
        .await
        .expect("enter should be handled");

        assert!(
            state.pending_user_messages.is_empty(),
            "需求不该再排队 —— 排队要等本轮跑完才发"
        );
        assert!(
            state
                .chat_history
                .iter()
                .any(|e| e.content == "fix the bug"),
            "需求要出现在聊天区里"
        );
        match agent_rx.try_recv() {
            Ok(AgentRequest::SendMessage { message, .. }) => {
                assert_eq!(message, "fix the bug", "发出去的必须是这条需求");
            }
            other => panic!("应当发出 SendMessage，实际是 {:?}", other),
        }
    }

    /// 运行中按 Esc：发出 Abort，并且**不清掉输入框里已经敲了一半的内容**。
    #[tokio::test]
    async fn esc_while_streaming_aborts_and_keeps_the_draft() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.is_processing = true;
        state.textarea.insert_str("half typed");
        sync_input_from_textarea(&mut state);

        let (agent_tx, mut agent_rx) = mpsc::channel(4);

        handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &agent_tx,
            None,
        )
        .await
        .expect("esc should be handled");

        assert!(
            matches!(agent_rx.try_recv(), Ok(AgentRequest::Abort)),
            "Esc 应发出 Abort"
        );
        assert_eq!(state.input, "half typed", "Esc 不该清掉输入框内容");
    }

    #[tokio::test]
    async fn session_only_runtime_requests_keep_revisions_without_persistence_state() {
        let mut state = ChatState::new();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let revision = request_runtime_settings_with_persistence_revision(
            &mut state,
            Some(crate::runtime::messages::RuntimeModelSelection {
                model: "fast-model".to_string(),
                provider_id: Some("provider-a".to_string()),
            }),
            None,
            None,
            crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
            &agent_tx,
        )
        .await;

        assert_eq!(revision, Some(1));
        assert!(state.runtime_settings_persistence.is_none());
        match agent_rx.recv().await {
            Some(AgentRequest::UpdateRuntimeSettings(mutation)) => {
                assert_eq!(mutation.ui_revision, 1);
                assert_eq!(
                    mutation.persistence,
                    crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly
                );
            }
            request => panic!("unexpected request: {request:?}"),
        }

        let (closed_tx, closed_rx) = mpsc::channel(1);
        drop(closed_rx);
        assert_eq!(
            request_runtime_settings_with_persistence_revision(
                &mut state,
                None,
                None,
                Some(crate::types::ThinkingEffort::High),
                crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
                &closed_tx,
            )
            .await,
            None
        );
        assert_eq!(state.pending_runtime_settings.len(), 1);
        assert_eq!(
            state
                .requested_runtime_settings
                .as_ref()
                .unwrap()
                .model
                .model,
            "fast-model"
        );
        assert!(state.runtime_settings_persistence.is_none());
    }

    #[tokio::test]
    async fn confirming_clear_saves_the_label_then_resets_identity() {
        let mut state = ChatState::test_fixture();
        state.chat_history = vec![crate::types::ChatEntry::user("previous turn")];
        state.show_clear_confirmation = true;
        state.pending_clear_name = Some(format!(
            "clear-checkpoint-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        state.active_session_id = Some("active-session".to_string());
        state.current_session_title = Some("Active".to_string());
        state.token_usage = Some(crate::types::StarUsage {
            total_tokens: 17,
            ..Default::default()
        });

        let (agent_tx, mut agent_rx) = mpsc::channel(4);
        let worker = tokio::spawn(async move {
            match agent_rx.recv().await.expect("save request") {
                AgentRequest::SaveSession {
                    id,
                    history,
                    last_usage,
                    response,
                } => {
                    assert!(id.starts_with("clear-checkpoint-"), "got {id}");
                    assert_eq!(history.len(), 1);
                    assert_eq!(last_usage.map(|usage| usage.total_tokens), Some(17));
                    response.send(Ok(())).await.unwrap();
                }
                request => panic!("expected SaveSession, got {request:?}"),
            }
            assert!(matches!(
                agent_rx.recv().await.expect("reset request"),
                AgentRequest::ResetSession
            ));
        });

        handle_inline_confirm(
            &mut state,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            &agent_tx,
        )
        .await
        .unwrap();

        assert!(!state.show_clear_confirmation);
        assert!(state.chat_history.is_empty());
        assert_eq!(state.active_session_id, None);
        assert_eq!(state.current_session_title, None);
        assert_eq!(state.pending_clear_name, None);
        assert!(state.token_usage.is_none());
        assert!(state
            .current_status_line
            .as_deref()
            .unwrap()
            .contains("Previous saved as"));
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_clear_keeps_history_and_drops_the_pending_label() {
        let mut state = ChatState::test_fixture();
        state.chat_history = vec![crate::types::ChatEntry::user("keep me")];
        state.show_clear_confirmation = true;
        state.pending_clear_name = Some("unused".to_string());
        let (agent_tx, _agent_rx) = mpsc::channel(1);

        handle_inline_confirm(
            &mut state,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &agent_tx,
        )
        .await
        .unwrap();

        assert!(!state.show_clear_confirmation);
        assert_eq!(state.chat_history.len(), 1);
        assert_eq!(state.pending_clear_name, None);
    }

    /// 编辑 provider 时，Model 字段必须回显**用户选中的那个模型**，
    /// 不能变成某个"默认的"。
    ///
    /// 用户的原始场景：只配 url + apikey（不配模型），在 `/models` 面板里选了
    /// 该 provider 的一个模型，之后打开编辑 provider —— 显示成默认的了。
    ///
    /// 修复前 `selected_model` 一度兼任"候选列表首项"，于是面板里那次选中
    /// 动作把列表头改掉了，表单回显出来的就不是用户选的那个了。
    #[tokio::test]
    async fn editing_a_provider_shows_the_model_the_user_picked() {
        use crate::core::config::provider_store::ProviderStore;

        let dir =
            std::env::temp_dir().join(format!("star-prefill-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("user-settings.json");
        let store = ProviderStore::with_paths(path.clone(), None);

        // 只配 url + apikey
        store
            .add_custom_provider(
                "e2e",
                Some("E2E"),
                "openai-compatible",
                Some("http://x/v1"),
                Some("k"),
                None,
            )
            .await
            .expect("provider created");
        // 在模型面板里选了一个模型
        store
            .set_active_provider_and_model("e2e", "picked-model")
            .await
            .expect("model selected");

        let mut form = crate::ui::state::palette::ProviderFormState::new();
        prefill_form_from_provider_with(&mut form, "e2e", &store).await;

        assert_eq!(
            form.model(),
            "picked-model",
            "编辑表单的 Model 字段必须回显用户选中的模型"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 候选列表里有多个模型时，编辑表单要**全部**列出，且声明的默认项排在最前 ——
    /// 不能只显示当前选中的那一个（其余的会显得"消失"了）。
    #[tokio::test]
    async fn editing_a_provider_lists_every_candidate() {
        use crate::core::config::provider_store::ProviderStore;

        let dir =
            std::env::temp_dir().join(format!("star-prefill2-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("user-settings.json");
        let store = ProviderStore::with_paths(path.clone(), None);

        store
            .add_custom_provider(
                "e2e",
                Some("E2E"),
                "openai-compatible",
                Some("http://x/v1"),
                Some("k"),
                None,
            )
            .await
            .expect("provider created");
        store
            .set_candidate_models("e2e", "zeta, alpha")
            .await
            .expect("candidates saved");
        // 面板里选了第二个
        store
            .set_active_provider_and_model("e2e", "alpha")
            .await
            .expect("model selected");

        let mut form = crate::ui::state::palette::ProviderFormState::new();
        prefill_form_from_provider_with(&mut form, "e2e", &store).await;

        assert_eq!(
            form.model(),
            "zeta, alpha",
            "候选列表必须完整列出，默认项在最前"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
