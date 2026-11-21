//! UI 循环里那几个纯函数的测试。
//!
use super::*;

use super::*;
use crate::types::ChatEntry;

#[test]
fn startup_input_uses_final_capture_instead_of_stale_preview() {
    for (preview, final_capture) in [
        ("", "/resume"),
        ("/res", "/resume"),
        ("/resume", "/resume"),
        ("stale text erased by backspace", ""),
        ("检查", "检查启动输入 🦀\n第二行"),
    ] {
        let mut state = ChatState::test_fixture();
        state.set_input_text(preview);
        restore_startup_input(
            &mut state,
            "",
            &crate::types::UserInput::command_eligible(final_capture.into()),
        );
        assert_eq!(state.input, final_capture, "preview={preview:?}");
        assert_eq!(state.textarea.lines().join("\n"), final_capture);
        assert_eq!(state.input_line_count, final_capture.split('\n').count());
    }
}

#[test]
fn startup_input_pasted_prefixes_remain_literal_after_restore() {
    for text in ["/help", "!printf should-not-run", "/help\n检查"] {
        let mut state = ChatState::test_fixture();
        restore_startup_input(&mut state, "", &text.into());
        let prepared = state.prepared_input().expect("restored input");
        assert_eq!(prepared.text, text);
        assert_eq!(prepared.intent, crate::types::InputIntent::Literal);
    }
}

#[test]
fn startup_input_restores_draft_before_final_capture_without_duplication() {
    let temp = tempfile::tempdir().expect("temporary draft directory");
    let path = temp.path().join("draft.txt");
    std::fs::write(&path, "saved draft").expect("write draft");
    let mut state = ChatState::test_fixture();
    state.draft_path = Some(path.clone());
    state.set_input_text("/res");
    restore_startup_input(
        &mut state,
        "",
        &crate::types::UserInput::command_eligible("/resume\nnext line".into()),
    );
    assert_eq!(state.input, "saved draft\n/resume\nnext line");
    assert_eq!(state.textarea.lines().join("\n"), state.input);
    assert_eq!(state.input_line_count, 3);
    assert!(!path.exists(), "restored draft must be consumed");
}

#[test]
fn startup_input_explicit_message_overrides_capture_and_preserves_draft() {
    let temp = tempfile::tempdir().expect("temporary draft directory");
    let path = temp.path().join("draft.txt");
    std::fs::write(&path, "saved draft").expect("write draft");
    let mut state = ChatState::test_fixture();
    state.draft_path = Some(path.clone());
    state.set_input_text("/res");
    restore_startup_input(&mut state, "explicit message", &"/resume".into());
    assert_eq!(state.input, "explicit message");
    assert_eq!(state.textarea.lines().join("\n"), "explicit message");
    assert_eq!(
        std::fs::read_to_string(path).expect("draft retained"),
        "saved draft"
    );
}

#[tokio::test]
async fn exit_shutdown_without_acknowledgements_stops_waiting_at_budget() {
    let (tx, mut rx) = mpsc::channel(2);
    let worker = tokio::spawn(async move {
        // 假 worker 保持回执通道存活但不回应，复现原来的无限等待。
        let save = rx.recv().await.expect("save request");
        let shutdown = rx.recv().await.expect("shutdown request");
        assert!(matches!(save, AgentRequest::SaveSession { .. }));
        assert!(matches!(shutdown, AgentRequest::Shutdown { .. }));
        std::future::pending::<()>().await;
        drop((save, shutdown));
    });
    let save = async {
        crate::commands::chat::request_session_save_then_shutdown(
            &tx,
            "exit-test".into(),
            vec![ChatEntry::user("unsaved")],
            None,
        )
        .await
        .ok()
        .map(|()| "exit-test".to_string())
    };
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        finish_worker_on_exit(save, worker, Duration::from_millis(20)),
    )
    .await
    .expect("exit must have a finite bound");
    assert!(result.is_none(), "没有完整回执不能宣称已保存");
}

