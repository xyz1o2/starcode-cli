//! handle_stream_update 的测试。
//!
//! 单独成文件：这个模块两千多行，比被测代码本身还大。
//!
use super::*;

use super::*;
use crate::types::StarToolCallFunction;

/// 打开模型面板前应先用磁盘缓存把列表填上，而不是先画一个假 loading。
///
/// 回归点：列表为空时 UI 立刻置 `awaiting_models`，用户每次开面板都被闪一下
/// "Loading models..."，可 `ListModels { force: false }` 走的是便宜路径，
/// 命中磁盘缓存时几乎立刻返回 —— 那个转圈状态并不代表任何真实的网络等待。
#[test]
fn seeding_from_disk_cache_fills_the_list_before_any_request() {
    let path = std::env::temp_dir().join(format!(
        "star-seed-models-{}-seedtest.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let cached = vec![
        crate::types::ModelInfo::new("space-bunny-free", "opencode"),
        crate::types::ModelInfo::new("deepseek-v4-pro", "opencode"),
    ];
    crate::agent::model_cache::save_to_path(&path, "opencode", &cached);

    let mut state = ChatState::new();
    assert!(
        state.available_models.is_empty(),
        "precondition: list starts empty"
    );

    let hit = crate::agent::model_cache::load_from_path(&path, "opencode")
        .expect("cache written a moment ago must load");
    apply_models_to_state(&mut state, &hit.models, Some(hit.age_secs));

    assert_eq!(
        state.available_models,
        vec![
            "space-bunny-free".to_string(),
            "deepseek-v4-pro".to_string()
        ],
        "panel must be able to render a real list immediately"
    );
    assert_eq!(
        state.model_provider_map.get("deepseek-v4-pro"),
        Some(&"opencode".to_string()),
        "provider map must be populated too"
    );
    // 有缓存可显示时就不该再标 loading，否则又会闪一下假转圈；
    // 面板上的 "List cached Xh ago" 也依赖这个年龄。
    assert!(
        state.models_list_age_secs().is_some(),
        "cache age must be recorded so the panel can say how old the list is"
    );

    let _ = std::fs::remove_file(&path);
}

/// 已有列表时不该被磁盘缓存覆盖 —— 用户刚拉过的新列表比旧缓存新。
#[test]
fn seeding_never_overwrites_an_existing_list() {
    let path = std::env::temp_dir().join(format!(
        "star-seed-models-{}-nooverwrite.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    crate::agent::model_cache::save_to_path(
        &path,
        "opencode",
        &[crate::types::ModelInfo::new("stale-model", "opencode")],
    );

    let mut state = ChatState::new();
    apply_models_to_state(
        &mut state,
        &[crate::types::ModelInfo::new("fresh-model", "opencode")],
        None,
    );

    assert!(
        !seed_models_from_disk_cache(&mut state, "opencode"),
        "seeding must report no-op when a list is already present, \
             and must not consult the real user cache file in a test"
    );
    assert_eq!(state.available_models, vec!["fresh-model".to_string()]);

    let _ = std::fs::remove_file(&path);
}

fn agent_tool_call() -> StarToolCall {
    StarToolCall {
        id: "agent-call".to_string(),
        call_type: "function".to_string(),
        function: StarToolCallFunction {
            name: "Agent".to_string(),
            arguments: r#"{"description":"scan"}"#.to_string(),
        },
    }
}

fn async_agent_launch_result() -> ToolResult {
    ToolResult {
        success: true,
        output: Some(String::new()),
        error: None,
        data: Some(serde_json::json!({
            "status": "async_launched",
            "agent_id": "worker-1",
            "agent_type": "Explore",
            "description": "scan",
        })),
        llm_content: None,
    }
}

fn runtime_acknowledgement(
    ui_revision: u64,
    outcome: crate::core::context_policy::RuntimeSettingOutcome,
    requested_context: crate::core::context_policy::ContextWindowSelection,
    effective_tokens: u32,
) -> RuntimeSettingsAcknowledgement {
    use crate::core::context_policy::{ContextWindowSource, ResolvedContextPolicy};
    use crate::runtime::messages::{
        ActiveRuntimeSettings, RequestedRuntimeSettings, RuntimeModelSelection,
        RuntimeSettingsSnapshot,
    };

    let policy = ResolvedContextPolicy {
        selection: requested_context,
        nominal_tokens: requested_context.as_fixed().unwrap_or(effective_tokens),
        effective_tokens,
        source: ContextWindowSource::FixedSelection,
        outcome,
        reason: (outcome == crate::core::context_policy::RuntimeSettingOutcome::Degraded)
            .then(|| "provider cap".to_string()),
        pre_send_threshold: effective_tokens * 80 / 100,
        budget_nudge_threshold: effective_tokens * 90 / 100,
        compact_max_tokens: effective_tokens * 75 / 100,
        compact_target_tokens: effective_tokens / 2,
    };
    let model = RuntimeModelSelection {
        model: "test-model".to_string(),
        provider_id: Some("test-provider".to_string()),
    };
    RuntimeSettingsAcknowledgement {
        ui_revision,
        outcome,
        snapshot: RuntimeSettingsSnapshot {
            runtime_revision: ui_revision,
            requested: RequestedRuntimeSettings {
                model: model.clone(),
                context_window: requested_context,
                thinking_effort: crate::types::ThinkingEffort::Medium,
            },
            active: ActiveRuntimeSettings {
                model,
                context_policy: policy,
                thinking_effort: crate::types::ThinkingEffort::Medium,
            },
        },
        reason: None,
    }
}

fn confirmed_context_tokens(state: &ChatState) -> u32 {
    state
        .confirmed_runtime_settings
        .as_ref()
        .expect("confirmed runtime settings")
        .active
        .context_policy
        .effective_tokens
}

/// 回归：模型切换失败时状态栏必须**指名道姓**。
///
/// 真实事故：用户在模型面板选了模型，什么都没发生，状态栏也没给出任何线索 ——
/// 只在日志深处有一条没人会看的记录。原来的文案是泛泛的
/// "Runtime settings not applied"，用户看不出是模型没切过去，更看不出为什么。
///
/// 判据用 worker 报回来的 `requested` vs `active`：切换没成时前者停在用户选的
/// 模型、后者仍是原来的那个。
#[tokio::test]
async fn a_failed_model_switch_says_which_model_and_why() {
    use crate::core::context_policy::{
        ContextWindowSelection, ContextWindowSource, ResolvedContextPolicy, RuntimeSettingOutcome,
    };

    let mut state = ChatState::new();
    let selection = ContextWindowSelection::default();

    // requested = 用户选的 deepseek；active = 仍然生效的旧模型 —— 切换没成。
    let mut ack = runtime_acknowledgement(1, RuntimeSettingOutcome::Failed, selection, 100_000);
    ack.reason = Some("Provider 'magpie' is not configured.".to_string());
    ack.snapshot.requested.model.model = "deepseek-v4.1-flash".to_string();
    ack.snapshot.requested.model.provider_id = Some("magpie".to_string());
    ack.snapshot.active.model.model = "some-other-model".to_string();
    ack.snapshot.active.model.provider_id = Some("yunjunet".to_string());

    apply_runtime_settings_acknowledgement(&mut state, ack);

    let line = state.current_status_line.expect("失败必须留下状态栏文案");
    assert!(
        line.contains("deepseek-v4.1-flash"),
        "必须点名没切过去的那个模型：{line}"
    );
    assert!(line.contains("magpie"), "必须给出原因：{line}");
}

/// 反向：切换成功（requested == active）时不该被说成"模型切换失败"。
/// context / thinking 之类设置出问题仍走原来那条泛化文案。
#[tokio::test]
async fn a_failed_context_setting_is_not_reported_as_a_model_switch_failure() {
    use crate::core::context_policy::{ContextWindowSelection, RuntimeSettingOutcome};

    let mut state = ChatState::new();
    let mut ack = runtime_acknowledgement(
        1,
        RuntimeSettingOutcome::Failed,
        ContextWindowSelection::default(),
        100_000,
    );
    ack.reason = Some("Context window size must be greater than zero.".to_string());

    apply_runtime_settings_acknowledgement(&mut state, ack);

    let line = state.current_status_line.expect("失败必须留下状态栏文案");
    assert!(
        !line.contains("test-model"),
        "模型其实没变，不该被说成模型切换失败：{line}"
    );
    assert!(line.contains("Runtime settings not applied"), "{line}");
}

#[tokio::test]
async fn initial_runtime_acknowledgement_hydrates_active_settings() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let acknowledgement = runtime_acknowledgement(
        0,
        crate::core::context_policy::RuntimeSettingOutcome::Applied,
        crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000),
        1_000_000,
    );

    handle_stream_update(
        &mut state,
        StreamMessage::RuntimeSettingsAcknowledged(acknowledgement),
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.current_model, "test-model");
    assert_eq!(state.thinking_effort, crate::types::ThinkingEffort::Medium);
    assert_eq!(
        state
            .confirmed_runtime_settings
            .as_ref()
            .expect("initial snapshot")
            .active
            .context_policy
            .effective_tokens,
        1_000_000
    );
}

