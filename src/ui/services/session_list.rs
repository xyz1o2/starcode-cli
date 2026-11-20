//! 会话选择器的磁盘读取在后台执行；UI 只接收当前选择器的结果。

use std::path::{Path, PathBuf};

use tokio::task::JoinHandle;

use crate::core::{i18n, utils::paths};
use crate::ui::state::{modal::Modal, ChatState};
use crate::utils::session_manager::{self, SessionSummary};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    QuickMenu { from_palette: bool },
    Browser,
}

type LoadResult = Result<Vec<SessionSummary>, String>;

pub(crate) struct SessionPicker {
    cwd: PathBuf,
    target: Target,
    job: Option<JoinHandle<LoadResult>>,
}

impl Drop for SessionPicker {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.abort();
        }
    }
}

pub(crate) fn start(state: &mut ChatState, target: Target) {
    let cwd = paths::current_dir_cached().clone();
    let directory = paths::current_project_star_dir().join("sessions");
    start_at(state, target, cwd, directory);
}

fn start_at(state: &mut ChatState, target: Target, cwd: PathBuf, directory: PathBuf) {
    open(state, target);
    state.session_picker = Some(SessionPicker {
        cwd,
        target,
        job: Some(tokio::spawn(async move {
            session_manager::list_session_summaries_in(&directory)
                .await
                .map_err(|error| error.to_string())
        })),
    });
}

fn open(state: &mut ChatState, target: Target) {
    state.close_palette();
    state.close_quick_menus();
    match target {
        Target::QuickMenu { from_palette } => {
            state.exit_input_modal();
            state.quick_menu_back = None;
            state.quick_menu_origin_palette = from_palette;
            state.show_command_hints = false;
            state.command_hints.clear();
            state.show_mention_hints = false;
            state.mention_hints.clear();
            state.available_sessions.clear();
            state.selected_session_index = 0;
            state.open_session_menu();
        }
        Target::Browser => {
            state.open_log_selector();
            state.log_selector_state.begin_loading();
        }
    }
}

pub(crate) fn quick_menu_is_loading(state: &ChatState) -> bool {
    state.session_picker.as_ref().is_some_and(|picker| {
        matches!(picker.target, Target::QuickMenu { .. }) && picker.job.is_some()
    })
}

pub(crate) fn cancel_quick_menu(state: &mut ChatState) {
    cancel_if(state, |target| matches!(target, Target::QuickMenu { .. }));
}

pub(crate) fn cancel_browser(state: &mut ChatState) {
    cancel_if(state, |target| target == Target::Browser);
}

fn cancel_if(state: &mut ChatState, matches: impl FnOnce(Target) -> bool) {
    if state
        .session_picker
        .as_ref()
        .is_some_and(|picker| matches(picker.target))
    {
        state.session_picker = None;
    }
}

/// 按键在后台结果轮询之前处理，确认时也必须检查项目归属。
pub(crate) fn invalidate_changed_directory(state: &mut ChatState) -> bool {
    invalidate_at(state, paths::current_dir_cached())
}

fn invalidate_at(state: &mut ChatState, cwd: &Path) -> bool {
    let Some(picker) = &state.session_picker else {
        return false;
    };
    if picker.cwd == cwd {
        return false;
    }
    let target = picker.target;
    state.session_picker = None;
    apply_result(
        state,
        target,
        Err(i18n::t1("ui.log_selector.project_changed")),
    );
    true
}

/// 只在任务已经结束时取结果，绝不在 UI 线程等待磁盘。
pub(crate) async fn poll(state: &mut ChatState) -> bool {
    poll_at(state, paths::current_dir_cached()).await
}

async fn poll_at(state: &mut ChatState, cwd: &Path) -> bool {
    let Some(picker) = &state.session_picker else {
        return false;
    };
    let target = picker.target;
    let visible = match target {
        Target::QuickMenu { .. } => state.show_session_menu,
        Target::Browser => state.modal_stack.contains(&Modal::LogSelector),
    };
    if !visible {
        state.session_picker = None;
        return false;
    }
    if invalidate_at(state, cwd) {
        return true;
    }
    let Some(job) = state.session_picker.as_mut().and_then(|picker| {
        if picker.job.as_ref().is_some_and(JoinHandle::is_finished) {
            picker.job.take()
        } else {
            None
        }
    }) else {
        return false;
    };
    let result = job.await.unwrap_or_else(|error| Err(error.to_string()));
    apply_result(state, target, result);
    true
}

