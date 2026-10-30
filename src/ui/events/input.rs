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

/// Result from clipboard operations (to avoid blocking async runtime)
enum ClipboardResult {
    Image(String, u32, u32),
    Text(String),
}

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
    let mut parts: Vec<String> = state.pending_user_messages.drain(..).collect();
    if !state.input.trim().is_empty() {
        parts.push(state.input.clone());
    }
    let text = parts.join("\n");
    reset_main_textarea(state);
    state.textarea.insert_str(&text);
    sync_input_from_textarea(state);
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

fn open_session_selection_menu(
    state: &mut ChatState,
    origin_palette: bool,
    sessions: Vec<crate::utils::session_manager::SessionSummary>,
) {
    clear_suggestion_overlays(state);
    state.close_palette();
    state.exit_input_modal();
    state.quick_menu_back = None;
    state.quick_menu_origin_palette = origin_palette;
    state.available_sessions = sessions;
    state.open_session_menu();
    state.selected_session_index = 0;
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
/// Model 从设置或内置默认值来。API Key 故意不填（见 `open_provider_form`）。
async fn prefill_form_from_provider(
    form: &mut crate::ui::state::palette::ProviderFormState,
    provider_id: &str,
) {
    prefill_form_from_provider_with(
        form,
        provider_id,
        &crate::core::config::provider_store::ProviderStore::new(),
    )
    .await
}

/// 预填的真正实现 —— store 由调用方给，便于测试注入隔离路径。
async fn prefill_form_from_provider_with(
    form: &mut crate::ui::state::palette::ProviderFormState,
    provider_id: &str,
    store: &crate::core::config::provider_store::ProviderStore,
) {
    use crate::ui::state::palette::{
        ANTHROPIC_COMPATIBLE_LABEL, OPENAI_COMPATIBLE_LABEL, PROVIDER_FORM_TYPES,
    };

    let settings = store
        .load()
        .await
        .unwrap_or_default()
        .providers
        .get(provider_id)
        .cloned();

    // 类型：已保存的设置优先，否则按内置 provider 推断，再不行就默认 OpenAI
    let type_label = settings
        .as_ref()
        .and_then(|s| s.r#type.as_deref())
        .map(|saved_type| {
            if saved_type == "anthropic-compatible" {
                ANTHROPIC_COMPATIBLE_LABEL
            } else {
                OPENAI_COMPATIBLE_LABEL
            }
        });
    let type_label = type_label.unwrap_or_else(|| {
        match crate::core::config::providers::provider_openai_compatible_mode(provider_id) {
            Some(false) => ANTHROPIC_COMPATIBLE_LABEL,
            _ => OPENAI_COMPATIBLE_LABEL,
        }
    });
    debug_assert!(
        PROVIDER_FORM_TYPES.contains(&type_label),
        "类型标签必须在候选列表里，否则 ←/→ 循环会失配"
    );

    let name = settings
        .as_ref()
        .and_then(|s| s.name.as_deref())
        .or_else(|| crate::core::config::providers::get_provider_by_id(provider_id).map(|p| p.name))
        .unwrap_or("");
    let base_url = crate::core::config::providers::resolve_provider_base_url(
        provider_id,
        settings.as_ref().and_then(|s| s.base_url.clone()),
    )
    .unwrap_or_default();
    // 预填整个候选列表（默认项在最前，见 `ProviderSettings::candidate_models`），
    // 逗号分隔进一个字段 —— 用户能一眼看出"这家配了哪几个模型"，也能直接改。
    // 只读 `selected_model` 的话，配了多个模型的用户打开编辑框只会看到一个，
    // 剩下的悄悄"消失"了。
    let model = settings
        .as_ref()
        .map(|s| s.candidate_models())
        .unwrap_or_default()
        .join(", ");

    form.values = vec![
        type_label.to_string(),
        name.to_string(),
        base_url,
        // API Key 留空：编辑时空着等于"保持原 key"，见 submit 的处理
        String::new(),
        model,
    ];
}

/// 把 `values[active]` 灌进 textarea，并按当前字段设占位符。
fn sync_textarea_to_active_field(state: &mut ChatState) {
    use crate::ui::state::palette::PROVIDER_FORM_FIELDS;

    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_placeholder_text(" ");
    textarea.set_cursor_style(
        ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
    );

    if let Some(value) = state
        .provider_form
        .values
        .get(state.provider_form.active_field)
    {
        if !value.is_empty() {
            textarea.insert_str(value);
        }
    }
    if let Some((label, hint, _)) = PROVIDER_FORM_FIELDS.get(state.provider_form.active_field) {
        // 编辑模式下 API Key 的占位提示含义不同（空着 = 保持原 key）
        let hint = if *label == "API Key" {
            crate::ui::state::palette::api_key_field_hint(state.provider_form.editing_id.is_some())
        } else {
            *hint
        };
        textarea.set_placeholder_text(hint);
    }
    state.modal_textarea = textarea;
}

/// 表单字段切换：当前 textarea 内容存回 `values[active]`，再切到目标字段。
/// `forward = false` 时往上走。字段区两端不循环，但末字段往下进操作行、
/// 操作行往下回 Type（循环一圈，免得 Tab 到了底就卡住）。
fn switch_provider_form_field(state: &mut ChatState, forward: bool) {
    use crate::ui::state::palette::PROVIDER_FORM_FIELDS;

    let active = state.provider_form.active_field;
    let on_actions = state.provider_form.on_actions;
    let last = PROVIDER_FORM_FIELDS.len().saturating_sub(1);

    let (next_field, next_on_actions) = match (forward, on_actions, active) {
        (true, true, _) => (0, false),                 // 操作行 → Type
        (true, false, a) if a >= last => (last, true), // 末字段 → 操作行
        (true, false, a) => (a + 1, false),
        (false, true, _) => (last, false), // 操作行 → 末字段
        (false, false, 0) => return,       // Type 顶头：不循环，停住
        (false, false, a) => (a - 1, false),
    };

    // 只要正离开一个文本字段，就得把 textarea 里的内容存走
    if !on_actions && (next_field != active || next_on_actions) {
        save_active_field_to_form(state);
    }
    state.provider_form.active_field = next_field;
    state.provider_form.on_actions = next_on_actions;
    state.provider_form.error = None;
    sync_textarea_to_active_field(state);
}

/// 把当前 textarea 的内容存回 `values[active]`。Type 字段（下标 0）不是文本，
/// textarea 里的东西不该覆盖它，直接跳过。
fn save_active_field_to_form(state: &mut ChatState) {
    if state.provider_form.active_field == 0 {
        return;
    }
    let value = collect_modal_input(&state.modal_textarea);
    if let Some(slot) = state
        .provider_form
        .values
        .get_mut(state.provider_form.active_field)
    {
        *slot = value;
    }
}

/// 取消表单、回上一层（Esc 和操作行上的 `‹ Back` 共用）。
///
/// 底层若还压着别的模态（如 Plugins 弹窗），只关输入框留在那里；否则回到
/// 打开表单前的 palette / quick menu。
fn cancel_provider_form(state: &mut ChatState) {
    state.input_context = None;
    state.exit_input_modal();
    if !state.is_modal_open() {
        show_palette_mode(state, state.palette_mode.clone());
    }
}

/// 提交 provider 表单（新增或编辑）。
///
/// 保存 provider 配置（一次 load+save，见 `ProviderStore::add_custom_provider`）：
/// 编辑模式沿用原 id，新增模式由名称或 URL host 派生 id。model 留空时走自动
/// 获取：发 `ListModels`（便宜路径，先吃两级缓存），面板进 Model 列表等结果；
/// 网络结果回来时 `list_models_with_mode` 会把列表写回内存缓存和磁盘缓存
/// （`model_cache`），`StreamMessage::ModelsList` 再把 UI 列表换掉 —— 下次开
/// 面板就直接命中缓存，不用再等网络。
async fn submit_provider_form(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    save_active_field_to_form(state);

    let form = state.provider_form.clone();
    let is_editing = form.editing_id.is_some();
    let provider_type = form.provider_type().to_string();
    let base_url = normalize_modal_base_url(form.base_url());
    let name = form.name().trim().to_string();
    let model = form.model().trim().to_string();

    if base_url.is_empty() {
        state.provider_form.error = Some("Base URL is required.".to_string());
        return Ok(true);
    }

    let store = crate::core::config::provider_store::ProviderStore::new();

    // 编辑模式沿用原 id；新增模式由名称 / URL host 派生并去重
    let provider_id = match form.editing_id.clone() {
        Some(id) => id,
        None => {
            let base_id = crate::core::config::provider_store::derive_provider_id(&name, &base_url);
            store.resolve_unique_provider_id(&base_id).await
        }
    };

    // 编辑模式下 API Key 留空 = 保持原 key。add_custom_provider 是整字段覆盖写，
    // 传 None 会把已存的 key 抹掉，所以这里把存着的 key 读出来原样写回。
    let mut api_key = normalize_modal_api_key(form.api_key());
    if api_key.is_empty() && is_editing {
        api_key = store
            .get_api_key(&provider_id)
            .await
            .unwrap_or(None)
            .unwrap_or_default();
    }

    // 模型字段是多值的：先取出解析后的名字列表，`add_custom_provider` 只拿首项当
    // **候选列表的头**（`default_model`），其余交给下面的 `set_candidate_models`
    // 写进 `models`。
    //
    // 注意这里写的是 `default_model` 而不是 `selected_model` —— 后者是"当前正在
    // 用的模型"，用户可能在 `/models` 面板里选了列表里的第二项。编辑 provider
    // 是在改**这张表**，不该顺手把用户当前用的模型改回列表头。
    let model_names = crate::core::config::models::split_model_names(&model);
    let default_model = model_names.first().map(String::as_str);

    store
        .add_custom_provider(
            &provider_id,
            if name.is_empty() { None } else { Some(&name) },
            &provider_type,
            Some(&base_url),
            if api_key.is_empty() {
                None
            } else {
                Some(&api_key)
            },
            default_model,
        )
        .await
        .map_err(|e| format!("Failed to save provider: {}", e))?;

    // 必须在 add_custom_provider 之后：那个方法会整体覆盖 `default_model`，
    // 而 `models` 里的手写 `context_window` / `alias` 要原样留着。
    if !store
        .set_candidate_models(&provider_id, &model)
        .await
        .map_err(|e| format!("Failed to save provider models: {}", e))?
    {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Error: provider '{}' vanished while saving models.",
                provider_id
            ),
        );
        return Ok(true);
    }

    // 激活 provider
    let _ = store.set_active_provider(&provider_id).await;
    register_configured_provider(state, &provider_id);
    state.current_provider_id = Some(provider_id.clone());

    let resolved_key = crate::core::config::providers::resolve_runtime_api_key(
        Some(&provider_id),
        if api_key.is_empty() {
            None
        } else {
            Some(api_key)
        },
    );
    let is_openai_compat = provider_type == "openai-compatible";
    let _ = agent_tx
        .send(AgentRequest::UpdateProviderConfig {
            provider_id: Some(provider_id.clone()),
            api_key: resolved_key,
            base_url: Some(base_url.clone()),
            is_openai_compatible: Some(is_openai_compat),
            // worker 只认一个"当前模型"，多出来的那些由 `models` 负责（面板）。
            model: default_model.map(str::to_string),
        })
        .await;

    state.exit_input_modal();
    close_quick_menus(state);
    state.provider_form = crate::ui::state::palette::ProviderFormState::new();

    let verb = if is_editing { "Updated" } else { "Created" };
    let display_name = if name.is_empty() {
        provider_id.as_str()
    } else {
        name.as_str()
    };

    if model_names.is_empty() {
        // 一个模型都没配 = 完全交给 API 自动发现。面板立刻能开，不用等网络。
        state.available_models.clear();
        state.available_models_info.clear();
        state.model_provider_map.clear();
        state.current_model.clear();
        // 先试磁盘缓存：这个 provider 以前拉过的话，面板打开就是完整列表，
        // 既不用闪 "Loading models..."，也不必空等。缓存按 provider 分桶，
        // 所以切回以前用过的 provider 是秒开。
        let seeded = crate::ui::services::stream::seed_models_from_disk_cache(state, &provider_id);
        state.awaiting_models = !seeded;
        state.awaiting_models_since = if seeded {
            None
        } else {
            Some(std::time::Instant::now())
        };
        let _ = agent_tx
            .send(AgentRequest::ListModels { force: false })
            .await;
        state.palette_history.clear();
        state.open_palette(PaletteMode::Model);
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "{} provider '{}' — {}",
                verb,
                display_name,
                if seeded {
                    "model list restored from cache"
                } else {
                    "fetching model list..."
                }
            ),
        );
    } else {
        // 配了名字：面板列出「声明的候选集 + API 实际提供的」，不必等网络 ——
        // 声明的排前面（用户的优先级），API 发现的补在后面。
        //
        // 之前这里是把 `available_models` 整个**替换**成声明的那几个，于是 API
        // 明明还返回了另外十几个模型，面板上却只看得到用户写的那一个 —— 得再点
        // 一次刷新才补得回来。用户描述的"下次再选时候它会只有这一个模型"就是这个。
        crate::ui::services::stream::merge_declared_and_discovered_models(
            state,
            &provider_id,
            &model_names,
        );
        state.available_models_info.clear();

        // 「当前正在用的模型」**不**因为编辑了表单就被改回列表头 ——
        // 用户可能刚在 `/models` 面板里选了列表里的第二项。只有当前模型压根
        // 不在候选集里（用户把它删了）才回落。
        let current_still_valid =
            !state.current_model.is_empty() && model_names.contains(&state.current_model);
        let default_model = if current_still_valid {
            state.current_model.clone()
        } else {
            model_names[0].clone()
        };
        state.current_model = default_model.clone();
        let _ = agent_tx
            .send(AgentRequest::SetModel {
                model: default_model.clone(),
                provider_id: Some(provider_id.clone()),
            })
            .await;
        state.palette_history.clear();
        state.open_palette(PaletteMode::Model);
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "{} provider '{}' — default '{}', {} model{} configured.",
                verb,
                display_name,
                default_model,
                model_names.len(),
                if model_names.len() == 1 { "" } else { "s" }
            ),
        );
    }

    Ok(true)
}