#[tokio::test]
async fn configured_providers_update_metadata_without_overwriting_confirmed_runtime_state() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let acknowledgement = runtime_acknowledgement(
        0,
        crate::core::context_policy::RuntimeSettingOutcome::Applied,
        crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000),
        1_000_000,
    );
    handle_stream_update(
        &mut state,
        StreamMessage::RuntimeSettingsAcknowledged(acknowledgement),
        &agent_tx,
    )
    .await
    .unwrap();
    let confirmed = state.confirmed_runtime_settings.clone();
    let model = state.current_model.clone();
    let provider = state.current_provider_id.clone();
    let effort = state.thinking_effort.clone();

    handle_stream_update(
        &mut state,
        StreamMessage::ConfiguredProviders(vec!["disk-provider".to_string()]),
        &agent_tx,
    )
    .await
    .unwrap();

    assert!(state
        .configured_providers
        .iter()
        .any(|provider| provider == "disk-provider"));
    assert_eq!(state.current_model, model);
    assert_eq!(state.current_provider_id, provider);
    assert_eq!(state.thinking_effort, effort);
    assert_eq!(state.confirmed_runtime_settings, confirmed);
}

#[test]
fn acknowledgement_keeps_newer_pending_intent() {
    let mut state = ChatState::new();
    state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
    );
    state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            2_000_000,
        )),
        None,
    );

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            1,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000),
            1_000_000,
        ),
    );

    assert_eq!(state.pending_runtime_settings.len(), 1);
    assert_eq!(
        state
            .requested_runtime_settings
            .as_ref()
            .expect("latest requested intent")
            .context_window,
        crate::core::context_policy::ContextWindowSelection::Fixed(2_000_000)
    );
    assert!(state.confirmed_runtime_settings.is_none());
}

#[test]
fn rejected_acknowledgement_preserves_requested_intent_and_active_snapshot() {
    let mut state = ChatState::new();
    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            0,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );
    let mutation = state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            2_000_000,
        )),
        None,
    );
    state.runtime_settings_persistence = Some(RuntimeSettingsPersistence::Pending {
        ui_revision: mutation.ui_revision,
    });

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Rejected,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert!(state.pending_runtime_settings.is_empty());
    assert_eq!(
        state
            .requested_runtime_settings
            .as_ref()
            .expect("preserved requested intent")
            .context_window,
        crate::core::context_policy::ContextWindowSelection::Fixed(2_000_000)
    );
    assert_eq!(confirmed_context_tokens(&state), 200_000);
    assert!(state.runtime_settings_persistence.is_none());
}

#[test]
fn degraded_acknowledgement_preserves_requested_capacity_and_active_limit() {
    let mut state = ChatState::new();
    let mutation = state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            2_000_000,
        )),
        None,
    );

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Degraded,
            crate::core::context_policy::ContextWindowSelection::Fixed(2_000_000),
            1_000_000,
        ),
    );

    let snapshot = state
        .confirmed_runtime_settings
        .as_ref()
        .expect("degraded active snapshot");
    assert_eq!(
        snapshot.requested.context_window.as_fixed(),
        Some(2_000_000)
    );
    assert_eq!(snapshot.active.context_policy.effective_tokens, 1_000_000);
    assert_eq!(
        state
            .requested_runtime_settings
            .as_ref()
            .expect("requested state")
            .context_window,
        crate::core::context_policy::ContextWindowSelection::Fixed(2_000_000)
    );
    assert_eq!(
        state
            .last_runtime_settings_ack
            .as_ref()
            .map(|ack| ack.outcome),
        Some(crate::core::context_policy::RuntimeSettingOutcome::Degraded)
    );
}