fn apply_result(state: &mut ChatState, target: Target, result: LoadResult) {
    match (target, result) {
        (Target::QuickMenu { .. }, Ok(sessions)) => state.available_sessions = sessions,
        (Target::QuickMenu { .. }, Err(error)) => {
            state.close_quick_menus();
            let label = i18n::t1("ui.log_selector.load_failed");
            state.push_toast(
                &format!("{label}: {error}"),
                crate::ui::state::ToastKind::Error,
            );
        }
        (Target::Browser, Ok(sessions)) => state
            .log_selector_state
            .set_sessions(sessions.into_iter().map(Into::into).collect()),
        (Target::Browser, Err(error)) => state.log_selector_state.set_load_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    const QUICK: Target = Target::QuickMenu {
        from_palette: false,
    };

    fn controlled_load(
        state: &mut ChatState,
        target: Target,
        cwd: &Path,
    ) -> oneshot::Sender<LoadResult> {
        open(state, target);
        let (tx, rx) = oneshot::channel();
        state.session_picker = Some(SessionPicker {
            cwd: cwd.to_owned(),
            target,
            job: Some(tokio::spawn(async { rx.await.unwrap() })),
        });
        tx
    }

    async fn wait_until_finished(state: &ChatState) {
        let job = state.session_picker.as_ref().unwrap().job.as_ref().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !job.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fixture load should finish");
    }

    #[tokio::test]
    async fn quick_menu_consumes_enter_and_can_close_before_loading_finishes() {
        let mut state = ChatState::test_fixture();
        let cwd = paths::current_dir_cached();
        let tx = controlled_load(&mut state, QUICK, cwd);
        assert!(state.show_session_menu);
        assert!(quick_menu_is_loading(&state));
        assert!(crate::ui::services::suggestions::handle_enter(&mut state));
        assert!(state.pending_palette_action.is_none());
        assert!(!poll_at(&mut state, cwd).await);

        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        crate::ui::events::input::handle_key_event(
            &mut state,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
            &agent_tx,
            None,
        )
        .await
        .unwrap();
        assert!(!state.show_session_menu);
        assert!(state.session_picker.is_none());
        let _ = tx.send(Ok(Vec::new()));
        assert!(!poll_at(&mut state, cwd).await);
    }

    #[tokio::test]
    async fn empty_picker_has_no_action_and_disk_failure_is_visible() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = paths::current_dir_cached();
        let missing = temp.path().join("missing");
        let mut state = ChatState::test_fixture();
        start_at(&mut state, QUICK, cwd.clone(), missing);
        wait_until_finished(&state).await;
        assert!(poll_at(&mut state, cwd).await);
        assert!(!quick_menu_is_loading(&state));
        assert!(state.show_session_menu);
        assert!(crate::ui::services::suggestions::handle_enter(&mut state));
        assert!(state.pending_palette_action.is_none());

        let not_a_directory = temp.path().join("file");
        std::fs::write(&not_a_directory, "fixture").unwrap();
        start_at(&mut state, QUICK, cwd.clone(), not_a_directory);
        wait_until_finished(&state).await;
        assert!(poll_at(&mut state, cwd).await);
        assert!(!state.show_session_menu);
        assert!(matches!(
            state.toast_queue.back().unwrap().kind,
            crate::ui::state::ToastKind::Error
        ));
    }

    #[tokio::test]
    async fn replacing_browser_discards_even_a_finished_old_result() {
        let mut state = ChatState::test_fixture();
        let cwd = Path::new("/fixture-project");
        let old = controlled_load(&mut state, Target::Browser, cwd);
        old.send(Err("old failure".into())).unwrap();
        wait_until_finished(&state).await;
        let current = controlled_load(&mut state, Target::Browser, cwd);
        assert!(!poll_at(&mut state, cwd).await);
        assert!(state.log_selector_state.is_loading);
        current.send(Err("new failure".into())).unwrap();
        wait_until_finished(&state).await;
        assert!(poll_at(&mut state, cwd).await);
        assert!(!state.log_selector_state.is_loading);
        assert_eq!(
            state.log_selector_state.load_error.as_deref(),
            Some("new failure")
        );
    }

    #[tokio::test]
    async fn quick_menu_rejects_a_project_change_even_after_showing_results() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("sessions");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("saved-session.json"),
            r#"{"id":"embedded-id","chat_history":[],"created_at":42}"#,
        )
        .unwrap();
        let mut state = ChatState::test_fixture();
        // 夹具目录与当前 cwd 不同，结果已显示后也必须由 Enter 拒绝。
        let cwd = temp.path();
        start_at(&mut state, QUICK, cwd.to_owned(), directory);
        wait_until_finished(&state).await;
        assert!(poll_at(&mut state, cwd).await);
        assert_eq!(state.available_sessions[0].id, "saved-session");
        assert!(crate::ui::services::suggestions::handle_enter(&mut state));
        assert!(state.pending_palette_action.is_none());
        assert!(!state.show_session_menu);
    }
}