#[tokio::test]
async fn exit_shutdown_preserves_fifo_and_reports_save_errors() {
    for save_succeeds in [true, false] {
        let (tx, mut rx) = mpsc::channel(2);
        let session_end = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = session_end.clone();
        let worker = tokio::spawn(async move {
            let Some(AgentRequest::SaveSession { id, response, .. }) = rx.recv().await else {
                panic!("save must precede shutdown");
            };
            assert_eq!(id, "exit-test");
            response
                .send(if save_succeeds {
                    Ok(())
                } else {
                    Err("disk full".into())
                })
                .await
                .unwrap();
            let Some(AgentRequest::Shutdown { response }) = rx.recv().await else {
                panic!("shutdown must follow save even on failure");
            };
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            response.send(Ok(())).await.unwrap();
        });
        let save = async {
            crate::commands::chat::request_session_save_then_shutdown(
                &tx,
                "exit-test".into(),
                vec![ChatEntry::user("test")],
                None,
            )
            .await
            .ok()
            .map(|()| "exit-test".to_string())
        };
        let result = finish_worker_on_exit(save, worker, Duration::from_secs(1)).await;
        assert_eq!(result.as_deref(), save_succeeds.then_some("exit-test"));
        assert!(session_end.load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[tokio::test]
async fn exit_shutdown_join_uses_the_same_deadline() {
    let worker = tokio::spawn(std::future::pending::<()>());
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        finish_worker_on_exit(
            async { Some("saved".into()) },
            worker,
            Duration::from_millis(20),
        ),
    )
    .await
    .expect("worker join must not block terminal cleanup");
    assert_eq!(result.as_deref(), Some("saved"));
}

/// 真实页面压缩输入区高度后，软换行光标仍须落在边框内。
#[test]
fn wrapped_prompt_keeps_cursor_visible_in_short_viewports() {
    use ratatui::{
        backend::{Backend, TestBackend},
        Terminal,
    };

    let text = "中文🙂 long-input ".repeat(40);
    for (width, height) in [(20, 12), (40, 18), (80, 24)] {
        let mut state = ChatState::test_fixture();
        state.set_input_text(&text);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| crate::ui::app::draw_ui(f, &mut state))
            .unwrap();
        let area = state.last_input_area.unwrap();
        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert!(terminal.backend().cursor_visible());
        assert!(
            area.contains(cursor),
            "{width}x{height}: {area:?}, {cursor:?}"
        );
        assert!(cursor.y > area.y && cursor.y < area.bottom() - 1);
        assert_eq!(state.input, text);
        assert_eq!(state.textarea.lines(), [text.as_str()]);
    }
}

#[test]
fn stream_wakeups_do_not_bypass_frame_throttling() {
    let frame = Duration::from_millis(33);
    for elapsed_ms in 0..33 {
        assert!(!should_draw_ui(
            true,
            0,
            true,
            Duration::from_millis(elapsed_ms),
            frame
        ));
    }
    assert!(should_draw_ui(true, 0, true, frame, frame));
    assert!(should_draw_ui(true, 1, true, Duration::ZERO, frame));
    assert!(should_draw_ui(true, 0, false, Duration::ZERO, frame));
    assert!(!should_draw_ui(false, 1, false, frame, frame));
}

#[tokio::test]
async fn stream_message_wakes_idle_ui_without_keyboard_input() {
    let (_key_tx, mut key_rx) = mpsc::channel(1);
    let (stream_tx, mut stream_rx) = mpsc::channel(1);
    let sender = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        stream_tx
            .send(StreamMessage::TextDelta {
                message_id: 1,
                content: "first token".into(),
            })
            .await
            .unwrap();
    });
    let wake = tokio::time::timeout(
        Duration::from_millis(200),
        wait_for_ui_activity(
            &mut key_rx,
            &mut stream_rx,
            Duration::from_secs(1),
            false,
            false,
        ),
    )
    .await
    .unwrap();
    assert!(matches!(
        wake,
        UiWakeup::Stream(Some(StreamMessage::TextDelta { .. }))
    ));
    sender.await.unwrap();
}

#[test]
fn clear_without_query_repaints_identical_content() {
    use ratatui::{backend::TestBackend, widgets::Paragraph, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
    terminal
        .draw(|f| f.render_widget(Paragraph::new("same content"), f.area()))
        .unwrap();
    let expected = terminal.backend().buffer().clone();
    for _ in 0..3 {
        clear_without_cursor_query(&mut terminal).unwrap();
        terminal
            .draw(|f| f.render_widget(Paragraph::new("same content"), f.area()))
            .unwrap();
        assert_eq!(terminal.backend().buffer(), &expected);
    }
    for text in ["中文🙂  tail", "", " a "] {
        let mut fresh = Terminal::new(TestBackend::new(20, 4)).unwrap();
        fresh
            .draw(|f| {
                f.render_widget(
                    Paragraph::new(text)
                        .style(ratatui::style::Style::default().fg(ratatui::style::Color::Green)),
                    f.area(),
                )
            })
            .unwrap();
        clear_without_cursor_query(&mut terminal).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    Paragraph::new(text)
                        .style(ratatui::style::Style::default().fg(ratatui::style::Color::Green)),
                    f.area(),
                )
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer(), fresh.backend().buffer());
    }
}

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