#[test]
fn stale_persistence_and_legacy_acknowledgement_cannot_overwrite_revisioned_state() {
    let mut state = ChatState::new();
    let mutation = state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
    );
    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000),
            1_000_000,
        ),
    );
    apply_runtime_settings_persistence(&mut state, mutation.ui_revision, None);
    assert!(matches!(
        state.runtime_settings_persistence,
        Some(RuntimeSettingsPersistence::Saved { ui_revision: 1 })
    ));

    apply_runtime_settings_persistence(&mut state, 0, Some("old failure".to_string()));
    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            0,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert_eq!(confirmed_context_tokens(&state), 1_000_000);
    assert!(matches!(
        state.runtime_settings_persistence,
        Some(RuntimeSettingsPersistence::Saved { ui_revision: 1 })
    ));
}

#[test]
fn persistence_result_does_not_change_confirmed_runtime_settings() {
    let mut state = ChatState::new();
    let mutation = state.begin_runtime_settings_update(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
    );
    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000),
            1_000_000,
        ),
    );
    let snapshot = state.confirmed_runtime_settings.clone();

    apply_runtime_settings_persistence(
        &mut state,
        mutation.ui_revision,
        Some("disk full".to_string()),
    );

    assert_eq!(state.confirmed_runtime_settings, snapshot);
    assert!(matches!(
        state.runtime_settings_persistence,
        Some(RuntimeSettingsPersistence::Failed { ui_revision: 1, ref reason })
            if reason == "disk full"
    ));
}