/// 把一个 provider id 记进 UI 的已配置列表，已存在就提到最前（新的在前）。
fn register_configured_provider(state: &mut ChatState, provider_id: &str) {
    state.configured_providers.retain(|id| id != provider_id);
    state
        .configured_providers
        .insert(0, provider_id.to_string());
}

/// 清空某个 provider 已保存的 API Key。
///
/// # 为什么必须立刻通知 worker
///
/// 磁盘上的 key 和运行中的 client 是两份状态。只改磁盘的话，正在跑的 client 还会
/// 继续用旧 key 发请求 —— 用户明明点了"清空"，行为却一点没变。`UpdateProviderConfig`
/// 的 `api_key: None` 表示"不变"，所以这里传 `Some(String::new())`：经过
/// `normalize_api_key_value` 之后正好落成 `None`，语义就是"没有 key"。
///
/// # 环境变量不受影响
///
/// `provider_has_usable_credentials` 会回落到 `resolve_api_key_from_env`。清掉存储值
/// 之后，如果对应环境变量还在，请求依然会带上它 —— 这是**正确**的：用户能自己决定
/// 要不要 `unset`。状态栏必须把这件事说清楚，否则"我明明清了，怎么还认证通过"
/// 会变成下一个查不到原因的问题。
async fn clear_provider_key(
    state: &mut ChatState,
    provider_id: &str,
    agent_tx: &mpsc::Sender<AgentRequest>,
) {
    let store = crate::core::config::provider_store::ProviderStore::new();
    let mut config = match store.load().await {
        Ok(config) => config,
        Err(e) => {
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("Error: could not read provider config: {}", e),
            );
            return;
        }
    };
    let Some(settings) = config.providers.get_mut(provider_id) else {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Error: provider '{}' has no stored configuration.",
                provider_id
            ),
        );
        return;
    };

    let had_key = settings
        .api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    settings.api_key = None;

    if let Err(e) = store.save(&config).await {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("Error: Failed to clear the API key: {}", e),
        );
        return;
    }

    // 运行中的 client 也要换掉，否则旧 key 会一直发出去。
    let _ = agent_tx
        .send(AgentRequest::UpdateProviderConfig {
            provider_id: Some(provider_id.to_string()),
            // `None` 在这里表示"保持不变"，所以要给一个空串让它归一化成"没有 key"。
            api_key: Some(String::new()),
            base_url: None,
            is_openai_compatible: None,
            model: None,
        })
        .await;

    // Provider 菜单上那枚"⚠ no API key"标记是现算的，重新拉一次名单让它更新。
    let _ = agent_tx.send(AgentRequest::LoadConfiguredProviders).await;

    crate::utils::logging::append_debug_log_line(&format!(
        "[ProviderKey] Cleared the stored API key for '{}' (had one: {})",
        provider_id, had_key
    ));
    crate::ui::app::logic::emit_status_text(
        state,
        0,
        &format!(
            "{}",
            crate::core::i18n::t(
                "provider.key_cleared",
                &format!(
                    "已清空 '{}' 保存的 API Key（环境变量仍可能生效）",
                    provider_id
                ),
                &format!(
                    "Cleared the stored API key for '{}' (environment variables still apply)",
                    provider_id
                ),
            )
        ),
    );
}

/// 删除一个自定义 provider。
///
/// 删除范围是本地配置里 `providers.<id>` 整项 —— endpoint、API key、选中的模型
/// 都在这一项里。如果它正是当前生效的 provider，`active_provider_id` /
/// `active_model` 也要一起清掉，否则下次启动会解析到一个已经不存在的 provider。
///
/// 内置 provider 不走这里（面板里也不给入口）：它们的"配置"只是 key 和 URL，
/// 删掉整项没有意义。
async fn delete_custom_provider(
    state: &mut ChatState,
    provider_id: &str,
    agent_tx: &mpsc::Sender<AgentRequest>,
) {
    if crate::core::config::providers::get_provider_by_id(provider_id).is_some() {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!(
                "Warning: '{}' is a built-in provider and cannot be removed.",
                provider_id
            ),
        );
        return;
    }

    let store = crate::core::config::provider_store::ProviderStore::new();
    let mut config = store.load().await.unwrap_or_default();
    let existed = config.providers.remove(provider_id).is_some();
    let was_active = config.active_provider_id.as_deref() == Some(provider_id);
    if was_active {
        config.active_provider_id = None;
        config.active_model = None;
    }

    if let Err(e) = store.save(&config).await {
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("Error: Failed to remove {}: {}", provider_id, e),
        );
        return;
    }

    state.configured_providers.retain(|id| id != provider_id);
    state.available_models.clear();
    state.available_models_info.clear();
    state.model_provider_map.retain(|_, pid| pid != provider_id);
    // 磁盘缓存里那个桶必须一起删掉。留着的话，全桶扫描会把里面的模型名认领给
    // 这个已经不存在的 provider，切换请求被 worker 以 "Provider '<x>' is not
    // configured." 拒掉 —— 而那个 reason 只落到状态栏、不写日志，用户看到的就是
    // "刚删完 provider，模型就切不动了"。
    crate::agent::model_cache::purge(provider_id);
    if was_active || state.current_provider_id.as_deref() == Some(provider_id) {
        state.current_provider_id = None;
        state.current_model.clear();
        state.pending_model_provider = None;
        state.pending_provider_selected_model = None;
    }

    // 模型列表按 provider 扇出，删完必须重拉，否则列表里还留着已删 provider 的模型。
    let _ = agent_tx
        .send(AgentRequest::ListModels { force: true })
        .await;

    let message = if !existed {
        format!(
            "'{}' was not in the local config; nothing to remove.",
            provider_id
        )
    } else if was_active {
        format!(
            "Removed provider '{}'. It was the active one — pick another provider.",
            provider_id
        )
    } else {
        format!("Removed provider '{}'.", provider_id)
    };
    crate::ui::app::logic::emit_status_text(state, 0, &message);

    // 回到 provider 列表：条目由 `get_items` 现算，重开一次才能反映删除结果。
    state.palette_history.clear();
    state.open_palette(PaletteMode::Provider);
}

