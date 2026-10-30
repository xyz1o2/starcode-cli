//! ChatState 的测试。
//!
use super::*;

/// 回归：模型列表的"正在拉"状态**不能**永远挂着。
///
/// 真实事故（用户截图里那个转不掉的 `◎ …`）：日志里只留下一条
/// `[ListModels] Client state: …` 就再没有后续，worker 那个 spawn 什么消息都
/// 没发出来，于是 `awaiting_models` 永远停在 true，模型面板的转圈也就永远
/// 停不下来 —— 用户没有任何办法从中恢复。
///
/// UI 侧现在靠 `models_loading()` 的上限兜底：宁可显示空列表 + 错误。
#[test]
fn a_lost_reply_cannot_latch_the_loading_state_forever() {
    let mut state = ChatState::new();
    assert!(!state.models_loading());

    state.awaiting_models = true;
    state.awaiting_models_since = Some(std::time::Instant::now());
    assert!(state.models_loading(), "刚置位时当然还在拉");

    // 模拟"回复丢了 + 时间过去了"：把起点往回拨，绕过真实等待。
    state.awaiting_models_since = Some(std::time::Instant::now() - MODELS_LOADING_DEADLINE);
    assert!(
        !state.models_loading(),
        "超过上限后必须判定为不再等待，否则转圈停不下来"
    );
}

/// 没记时刻时按"仍在拉"处理 —— 否则旧代码路径里直接置位的地方会误判。
#[test]
fn a_loading_flag_without_a_timestamp_still_counts_as_loading() {
    let mut state = ChatState::new();
    state.awaiting_models = true;
    state.awaiting_models_since = None;
    assert!(state.models_loading());
}

/// 收到回复后必须立刻不再转圈 —— 上限只是兜底，不是延后。
#[test]
fn a_reply_clears_the_loading_state_immediately() {
    let mut state = ChatState::new();
    state.awaiting_models = true;
    state.awaiting_models_since = Some(std::time::Instant::now());
    state.awaiting_models = false;
    state.awaiting_models_since = None;
    assert!(!state.models_loading());
}

/// 标志没置位时，无论时刻是什么都不该转圈。
#[test]
fn no_flag_means_not_loading_even_with_a_stale_timestamp() {
    let mut state = ChatState::new();
    state.awaiting_models = false;
    state.awaiting_models_since = Some(std::time::Instant::now() - MODELS_LOADING_DEADLINE);
    assert!(!state.models_loading());
}
use super::*;
use crate::types::{ChatEntry, ChatEntryType};

/// 测试用例的输入文本绝不能落进用户真实的草稿文件：`save_draft` 每次输入变化都写盘，
/// 一旦写到 `~/.star/draft.txt`，下次启动 `restore_draft` 就会把测试字符串
/// （"queued\nhalf typed" 之类）原样填进输入框。
#[test]
fn draft_path_in_tests_isolates_the_users_real_draft() {
    let state = ChatState::new();
    let path = state
        .draft_path
        .as_ref()
        .expect("draft path should always be resolved");

    assert!(
        path.starts_with(std::env::temp_dir()),
        "test draft must live in temp dir, got {}",
        path.display()
    );
    assert!(
        path.to_string_lossy().contains("starcode-test-draft"),
        "unexpected draft path: {}",
        path.display()
    );
}

