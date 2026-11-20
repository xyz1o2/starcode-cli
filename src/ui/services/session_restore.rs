//! 读盘、投递恢复请求和提交 UI 历史分三步，等待期间仍然消费 worker 流。

use std::path::{Path, PathBuf};

use tokio::{sync::mpsc, task::JoinHandle};

use crate::core::{i18n, utils::paths};
use crate::runtime::messages::AgentRequest;
use crate::types::{ChatEntry, StarUsage};
use crate::ui::state::{ChatState, ToastKind};
use crate::utils::session_manager;

pub(crate) enum Selection {
    Resume(String),
    Continue(Option<String>),
}

pub(crate) struct RestoreSnapshot {
    id: String,
    history: Vec<ChatEntry>,
    usage: Option<StarUsage>,
    continuing: bool,
}

pub(crate) struct Loaded {
    snapshot: RestoreSnapshot,
    request: AgentRequest,
}

type LoadResult = Result<Option<Loaded>, String>;

pub(crate) enum PendingRestore {
    Loading { cwd: PathBuf, job: LoadJob },
    Ready { cwd: PathBuf, loaded: Box<Loaded> },
    AwaitingWorker(RestoreSnapshot),
}

pub(crate) struct LoadJob(JoinHandle<LoadResult>);

impl Drop for LoadJob {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn start(state: &mut ChatState, selection: Selection) -> Result<(), String> {
    start_at(
        state,
        selection,
        paths::current_dir_cached().clone(),
        paths::current_project_star_dir().join("sessions"),
    )
}

fn start_at(
    state: &mut ChatState,
    selection: Selection,
    cwd: PathBuf,
    directory: PathBuf,
) -> Result<(), String> {
    if state.pending_session_restore.is_some() {
        return Err(i18n::t1("ui.session.restore.loading"));
    }
    state.pending_session_restore = Some(PendingRestore::Loading {
        cwd,
        job: LoadJob(tokio::spawn(load(directory, selection))),
    });
    Ok(())
}

async fn load(directory: PathBuf, selection: Selection) -> LoadResult {
    let continuing = matches!(selection, Selection::Continue(_));
    let id = match selection {
        Selection::Resume(id) | Selection::Continue(Some(id)) => id,
        Selection::Continue(None) => {
            let Some(id) = session_manager::read_latest_session_id_in(&directory)
                .await
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            id
        }
    };
    let session = session_manager::load_session_in(&directory, &id)
        .await
        .map_err(|error| error.to_string())?;
    let messages = session_manager::agent_messages_for_restore(&session);
    Ok(Some(Loaded {
        snapshot: RestoreSnapshot {
            id,
            history: session.chat_history,
            usage: session.last_usage,
            continuing,
        },
        request: AgentRequest::RestoreSession {
            messages,
            pending_local_context: session.pending_local_context.unwrap_or_default(),
        },
    }))
}

pub(crate) fn block_submission(state: &mut ChatState) -> bool {
    if state.pending_session_restore.is_none() {
        return false;
    }
    state.push_toast(&i18n::t1("ui.session.restore.loading"), ToastKind::Info);
    true
}

/// 已投递的请求不能取消：退出保存也必须等同一条流上的切换边界。
pub(crate) fn awaiting_worker(state: &ChatState) -> bool {
    matches!(
        state.pending_session_restore,
        Some(PendingRestore::AwaitingWorker(_))
    )
}

pub(crate) async fn poll(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>) -> bool {
    poll_at(state, agent_tx, paths::current_dir_cached()).await
}

async fn poll_at(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>, cwd: &Path) -> bool {
    let Some(pending) = state.pending_session_restore.take() else {
        return false;
    };
    let (owner, loaded) = match pending {
        PendingRestore::Loading {
            cwd: owner,
            mut job,
        } if job.0.is_finished() => match (&mut job.0).await {
            Ok(Ok(Some(loaded))) => (owner, Box::new(loaded)),
            Ok(Ok(None)) => {
                let message = i18n::t1("ui.log_selector.no_sessions");
                state.current_status_line = Some(message.clone());
                state.push_toast(&message, ToastKind::Info);
                recover_queued_input(state);
                return true;
            }
            Ok(Err(error)) => return fail(state, &error),
            Err(error) => return fail(state, &error.to_string()),
        },
        PendingRestore::Ready { cwd: owner, loaded } => (owner, loaded),
        other => {
            state.pending_session_restore = Some(other);
            return false;
        }
    };
    if owner != cwd {
        return fail(state, &i18n::t1("ui.log_selector.project_changed"));
    }
    let Loaded { snapshot, request } = *loaded;
    match agent_tx.try_send(request) {
        Ok(()) => {
            state.pending_session_restore = Some(PendingRestore::AwaitingWorker(snapshot));
            false
        }
        Err(mpsc::error::TrySendError::Full(request)) => {
            state.pending_session_restore = Some(PendingRestore::Ready {
                cwd: owner,
                loaded: Box::new(Loaded { snapshot, request }),
            });
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => worker_disconnected(state),
    }
}

pub(crate) fn finish(state: &mut ChatState, result: Result<(), String>) {
    let Some(pending) = state.pending_session_restore.take() else {
        return;
    };
    let PendingRestore::AwaitingWorker(snapshot) = pending else {
        state.pending_session_restore = Some(pending);
        return;
    };
    if let Err(error) = result {
        fail(state, &error);
        return;
    }

    let RestoreSnapshot {
        id,
        history,
        usage,
        continuing,
    } = snapshot;
    let message_key = if continuing {
        i18n::t1("ui.session.restore.continued")
    } else {
        i18n::t1("ui.session.restore.resumed")
    };
    let message = message_key
        .replace("{count}", &history.len().to_string())
        .replace("{id}", &id);
    crate::commands::chat::restore_session_history(state, history, usage, &format!("'{id}'"));
    state.active_session_id = Some(id);
    state.session_schedules.change_session();
    state.current_status_line = Some(message.clone());
    state.push_toast(&message, ToastKind::Success);
    recover_queued_input(state);
}

pub(crate) fn worker_disconnected(state: &mut ChatState) -> bool {
    state.pending_session_restore = None;
    fail(state, &i18n::t1("ui.session.restore.worker_closed"))
}

fn fail(state: &mut ChatState, error: &str) -> bool {
    let message = i18n::t1("ui.session.restore.failed").replace("{error}", error);
    state
        .chat_history
        .push(ChatEntry::assistant(message.clone()).with_streaming(false));
    state.push_toast(&message, ToastKind::Error);
    recover_queued_input(state);
    true
}

fn recover_queued_input(state: &mut ChatState) {
    // 用户是在旧会话中排的队；不把剩余命令或附件自动提交到新会话。
    crate::ui::events::input::restore_queued_messages_to_input(state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::messages::StreamMessage;
    use crate::ui::services::stream::handle_stream_update;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);
    const ID: &str = "saved-session";

    fn fixture(directory: &Path) {
        std::fs::create_dir_all(directory).unwrap();
        let session = session_manager::Session {
            id: "embedded-id".into(),
            created_at: 42,
            chat_history: vec![ChatEntry::user("saved history")],
            agent_messages: Some(vec![crate::types::StarMessage::user("saved context")]),
            pending_local_context: Some(vec!["saved local context".into()]),
            last_usage: None,
        };
        std::fs::write(
            directory.join(format!("{ID}.json")),
            serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join(".latest"), ID).unwrap();
    }

    async fn wait_until_loaded(state: &ChatState) {
        let Some(PendingRestore::Loading { job, .. }) = &state.pending_session_restore else {
            panic!("restore did not start in the background");
        };
        tokio::time::timeout(WAIT, async {
            while !job.0.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fixture load should finish");
    }

    struct RestoreDirectory(PathBuf);
    impl Drop for RestoreDirectory {
        fn drop(&mut self) {
            paths::set_current_dir(&self.0).expect("restore original cwd");
        }
    }

    #[tokio::test]
    async fn queued_resume_returns_from_done_while_worker_is_blocked_on_stream() {
        let project = tempfile::tempdir().unwrap();
        let _cwd = RestoreDirectory(std::env::current_dir().unwrap());
        paths::set_current_dir(project.path()).unwrap();
        fixture(&paths::current_project_star_dir().join("sessions"));
        let mut state = ChatState::test_fixture();
        state.active_session_id = Some("old".into());
        state.is_processing = true;
        state.is_streaming = true;
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::command_eligible(format!(
                "/resume {ID}"
            )));
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        let (stream_tx, mut stream_rx) = mpsc::channel(1);
        let (full_tx, full_rx) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            stream_tx
                .send(StreamMessage::Done { message_id: 7 })
                .await
                .unwrap();
            stream_tx
                .send(StreamMessage::AssistantNote {
                    message_id: 7,
                    content: "first old note".into(),
                })
                .await
                .unwrap();
            full_tx.send(()).unwrap();
            // 收尾消息必须先出队，worker 才有机会接恢复请求。
            stream_tx
                .send(StreamMessage::AssistantNote {
                    message_id: 7,
                    content: "second old note".into(),
                })
                .await
                .unwrap();
            let Some(AgentRequest::RestoreSession {
                messages,
                pending_local_context,
            }) = agent_rx.recv().await
            else {
                panic!("expected one restore request");
            };
            assert_eq!(messages[0].content.as_deref(), Some("saved context"));
            assert_eq!(pending_local_context, ["saved local context"]);
            stream_tx
                .send(StreamMessage::SessionRestoreFinished(Ok(())))
                .await
                .unwrap();
        });
        let done = stream_rx.recv().await.unwrap();
        full_rx.await.unwrap();
        assert_eq!(stream_rx.len(), 1);
        tokio::time::timeout(WAIT, handle_stream_update(&mut state, done, &agent_tx))
            .await
            .expect("Done must not wait for the restore acknowledgement")
            .unwrap();
        wait_until_loaded(&state).await;
        poll(&mut state, &agent_tx).await;
        assert!(awaiting_worker(&state));
        assert_eq!(state.active_session_id.as_deref(), Some("old"));
        assert!(state
            .chat_history
            .iter()
            .all(|entry| entry.content != "saved history"));
        state.set_input_text("unsent draft");
        crate::ui::events::input::handle_key_event(
            &mut state,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            &agent_tx,
            None,
        )
        .await
        .unwrap();
        assert_eq!(state.input, "unsent draft");

        for _ in 0..3 {
            let message = tokio::time::timeout(WAIT, stream_rx.recv())
                .await
                .unwrap()
                .unwrap();
            handle_stream_update(&mut state, message, &agent_tx)
                .await
                .unwrap();
        }
        worker.await.unwrap();
        assert!(state.pending_session_restore.is_none());
        assert_eq!(state.active_session_id.as_deref(), Some(ID));
        assert_eq!(state.chat_history.len(), 1);
        assert_eq!(state.chat_history[0].content, "saved history");
        assert_eq!(state.input, "unsent draft");
    }

    #[tokio::test]
    async fn continue_handles_full_queue_and_worker_rejection_without_replacing_history() {
        let project = tempfile::tempdir().unwrap();
        fixture(project.path());
        let mut state = ChatState::test_fixture();
        state.chat_history = vec![ChatEntry::user("old history")];
        state.active_session_id = Some("old".into());
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        agent_tx.try_send(AgentRequest::Abort).unwrap();
        start_at(
            &mut state,
            Selection::Continue(None),
            project.path().into(),
            project.path().into(),
        )
        .unwrap();
        wait_until_loaded(&state).await;
        poll_at(&mut state, &agent_tx, project.path()).await;
        assert!(matches!(
            state.pending_session_restore,
            Some(PendingRestore::Ready { .. })
        ));
        assert!(matches!(agent_rx.try_recv(), Ok(AgentRequest::Abort)));
        poll_at(&mut state, &agent_tx, project.path()).await;
        assert!(awaiting_worker(&state));
        assert!(matches!(
            agent_rx.try_recv(),
            Ok(AgentRequest::RestoreSession { .. })
        ));
        finish(&mut state, Err("fixture rejection".into()));
        assert_eq!(state.active_session_id.as_deref(), Some("old"));
        assert_eq!(state.chat_history[0].content, "old history");
        assert!(state
            .toast_queue
            .back()
            .unwrap()
            .message
            .contains("fixture rejection"));
        assert!(state.pending_session_restore.is_none());
    }

    #[tokio::test]
    async fn scheduled_message_waits_for_restore_and_never_enters_replaced_context() {
        use crate::ui::services::session_schedules;

        let project = tempfile::tempdir().unwrap();
        fixture(project.path());
        for due_before_request in [true, false] {
            for restore_succeeds in [true, false] {
                let mut state = ChatState::test_fixture();
                // 即使恢复同一个存档 ID，旧的定时任务也不应进入替换后的上下文。
                state.active_session_id = Some(ID.into());
                state.chat_history = vec![ChatEntry::user("old history")];
                let (agent_tx, mut agent_rx) = mpsc::channel(1);
                let (stream_tx, mut stream_rx) = mpsc::channel(1);
                start_at(
                    &mut state,
                    Selection::Resume(ID.into()),
                    project.path().into(),
                    project.path().into(),
                )
                .unwrap();
                if due_before_request {
                    state
                        .session_schedules
                        .mark_due_for_test("old-trigger", "old prompt");
                    assert!(!session_schedules::poll(&mut state, &agent_tx));
                    assert!(agent_rx.try_recv().is_err());
                }
                wait_until_loaded(&state).await;
                poll_at(&mut state, &agent_tx, project.path()).await;
                assert!(awaiting_worker(&state));
                if !due_before_request {
                    state
                        .session_schedules
                        .mark_due_for_test("old-trigger", "old prompt");
                }
                assert!(!session_schedules::poll(&mut state, &agent_tx));

                let request = agent_rx
                    .try_recv()
                    .expect("expected the restore request first");
                let mut agent = crate::agent::StarAgent::new_for_runtime_settings_test().await;
                agent.replace_session_context(
                    vec![crate::types::StarMessage::user("old context")],
                    vec![],
                );
                if restore_succeeds {
                    crate::runtime::agent_runtime::process_control_request(
                        &mut agent, &stream_tx, request,
                    )
                    .await;
                } else {
                    stream_tx
                        .try_send(StreamMessage::SessionRestoreFinished(Err(
                            "fixture worker busy".into(),
                        )))
                        .unwrap();
                }
                // worker 已处理恢复请求，但 UI 尚未消费回执。
                assert!(!session_schedules::poll(&mut state, &agent_tx));
                assert!(agent_rx.try_recv().is_err());
                handle_stream_update(&mut state, stream_rx.try_recv().unwrap(), &agent_tx)
                    .await
                    .unwrap();
                assert!(session_schedules::poll(&mut state, &agent_tx));

                let (context, _) = agent.session_context_snapshot();
                if restore_succeeds {
                    assert_eq!(context[0].content.as_deref(), Some("saved context"));
                    assert_eq!(state.chat_history[0].content, "saved history");
                    assert!(agent_rx.try_recv().is_err());
                    assert!(state
                        .chat_history
                        .iter()
                        .all(|entry| entry.content != "old prompt"));
                } else {
                    assert_eq!(context[0].content.as_deref(), Some("old context"));
                    assert_eq!(state.chat_history[0].content, "old history");
                    assert!(
                        matches!(agent_rx.try_recv(), Ok(AgentRequest::SendMessage { message, .. }) if message == "old prompt")
                    );
                    assert_eq!(state.chat_history.last().unwrap().content, "old prompt");
                }
                assert!(agent_rx.try_recv().is_err());
            }
        }
    }

    #[tokio::test]
    async fn loaded_restore_never_crosses_a_project_change() {
        let project = tempfile::tempdir().unwrap();
        let other_project = tempfile::tempdir().unwrap();
        fixture(project.path());
        let mut state = ChatState::test_fixture();
        state.active_session_id = Some("old".into());
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        start_at(
            &mut state,
            Selection::Resume(ID.into()),
            project.path().into(),
            project.path().into(),
        )
        .unwrap();
        wait_until_loaded(&state).await;
        assert!(poll_at(&mut state, &agent_tx, other_project.path()).await);
        assert!(state.pending_session_restore.is_none());
        assert_eq!(state.active_session_id.as_deref(), Some("old"));
        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn worker_disconnect_does_not_commit_loaded_history() {
        let project = tempfile::tempdir().unwrap();
        fixture(project.path());
        let mut state = ChatState::test_fixture();
        state.active_session_id = Some("old".into());
        state.chat_history = vec![ChatEntry::user("old history")];
        let (agent_tx, agent_rx) = mpsc::channel(1);
        start_at(
            &mut state,
            Selection::Resume(ID.into()),
            project.path().into(),
            project.path().into(),
        )
        .unwrap();
        wait_until_loaded(&state).await;
        drop(agent_rx);
        assert!(poll_at(&mut state, &agent_tx, project.path()).await);
        assert_eq!(state.active_session_id.as_deref(), Some("old"));
        assert_eq!(state.chat_history[0].content, "old history");
        assert!(state.pending_session_restore.is_none());
        assert!(matches!(
            state.toast_queue.back().unwrap().kind,
            ToastKind::Error
        ));
    }

    #[tokio::test]
    async fn continue_without_saves_does_not_open_a_menu_or_send_a_request() {
        let project = tempfile::tempdir().unwrap();
        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        start_at(
            &mut state,
            Selection::Continue(None),
            project.path().into(),
            project.path().into(),
        )
        .unwrap();
        wait_until_loaded(&state).await;
        assert!(poll_at(&mut state, &agent_tx, project.path()).await);
        assert!(state.pending_session_restore.is_none());
        assert!(!state.show_session_menu);
        assert_eq!(
            state.current_status_line.as_deref(),
            Some(i18n::t1("ui.log_selector.no_sessions").as_str())
        );
        assert!(agent_rx.try_recv().is_err());
    }
}