pub(crate) async fn execute_palette_action(
    state: &mut ChatState,
    action: PaletteAction,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        PaletteAction::Navigate(mode) => {
            state.palette_history.push(state.palette_mode.clone());

            if matches!(mode, PaletteMode::Model) && state.available_models.is_empty() {
                // 先用磁盘缓存把列表填上：面板立刻能显示真实模型，
                // 不会先闪一下 "Loading models..." 再等一个多半会命中
                // 缓存的请求回来。随后那条请求照发，拿到新数据再覆盖。
                let pid = state
                    .current_provider_id
                    .clone()
                    .unwrap_or_else(|| crate::agent::model_cache::DEFAULT_BUCKET.to_string());
                crate::ui::services::stream::seed_models_from_disk_cache(state, &pid);

                let _ = agent_tx
                    .send(AgentRequest::ListModels { force: false })
                    .await;
            }

            // Entering from the settings modal (ShowStatus closes the palette)
            // must reopen the palette, otherwise navigation appears to do nothing.
            state.pop_modal();
            let items = crate::ui::components::palette::get_items(&mode, state);
            state.palette_mode = mode;
            state.palette_items = items;
            state.selected_palette_index = 0;
            state.palette_filter.clear();
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::ShowStatus => {
            state.close_palette();
            state.open_status_modal();
        }
        PaletteAction::ShowModelMenu => {
            if state.available_models.is_empty() {
                // 同上：先吃磁盘缓存，避免假 loading。
                let pid = state
                    .current_provider_id
                    .clone()
                    .unwrap_or_else(|| crate::agent::model_cache::DEFAULT_BUCKET.to_string());
                let seeded = crate::ui::services::stream::seed_models_from_disk_cache(state, &pid);
                // 只有缓存也没能给出列表时才标 loading —— 有缓存可显示时
                // 面板已经是完整列表，不该再显示"正在获取"。
                if !seeded {
                    state.awaiting_models = true;
                }
                let _ = agent_tx
                    .send(AgentRequest::ListModels { force: false })
                    .await;
            }
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::RefreshModels => {
            // 显式刷新：跳过内存/磁盘缓存，扇出到所有已配置 provider。
            // 面板留在原地（不 push 历史，否则 Back 会退回自己），只把 "⟳ Fetching..."
            // 的状态刷出来；列表到货后由 StreamMessage::ModelsList 重建 items。
            state.awaiting_models = true;
            if !state.is_palette_open() {
                state.palette_history.clear();
                state.open_palette(crate::ui::state::palette::PaletteMode::Model);
            } else {
                state.palette_mode = crate::ui::state::palette::PaletteMode::Model;
            }
            state.palette_items = crate::ui::components::palette::get_items(
                &crate::ui::state::palette::PaletteMode::Model,
                state,
            );
            let _ = agent_tx
                .send(AgentRequest::ListModels { force: true })
                .await;
        }
        PaletteAction::ShowProviderMenu => {
            open_provider_selection_menu(state, None, true);
        }
        PaletteAction::OpenMcpModal => {
            state.close_palette();
            state.open_mcp_modal();
            crate::ui::state::modal::load_mcp_server_rows(state).await;
        }
        PaletteAction::OpenMarketModal => {
            state.open_market_modal().await;
        }
        PaletteAction::ShowSessionMenu => {
            let sessions = crate::utils::session_manager::list_session_summaries()
                .await
                .unwrap_or_default();
            open_session_selection_menu(state, true, sessions);
        }
        PaletteAction::SetModel { model, provider_id } => {
            // If conversation has history, preserve the existing confirmation gate. Only the
            // eventual worker acknowledgement may change the active model display.
            let has_history = state.chat_history.iter().any(|entry| {
                !entry.is_welcome
                    && matches!(
                        entry.entry_type,
                        ChatEntryType::User | ChatEntryType::Assistant
                    )
                    && !entry.content.trim().is_empty()
            });

            if has_history
                && !state.pending_model_confirmation
                && !state.waive_next_model_confirmation
            {
                state.close_palette();
                // 归属随这次意图一起存下。确认框是有延迟的，等按 `y` 时列表和
                // 磁盘缓存都可能已经变了，那时再推导就可能推到一个已被删除的
                // provider 身上（见 `PendingModelChange`）。
                state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
                    model: model.clone(),
                    provider_id: provider_id.clone(),
                });
                state.pending_model_confirmation = true;
                // 提示本体由状态栏那一行常驻渲染（见 status_line::build_status_lines）——
                // 只写进 `current_status_line` 的话，空闲状态下它根本不显示。
                state.current_status_line = Some(crate::core::i18n::t(
                    "ui.model.confirm_switch",
                    &format!("切换模型到 {}? (y/n)", model),
                    &format!("Switch model to {}? (y/n)", model),
                ));
                return Ok(());
            }

            state.close_palette();
            state.pending_model_confirmation = false;
            state.waive_next_model_confirmation = false;
            let provider_id = provider_id
                .clone()
                .or_else(|| resolve_provider_for_model(state, &model));
            if request_runtime_model_change(state, model.clone(), provider_id, agent_tx).await {
                state.current_status_line = Some(format!("Switching to model {}…", model));
            }
        }
        PaletteAction::SetAgentMode(mode) => {
            state.close_palette();
            let approval_mode = match mode.as_str() {
                "yolo" => crate::types::ApprovalMode::Yolo,
                "plan" => crate::types::ApprovalMode::Plan,
                _ => crate::types::ApprovalMode::Default,
            };
            state.approval_mode = approval_mode.clone();
            let _ = agent_tx
                .send(AgentRequest::SetApprovalMode(approval_mode))
                .await;
        }
        PaletteAction::SetContextWindow(selection) => {
            state.close_palette();
            if request_runtime_settings(state, None, Some(selection), None, agent_tx).await {
                state.current_status_line = Some(format!(
                    "Requesting context window {}…",
                    selection.display()
                ));
            }
        }
        PaletteAction::InputContextWindow => {
            state.close_palette();
            state.enter_input_modal();
            state.input_modal_title = "Context Window Size".to_string();
            state.input_modal_prompt =
                "Enter a positive size (e.g. 128k, 1M, 1048576), or auto:".to_string();
            state.input_modal_value = String::new();
            let mut textarea = tui_textarea::TextArea::default();
            textarea.set_cursor_line_style(ratatui::style::Style::default());
            textarea.set_placeholder_text("128k");
            textarea.set_cursor_style(
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
            );
            state.modal_textarea = textarea;
            state.input_context = Some(crate::ui::state::palette::InputContext::ContextWindow);
        }
        PaletteAction::SetThinkingEffort(effort) => {
            state.close_palette();
            if request_runtime_settings(state, None, None, Some(effort.clone()), agent_tx).await {
                state.current_status_line =
                    Some(format!("Requesting {}…", effort.notification_text()));
            }
        }
        PaletteAction::SetTheme(theme_name) => {
            state.close_palette();
            if state.theme_manager.set_theme(&theme_name) {
                state.current_status_line = Some(format!("Theme: {}", theme_name));
                // Persist to user settings
                tokio::spawn(async move {
                    if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                        if let Ok(mut settings) = mgr.load_user_settings().await {
                            settings.ui_language = Some(theme_name); // Reuse ui_language field for now
                            let _ = mgr.save_user_settings(&settings).await;
                        }
                    }
                });
            }
        }
        PaletteAction::Back => {
            if let Some(prev_mode) = state.palette_history.pop() {
                state.palette_mode = prev_mode.clone();
                state.palette_items = crate::ui::components::palette::get_items(&prev_mode, state);
                state.selected_palette_index = 0;
                state.palette_filter.clear();
            } else {
                state.close_palette();
            }
        }
        PaletteAction::ExecuteCommand(cmd) => {
            state.close_palette();
            state.input = cmd;
            crate::ui::app::logic::enqueue_user_message(state, state.input.clone(), agent_tx)
                .await?;
            state.input.clear();
        }
        PaletteAction::TypeCommand(cmd) => {
            state.close_palette();
            state.input = cmd;
        }
        PaletteAction::SelectProvider(provider_id) => {
            state.available_models.clear();
            // 列表被清空了，但模型面板**马上**就要打开（见本分支末尾的
            // `push_palette_mode(Model)`）。不给 `awaiting_models` 置位，那个面板就会
            // 以"空列表 + 没有加载态"的样子弹出来 —— 用户看到的是一片空白、既没有
            // 模型可选、也没有任何说明在加载。按 Enter 挑不了任何东西，体感就是
            // "彻底没反应"。
            //
            // 这条路径以前**没有**置位，而 `submit_provider_form` 那条有 —— 同一个
            // "打开模型面板"的场景，两条路径的表现不一致。
            state.pending_model_provider = Some(provider_id.clone());
            // 切 provider 是明确意图：紧接着的第一次模型选择免 (y/n) 确认
            state.waive_next_model_confirmation = true;
            // 清除模型缓存，因为切换了 provider
            crate::agent::model_list::clear_model_cache();
            let store = crate::core::config::provider_store::ProviderStore::new();
            let selected_model = store.get_selected_model(&provider_id).await.unwrap_or(None);
            state.pending_provider_selected_model = selected_model.clone();
            state.current_provider_id = Some(provider_id.clone());
            if let Some(model) = selected_model.clone() {
                state.current_model = model;
            } else {
                state.current_model.clear();
            }
            if crate::core::config::providers::get_provider_by_id(&provider_id)
                .map(|provider| !provider.requires_api_key)
                .unwrap_or(false)
            {
                register_configured_provider(state, &provider_id);
            }

            let pid = provider_id.clone();
            let tx = agent_tx.clone();
            tokio::spawn(async move {
                let store = crate::core::config::provider_store::ProviderStore::new();
                let _ = store.set_active_provider(&pid).await;

                let api_key = crate::core::config::providers::resolve_runtime_api_key(
                    Some(&pid),
                    store.get_api_key(&pid).await.unwrap_or(None),
                );

                let base_url = crate::core::config::providers::resolve_provider_base_url(
                    &pid,
                    store.get_base_url(&pid).await.unwrap_or(None),
                );

                // Check is_openai_compatible: built-in check first, then custom provider type
                let provider_config = store.load().await.unwrap_or_default();
                let is_openai_compat =
                    crate::core::config::providers::resolve_openai_compatible_mode(
                        &pid,
                        provider_config
                            .providers
                            .get(&pid)
                            .and_then(|s| s.r#type.as_deref()),
                    );

                let _ = tx
                    .send(AgentRequest::UpdateProviderConfig {
                        provider_id: Some(pid.clone()),
                        api_key: api_key.clone(),
                        base_url,
                        is_openai_compatible: is_openai_compat,
                        model: selected_model,
                    })
                    .await;

                let _ = tx.send(AgentRequest::ListModels { force: false }).await;
            });

            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!(
                    "{} {}，{}",
                    crate::core::i18n::t("provider.selected", "已选择", "Selected"),
                    provider_id,
                    crate::core::i18n::t(
                        "provider.next_select_model",
                        "接下来选择模型",
                        "now select a model"
                    )
                ),
            );
            // 先用磁盘缓存把面板填上：这个 provider 以前拉过的话，一按就是完整列表，
            // 既不用空白闪一下，也不必等网络。缓存按 provider 分桶，切回用过的
            // provider 是秒开。缓存为空才真的进入"加载中"。
            let seeded =
                crate::ui::services::stream::seed_models_from_disk_cache(state, &provider_id);
            state.awaiting_models = !seeded;
            state.awaiting_models_since = if seeded {
                None
            } else {
                Some(std::time::Instant::now())
            };
            state.push_palette_mode(PaletteMode::Model);
            if !state.is_palette_open() {
                state.modal_stack.push(crate::ui::state::Modal::Palette);
            }
        }
        PaletteAction::EditProvider(provider_id) => {
            open_provider_form(state, Some(provider_id)).await;
        }
        PaletteAction::OpenProviderForm => {
            open_provider_form(state, None).await;
        }
        PaletteAction::DeleteProvider(provider_id) => {
            delete_custom_provider(state, &provider_id, agent_tx).await;
        }
        PaletteAction::ClearProviderKey(provider_id) => {
            clear_provider_key(state, &provider_id, agent_tx).await;
        }
        PaletteAction::ToggleFeature(feature) => match feature.as_str() {
            "context_viz" => {
                if matches!(
                    state.top_modal(),
                    Some(crate::ui::state::modal::Modal::ContextViz)
                ) {
                    state.pop_modal();
                } else {
                    state.open_context_viz();
                    state.context_breakdown = build_context_breakdown(state);
                }
            }
            _ => {}
        },
        PaletteAction::SetOutputStyle(style) => {
            state.close_palette();
            let style_clone = style.clone();
            tokio::spawn(async move {
                if let Ok(mgr) = crate::core::config::settings_manager::SettingsManager::new() {
                    if let Ok(mut settings) = mgr.load_user_settings().await {
                        settings.output_style = Some(style_clone);
                        let _ = mgr.save_user_settings(&settings).await;
                    }
                }
            });
            crate::ui::app::logic::emit_status_text(state, 0, &format!("Output style: {}", style));
        }
        PaletteAction::ShowLogSelector => {
            state.close_palette();
            state.open_log_selector();
            state.log_selector_state.begin_loading();

            match crate::utils::session_manager::list_session_summaries().await {
                Ok(sessions) => {
                    state
                        .log_selector_state
                        .set_sessions(sessions.into_iter().map(Into::into).collect());
                }
                Err(error) => {
                    let error = error.to_string();
                    state.log_selector_state.set_load_error(error.clone());
                    crate::ui::app::logic::emit_status_text(
                        state,
                        0,
                        &format!("Could not load saved sessions: {}", error),
                    );
                }
            }
        }
        PaletteAction::ShowContextViz => {
            state.close_palette();
            state.open_context_viz();
            state.context_breakdown = build_context_breakdown(state);
        }
        PaletteAction::ToggleVimMode => {
            state.close_palette();
            state.vim_enabled = !state.vim_enabled;
            if state.vim_enabled {
                state.vim_state = crate::ui::vim::VimState::new();
            }
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!("Vim mode: {}", if state.vim_enabled { "ON" } else { "OFF" }),
            );
        }
        PaletteAction::ToggleMouseCapture => {
            state.close_palette();
            // 真的去改终端模式：转义序列直接写 stdout。失败要说真话，
            // 并且把 state 里的标志回滚 —— 否则下次启动会带着错误的初值。
            let want = !state.mouse_capture_enabled;
            match crate::ui::utils::term_recovery::set_mouse_capture(want) {
                Ok(()) => {
                    state.mouse_capture_enabled = want;
                    state.current_status_line = Some(if want {
                        "Mouse capture: ON (terminal text selection unavailable)".to_string()
                    } else {
                        "Mouse capture: OFF (terminal text selection restored; wheel = arrow keys)"
                            .to_string()
                    });
                }
                Err(e) => {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[MOUSE] toggle failed: {}",
                        e
                    ));
                    state.current_status_line = Some(format!("Mouse capture toggle failed: {e}"));
                }
            }
        }
        PaletteAction::ToggleUiVerbose => {
            state.close_palette();
            state.ui_verbose = !state.ui_verbose;
            state.rendered_cache.clear();
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &format!(
                    "Verbose UI: {}",
                    if state.ui_verbose { "ON" } else { "OFF" }
                ),
            );
        }
        PaletteAction::CreatePr => {
            state.close_palette();
            // Send a message to the agent to create a PR
            let msg = crate::core::i18n::t(
                "ui.pr.create_request",
                "请帮我创建一个 Pull Request。使用 `gh pr create` 命令，自动填充标题和描述。",
                "Please help me create a Pull Request. Use `gh pr create` command with auto-filled title and description.",
            ).to_string();
            let message_id = state.next_message_id;
            state.next_message_id += 1;
            state.chat_history.push(crate::types::ChatEntry::new(
                crate::types::ChatEntryType::User,
                msg.clone(),
            ));
            let _ = agent_tx
                .send(crate::runtime::messages::AgentRequest::SendMessage {
                    message_id,
                    message: msg,
                })
                .await;
        }
        PaletteAction::ToggleColorblindMode => {
            state.close_palette();
            state.colorblind_mode = !state.colorblind_mode;
            crate::ui::app::logic::emit_status_text(
                state,
                0,
                &crate::core::i18n::t(
                    "ui.status.colorblind",
                    &format!(
                        "色盲模式: {}",
                        if state.colorblind_mode {
                            "开启"
                        } else {
                            "关闭"
                        }
                    ),
                    &format!(
                        "Colorblind mode: {}",
                        if state.colorblind_mode { "ON" } else { "OFF" }
                    ),
                ),
            );
        }
    }

    Ok(())
}

