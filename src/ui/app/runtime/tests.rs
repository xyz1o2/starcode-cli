//! UI 循环里那几个纯函数的测试。
//!
use super::*;

use super::*;
use crate::types::ChatEntry;

#[test]
fn processing_watchdog_waits_for_recorded_tools() {
    let mut state = ChatState::new();
    state.is_processing = true;
    state.last_token_time = Some(Instant::now() - Duration::from_secs(31));
    assert_eq!(
        stalled_processing_secs_without_active_tool(&state),
        None,
        "登记在册的工具由执行器负责生命周期，UI 不该替它宣布超时"
    );

    state.tool_started_at.insert(
        "call-1".to_string(),
        Instant::now() - Duration::from_secs(31),
    );
    assert_eq!(stalled_processing_secs_without_active_tool(&state), None);

    state.tool_started_at.clear();
    state.is_processing = false;
    assert_eq!(stalled_processing_secs_without_active_tool(&state), None);

    state.is_processing = true;
    state.last_token_time = None;
    assert_eq!(stalled_processing_secs_without_active_tool(&state), None);
}

/// 回归：等待模型出首 token **不能**被判成"回合已结束"。
///
/// 看门狗此前卡在 30s，于是每个慢回合都会在第 30 秒被 UI 判死：`is_processing`
/// 被清掉 → spinner 停转，`current_status_line` 被写成 "✓ Done (timeout)" →
/// 界面上什么都不剩，而 agent 还在跑。用户看到的正是"任务还在运行中，指示却
/// 消失了"。30s 是模型出首 token 的正常量级（agent 侧自己的请求超时默认就有
/// 120s），拿它当"agent 死了"的判据必然误杀。
#[test]
fn slow_first_token_does_not_release_the_spinner() {
    let mut state = ChatState::new();
    state.is_processing = true;

    for silent_secs in [5u64, 31, 90, 240] {
        state.last_token_time = Some(Instant::now() - Duration::from_secs(silent_secs));
        assert_eq!(
            stalled_processing_secs_without_active_tool(&state),
            None,
            "静默 {silent_secs}s 仍在正常等待范围内，spinner 不能消失"
        );
    }
}

/// 反向保证：agent 侧彻底失联时，UI 仍要能把 spinner 收回来。
#[test]
fn a_truly_dead_agent_still_releases_the_spinner() {
    let mut state = ChatState::new();
    state.is_processing = true;
    let beyond_limit = ui_stall_limit_secs() + 30;
    state.last_token_time = Some(Instant::now() - Duration::from_secs(beyond_limit));
    assert_eq!(
        stalled_processing_secs_without_active_tool(&state),
        Some(beyond_limit)
    );
}

#[test]
fn restore_initial_session_keeps_welcome_and_confirmed_usage() {
    let mut state = ChatState::new();
    let welcome_len = state.chat_history.len();
    let mut stale = crate::types::StarUsage {
        total_tokens: 999,
        cache_read_tokens: 888,
        cache_creation_tokens: 777,
        cache_telemetry_reported: true,
        ..Default::default()
    };
    state.token_usage = Some(stale.clone());
    state.token_count = stale.total_tokens;
    state.cache_read_tokens = stale.cache_read_tokens as u64;
    state.cache_creation_tokens = stale.cache_creation_tokens as u64;

    restore_initial_session_state(
        &mut state,
        vec![ChatEntry::user("restored"), {
            let mut old_welcome = ChatEntry::assistant("old welcome");
            old_welcome.is_welcome = true;
            old_welcome
        }],
        Some(crate::types::StarUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            cache_read_tokens: 70,
            cache_creation_tokens: 10,
            cache_telemetry_reported: true,
        }),
        Some("resumed-id".to_string()),
    );

    assert_eq!(state.active_session_id.as_deref(), Some("resumed-id"));
    assert_eq!(state.chat_history.len(), welcome_len + 1);
    assert!(state.chat_history[0].is_welcome);
    assert_eq!(state.chat_history[1].content, "restored");
    assert_eq!(state.token_count, 120);
    assert_eq!(state.cache_read_tokens, 70);
    assert_eq!(state.cache_creation_tokens, 10);
    assert_eq!(state.chat_list_state.selected(), Some(1));

    stale.cache_telemetry_reported = false;
    restore_initial_session_state(&mut state, Vec::new(), Some(stale), None);
    assert_eq!(state.token_count, 999);
    assert_eq!(state.cache_read_tokens, 0);
    assert_eq!(state.cache_creation_tokens, 0);
}

#[test]
fn exit_session_id_preserves_resumed_canonical_identity() {
    let mut fresh = crate::core::config::ConfigParameters::default();
    fresh.session_id = "uuid-123".to_string();
    let fresh = crate::core::config::Config::new(fresh);
    assert_eq!(exit_session_id(&ChatState::new(), &fresh), "auto-uuid-123");

    let mut resumed = crate::core::config::ConfigParameters::default();
    resumed.session_id = "named.session".to_string();
    resumed.resume_session = true;
    let resumed = crate::core::config::Config::new(resumed);
    assert_eq!(
        exit_session_id(&ChatState::new(), &resumed),
        "named.session"
    );

    let mut switched = ChatState::new();
    switched.active_session_id = Some("session-chosen-in-ui".to_string());
    assert_eq!(exit_session_id(&switched, &resumed), "session-chosen-in-ui");
}