/// 草稿恢复必须同步 `input` 和行数计数器：只动 textarea 的话多行草稿会被
/// 压进按 0 行算的输入框，`state.input` 也仍是空的。
#[test]
fn restore_draft_syncs_input_and_line_count() {
    let dir = std::env::temp_dir().join(format!("starcode-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("draft.txt");
    std::fs::write(&path, "line one\nline two\nline three").unwrap();

    let mut state = ChatState::new();
    state.draft_path = Some(path.clone());
    state.restore_draft();

    assert_eq!(state.input, "line one\nline two\nline three");
    assert_eq!(state.input_line_count, 3);
    assert_eq!(
        state.textarea.lines().join("\n"),
        "line one\nline two\nline three"
    );
    assert!(!path.exists(), "草稿取完应当即删除，避免重复恢复");
}

/// 草稿恢复发生在初始化完成之后，此时输入框可能已有等待期间敲的内容：
/// 草稿在前、已敲的内容接在后面，不能互相覆盖。
#[test]
fn restore_draft_appends_after_what_was_already_typed() {
    let dir = std::env::temp_dir().join(format!("starcode-draft-{}-2", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("draft.txt");
    std::fs::write(&path, "the draft").unwrap();

    let mut state = ChatState::new();
    state.draft_path = Some(path);
    state.set_input_text("typed during loading");
    state.restore_draft();

    assert_eq!(state.input, "the draft\ntyped during loading");
}

/// 列表年龄要按"拉取时刻"算，而不是"收到消息的时刻"：会话开着两小时后再看，
/// 一份两小时前从磁盘缓存读出来的列表不能显示成"刚拉的"。
#[test]
fn model_list_age_counts_from_the_fetch_not_from_receipt() {
    let mut state = ChatState::new();
    assert_eq!(state.models_list_age_secs(), None);

    // 刚从 API 拉的 -> 年龄约 0
    state.set_models_list_age(None);
    assert!(state.models_list_age_secs().unwrap() < 5);

    // 缓存里读出来的，两小时前拉的 -> 年龄从那时算起
    state.set_models_list_age(Some(7_200));
    let age = state.models_list_age_secs().unwrap();
    assert!((7_200..7_205).contains(&age), "age was {}", age);
}

/// 荒谬的缓存年龄既不能 panic 也不能被悄悄压成"刚拉的"。
///
/// 以前这个位置会退化成"现在"（`checked_sub` 失败 → `or(Some(now))`），等于把
/// 一份明显很旧的列表报成刚拉的。现在是饱和加法：保留原值，`u64::MAX` 加 elapsed
/// 停在 `u64::MAX`，既不溢出也不回退。
#[test]
fn an_absurd_cache_age_is_kept_rather_than_reset_to_now() {
    let mut state = ChatState::new();
    state.set_models_list_age(Some(u64::MAX));
    assert_eq!(
        state.models_list_age_secs(),
        Some(u64::MAX),
        "荒谬年龄应原样保留，而不是被悄悄改成 0（显示成刚拉的）"
    );

    // 已经观察了一段时间时也必须饱和，而不是绕回。
    state.set_models_list_age(Some(u64::MAX));
    assert!(state.models_list_age_secs().is_some());
}

/// 缓存年龄大于机器已开机时长时必须照报 —— 这正是旧实现静默归零的场景。
///
/// 回归：`Instant::now().checked_sub(Duration::from_secs(age))` 在 `Instant`
/// 可回溯范围不足时返回 `None`（机器刚重启时必然如此），于是任何"比开机时长更
/// 旧"的缓存都被显示成刚拉的。`model_list_age_counts_from_the_fetch_not_from_
/// receipt` 用 7200 秒覆盖了这条路径；这里把上限再抬高一档并钉住语义。
#[test]
fn a_cache_older_than_uptime_is_still_reported_honestly() {
    let mut state = ChatState::new();
    let absurd_but_plausible = 30 * 24 * 60 * 60; // 30 天
    state.set_models_list_age(Some(absurd_but_plausible));
    assert!(
        state.models_list_age_secs().unwrap() >= absurd_but_plausible,
        "比进程/机器寿命长得多的缓存年龄必须照报，不能塌成 0"
    );
}

/// Ctrl+O / `/tui transcript` 共用的开关：开启时展开所有 thinking 块，关闭时收起，
/// 对标 Claude Code transcript 视图「摊开全部原始输出」的语义。
#[test]
fn toggle_transcript_mode_drives_thinking_expansion() {
    let mut state = ChatState::new();
    state.chat_history.clear();
    state
        .chat_history
        .push(ChatEntry::new(ChatEntryType::User, "hi".to_string()));
    let mut with_reasoning = ChatEntry::assistant("answer");
    with_reasoning.reasoning_content = Some("thought".to_string());
    state.chat_history.push(with_reasoning);
    state
        .chat_history
        .push(ChatEntry::assistant("no reasoning here"));

    assert!(!state.is_transcript_mode);
    assert!(state.expanded_thinking_indices.is_empty());

    assert!(state.toggle_transcript_mode());
    // 只有带 reasoning_content 的条目会被展开
    let expanded: Vec<usize> = state.expanded_thinking_indices.iter().copied().collect();
    assert_eq!(expanded, vec![1]);

    assert!(!state.toggle_transcript_mode());
    assert!(state.expanded_thinking_indices.is_empty());
}

/// 造一个后台代理任务：`started_at` 用「now - offset_ms」拉开先后顺序
fn bg_task(id: &str, is_async: bool, offset_ms: u64) -> AgentTaskInfo {
    AgentTaskInfo {
        task_id: id.to_string(),
        agent_type: "Explore".to_string(),
        description: format!("desc {}", id),
        status: crate::types::AgentTaskStatus::Running,
        tool_use_count: 0,
        tokens: 0,
        is_async,
        is_resolved: false,
        is_error: false,
        last_tool_info: None,
        name: None,
        task_description: None,
        started_at: Instant::now() - std::time::Duration::from_millis(offset_ms),
        finished_at: None,
        sub_entries: Vec::new(),
        entry_idx: 0,
    }
}

/// 选择器的数据源：只收后台（is_async）任务，按启动时间升序，超出上限丢最老的。
#[test]
fn background_agent_rows_filters_sorts_and_caps() {
    let mut state = ChatState::new();
    // 同步子代理不进选择器
    state
        .active_agent_tasks
        .insert("sync".to_string(), bg_task("sync", false, 500));
    // 故意乱序插入（HashMap 遍历顺序本身也不稳定）
    state
        .active_agent_tasks
        .insert("b".to_string(), bg_task("b", true, 200));
    state
        .active_agent_tasks
        .insert("a".to_string(), bg_task("a", true, 300));
    state
        .active_agent_tasks
        .insert("c".to_string(), bg_task("c", true, 100));

    let ids: Vec<&str> = state
        .background_agent_rows()
        .iter()
        .map(|i| i.task_id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"], "先启动的排在前面，同步任务被过滤");

    // 超过上限时保留最近启动的 MAX_BG_AGENT_ROWS 个
    for n in 0..MAX_BG_AGENT_ROWS + 3 {
        let id = format!("x{}", n);
        // offset 越大越老 → x0 最老
        let task = bg_task(&id, true, (MAX_BG_AGENT_ROWS + 3 - n) as u64 * 1000);
        state.active_agent_tasks.insert(id, task);
    }
    let rows = state.background_agent_rows();
    assert_eq!(rows.len(), MAX_BG_AGENT_ROWS);
    // 上限内仍然保持升序
    for pair in rows.windows(2) {
        assert!(pair[0].started_at <= pair[1].started_at);
    }
    assert!(
        !rows.iter().any(|i| i.task_id == "x0"),
        "最老的一个应当被上限挤掉"
    );
}

/// 进入/退出详情视图只动 `viewing_agent_task_id`，不碰选择器焦点。
#[test]
fn teammate_view_toggles_without_moving_selection() {
    let mut state = ChatState::new();
    state
        .active_agent_tasks
        .insert("a".to_string(), bg_task("a", true, 10));
    state.bg_agent_selection = Some(1);

    state.enter_teammate_view("a");
    assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));
    assert_eq!(state.bg_agent_selection, Some(1), "焦点必须留在原行");

    // 重复进入同一个 task 是幂等的
    state.enter_teammate_view("a");
    assert_eq!(state.viewing_agent_task_id.as_deref(), Some("a"));

    state.exit_teammate_view();
    assert!(state.viewing_agent_task_id.is_none());
    assert_eq!(state.bg_agent_selection, Some(1));
}

/// 左键拖选松开发生在下一次渲染之后。此时流式条目的缓存必须仍然可取，
/// 否则 `get_selected_text()` 拿不到选中文本，无法复制。
#[test]
fn selected_text_survives_render_after_drag() {
    let mut state = ChatState::new();
    state
        .chat_history
        .push(ChatEntry::assistant("hello streaming world"));

    let line = ratatui::text::Line::from(vec![
        ratatui::text::Span::raw("  "),
        ratatui::text::Span::raw("hello streaming world"),
    ]);
    state.rendered_cache.insert(0, (1, vec![line]));
    state.text_selection.start_selection(0, 0, 2);
    state.text_selection.update_selection(0, 0, 23);

    assert_eq!(
        state.get_selected_text().as_deref(),
        Some("hello streaming world")
    );
}

/// 可视列锚点在 CJK 文本上必须先换算成字符下标再切片：
/// 鼠标列是可视列（中文一格占 2 列），直接当字符数用会只复制到一半。
#[test]
fn selected_text_handles_cjk_visual_columns() {
    let mut state = ChatState::new();
    state.chat_history.push(ChatEntry::assistant(""));

    let line = ratatui::text::Line::from(vec![
        ratatui::text::Span::raw("  "),
        ratatui::text::Span::raw("分析问题"),
    ]);
    state.rendered_cache.insert(0, (1, vec![line]));

    // 选中可视列 2..=9：覆盖全部 4 个中文字符
    state.text_selection.start_selection(0, 0, 2);
    state.text_selection.update_selection(0, 0, 9);
    assert_eq!(state.get_selected_text().as_deref(), Some("分析问题"));

    // 选中可视列 2..=5：只覆盖前两个字符
    state.text_selection.start_selection(0, 0, 2);
    state.text_selection.update_selection(0, 0, 5);
    assert_eq!(state.get_selected_text().as_deref(), Some("分析"));
}

/// 折叠切换：读写两端必须是同一份逻辑。
///
/// 以前 `tool_render.rs`（读）、`mouse.rs`（单击写）、`input.rs`（Tab 写）
/// 各写一遍，读的地方算 `default_expanded ^ toggled`，写的地方直接增删
/// `expanded_tool_call_ids` —— 收尾动作还不一致（Tab 会标脏 virtual_list，
/// 单击不会）。
#[test]
fn toggling_a_tool_block_is_observed_by_the_render_path() {
    let mut state = ChatState::new();
    let tool_call = crate::types::StarToolCall {
        id: "call-1".to_string(),
        call_type: "function".to_string(),
        function: crate::types::StarToolCallFunction {
            name: "Read".to_string(),
            arguments: "{}".to_string(),
        },
    };
    // ToolResult + Read（非编辑类工具）→ 默认折叠
    state.chat_history.push(ChatEntry::tool_result(
        "",
        tool_call,
        crate::types::ToolResult {
            success: true,
            output: Some("ok".to_string()),
            error: None,
            data: None,
            llm_content: None,
        },
    ));
    // `ChatState::new()` 里已经塞了一条欢迎语，占 index 0。
    let idx = state.chat_history.len() - 1;

    let entry = &state.chat_history[idx];
    assert!(
        !state.is_tool_expanded(entry),
        "非编辑类工具的 ToolResult 默认折叠"
    );

    assert!(state.toggle_tool_expansion(idx));
    assert!(state.is_tool_expanded(&state.chat_history[idx]));

    assert!(state.toggle_tool_expansion(idx));
    assert!(!state.is_tool_expanded(&state.chat_history[idx]));
}

/// 折叠状态翻转后必须让渲染缓存失效，否则画面上还是旧的展开状态。
#[test]
fn toggling_invalidates_the_render_cache() {
    let mut state = ChatState::new();
    let tool_call = crate::types::StarToolCall {
        id: "call-2".to_string(),
        call_type: "function".to_string(),
        function: crate::types::StarToolCallFunction {
            name: "Grep".to_string(),
            arguments: "{}".to_string(),
        },
    };
    state.chat_history.push(ChatEntry::tool_call("", tool_call));
    let idx = state.chat_history.len() - 1;
    state
        .rendered_cache
        .insert(idx, (1, vec![ratatui::text::Line::from("x")]));

    assert!(state.toggle_tool_expansion(idx));
    assert!(
        !state.rendered_cache.contains_key(&idx),
        "切换后渲染缓存必须失效"
    );
}

/// 没有工具调用的条目不是可折叠对象。
#[test]
fn an_entry_without_a_tool_call_cannot_be_toggled() {
    let mut state = ChatState::new();
    state.chat_history.push(ChatEntry::assistant("hello"));
    assert!(!state.toggle_tool_expansion(0));
    assert!(!state.toggle_tool_expansion(99));
}

#[test]
fn discarding_a_runtime_mutation_rebuilds_requested_intent_from_remaining_queue() {
    let mut state = ChatState::new();
    state.current_model = "active-model".to_string();
    state.current_provider_id = Some("active-provider".to_string());
    let first = state.begin_runtime_settings_update_with_persistence(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );
    let second = state.begin_runtime_settings_update_with_persistence(
        None,
        None,
        Some(crate::types::ThinkingEffort::High),
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );

    state.discard_runtime_settings_update(second.ui_revision);

    assert_eq!(state.pending_runtime_settings.len(), 1);
    assert_eq!(
        state.pending_runtime_settings[0].ui_revision,
        first.ui_revision
    );
    let requested = state.requested_runtime_settings.as_ref().unwrap();
    assert_eq!(
        requested.context_window,
        crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000)
    );
    assert_eq!(requested.thinking_effort, crate::types::ThinkingEffort::Off);
}