/// Handle keyboard input for the AskUserQuestion confirmation dialog.
/// Supports option navigation (Up/Down/j/k), direct selection (1-9),
/// multi-select toggle (Space), confirm (Enter), cancel (Esc),
/// and "Other" text input (Tab to switch, then type).
async fn handle_ask_user_question_input(
    key: KeyEvent,
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Extract option info from the confirmation entry
    let (option_count, multi_select) = state
        .pending_confirmation_entry_idx
        .and_then(|idx| state.chat_history.get(idx))
        .and_then(|entry| entry.confirmation.as_ref())
        .map(|conf| match &conf.details {
            crate::types::ConfirmationDetails::AskUserQuestion {
                options,
                multi_select,
                ..
            } => (options.len(), *multi_select),
            _ => (0, false),
        })
        .unwrap_or((0, false));

    let invalidate_cache = |state: &mut ChatState| {
        if let Some(idx) = state.pending_confirmation_entry_idx {
            state.rendered_cache.remove(&idx);
        }
    };

    if state.pending_question_other_focused {
        // "Other" text input mode
        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                        feedback: None,
                    })
                    .await;
                state.clear_pending_confirmation();
            }
            KeyCode::Tab => {
                // Switch back to option navigation
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.pending_question_other_focused = false;
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                // Confirm with other input included
                submit_ask_user_question_answer(
                    state,
                    agent_tx,
                    tool_call_id,
                    option_count,
                    multi_select,
                )
                .await?;
            }
            KeyCode::Backspace => {
                state.pending_other_input.pop();
                invalidate_cache(state);
            }
            KeyCode::Char(ch) => {
                state.pending_other_input.push(ch);
                invalidate_cache(state);
            }
            _ => {}
        }
    } else {
        // Option navigation mode
        // "Other" is at index option_count (after all regular options)
        let other_idx = option_count;
        let total_items = option_count + 1; // options + Other

        match key.code {
            KeyCode::Esc => {
                let _ = agent_tx
                    .send(AgentRequest::ConfirmTool {
                        tool_call_id: tool_call_id.to_string(),
                        outcome: crate::types::ToolConfirmationOutcome::Cancel,
                        feedback: None,
                    })
                    .await;
                state.clear_pending_confirmation();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.pending_confirmation_choice > 0 {
                    state.pending_confirmation_choice -= 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = total_items - 1;
                }
                invalidate_cache(state);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total_items > 0 && state.pending_confirmation_choice + 1 < total_items {
                    state.pending_confirmation_choice += 1;
                } else if total_items > 0 {
                    state.pending_confirmation_choice = 0;
                }
                invalidate_cache(state);
            }
            KeyCode::Tab => {
                // Switch to "Other" input mode
                state.pending_question_other_focused = true;
                state.pending_confirmation_choice = other_idx;
                invalidate_cache(state);
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let digit = ch.to_digit(10).unwrap_or(0) as usize;
                if digit >= 1 && digit <= option_count {
                    let opt_idx = digit - 1;
                    if multi_select {
                        // Multi-select: just focus that option
                        state.pending_confirmation_choice = opt_idx;
                    } else {
                        // Single-select: select and confirm immediately
                        state.pending_confirmation_choice = opt_idx;
                        submit_ask_user_question_answer(
                            state,
                            agent_tx,
                            tool_call_id,
                            option_count,
                            multi_select,
                        )
                        .await?;
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Char(' ') if multi_select => {
                // Toggle current option in multi-select mode
                let focused = state.pending_confirmation_choice;
                if focused < option_count {
                    if let Some(pos) = state
                        .pending_question_selections
                        .iter()
                        .position(|&i| i == focused)
                    {
                        state.pending_question_selections.remove(pos);
                    } else {
                        state.pending_question_selections.push(focused);
                    }
                }
                invalidate_cache(state);
            }
            KeyCode::Enter => {
                if !multi_select && state.pending_confirmation_choice == other_idx {
                    // "Other" is focused — switch to text input mode instead of submitting
                    state.pending_question_other_focused = true;
                    invalidate_cache(state);
                } else if multi_select && !state.pending_question_selections.is_empty() {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                } else if !multi_select && option_count > 0 {
                    submit_ask_user_question_answer(
                        state,
                        agent_tx,
                        tool_call_id,
                        option_count,
                        multi_select,
                    )
                    .await?;
                }
                // If multi_select but nothing selected, ignore Enter
            }
            _ => {}
        }
    }

    Ok(())
}

/// Collect the user's answer from the AskUserQuestion UI state and submit it.
async fn submit_ask_user_question_answer(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
    tool_call_id: &str,
    option_count: usize,
    multi_select: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect answer labels from the confirmation entry options
    let answers: Vec<String> = if multi_select {
        let mut indices = state.pending_question_selections.clone();
        indices.sort();
        indices
            .iter()
            .filter_map(|&idx| {
                state
                    .pending_confirmation_entry_idx
                    .and_then(|entry_idx| state.chat_history.get(entry_idx))
                    .and_then(|entry| entry.confirmation.as_ref())
                    .and_then(|conf| match &conf.details {
                        crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                            options.get(idx).map(|o| o.label.clone())
                        }
                        _ => None,
                    })
            })
            .collect()
    } else if option_count > 0 {
        let focused = state.pending_confirmation_choice;
        state
            .pending_confirmation_entry_idx
            .and_then(|entry_idx| state.chat_history.get(entry_idx))
            .and_then(|entry| entry.confirmation.as_ref())
            .and_then(|conf| match &conf.details {
                crate::types::ConfirmationDetails::AskUserQuestion { options, .. } => {
                    options.get(focused).map(|o| o.label.clone())
                }
                _ => None,
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };

    let text_input = if state.pending_other_input.is_empty() {
        None
    } else {
        Some(state.pending_other_input.clone())
    };

    // Update ChatEntry outcome text
    if let Some(idx) = state.pending_confirmation_entry_idx {
        if let Some(entry) = state.chat_history.get_mut(idx) {
            if let Some(conf) = entry.confirmation.as_mut() {
                let outcome_str = if answers.is_empty() && text_input.is_some() {
                    format!("User input: {}", text_input.as_ref().unwrap())
                } else if answers.is_empty() {
                    "No selection".to_string()
                } else if answers.len() == 1 {
                    if let Some(ref ti) = text_input {
                        format!("User selected: {} (with input: {})", answers[0], ti)
                    } else {
                        format!("User selected: {}", answers[0])
                    }
                } else {
                    format!(
                        "User selected {} options: {}",
                        answers.len(),
                        answers.join(", ")
                    )
                };
                conf.outcome = Some(outcome_str);
            }
        }
        state.rendered_cache.remove(&idx);
    }

    let _ = agent_tx
        .send(AgentRequest::ConfirmTool {
            tool_call_id: tool_call_id.to_string(),
            outcome: crate::types::ToolConfirmationOutcome::UserAnswer {
                answers,
                text_input,
            },
            feedback: None,
        })
        .await;

    state.clear_pending_confirmation();

    Ok(())
}

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
                state.current_session_title = None;
                state.chat_history.clear();
                // 会话已重置，上下文读数必须一起归零：否则新会话首轮会顶着一个
                // 与空历史矛盾的旧基线，直到下一次 provider usage 到来。
                state.token_usage = None;
                state.token_count = 0;
                state.cache_read_tokens = 0;
                state.cache_creation_tokens = 0;
                let _ = agent_tx
                    .send(crate::runtime::messages::AgentRequest::ResetSession)
                    .await;
                state.current_status_line = Some(match saved {
                    Some(id) => format!("Conversation cleared. Previous saved as `{}`.", id),
                    None => crate::core::i18n::t(
                        "ui.status.cleared",
                        "对话已清除",
                        "Conversation cleared",
                    )
                    .to_string(),
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

/// 打开对话内搜索面板（Ctrl+F）。
///
/// 搜的是**已经渲染出来的对话行**，不是 `chat_history` 里的结构化条目 ——
/// 用户记得的是"屏幕上那行写着 `cargo test`"，让他按渲染行去搜才对得上。
/// 命中行号落在 `ChatSearchState::matches` 里，与滚动/高亮共用一套坐标。
pub fn open_chat_search(state: &mut ChatState) {
    let scroll = state.scroll;
    state.chat_search.open(scroll);
    rerun_chat_search(state);
    // 面板是叠加层，不抢输入焦点：主输入框保持可打字
    state.auto_follow = false;
}

/// 关闭面板并回到打开前的滚动位置。
pub fn close_chat_search(state: &mut ChatState) {
    let restore = state.chat_search.restore_scroll;
    state.chat_search.close();
    let max_scroll = state
        .total_rendered_lines
        .saturating_sub(state.last_chat_height as usize);
    state.scroll = restore.min(max_scroll);
    // 只有原本就贴着底部才恢复跟随，否则会把用户正在读的历史拽走
    state.auto_follow = restore >= max_scroll;
}

/// 用当前 query 重跑搜索，并跳到视口内的第一个命中。
fn rerun_chat_search(state: &mut ChatState) {
    let haystack = chat_search_haystack(state);
    state.chat_search.run(&haystack);

    let height = state.last_chat_height as usize;
    let top = state.scroll;
    let bottom = top + height;
    let viewport = |row: usize| row >= top && row < bottom;
    // 视口内没有命中时跳第一个，否则跳第一个落在视口里的
    let target = state
        .chat_search
        .matches
        .iter()
        .position(|row| viewport(*row))
        .unwrap_or(0);
    state.chat_search.selected = target;
    reveal_chat_search_match(state);
}

/// 把命中行滚进视口中央。
fn reveal_chat_search_match(state: &mut ChatState) {
    let Some(row) = state.chat_search.current_match() else {
        return;
    };
    let height = state.last_chat_height as usize;
    if height == 0 {
        return;
    }
    state.scroll = row.saturating_sub(height / 3);
    let max_scroll = state.total_rendered_lines.saturating_sub(height);
    state.scroll = state.scroll.min(max_scroll);
    state.auto_follow = false;
}

/// 搜索用的逐行文本：与 `render_chat_lines` 的输出逐行对齐。
///
/// 必须与渲染同源，否则命中行号会整体错位（用户跳过去看到的是隔壁那条）。
/// 因此这里**按 entry 下标顺序**遍历 `rendered_cache` —— 它是 `HashMap`，
/// 直接 `.values()` 拿到的是任意顺序，命中会全落到错误的行上。
fn chat_search_haystack(state: &ChatState) -> Vec<String> {
    let mut lines = Vec::with_capacity(state.total_rendered_lines);
    for idx in 0..state.chat_history.len() {
        // 缓存里没有的条目（尚未渲染过）当空行跳过：它们的行数在缓存缺失时不可知，
        // 猜一个高度只会让后面的行号全错。
        let Some((_, entry_lines)) = state.rendered_cache.get(&idx) else {
            continue;
        };
        for line in entry_lines {
            lines.push(
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>(),
            );
        }
    }
    lines
}

/// 搜索面板的按键处理。返回 `true` 表示这个键已被面板消费；`false` 表示应当
/// 继续走后面的处理层（目前只有 `Ctrl+Shift+F`：先收面板，再开全局文件搜索）。
///
/// 面板打开期间吞掉除下面这些键以外的一切 —— 漏一个就会表现为"打一个字进了主输入框"。
fn handle_chat_search_key(state: &mut ChatState, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            close_chat_search(state);
            true
        }
        // Ctrl+F 再按一次关掉自己；带 Shift 时顺手让全局文件搜索接手
        KeyCode::Char('f') if ctrl => {
            close_chat_search(state);
            !key.modifiers.contains(KeyModifiers::SHIFT)
        }
        KeyCode::Enter => {
            if state.chat_search.step(true).is_some() {
                reveal_chat_search_match(state);
            }
            true
        }
        KeyCode::Backspace => {
            if state.chat_search.cursor > 0 {
                let cursor = state.chat_search.cursor - 1;
                state.chat_search.query.remove(cursor);
                state.chat_search.cursor = cursor;
                // Ctrl+Backspace 按词退格：连着退到分隔符之前（与编辑器一致）
                if ctrl {
                    let head = state.chat_search.query[..cursor].to_string();
                    let trimmed = head.trim_end();
                    let removed = head.len() - trimmed.len();
                    state.chat_search.query.truncate(cursor - removed);
                    state.chat_search.cursor = cursor - removed;
                }
                rerun_chat_search(state);
            }
            true
        }
        KeyCode::Char(c) => {
            state.chat_search.query.insert(state.chat_search.cursor, c);
            state.chat_search.cursor += 1;
            rerun_chat_search(state);
            true
        }
        KeyCode::Tab | KeyCode::Down => {
            if state.chat_search.step(true).is_some() {
                reveal_chat_search_match(state);
            }
            true
        }
        KeyCode::Up => {
            state.chat_search.step(false);
            reveal_chat_search_match(state);
            true
        }
        _ => true,
    }
}

/// Shift+Tab（`TogglePlanMode`）：Default → AcceptEdits → Plan → Yolo → Default
/// （对标 app:cycleMode / CC 的 shift+tab）。
///
/// `handle_key_event` 里有一个**排在它前面**的调用点（必须抢在两个行内确认之前），
/// 注册表分发器里也有一份 —— 两者共用这个函数，避免同一套模式循环写两遍。
async fn cycle_approval_mode(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>) {
    state.approval_mode = match state.approval_mode {
        ApprovalMode::Default => ApprovalMode::AcceptEdits,
        ApprovalMode::AcceptEdits => ApprovalMode::Plan,
        ApprovalMode::Plan => ApprovalMode::Yolo,
        ApprovalMode::Yolo => ApprovalMode::Default,
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
        // 已经在 `handle_key_event` 开头处理过（要抢在行内确认之前），这里只是
        // 保证注册表里每个绑定都有唯一实现。
        GlobalBinding::TogglePlanMode => {
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

    // 流输出"僵住"：取消宽限期已过但 is_streaming 没复位（例如 Done 落在宽限窗口里）。
    let cancelling_stale = state.is_streaming
        && state
            .cancelling_since
            .map(|t| t.elapsed() > Duration::from_millis(1500))
            .unwrap_or(false);
    if cancelling_stale {
        state.is_streaming = false;
        state.is_processing = false;
        state.cancelling_since = None;
    }

    if state.is_streaming || state.is_processing {
        // Cancel streaming; first Ctrl+C resets the double-press timer
        let _ = agent_tx.send(AgentRequest::Abort).await;
        state.current_status_line = Some("Cancelling...".to_string());
        state.is_processing = false;
        state.last_ctrl_c = None;
        state.cancelling_since = Some(Instant::now());
        return;
    }

    // Double-press detection: two Ctrl+C within 1.5 s exits
    let now = Instant::now();
    if let Some(prev) = state.last_ctrl_c {
        if now.duration_since(prev) <= Duration::from_millis(1500) {
            state.should_exit = true;
            return;
        }
    }
    state.last_ctrl_c = Some(now);

    if !state.input.is_empty() {
        // First press: clear input
        reset_main_textarea(state);
        crate::ui::services::suggestions::on_input_changed(state);
    }
    state.current_status_line = Some("Press Ctrl+C again to exit".to_string());
}

pub async fn handle_key_event(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
    last_key_time: Option<Instant>,
) -> Result<(), Box<dyn std::error::Error>> {
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

    // Handle Shift+Tab for mode cycle: Default → Plan → Yolo → Default
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
        // 这些上下文已经在阶梯更早处被独占处理（它们吃掉所有键），走到这里
        // 说明处理完了但键没被消费 —— 直接结束，别漏进主输入框。
        KeyContext::InlineConfirm
        | KeyContext::ModelConfirm
        | KeyContext::Modal
        | KeyContext::ThemePicker
        | KeyContext::ReadOnlyDialog
        | KeyContext::Palette
        | KeyContext::Help => return Ok(()),
        KeyContext::Vim | KeyContext::TextInput => {}
    }

    // Code block copy: press 'c' when no overlay is active to copy last code block
    if key.code == KeyCode::Char('c') && key.modifiers.is_empty() {
        if state.input.is_empty() {
            if let Some(ref code) = state.last_code_block_content {
                // Copy to clipboard（含 OSC 52 兜底）
                match crate::ui::utils::clipboard::copy_to_clipboard(code) {
                    Ok(()) => state.push_toast(
                        &crate::core::i18n::t(
                            "ui.status.copied_code_block",
                            "已复制代码块到剪贴板",
                            "Code block copied to clipboard",
                        ),
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
    // ErrorOverlay、StatusModal、ThemePicker、InputModal 都在那张表里），
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

                let raw_input = state.textarea.lines().join("\n");
                let input =
                    crate::ui::state::expand_paste_segments(&raw_input, &state.paste_segments);

                let trimmed_raw = raw_input.trim();
                if trimmed_raw == "/model" || trimmed_raw == "/models" {
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

                if let Some(model_name) = trimmed_raw.strip_prefix("/model ") {
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
                            format!(
                                " {}",
                                crate::core::i18n::t(
                                    "ui.model.not_in_list",
                                    "(不在已获取的模型列表中，仍按自定义名称使用)",
                                    "(not in fetched model list; using as custom name)",
                                )
                            )
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
                    // Important: Clear textarea BEFORE enqueueing message to prevent UI glitch
                    // where Enter keeps triggering and input remains.
                    reset_main_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);

                    crate::ui::app::logic::enqueue_user_message(state, input, agent_tx).await?;
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
            // Handle Streaming Cancellation
            if state.is_streaming || state.is_processing {
                let _ = agent_tx.send(AgentRequest::Abort).await;
                state.current_status_line = Some("Cancelling...".to_string());
                state.is_processing = false;
                state.cancelling_since = Some(Instant::now());
                return Ok(());
            }

            // 正在看某个后台代理的详情 → Esc 先回主会话（对标详情视图顶部那句
            // "Esc to return"）。选择器自己的 Esc 在 handle_bg_agent_selector_key 里
            // 已经被消费掉，走不到这儿。
            if state.viewing_agent_task_id.is_some() {
                state.exit_teammate_view();
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
                    let input = state.input.trim().to_string();
                    let is_dup = state
                        .command_history
                        .front()
                        .map(|l| l == &input)
                        .unwrap_or(false);
                    if !is_dup {
                        state.command_history.push_front(input);
                        state.command_history.truncate(100);
                        crate::core::config::history_store::save_history(&state.command_history);
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
            // Tab: toggle expand/collapse of the nearest tool entry
            if !state.is_palette_open() && !state.is_awaiting_confirmation {
                let focused_idx = find_focused_tool_entry(state);
                if let Some(idx) = focused_idx {
                    // 与鼠标单击工具头走同一条路径（ChatState::toggle_tool_expansion）。
                    state.toggle_tool_expansion(idx);
                    // 展开/折叠会改变该块的高度，视口锚点必须跟着重算，
                    // 否则用户展开的是当前屏幕中央那块，看的却是别处的滚动位置。
                    state.auto_follow = false;
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
            let input_is_empty = state.input.trim().is_empty();

            // 鼠标滚轮检测：连续 Up/Down 事件间隔 < 50ms 视为滚轮
            let now = Instant::now();
            let is_mouse_scroll = if let Some(pending_time) = state.pending_scroll_time {
                let elapsed = now.duration_since(pending_time).as_millis();
                if elapsed < 50 {
                    // 第二个事件快速到达，确认是鼠标滚轮
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    true
                } else {
                    // 超时，第一个事件是键盘按键，执行历史导航
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    false
                }
            } else {
                // 第一个事件，缓存为 pending
                state.pending_scroll_direction = Some(-1); // -1 = Up
                state.pending_scroll_time = Some(now);
                false
            };

            if is_mouse_scroll {
                // 鼠标滚轮：滚动聊天而非导航历史
                crate::ui::state::scroll_chat(state, -3);
            } else if cursor_row == 0 && !state.pending_user_messages.is_empty() {
                // 队列优先于命令历史（对标 PromptInput 的 handleHistoryUp）：
                // 光标在首行时 ↑ 先把排队的输入取回来改，而不是翻历史
                let n = state.pending_user_messages.len();
                restore_queued_messages_to_input(state);
                state.current_status_line = Some(format!(
                    "{} queued message{} moved back to the prompt",
                    n,
                    if n == 1 { "" } else { "s" }
                ));
            } else if cursor_row == 0 && (input_is_empty || state.history_index.is_some()) {
                // Command history navigation: Up when cursor is on first line and input is
                // empty or already in history mode
                if !state.command_history.is_empty() {
                    let idx = match state.history_index {
                        Some(i) => i.saturating_add(1),
                        None => {
                            state.history_input_snapshot = Some(state.input.clone());
                            0
                        }
                    };
                    if idx < state.command_history.len() {
                        state.history_index = Some(idx);
                        let hist_entry = state.command_history[idx].clone();
                        state.textarea.select_all();
                        state.textarea.delete_str(state.input.len());
                        state.textarea.insert_str(&hist_entry);
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

            // 鼠标滚轮检测：连续 Up/Down 事件间隔 < 50ms 视为滚轮
            let now = Instant::now();
            let is_mouse_scroll = if let Some(pending_time) = state.pending_scroll_time {
                let elapsed = now.duration_since(pending_time).as_millis();
                if elapsed < 50 {
                    // 第二个事件快速到达，确认是鼠标滚轮
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    true
                } else {
                    // 超时，第一个事件是键盘按键，执行历史导航
                    state.pending_scroll_direction = None;
                    state.pending_scroll_time = None;
                    false
                }
            } else {
                // 第一个事件，缓存为 pending
                state.pending_scroll_direction = Some(1); // 1 = Down
                state.pending_scroll_time = Some(now);
                false
            };

            if is_mouse_scroll {
                // 鼠标滚轮：滚动聊天而非导航历史
                crate::ui::state::scroll_chat(state, 3);
            } else if cursor_row + 1 >= line_count && state.history_index.is_some() {
                // Command history navigation: Down when cursor is on last line and in history mode
                let idx = state.history_index.unwrap();
                if idx == 0 {
                    state.history_index = None;
                    let snapshot = state.history_input_snapshot.take().unwrap_or_default();
                    state.textarea.select_all();
                    state.textarea.delete_str(state.input.len());
                    state.textarea.insert_str(&snapshot);
                    sync_input_from_textarea(state);
                    crate::ui::services::suggestions::on_input_changed(state);
                } else {
                    let new_idx = idx.saturating_sub(1);
                    state.history_index = Some(new_idx);
                    let hist_entry = state.command_history[new_idx].clone();
                    state.textarea.select_all();
                    state.textarea.delete_str(state.input.len());
                    state.textarea.insert_str(&hist_entry);
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
                push_kill(state, line[..col.min(line.len())].to_string());
            }
            state.textarea.delete_line_by_head();
            sync_input_from_textarea(state);
            crate::ui::services::suggestions::on_input_changed(state);
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+K: Delete from cursor to end of line (Unix readline)
            let (row, col) = state.textarea.cursor();
            if let Some(line) = state.textarea.lines().get(row) {
                push_kill(state, line[col.min(line.len())..].to_string());
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
                let line = line.as_str();
                let before = &line[..col.min(line.len())];
                let trimmed = before.trim_end();
                let new_col = if trimmed.is_empty() {
                    0
                } else {
                    // Find last word boundary: skip trailing spaces, then skip word chars
                    let bytes = trimmed.as_bytes();
                    let mut i = bytes.len() - 1;
                    // Skip current char if it's a word boundary
                    if !trimmed.is_char_boundary(i) {
                        i -= 1;
                    }
                    let last_char = trimmed[i..].chars().next().unwrap_or(' ');
                    if last_char.is_alphanumeric() || last_char == '_' {
                        // Skip word chars backward
                        while i > 0 && {
                            let c = trimmed[..i].chars().last().unwrap_or(' ');
                            c.is_alphanumeric() || c == '_'
                        } {
                            i -= 1;
                        }
                    } else {
                        // Skip non-word chars backward
                        while i > 0 && {
                            let c = trimmed[..i].chars().last().unwrap_or(' ');
                            !c.is_alphanumeric() && c != '_'
                        } {
                            i -= 1;
                        }
                    }
                    i
                };
                // Rebuild line: keep chars before new_col, skip chars from new_col to col
                let killed = line[new_col..col.min(line.len())].to_string();
                push_kill(state, killed);
                let mut new_line = String::new();
                new_line.push_str(&line[..new_col]);
                if col < line.len() {
                    new_line.push_str(&line[col..]);
                }
                // Delete the entire line and reinsert
                state.textarea.move_cursor(tui_textarea::CursorMove::Head);
                state.textarea.delete_line_by_end();
                state.textarea.insert_str(&new_line);
                // Move cursor to new_col
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

/// 后台代理选择器的按键处理（对标 background agent selector）。
///
/// 返回 `true` 表示按键已被选择器消费，调用方应直接结束本次事件处理。
///
/// 「光标停在哪一行」（`bg_agent_selection`）与「正在看谁的输出」（`viewing_agent_task_id`）
/// 是分开的两件事：`Enter` 只切换后者，焦点留在原行，用户可以连着 ↑/↓ 逐个翻看；
/// `Esc` 只收起选择器，不改变当前正在看的详情。
fn handle_bg_agent_selector_key(state: &mut ChatState, key: KeyEvent) -> bool {
    // 行数每帧都可能变（后台代理还在陆续启动），先按当前快照收敛焦点
    let task_ids: Vec<String> = state
        .background_agent_rows()
        .iter()
        .map(|info| info.task_id.clone())
        .collect();
    if task_ids.is_empty() {
        state.bg_agent_selection = None;
        return false;
    }
    let last_idx = task_ids.len(); // 索引 0 是 main 行，代理从 1 开始
    let selected = state.bg_agent_selection.unwrap_or(0).min(last_idx);

    match key.code {
        KeyCode::Up => {
            if selected == 0 {
                // 已在 main 行再往上 → 焦点还给输入框
                state.bg_agent_selection = None;
            } else {
                state.bg_agent_selection = Some(selected - 1);
            }
            true
        }
        KeyCode::Down => {
            state.bg_agent_selection = Some((selected + 1).min(last_idx));
            true
        }
        KeyCode::Enter => {
            if selected == 0 {
                state.exit_teammate_view();
            } else {
                let task_id = task_ids[selected - 1].clone();
                if state.viewing_agent_task_id.as_deref() == Some(task_id.as_str()) {
                    // 再按一次 Enter 收回主会话，跟参照实现的 toggle 语义一致
                    state.exit_teammate_view();
                } else {
                    state.enter_teammate_view(&task_id);
                }
            }
            true
        }
        KeyCode::Esc => {
            state.bg_agent_selection = None;
            true
        }
        _ => false,
    }
}

/// 状态弹窗（Settings）：↑↓ 选设置项、Enter 执行、Esc 关闭。
pub(crate) async fn handle_status_modal_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Esc => {
            state.pop_modal();
        }
        KeyCode::Up => {
            let count = crate::ui::components::status_modal::settings_item_count();
            state.settings_selected_index = state
                .settings_selected_index
                .saturating_add(count)
                .saturating_sub(1)
                % count;
        }
        KeyCode::Down => {
            let count = crate::ui::components::status_modal::settings_item_count();
            state.settings_selected_index = (state.settings_selected_index + 1) % count;
        }
        KeyCode::Enter => {
            // Get the action for the selected setting and execute it
            if let Some(action) = crate::ui::components::status_modal::get_settings_action(state) {
                state.pop_modal();
                // Re-dispatch the action through execute_palette_action
                execute_palette_action(state, action, agent_tx).await?;
            }
        }
        _ => {}
    }
    Ok(true)
}

/// 日志/会话选择器：↑↓ 选择、输入过滤、Enter 恢复会话、Esc 关闭。
pub(crate) async fn handle_log_selector_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    match key.code {
        KeyCode::Enter => {
            if state.log_selector_state.is_loading {
                return Ok(true);
            }

            // 走统一命令入口，保留流式中的排队保护，并复用 /chat resume 的状态清理。
            if let Some(session) = state.log_selector_state.get_selected_session() {
                let command = format!("/chat resume {}", session.id);
                state.pop_modal();
                crate::ui::app::logic::enqueue_user_message(state, command, agent_tx).await?;
            }
        }
        KeyCode::Esc => {
            state.pop_modal();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.select_prev();
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.select_next();
            }
        }
        KeyCode::Char('/') => {
            if !state.log_selector_state.is_loading {
                // Focus search
                state.log_selector_state.search_query.clear();
            }
        }
        KeyCode::Backspace => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.search_query.pop();
                state.log_selector_state.selected_index = 0;
            }
        }
        KeyCode::Char(c) => {
            if !state.log_selector_state.is_loading {
                state.log_selector_state.search_query.push(c);
                state.log_selector_state.selected_index = 0;
            }
        }
        _ => {}
    }
    // 未匹配也消费：选择器是模态，键不该漏到下面的输入框。
    Ok(true)
}

/// 重发最近一条用户消息。
///
/// 只在**当前回合确实失败**时返回 `Some`：一个正常收尾的会话里按 Esc 不该凭空
/// 多出一条消息。`last_turn_failed` 由 `handle_error_message` 置位，由
/// `handle_done_message` 和真正发起新回合时清掉，所以它同时也是"当前输入框是空的
/// 且没有别的理由不发消息"这一约束的判据。
pub(crate) async fn retry_last_user_message(
    state: &mut ChatState,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !state.last_turn_failed {
        return Ok(false);
    }
    let Some(last_user) = state
        .chat_history
        .iter()
        .rev()
        .find(|e| e.entry_type == ChatEntryType::User)
    else {
        // 失败回合之前没有用户消息（例如恢复出来的半截会话）：没有可重发的内容，
        // 但要把状态清掉，否则 Esc 会永远卡在这个分支里。
        state.last_turn_failed = false;
        return Ok(false);
    };
    let msg = last_user.content.clone();
    crate::ui::app::logic::enqueue_user_message(state, msg, agent_tx).await?;
    Ok(true)
}

/// 错误覆盖层：Enter 执行选中项、r 重试上一条、s 换 provider、c/Esc 关闭、↑↓ 切换选项。
pub(crate) async fn handle_error_overlay_key(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Enter 执行当前高亮的选项。之前界面画着 "Enter 确认" 却没有 Enter 分支，
    // `_ => {}` + `Ok(true)` 把按键吞掉 —— 高亮的那一项按下去什么都不会发生。
    if key.code == KeyCode::Enter {
        match state.error_overlay_state.selected_action {
            crate::ui::components::error_overlay::ErrorAction::Retry => {
                state.pop_modal();
                retry_last_user_message(state, agent_tx).await?;
            }
            crate::ui::components::error_overlay::ErrorAction::SwitchProvider => {
                state.pop_modal();
                state.open_palette(PaletteMode::Provider);
            }
            crate::ui::components::error_overlay::ErrorAction::Cancel => {
                state.pop_modal();
            }
        }
        return Ok(true);
    }
    match key.code {
        KeyCode::Char('r') | KeyCode::Char('R') => {
            state.pop_modal();
            retry_last_user_message(state, agent_tx).await?;
        }
        KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => {
            state.pop_modal();
            // 关掉浮层不等于放弃这次重试机会：Esc 之后在输入框按 Esc 仍然能重发。
            // 只有用户发了新消息或跑完一轮，这个标志才会被清掉。
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            // Switch provider
            state.pop_modal();
            state.open_palette(PaletteMode::Provider);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.error_overlay_state.select_prev();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.error_overlay_state.select_next();
        }
        _ => {}
    }
    Ok(true)
}

/// 主题选择器：↑↓ 导航（实时预览）、Enter 应用、Esc 取消。
///
/// 从 `handle_overlay_input` 抽出来的独立处理器：模态的按键处理应当住在
/// `modal_input::handle_modal_key` 那张表里，而不是散落在阶梯的中段。
pub(crate) fn handle_theme_picker_key(state: &mut ChatState, key: KeyEvent) -> bool {
    let themes = crate::ui::components::highlight::theme_picker::available_themes();
    let count = themes.len();
    let prev_index = state.selected_theme_index;
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected_theme_index = if state.selected_theme_index == 0 {
                count.saturating_sub(1)
            } else {
                state.selected_theme_index - 1
            };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected_theme_index = (state.selected_theme_index + 1) % count.max(1);
        }
        KeyCode::Enter => {
            let name = themes
                .get(state.selected_theme_index)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            if state.theme_manager.set_theme(&name) {
                state.current_status_line = Some(format!("Theme: {}", name));
            }
            state.pop_modal();
            return true;
        }
        KeyCode::Esc => {
            // 取消：恢复进入 picker 前的主题
            if let Some(crate::ui::state::modal::Modal::ThemePicker { prev_theme }) =
                state.pop_modal()
            {
                if let Some(prev) = prev_theme {
                    state.theme_manager.set_theme(&prev);
                }
            }
            return true;
        }
        _ => {}
    }
    // 实时预览：光标移动即切换主题
    if prev_index != state.selected_theme_index {
        if let Some(t) = themes.get(state.selected_theme_index) {
            state.theme_manager.set_theme(&t.name);
        }
    }
    true
}

/// 工具审批（AskUserQuestion / 权限确认）与权限说明、权限调试面板。
///
/// 只处理 [`KeyContext::ToolConfirmation`]：这个上下文本身就意味着「模态栈空、
/// 没有行内确认」，因此这里不必再重复判断，也就不存在「漏写一个守卫就穿透」
/// 的可能（那正是 `has_overlay_active()` 时代的毛病）。
///
/// 主题选择器、状态弹窗、任务面板都已在自己的分发表项里，不再经过本函数。
async fn handle_tool_approval(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if state.is_awaiting_confirmation {
        if let Some(id) = state.pending_tool_call_id.clone() {
            let is_ask = state
                .pending_confirmation_entry_idx
                .and_then(|idx| state.chat_history.get(idx))
                .and_then(|entry| entry.confirmation.as_ref())
                .map(|conf| {
                    matches!(
                        conf.operation_type,
                        crate::types::ConfirmationType::AskUserQuestion
                    )
                })
                .unwrap_or(false);

            if is_ask {
                handle_ask_user_question_input(key, state, agent_tx, &id).await?;
            } else if state.confirmation_feedback_mode {
                // Feedback input mode: Tab back to options, Enter submits with feedback
                match key.code {
                    KeyCode::Tab => {
                        state.confirmation_feedback_mode = false;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Esc => {
                        state.confirmation_feedback_mode = false;
                        state.pending_confirmation_feedback.clear();
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Backspace => {
                        state.pending_confirmation_feedback.pop();
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Enter => {
                        // Submit with current choice + feedback
                        let outcome = match state.pending_confirmation_choice {
                            1 => crate::types::ToolConfirmationOutcome::ProceedOnce,
                            2 => crate::types::ToolConfirmationOutcome::AllowSession,
                            3 => crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave,
                            _ => crate::types::ToolConfirmationOutcome::Cancel,
                        };
                        let feedback = if state.pending_confirmation_feedback.is_empty() {
                            None
                        } else {
                            Some(state.pending_confirmation_feedback.clone())
                        };
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    let outcome_str = match state.pending_confirmation_choice {
                                        1 => "Allowed (once)",
                                        2 => "Allowed (session)",
                                        3 => "Allowed (always)",
                                        _ => "Denied",
                                    };
                                    if let Some(ref fb) = feedback {
                                        conf.outcome =
                                            Some(format!("{} with feedback: {}", outcome_str, fb));
                                    } else {
                                        conf.outcome = Some(outcome_str.to_string());
                                    }
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome,
                                feedback,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    KeyCode::Char(ch) => {
                        state.pending_confirmation_feedback.push(ch);
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    // Tab: 切换到反馈输入模式（对标 Claude Code "Tab to provide feedback"）
                    KeyCode::Tab => {
                        state.confirmation_feedback_mode = true;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    // Shift+Tab: 快速批准（对标 Claude Code quick approve）
                    KeyCode::BackTab => {
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    conf.outcome = Some("Quick approved (Shift+Tab)".to_string());
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome: crate::types::ToolConfirmationOutcome::ProceedOnce,
                                feedback: None,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    // Ctrl+E: 切换权限解释区 / Ctrl+D: 切换 debug 详情（Claude Code 风格）
                    KeyCode::Char('e') | KeyCode::Char('E')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        state.show_permission_explanation = !state.show_permission_explanation;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    // 权限调试面板。绑 Ctrl+Shift+D 而不是 Ctrl+D —— Ctrl+D 现在唯一表示
                    // 「退出」（见本文件上方的 Ctrl+D 分支），同一个键挂三种语义必然打架。
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        state.show_permission_debug = !state.show_permission_debug;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('1' | 'y' | 'Y') => {
                        state.pending_confirmation_choice = 1;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('2' | 's' | 'S') => {
                        state.pending_confirmation_choice = 2;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('3' | 'a' | 'A') => {
                        state.pending_confirmation_choice = 3;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Char('4' | 'd' | 'D' | 'n' | 'N') => {
                        state.pending_confirmation_choice = 4;
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Esc => {
                        state.pending_confirmation_choice = 4;
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome: crate::types::ToolConfirmationOutcome::Cancel,
                                feedback: None,
                            })
                            .await;
                        // 先取索引再清状态：`clear_pending_confirmation` 会把
                        // `pending_confirmation_entry_idx` 置空，卡片上的 "Denied"
                        // 就没机会作废重绘了。
                        let rendered_idx = state.pending_confirmation_entry_idx;
                        state.clear_pending_confirmation();
                        if let Some(idx) = rendered_idx {
                            state.rendered_cache.remove(&idx);
                        }
                    }
                    KeyCode::Up => {
                        if state.pending_confirmation_choice > 1 {
                            state.pending_confirmation_choice -= 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Down => {
                        if state.pending_confirmation_choice < 4 {
                            state.pending_confirmation_choice += 1;
                            if let Some(idx) = state.pending_confirmation_entry_idx {
                                state.rendered_cache.remove(&idx);
                            }
                        }
                    }
                    KeyCode::Enter => {
                        let outcome = match state.pending_confirmation_choice {
                            1 => crate::types::ToolConfirmationOutcome::ProceedOnce,
                            2 => crate::types::ToolConfirmationOutcome::AllowSession,
                            3 => crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave,
                            _ => crate::types::ToolConfirmationOutcome::Cancel,
                        };
                        if let Some(idx) = state.pending_confirmation_entry_idx {
                            if let Some(entry) = state.chat_history.get_mut(idx) {
                                if let Some(conf) = entry.confirmation.as_mut() {
                                    let outcome_str = match state.pending_confirmation_choice {
                                        1 => "Allowed (once)",
                                        2 => "Allowed (session)",
                                        3 => "Allowed (always)",
                                        _ => "Denied",
                                    };
                                    conf.outcome = Some(outcome_str.to_string());
                                }
                            }
                            state.rendered_cache.remove(&idx);
                        }
                        // Collect feedback if user typed any
                        let feedback = if state.pending_confirmation_feedback.is_empty() {
                            None
                        } else {
                            Some(state.pending_confirmation_feedback.clone())
                        };
                        let _ = agent_tx
                            .send(AgentRequest::ConfirmTool {
                                tool_call_id: id,
                                outcome,
                                feedback,
                            })
                            .await;
                        state.show_permission_explanation = false;
                        state.show_permission_debug = false;
                        state.clear_pending_confirmation();
                    }
                    _ => {}
                }
            }
        } else {
            // 没有可回答的 id 却还挂着等待态：这是状态已经错乱的表现，
            // 只翻一个标志位会把 `pending_*` 留在半清不楚的状态上。
            state.clear_pending_confirmation();
        }
        return Ok(true);
    }

    Ok(false)
}

/// 任务面板（常驻）：编辑、上下移动、状态切换、层级调整。
///
/// 由 [`handle_key_event`] 的上下文分发表在 `KeyContext::TaskPanel` 时调用。
/// 面板只吃 Ctrl/Alt 组合键与 ↑/↓/Enter，普通字符落到输入框 —— 这条规则现在
/// 由 `KeyContext` 表达，不再需要各处手写 `&& !modal_open` 守卫。
pub(crate) fn handle_task_panel_key(state: &mut ChatState, key: KeyEvent) -> bool {
    if state.task_panel.is_editing() {
        state.task_panel.handle_edit_input(key);
        return true;
    }

    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL) {
        state
            .task_panel
            .enter_edit_mode(crate::ui::components::task_panel::EditMode::Title);
        return true;
    }

    // 与权限调试面板一致：Ctrl+D 归「退出」，任务描述编辑改用 Ctrl+Shift+D。
    if key.code == KeyCode::Char('d')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        state
            .task_panel
            .enter_edit_mode(crate::ui::components::task_panel::EditMode::Description);
        return true;
    }

    if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Char('v') {
        state.task_panel.cycle_view_mode();
        crate::ui::app::logic::emit_status_text(
            state,
            0,
            &format!("View: {}", state.task_panel.view_mode),
        );
        return true;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('j') => {
                state.task_panel.next();
                return true;
            }
            KeyCode::Char('k') => {
                state.task_panel.previous();
                return true;
            }
            KeyCode::Char(' ') => {
                state.task_panel.toggle_status();
                crate::ui::app::logic::emit_status_text(state, 0, "Task status toggled");
                return true;
            }
            KeyCode::Char('s') => {
                state.task_panel.skip_task();
                crate::ui::app::logic::emit_status_text(state, 0, "Task skipped");
                return true;
            }
            KeyCode::Char('n') => {
                state.task_panel.add_new_task();
                crate::ui::app::logic::emit_status_text(state, 0, "New task added");
                return true;
            }
            KeyCode::Delete => {
                state.task_panel.delete_selected();
                crate::ui::app::logic::emit_status_text(state, 0, "Task deleted");
                return true;
            }
            KeyCode::Up => {
                state.task_panel.move_up();
                return true;
            }
            KeyCode::Down => {
                state.task_panel.move_down();
                return true;
            }
            _ => {}
        }
    }

    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Right => {
                state.task_panel.indent();
                return true;
            }
            KeyCode::Left => {
                state.task_panel.outdent();
                return true;
            }
            _ => {}
        }
    }

    false
}

/// 通用输入对话框（API Key / Base URL / 重命名）与 provider 多字段表单。
///
/// 由 `modal_input::handle_modal_key` 在栈顶是 [`Modal::InputModal`] 时调用。
pub(crate) async fn handle_input_modal(
    state: &mut ChatState,
    key: KeyEvent,
    agent_tx: &mpsc::Sender<AgentRequest>,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !state.is_input_modal_open() {
        return Ok(false);
    }

    // "新增 provider" 表单：字段切换 / 类型切换 / 操作行 由这里接管。Type 字段
    // （下标 0）和操作行都是选项不是文本，普通字符键必须吃掉，否则会灌进
    // 共享的 textarea 里。
    let is_provider_form = matches!(
        state.input_context,
        Some(crate::ui::state::palette::InputContext::ProviderForm)
    );
    let form_on_type_field = is_provider_form && state.provider_form.active_field == 0;
    let form_on_actions = is_provider_form && state.provider_form.on_actions;

    match key.code {
        KeyCode::Tab | KeyCode::Down if is_provider_form => {
            switch_provider_form_field(state, true);
            return Ok(true);
        }
        KeyCode::BackTab | KeyCode::Up if is_provider_form => {
            switch_provider_form_field(state, false);
            return Ok(true);
        }
        KeyCode::Left if form_on_type_field => {
            state.provider_form.cycle_type(false);
            return Ok(true);
        }
        KeyCode::Right if form_on_type_field => {
            state.provider_form.cycle_type(true);
            return Ok(true);
        }
        // Type 是选项字段，回车不当提交用：确认当前类型并跳到 Name。不然焦点
        // 落在 Type 上时按回车只会去提交一个空表单，报 "Base URL is required."
        KeyCode::Enter if form_on_type_field => {
            switch_provider_form_field(state, true);
            return Ok(true);
        }
        // 操作行：←/→ 在 Back / Save 之间选
        KeyCode::Left | KeyCode::Right if form_on_actions => {
            state.provider_form.action_index = 1 - state.provider_form.action_index;
            return Ok(true);
        }
        // 操作行回车：选中项是 Save 就提交，是 Back 就取消回上一层
        KeyCode::Enter if form_on_actions => {
            if state.provider_form.action_index == 1 {
                state.input_context = Some(crate::ui::state::palette::InputContext::ProviderForm);
                return submit_provider_form(state, agent_tx).await;
            }
            cancel_provider_form(state);
            return Ok(true);
        }
        // Type 字段和操作行不接收自由文本，按键一律吞掉，避免污染共享 textarea
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
            if form_on_type_field || form_on_actions =>
        {
            return Ok(true);
        }
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Ok(mut clipboard) = Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    state.modal_textarea.insert_str(&text);
                }
            }
        }
        KeyCode::Esc => {
            // 底层还有模态（如 Plugins 弹窗）时 cancel 只关输入框，回到该模态
            cancel_provider_form(state);
            return Ok(true);
        }
        KeyCode::Enter => {
            state.input_modal_value = collect_modal_input(&state.modal_textarea);

            if let Some(ctx) = state.input_context.take() {
                match ctx {
                    crate::ui::state::palette::InputContext::ContextWindow => {
                        let input = state.input_modal_value.trim().to_string();
                        match crate::core::context_policy::ContextWindowSelection::parse(&input) {
                            Ok(selection) => {
                                state.exit_input_modal();
                                if request_runtime_settings(
                                    state,
                                    None,
                                    Some(selection),
                                    None,
                                    agent_tx,
                                )
                                .await
                                {
                                    state.current_status_line = Some(format!(
                                        "Requesting context window {}…",
                                        selection.display()
                                    ));
                                }
                            }
                            Err(error) => {
                                state.current_status_line =
                                    Some(format!("Invalid context window size: {}.", error));
                            }
                        }
                    }
                    crate::ui::state::palette::InputContext::ProviderForm => {
                        // 提交校验失败时 submit 不关模态只设 error。context 被
                        // take 走又没还回来的话，show_input_modal 还开着、
                        // 渲染却退化成单字段的通用输入框（还带着别的弹窗残留的
                        // title）。先还回去，失败时表单还在原地。
                        state.input_context =
                            Some(crate::ui::state::palette::InputContext::ProviderForm);
                        return submit_provider_form(state, agent_tx).await;
                    }
                    crate::ui::state::palette::InputContext::MarketplaceSource => {
                        let source = state.input_modal_value.trim().to_string();
                        state.exit_input_modal();

                        if source.is_empty() {
                            state.input_context = None;
                            return Ok(true);
                        }

                        // add_marketplace 需要克隆仓库，可能耗时数秒：后台执行，
                        // 完成后经 StreamMessage::PluginOpResult 回填消息并刷新
                        let cwd = std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."));
                        state.plugin_op_pending = true;
                        state.plugin_message = Some(format!("Adding marketplace {}...", source));
                        let _ = agent_tx
                            .send(AgentRequest::PluginOp {
                                project_root: cwd,
                                op: crate::runtime::messages::PluginOp::AddMarketplace { source },
                            })
                            .await;
                        state.plugin_index = 0;
                        state.input_context = None;
                        return Ok(true);
                    }
                    crate::ui::state::palette::InputContext::AddWorkingDir => {
                        let path = state.input_modal_value.clone();
                        state.exit_input_modal();
                        state.input_context = None;

                        if path.trim().is_empty() {
                            return Ok(true);
                        }

                        match crate::commands::extended::add_working_dir_to_state(state, &path) {
                            Ok(msg) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(msg).with_streaming(false),
                                );
                            }
                            Err(e) => {
                                state.chat_history.push(
                                    crate::types::ChatEntry::assistant(format!("❌ {}", e))
                                        .with_streaming(false),
                                );
                            }
                        }
                        return Ok(true);
                    }
                }
            } else {
                state.exit_input_modal();
            }
        }
        _ => {
            state.modal_textarea.input(key);
        }
    }
    Ok(true)
}

async fn handle_paste(
    state: &mut ChatState,
    key: KeyEvent,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) || key.code != KeyCode::Char('v') {
        return Ok(false);
    }

    state.paste_in_progress = true;
    state.paste_end_time = Some(Instant::now());

    // Timeout prevents hangs on WSL2/SSH where clipboard access may block indefinitely
    const CLIPBOARD_TIMEOUT_MS: u64 = 500;
    let clipboard_result = timeout(
        Duration::from_millis(CLIPBOARD_TIMEOUT_MS),
        tokio::task::spawn_blocking(|| {
            if let Some((path, w, h)) = save_clipboard_image() {
                return Ok(ClipboardResult::Image(path, w, h));
            }
            if let Ok(mut clipboard) = Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    return Ok(ClipboardResult::Text(text));
                }
            }
            Err("Clipboard empty".to_string())
        }),
    )
    .await;

    match clipboard_result {
        Ok(Ok(Ok(ClipboardResult::Image(path, w, h)))) => {
            insert_image_paste_block(state, path, w, h);
            sync_input_from_textarea(state);
            crate::ui::services::suggestions::on_input_changed(state);
        }
        Ok(Ok(Ok(ClipboardResult::Text(text)))) => {
            if let Some(file_paths) = detect_file_paths(&text) {
                insert_file_paste_block(state, file_paths);
            } else {
                insert_paste_block(state, text);
                if state.paste_segments.is_empty() {
                    maybe_auto_fold_input(state);
                }
            }
            sync_input_from_textarea(state);
            crate::ui::services::suggestions::on_input_changed(state);
        }
        Err(_) => {
            // Timeout — clipboard access hung (common on WSL2/SSH)
            crate::utils::logging::append_debug_log_line(
                "[PASTE] Clipboard read timed out, ignoring Ctrl+V",
            );
            return Ok(false);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Find the nearest ToolCall entry to the current viewport center for keyboard toggle.
/// Returns the index into chat_history, or None if no ToolCall entry is visible.
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

    // 关键：条目的**屏幕行区间**由 `virtual_list` 记录的真实高度累加得到。
    //
    // 原实现拿 entry 下标跟 `scroll / 2` 比距离 —— entry 下标是序号不是行号，
    // 一个 30 行的工具块和一个 1 行的回复在下标空间里等价，于是 Tab 经常展开
    // 视口里根本看不见的那一块，用户看到的是"什么都没发生"。
    let mut best_idx: Option<usize> = None;
    let mut best_distance: usize = usize::MAX;
    let mut top = 0usize;
    for (idx, entry) in state.chat_history.iter().enumerate() {
        let height = state.virtual_list.item_height(idx) as usize;
        let bottom = top + height;
        if entry.entry_type == ChatEntryType::ToolCall && height > 0 {
            // 条目覆盖视口中心才算是"焦点所在"
            if viewport_center >= top && viewport_center < bottom {
                let mid = top + height / 2;
                let distance = mid.abs_diff(viewport_center);
                if distance < best_distance {
                    best_distance = distance;
                    best_idx = Some(idx);
                }
            }
        }
        top = bottom;
    }

    best_idx
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
            state.pending_user_messages.iter().collect::<Vec<_>>(),
            vec![&"/chat resume saved-session".to_string()]
        );
        assert!(agent_rx.try_recv().is_err());
    }

    /// 排队的输入按入队顺序拼回输入框，队列随之清空。
    #[test]
    fn restoring_the_queue_joins_messages_in_order() {
        let mut state = ChatState::new();
        state.pending_user_messages.push_back("first".to_string());
        state.pending_user_messages.push_back("second".to_string());

        assert!(restore_queued_messages_to_input(&mut state));
        assert_eq!(state.input, "first\nsecond");
        assert!(state.pending_user_messages.is_empty());
    }

    /// 输入框里已经敲了一半的内容不能被队列覆盖：它接在队列后面。
    #[test]
    fn restoring_the_queue_keeps_the_half_typed_draft() {
        let mut state = ChatState::new();
        state.pending_user_messages.push_back("queued".to_string());
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