#[test]
fn fast_transition_commits_only_on_matching_successful_acknowledgement() {
    use crate::runtime::messages::RuntimeModelSelection;
    use crate::ui::state::store::PendingFastModeTransition;

    let mut state = ChatState::new();
    let mutation = state.begin_runtime_settings_update_with_persistence(
        Some(RuntimeModelSelection {
            model: "fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        }),
        None,
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );
    state.pending_fast_mode_transition = Some(PendingFastModeTransition {
        ui_revision: mutation.ui_revision,
        enabled: true,
        previous_model: Some(RuntimeModelSelection {
            model: "original-model".to_string(),
            provider_id: Some("original-provider".to_string()),
        }),
        target_model: RuntimeModelSelection {
            model: "fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        },
    });

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert!(state.fast_mode);
    assert_eq!(
        state.fast_mode_prev_model,
        Some(RuntimeModelSelection {
            model: "original-model".to_string(),
            provider_id: Some("original-provider".to_string()),
        })
    );
    assert!(state.pending_fast_mode_transition.is_none());
}

#[test]
fn superseded_fast_transition_clears_only_its_transition_and_keeps_newer_intent() {
    use crate::runtime::messages::RuntimeModelSelection;
    use crate::ui::state::store::PendingFastModeTransition;

    let mut state = ChatState::new();
    let fast = state.begin_runtime_settings_update_with_persistence(
        Some(RuntimeModelSelection {
            model: "fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        }),
        None,
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );
    state.pending_fast_mode_transition = Some(PendingFastModeTransition {
        ui_revision: fast.ui_revision,
        enabled: true,
        previous_model: Some(RuntimeModelSelection {
            model: "original-model".to_string(),
            provider_id: Some("original-provider".to_string()),
        }),
        target_model: RuntimeModelSelection {
            model: "fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        },
    });
    let newer = state.begin_runtime_settings_update_with_persistence(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            fast.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Superseded,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert!(!state.fast_mode);
    assert!(state.fast_mode_prev_model.is_none());
    assert!(state.pending_fast_mode_transition.is_none());
    assert_eq!(state.pending_runtime_settings.len(), 1);
    assert_eq!(
        state
            .pending_runtime_settings
            .front()
            .map(|mutation| mutation.ui_revision),
        Some(newer.ui_revision)
    );
    assert_eq!(
        state
            .requested_runtime_settings
            .as_ref()
            .expect("newer requested intent")
            .context_window,
        crate::core::context_policy::ContextWindowSelection::Fixed(1_000_000)
    );
    assert!(state.confirmed_runtime_settings.is_none());
}

#[test]
fn failed_fast_transition_keeps_existing_fast_mode_and_restore_target() {
    use crate::runtime::messages::RuntimeModelSelection;
    use crate::ui::state::store::PendingFastModeTransition;

    let mut state = ChatState::new();
    state.fast_mode = true;
    state.fast_mode_prev_model = Some(RuntimeModelSelection {
        model: "original-model".to_string(),
        provider_id: Some("original-provider".to_string()),
    });
    let mutation = state.begin_runtime_settings_update_with_persistence(
        Some(RuntimeModelSelection {
            model: "new-fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        }),
        None,
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );
    state.pending_fast_mode_transition = Some(PendingFastModeTransition {
        ui_revision: mutation.ui_revision,
        enabled: false,
        previous_model: None,
        target_model: RuntimeModelSelection {
            model: "original-model".to_string(),
            provider_id: Some("original-provider".to_string()),
        },
    });

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            mutation.ui_revision,
            crate::core::context_policy::RuntimeSettingOutcome::Failed,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert!(state.fast_mode);
    assert_eq!(
        state.fast_mode_prev_model,
        Some(RuntimeModelSelection {
            model: "original-model".to_string(),
            provider_id: Some("original-provider".to_string()),
        })
    );
    assert!(state.pending_fast_mode_transition.is_none());
}

#[test]
fn stale_acknowledgement_cannot_commit_a_fast_transition() {
    use crate::runtime::messages::RuntimeModelSelection;
    use crate::ui::state::store::PendingFastModeTransition;

    let mut state = ChatState::new();
    let newer = state.begin_runtime_settings_update_with_persistence(
        None,
        Some(crate::core::context_policy::ContextWindowSelection::Fixed(
            1_000_000,
        )),
        None,
        crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
    );
    state.pending_fast_mode_transition = Some(PendingFastModeTransition {
        ui_revision: newer.ui_revision,
        enabled: true,
        previous_model: None,
        target_model: RuntimeModelSelection {
            model: "fast-model".to_string(),
            provider_id: Some("fast-provider".to_string()),
        },
    });
    state.last_runtime_settings_ack = Some(runtime_acknowledgement(
        newer.ui_revision,
        crate::core::context_policy::RuntimeSettingOutcome::Applied,
        crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
        200_000,
    ));

    apply_runtime_settings_acknowledgement(
        &mut state,
        runtime_acknowledgement(
            newer.ui_revision.saturating_sub(1),
            crate::core::context_policy::RuntimeSettingOutcome::Applied,
            crate::core::context_policy::ContextWindowSelection::Fixed(200_000),
            200_000,
        ),
    );

    assert!(!state.fast_mode);
    assert_eq!(
        state
            .pending_fast_mode_transition
            .as_ref()
            .map(|transition| transition.ui_revision),
        Some(newer.ui_revision)
    );
}

#[test]
fn async_agent_launch_remaps_indices_before_follow_up_updates() {
    let mut state = ChatState::new();
    state.chat_history.clear();
    state.chat_history.push(ChatEntry::assistant("before"));
    state
        .chat_history
        .push(ChatEntry::tool_call("Agent", agent_tool_call()).with_streaming(true));
    state
        .chat_history
        .push(ChatEntry::new(ChatEntryType::ToolConfirmation, String::new()).with_streaming(false));
    state
        .chat_history
        .push(ChatEntry::assistant("after").with_streaming(true));
    state.stream_targets.insert(42, 3);
    state.message_start_indices.insert(42, 1);
    state.pending_confirmation_entry_idx = Some(2);
    state.preview_last_entry_idx = Some(3);
    state.chat_list_state.select(Some(3));
    state.expanded_thinking_indices.insert(3);
    state.last_item_heights = vec![5, 7, 11, 13];
    state.virtual_list.resize(4);
    for (idx, height) in [5, 7, 11, 13].into_iter().enumerate() {
        state.virtual_list.set_height(idx, height);
    }
    state.total_rendered_lines = state.virtual_list.total_lines();
    state.rendered_cache.insert(1, (7, Vec::new()));
    state.rendered_cache.insert(3, (13, Vec::new()));
    state.last_rendered_stream_key.insert(3, (1, 2));
    state.streaming_height_floor.insert(3, 13);

    handle_tool_result_message(
        &mut state,
        42,
        agent_tool_call(),
        async_agent_launch_result(),
    );

    assert_eq!(state.chat_history.len(), 4);
    assert_eq!(state.stream_targets.get(&42), Some(&4));
    assert_eq!(state.message_start_indices.get(&42), Some(&1));
    assert_eq!(state.pending_confirmation_entry_idx, Some(1));
    assert_eq!(state.preview_last_entry_idx, Some(2));
    assert_eq!(state.chat_list_state.selected(), Some(2));
    assert_eq!(
        state.expanded_thinking_indices,
        std::collections::HashSet::from([2])
    );
    assert_eq!(state.last_item_heights, vec![5, 11, 13]);
    assert_eq!(state.virtual_list.len(), 3);
    assert_eq!(state.total_rendered_lines, 29);
    assert!(!state.rendered_cache.contains_key(&1));
    assert!(state.rendered_cache.contains_key(&2));
    assert_eq!(state.last_rendered_stream_key.get(&2), Some(&(1, 2)));
    assert_eq!(state.streaming_height_floor.get(&2), Some(&13));
    assert_eq!(state.active_agent_tasks["worker-1"].entry_idx, 3);
    assert_eq!(state.chat_history[3].entry_type, ChatEntryType::AgentTask);

    handle_content_message(&mut state, 42, " updated");
    assert_eq!(state.chat_history[4].content, " updated");
    assert_eq!(state.chat_history[2].content, "after");

    handle_agent_task_update(
        &mut state,
        AgentTaskUpdateArgs {
            task_id: "worker-1".to_string(),
            agent_type: "Explore".to_string(),
            description: "scan".to_string(),
            status: crate::types::AgentTaskStatus::Running,
            tool_use_count: 1,
            tokens: 12,
            is_async: true,
            is_resolved: false,
            is_error: false,
            last_tool_info: Some("Reading config".to_string()),
            name: None,
            task_description: Some("scan".to_string()),
            new_sub_entries: Vec::new(),
        },
    );

    assert_eq!(state.active_agent_tasks["worker-1"].entry_idx, 3);
    assert_eq!(state.chat_history[3].agent_tool_use_count, Some(1));
    assert_eq!(
        state.chat_history[3].agent_last_tool_info.as_deref(),
        Some("Reading config")
    );
}

#[tokio::test]
async fn reasoning_delta_refreshes_stream_liveness() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let message_id = 7;
    state.stream_targets.insert(message_id, 0);
    state.last_token_time = Some(std::time::Instant::now() - std::time::Duration::from_secs(31));

    handle_stream_update(
        &mut state,
        StreamMessage::ReasoningDelta {
            message_id,
            content: "Checking the implementation".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();

    let elapsed = state.last_token_time.unwrap().elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1));
    assert_eq!(
        state
            .chat_history
            .last()
            .unwrap()
            .reasoning_content
            .as_deref(),
        Some("Checking the implementation")
    );
}

#[tokio::test]
async fn active_tool_output_and_results_preserve_parallel_lifecycle() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let message_id = 9;
    state.stream_targets.insert(message_id, 0);
    let first = StarToolCall {
        id: "call-a".to_string(),
        call_type: "function".to_string(),
        function: StarToolCallFunction {
            name: "Read".to_string(),
            arguments: "{}".to_string(),
        },
    };
    let second = StarToolCall {
        id: "call-b".to_string(),
        call_type: "function".to_string(),
        function: StarToolCallFunction {
            name: "Bash".to_string(),
            arguments: "{}".to_string(),
        },
    };

    handle_stream_update(
        &mut state,
        StreamMessage::ToolCalls {
            message_id,
            tool_calls: vec![first.clone(), second.clone()],
        },
        &agent_tx,
    )
    .await
    .unwrap();
    state.last_token_time = Some(Instant::now() - std::time::Duration::from_secs(31));

    handle_stream_update(
        &mut state,
        StreamMessage::ToolOutput {
            message_id,
            tool_call_id: second.id.clone(),
            output: "still running".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert!(state.last_token_time.unwrap().elapsed() < std::time::Duration::from_secs(1));

    let success = ToolResult {
        success: true,
        output: None,
        error: None,
        data: None,
        llm_content: None,
    };
    handle_tool_result_message(&mut state, message_id, first.clone(), success.clone());
    assert!(!state.tool_started_at.contains_key(&first.id));
    assert!(state.tool_started_at.contains_key(&second.id));
    assert_eq!(state.current_tool_name.as_deref(), Some("Bash"));
    assert!(state.chat_history.iter().any(|entry| {
        entry.tool_call.as_ref().map(|call| call.id.as_str()) == Some(second.id.as_str())
            && entry.is_streaming == Some(true)
    }));

    handle_tool_result_message(&mut state, message_id, second.clone(), success);
    assert!(state.tool_started_at.is_empty());
    assert_eq!(state.current_tool_name, None);
}

#[tokio::test]
async fn global_search_results_apply_only_to_open_current_request() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    state.open_global_search();
    state.global_search_state.begin_request(7);

    handle_stream_update(
        &mut state,
        StreamMessage::GlobalSearchResults {
            request_id: 7,
            results: vec![crate::runtime::messages::GlobalSearchMatch {
                file: "src/lib.rs".to_string(),
                line_number: 3,
                content: "needle".to_string(),
                score: 0,
            }],
            truncated: false,
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.global_search_state.results.len(), 1);
    assert_eq!(state.global_search_state.results[0].file, "src/lib.rs");
    assert!(!state.global_search_state.is_searching);

    state.global_search_state.begin_request(8);
    handle_stream_update(
        &mut state,
        StreamMessage::GlobalSearchResults {
            request_id: 7,
            results: Vec::new(),
            truncated: true,
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.global_search_state.active_request_id, Some(8));
    assert_eq!(state.global_search_state.results.len(), 1);
    assert!(!state.global_search_state.truncated);

    state.pop_modal();
    handle_stream_update(
        &mut state,
        StreamMessage::GlobalSearchResults {
            request_id: 8,
            results: Vec::new(),
            truncated: true,
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.global_search_state.results.len(), 1);
    assert!(!state.global_search_state.truncated);
}

#[tokio::test]
async fn confirmation_feedback_draft_resets_for_new_request_and_matching_result() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    state.confirmation_feedback_mode = true;
    state.pending_confirmation_feedback = "stale draft".to_string();
    let confirmation = crate::types::ToolConfirmation {
        tool_name: "Bash".to_string(),
        operation_type: crate::types::ConfirmationType::Generic,
        details: crate::types::ConfirmationDetails::Generic {
            title: "Confirm".to_string(),
            prompt: "Continue?".to_string(),
        },
        is_dangerous: false,
        outcome: None,
    };

    handle_stream_update(
        &mut state,
        StreamMessage::ToolConfirmationRequest {
            message_id: 1,
            tool_call_id: "corr-1".to_string(),
            originating_tool_call_id: Some("call-1".to_string()),
            confirmation,
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert!(!state.confirmation_feedback_mode);
    assert!(state.pending_confirmation_feedback.is_empty());

    state.confirmation_feedback_mode = true;
    state.pending_confirmation_feedback = "result draft".to_string();
    handle_tool_result_message(
        &mut state,
        1,
        agent_tool_call(),
        ToolResult {
            success: true,
            output: None,
            error: None,
            data: None,
            llm_content: None,
        },
    );
    assert!(state.confirmation_feedback_mode);
    assert_eq!(state.pending_confirmation_feedback, "result draft");

    handle_tool_result_message(
        &mut state,
        1,
        StarToolCall {
            id: "call-1".to_string(),
            ..agent_tool_call()
        },
        ToolResult {
            success: true,
            output: None,
            error: None,
            data: None,
            llm_content: None,
        },
    );
    assert!(!state.confirmation_feedback_mode);
    assert!(state.pending_confirmation_feedback.is_empty());
}

/// 回归：工具结束了，等待确认的卡片必须跟着收掉。
///
/// 之前这里比的是 `pending_tool_call_id`（bus 的 correlation id，UUID）和
/// `tool_call.id`（模型侧 id），两个命名空间永远比不相等。于是确认过期 /
/// 执行失败之后 `is_awaiting_confirmation` 一直为真：按键全被喂给一张已经
/// 没有接收者的卡片，提示框也打不了字，后面每一次确认请求都会被当成
/// "已有一张卡"而被顶掉。
#[tokio::test]
async fn tool_result_for_the_awaited_call_clears_the_pending_confirmation() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);

    handle_stream_update(
        &mut state,
        StreamMessage::ToolConfirmationRequest {
            message_id: 1,
            tool_call_id: "corr-1".to_string(),
            originating_tool_call_id: Some("call-1".to_string()),
            confirmation: shell_confirmation("grep -n 604800 dump.sql"),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert!(state.is_awaiting_confirmation);

    // 别的工具先结束，不该动这张卡。
    handle_tool_result_message(
        &mut state,
        1,
        StarToolCall {
            id: "call-other".to_string(),
            ..agent_tool_call()
        },
        ToolResult {
            success: true,
            output: None,
            error: None,
            data: None,
            llm_content: None,
        },
    );
    assert!(state.is_awaiting_confirmation, "无关工具的结果不该收掉卡片");

    handle_tool_result_message(
        &mut state,
        1,
        StarToolCall {
            id: "call-1".to_string(),
            ..agent_tool_call()
        },
        ToolResult {
            success: false,
            output: None,
            error: Some("Confirmation timeout or error".to_string()),
            data: None,
            llm_content: None,
        },
    );
    assert!(
        !state.is_awaiting_confirmation,
        "自己那次调用的结果必须收掉卡片"
    );
    assert!(state.pending_tool_call_id.is_none());
    assert!(state.pending_confirmation_originating_id.is_none());
}

/// 回归：新的确认请求不许把用户正在看的那张卡换成别的命令。
///
/// 之前是把旧卡的内容就地替换成新请求：用户照着刚读到的那条命令按"1"，
/// 批准到的却是另一条命令 —— 门控拦不住的不是命令，是"用户批准的那一条"
/// 和"实际执行的那一条"之间的对应关系。
#[tokio::test]
async fn a_new_confirmation_never_retargets_a_pending_card() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);

    handle_stream_update(
        &mut state,
        StreamMessage::ToolConfirmationRequest {
            message_id: 1,
            tool_call_id: "corr-1".to_string(),
            originating_tool_call_id: Some("call-1".to_string()),
            confirmation: shell_confirmation("grep -n 604800 dump.sql"),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    let first_idx = state
        .pending_confirmation_entry_idx
        .expect("第一张卡应当是待确认状态");

    handle_stream_update(
        &mut state,
        StreamMessage::ToolConfirmationRequest {
            message_id: 1,
            tool_call_id: "corr-2".to_string(),
            originating_tool_call_id: Some("call-2".to_string()),
            confirmation: shell_confirmation("ls -la /mnt/d/ai-workspace"),
        },
        &agent_tx,
    )
    .await
    .unwrap();

    // 旧卡必须原样保留，且不再是可以回答的那一张。
    let first = &state.chat_history[first_idx];
    let first_prompt = match &first.confirmation.as_ref().expect("确认卡").details {
        crate::types::ConfirmationDetails::Generic { prompt, .. } => prompt.clone(),
        other => panic!("期望 Generic 确认卡，实际 {other:?}"),
    };
    assert!(
        first_prompt.contains("grep -n 604800"),
        "旧卡的内容不能被新请求换掉：{first_prompt}"
    );
    assert!(
        first
            .confirmation
            .as_ref()
            .expect("确认卡")
            .outcome
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains("queued"),
        "被顶下去的卡必须标成排队中，不能看起来还能按"
    );
    assert_eq!(
        state.pending_confirmation_queue.len(),
        1,
        "被顶下去的请求必须留在队列里，否则它的门禁会永远等"
    );

    // 新的那张才是当前等待回答的。
    assert_eq!(state.pending_tool_call_id.as_deref(), Some("corr-2"));
    assert_eq!(
        state.pending_confirmation_originating_id.as_deref(),
        Some("call-2")
    );
    assert_ne!(
        state.pending_confirmation_entry_idx,
        Some(first_idx),
        "新请求必须另起一张卡，不能复用旧的索引"
    );

    // 回答完新的那张，旧请求必须接上 —— 不能像以前那样被丢掉。
    state.clear_pending_confirmation();
    assert!(
        state.is_awaiting_confirmation,
        "队列里还有请求时必须继续等待回答"
    );
    assert_eq!(state.pending_tool_call_id.as_deref(), Some("corr-1"));
    assert_eq!(state.pending_confirmation_entry_idx, Some(first_idx));
    assert!(
        state.chat_history[first_idx]
            .confirmation
            .as_ref()
            .expect("确认卡")
            .outcome
            .is_none(),
        "重新激活的卡必须恢复成可回答状态"
    );
}

/// 三张并发确认卡必须一张不漏地问完。
///
/// 工具批量执行时每张卡背后都有一个门禁在等答复；UI 一次只问一张，
/// 其余必须排队。旧实现把被顶下去的卡标成 superseded 后直接丢弃，
/// 那些门禁就永远等不到答复 —— 整轮卡在 "Canoodling..."。
#[tokio::test]
async fn concurrent_confirmations_are_asked_in_turn_without_losing_any() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);

    for i in 1..=3 {
        handle_stream_update(
            &mut state,
            StreamMessage::ToolConfirmationRequest {
                message_id: 1,
                tool_call_id: format!("corr-{i}"),
                originating_tool_call_id: Some(format!("call-{i}")),
                confirmation: shell_confirmation(&format!("echo {i}")),
            },
            &agent_tx,
        )
        .await
        .unwrap();
    }

    assert_eq!(state.pending_tool_call_id.as_deref(), Some("corr-3"));
    assert_eq!(
        state.pending_confirmation_queue.len(),
        2,
        "另外两张必须排队，不能丢"
    );

    let mut asked = Vec::new();
    while state.is_awaiting_confirmation {
        asked.push(state.pending_tool_call_id.clone().unwrap_or_default());
        state.clear_pending_confirmation();
    }

    assert_eq!(
        asked,
        vec!["corr-3", "corr-2", "corr-1"],
        "最近被顶下去的那张先答，三张都要轮到"
    );
    assert!(state.pending_confirmation_queue.is_empty());
}

/// 排队中的请求如果先等到工具结果（中断/取消），必须从队列摘掉，
/// 不能再被激活成一张没有接收者的卡。
#[tokio::test]
async fn a_tool_result_removes_its_queued_confirmation() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);

    for i in 1..=2 {
        handle_stream_update(
            &mut state,
            StreamMessage::ToolConfirmationRequest {
                message_id: 1,
                tool_call_id: format!("corr-{i}"),
                originating_tool_call_id: Some(format!("call-{i}")),
                confirmation: shell_confirmation(&format!("echo {i}")),
            },
            &agent_tx,
        )
        .await
        .unwrap();
    }
    assert_eq!(state.pending_confirmation_queue.len(), 1);

    handle_tool_result_message(
        &mut state,
        1,
        StarToolCall {
            id: "call-1".to_string(),
            ..agent_tool_call()
        },
        ToolResult {
            success: false,
            output: None,
            error: Some("cancelled".to_string()),
            data: None,
            llm_content: None,
        },
    );

    assert!(
        state.pending_confirmation_queue.is_empty(),
        "已经有结果的请求不该继续排队"
    );
    assert_eq!(
        state.pending_tool_call_id.as_deref(),
        Some("corr-2"),
        "当前这张不受影响"
    );
}

fn shell_confirmation(command: &str) -> crate::types::ToolConfirmation {
    crate::types::ToolConfirmation {
        tool_name: "Bash".to_string(),
        operation_type: crate::types::ConfirmationType::Generic,
        details: crate::types::ConfirmationDetails::Generic {
            title: "Permission required: Bash".to_string(),
            prompt: format!("`Bash` needs approval before it can run.\n\n{command}"),
        },
        is_dangerous: true,
        outcome: None,
    }
}

#[tokio::test]
async fn cache_warnings_are_transient_deduplicated_and_do_not_touch_history() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let history_len = state.chat_history.len();
    let costly_cache_write = crate::types::StarUsage {
        prompt_tokens: 900,
        completion_tokens: 10,
        total_tokens: 910,
        cache_read_tokens: 100,
        cache_creation_tokens: 20_000,
        cache_telemetry_reported: true,
    };

    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id: 41,
            tokens: 910,
            usage: Some(costly_cache_write.clone()),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.toast_queue.len(), 1);
    assert!(matches!(state.toast_queue[0].kind, ToastKind::Warning));
    assert_eq!(state.chat_history.len(), history_len);

    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id: 41,
            tokens: 910,
            usage: Some(costly_cache_write),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.toast_queue.len(), 1);

    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id: 41,
            tokens: 1_010,
            usage: Some(crate::types::StarUsage {
                prompt_tokens: 100,
                completion_tokens: 10,
                total_tokens: 1_010,
                cache_read_tokens: 900,
                cache_creation_tokens: 30_000,
                cache_telemetry_reported: true,
            }),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.toast_queue.len(), 1);
    assert_eq!(state.chat_history.len(), history_len);
}

#[tokio::test]
async fn latest_provider_usage_replaces_prior_response_and_unknown_cache_clears_counters() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let message_id = 43;
    let first = crate::types::StarUsage {
        prompt_tokens: 900,
        completion_tokens: 10,
        total_tokens: 910,
        cache_read_tokens: 800,
        cache_creation_tokens: 50,
        cache_telemetry_reported: true,
    };
    let second = crate::types::StarUsage {
        prompt_tokens: 20,
        completion_tokens: 5,
        total_tokens: 25,
        cache_read_tokens: 999,
        cache_creation_tokens: 999,
        cache_telemetry_reported: false,
    };

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id,
            kind: StreamStartKind::UserTurn {
                model: "gpt-4o".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: first.total_tokens,
            usage: Some(first.clone()),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: second.total_tokens,
            usage: Some(second.clone()),
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.token_count, 25);
    let usage = state.token_usage.expect("latest usage");
    assert_eq!(usage.prompt_tokens, 20);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 25);
    assert!(!usage.cache_telemetry_reported);
    assert_eq!(state.cache_read_tokens, 0);
    assert_eq!(state.cache_creation_tokens, 0);
    let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
        + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
    assert_eq!(state.response_costs.get(&message_id), Some(&expected));
}

#[tokio::test]
async fn operation_start_preserves_latest_provider_usage_and_cache_counters() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let usage = crate::types::StarUsage {
        prompt_tokens: 100,
        completion_tokens: 10,
        total_tokens: 110,
        cache_read_tokens: 40,
        cache_creation_tokens: 3,
        cache_telemetry_reported: true,
    };
    state.token_count = usage.total_tokens;
    state.token_usage = Some(usage.clone());
    state.cache_read_tokens = usage.cache_read_tokens as u64;
    state.cache_creation_tokens = usage.cache_creation_tokens as u64;

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id: 44,
            kind: StreamStartKind::Operation,
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.token_count, 110);
    let restored = state.token_usage.as_ref().expect("usage is preserved");
    assert_eq!(restored.total_tokens, usage.total_tokens);
    assert_eq!(restored.cache_read_tokens, usage.cache_read_tokens);
    assert_eq!(restored.cache_creation_tokens, usage.cache_creation_tokens);
    assert!(restored.cache_telemetry_reported);
    assert_eq!(state.cache_read_tokens, 40);
    assert_eq!(state.cache_creation_tokens, 3);
}

#[tokio::test]
async fn new_request_keeps_usage_as_baseline_while_compression_stats_preserve_it() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let usage = crate::types::StarUsage {
        prompt_tokens: 100,
        completion_tokens: 10,
        total_tokens: 110,
        cache_read_tokens: 40,
        cache_creation_tokens: 3,
        cache_telemetry_reported: true,
    };
    state.token_count = usage.total_tokens;
    state.token_usage = Some(usage.clone());
    state.cache_read_tokens = usage.cache_read_tokens as u64;
    state.cache_creation_tokens = usage.cache_creation_tokens as u64;

    handle_stream_update(
        &mut state,
        StreamMessage::StatsUpdate {
            au2_compressed: true,
            token_usage: Some(crate::types::StarUsage {
                total_tokens: 9_999,
                ..Default::default()
            }),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.token_usage.as_ref().unwrap().total_tokens, 110);
    assert_eq!(state.cache_read_tokens, 40);

    state.cache_warning_shown.insert((
        1,
        crate::agent::cache_warning::CacheWarningType::HighCreationCost,
    ));
    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id: 44,
            kind: StreamStartKind::UserTurn {
                model: "test-model".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.token_count, 0);
    // 上一回合用量保留为本回合基线，直到下次 TokenCount 整体替换：
    // 真实 usage 只在回合末到达，清空它会让 UI 退回字符数估算并横跳。
    assert_eq!(state.token_usage.as_ref().unwrap().total_tokens, 110);
    assert_eq!(state.cache_read_tokens, 40);
    assert_eq!(state.cache_creation_tokens, 3);
    assert!(state.cache_warning_shown.is_empty());
}

#[tokio::test]
async fn terminal_messages_settle_accumulated_cost_once_and_ignore_late_usage() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let message_id = 45;
    let first = crate::types::StarUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        ..Default::default()
    };
    let second = crate::types::StarUsage {
        prompt_tokens: 200,
        completion_tokens: 30,
        total_tokens: 230,
        ..Default::default()
    };
    let late = crate::types::StarUsage {
        prompt_tokens: 999,
        completion_tokens: 999,
        total_tokens: 1_998,
        ..Default::default()
    };

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id,
            kind: StreamStartKind::UserTurn {
                model: "gpt-4o".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TextDelta {
            message_id,
            content: "answer".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    for usage in [&first, &second] {
        handle_stream_update(
            &mut state,
            StreamMessage::TokenCount {
                message_id,
                tokens: usage.total_tokens,
                usage: Some((*usage).clone()),
            },
            &agent_tx,
        )
        .await
        .unwrap();
    }

    let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
        + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
    handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
        .await
        .unwrap();
    assert_eq!(state.total_cost, expected);
    assert_eq!(
        state.chat_history.last().and_then(|entry| entry.cost),
        Some(expected)
    );
    assert!(state.complete_task_message_ids.contains(&message_id));
    assert!(!state.response_costs.contains_key(&message_id));
    assert!(!state.response_models.contains_key(&message_id));

    handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
        .await
        .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: late.total_tokens,
            usage: Some(late),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    assert_eq!(state.total_cost, expected);
    assert_eq!(state.token_count, second.total_tokens);
}

#[tokio::test]
async fn error_settles_accumulated_cost_once() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    let message_id = 46;
    let usage = crate::types::StarUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        ..Default::default()
    };
    let expected = crate::ui::utils::cost::compute_response_cost(&usage, "gpt-4o");

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id,
            kind: StreamStartKind::UserTurn {
                model: "gpt-4o".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: usage.total_tokens,
            usage: Some(usage),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::Error {
            message_id,
            error: "invalid API key".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.total_cost, expected);
    assert_eq!(
        state.chat_history.last().and_then(|entry| entry.cost),
        Some(expected)
    );
    assert!(state.complete_task_message_ids.contains(&message_id));
    handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
        .await
        .unwrap();
    assert_eq!(state.total_cost, expected);
}

#[tokio::test]
async fn auto_continue_retains_accumulated_cost_for_same_message_id() {
    let mut state = ChatState::new();
    let (agent_tx, mut agent_rx) = mpsc::channel(1);
    let message_id = 47;
    let first = crate::types::StarUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        ..Default::default()
    };
    let second = crate::types::StarUsage {
        prompt_tokens: 50,
        completion_tokens: 10,
        total_tokens: 60,
        ..Default::default()
    };
    state.auto_continue_enabled = true;
    state.auto_continue_remaining = 1;

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id,
            kind: StreamStartKind::UserTurn {
                model: "gpt-4o".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TextDelta {
            message_id,
            content: "working".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::ToolCalls {
            message_id,
            tool_calls: vec![StarToolCall {
                id: "call-1".to_string(),
                call_type: "function".to_string(),
                function: StarToolCallFunction {
                    name: "Read".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: first.total_tokens,
            usage: Some(first.clone()),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
        .await
        .unwrap();
    let AgentRequest::SendMessage {
        message_id: continued_id,
        ..
    } = agent_rx.recv().await.expect("auto-continue request")
    else {
        panic!("expected auto-continue request");
    };
    assert_eq!(continued_id, message_id);
    assert_eq!(state.total_cost, 0.0);
    assert!(state.response_costs.contains_key(&message_id));

    handle_stream_update(
        &mut state,
        StreamMessage::Start {
            message_id,
            kind: StreamStartKind::UserTurn {
                model: "different-model".to_string(),
            },
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TextDelta {
            message_id,
            content: " final".to_string(),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id,
            tokens: second.total_tokens,
            usage: Some(second.clone()),
        },
        &agent_tx,
    )
    .await
    .unwrap();
    handle_stream_update(&mut state, StreamMessage::Done { message_id }, &agent_tx)
        .await
        .unwrap();

    let expected = crate::ui::utils::cost::compute_response_cost(&first, "gpt-4o")
        + crate::ui::utils::cost::compute_response_cost(&second, "gpt-4o");
    assert_eq!(state.total_cost, expected);
    assert!(!state.response_costs.contains_key(&message_id));
    assert!(!state.response_models.contains_key(&message_id));
}

#[tokio::test]
async fn reported_zero_cache_telemetry_replaces_prior_cache_counters() {
    let mut state = ChatState::new();
    let (agent_tx, _agent_rx) = mpsc::channel(1);
    state.cache_read_tokens = 500;
    state.cache_creation_tokens = 250;

    handle_stream_update(
        &mut state,
        StreamMessage::TokenCount {
            message_id: 42,
            tokens: 100,
            usage: Some(crate::types::StarUsage {
                prompt_tokens: 100,
                total_tokens: 100,
                cache_telemetry_reported: true,
                ..Default::default()
            }),
        },
        &agent_tx,
    )
    .await
    .unwrap();

    assert_eq!(state.cache_read_tokens, 0);
    assert_eq!(state.cache_creation_tokens, 0);
}

/// 流式期间中插条目（工具块等）不得清掉进行中的拖选：
/// 锚点索引自适应 +1，选区保住（此前直接 clear，表现为“拖选总是断”）。
#[test]
fn insert_shifts_selection_anchors_instead_of_clearing() {
    let mut state = crate::ui::state::ChatState::new();
    state.text_selection.start_selection(1, 0, 0);
    state.text_selection.update_selection(2, 3, 7);

    shift_index_caches_after_insert(&mut state, 2);

    assert_eq!(state.text_selection.start_entry_idx, Some(1));
    assert_eq!(state.text_selection.end_entry_idx, Some(3));
    assert_eq!(state.text_selection.end, Some((3, 7)));
    assert!(state.text_selection.is_selecting, "拖选进行中不得被打断");
}

/// 插入点在选区之前：两个锚点都要 +1。
#[test]
fn insert_before_selection_shifts_both_anchors() {
    let mut state = crate::ui::state::ChatState::new();
    state.text_selection.start_selection(2, 1, 0);
    state.text_selection.update_selection(4, 0, 5);

    shift_index_caches_after_insert(&mut state, 1);

    assert_eq!(state.text_selection.start_entry_idx, Some(3));
    assert_eq!(state.text_selection.end_entry_idx, Some(5));
}

/// 删除条目：锚点命中被删条目才清除，否则索引自适应 -1 保住选区。
#[test]
fn removal_keeps_selection_unless_anchor_entry_is_removed() {
    let mut state = crate::ui::state::ChatState::new();
    // 占位条目撑出足够的长度，避免触发 pos >= len 早退保护
    while state.chat_history.len() < 5 {
        state.chat_history.push(ChatEntry::assistant("pad"));
    }
    state.text_selection.start_selection(1, 0, 0);
    state.text_selection.update_selection(3, 2, 4);

    remove_chat_entry_and_shift_indices(&mut state, 2);
    assert_eq!(state.text_selection.start_entry_idx, Some(1));
    assert_eq!(state.text_selection.end_entry_idx, Some(2));

    remove_chat_entry_and_shift_indices(&mut state, 1);
    assert!(
        state.text_selection.start.is_none(),
        "锚点条目被删时选区必须清除"
    );
}
