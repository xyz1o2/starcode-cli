//! 到期的 `/schedule` 消息只在所属会话空闲时交给 worker。

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::mpsc;

use crate::core::{i18n, offline, trigger_scheduler};
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use crate::ui::state::{ChatState, ToastKind};

pub(crate) struct SessionSchedules {
    generation: u64,
    tx: mpsc::Sender<Due>,
    rx: mpsc::Receiver<Due>,
    pending: Option<Due>,
    /// 从创建到交给 worker 前一直登记，允许 UI 取消已到期但尚未投递的任务。
    active: Arc<Mutex<HashSet<(PathBuf, String)>>>,
}

struct Due {
    generation: u64,
    path: PathBuf,
    id: String,
    message: String,
}

impl Default for SessionSchedules {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(16);
        Self {
            generation: 0,
            tx,
            rx,
            pending: None,
            active: std::sync::Arc::default(),
        }
    }
}

impl SessionSchedules {
    pub(crate) fn change_session(&mut self) {
        self.generation += 1;
    }

    fn is_active(&self, due: &Due) -> bool {
        self.active
            .lock()
            .expect("schedule registry lock poisoned")
            .contains(&(due.path.clone(), due.id.clone()))
    }

    #[cfg(test)]
    fn mark_taken_for_test(&mut self, path: PathBuf, id: &str, message: &str) {
        let due = Due {
            generation: self.generation,
            path,
            id: id.into(),
            message: message.into(),
        };
        self.active
            .lock()
            .expect("schedule registry lock poisoned")
            .insert((due.path.clone(), due.id.clone()));
        self.tx.try_send(due).expect("fixture queue has capacity");
    }

    pub(crate) fn cancel(&mut self, path: &Path, id: &str) -> bool {
        let persisted = trigger_scheduler::remove_trigger_at(path, id);
        let key = (path.to_path_buf(), id.to_string());
        let pending = self
            .active
            .lock()
            .expect("schedule registry lock poisoned")
            .remove(&key);
        if pending {
            self.pending = self
                .pending
                .take()
                .filter(|due| (due.path.as_path(), due.id.as_str()) != (path, id));
        }
        persisted || pending
    }

    #[cfg(test)]
    pub(crate) fn mark_due_for_test(&mut self, id: &str, message: &str) {
        let path = PathBuf::new();
        let due = Due {
            generation: self.generation,
            path,
            id: id.into(),
            message: message.into(),
        };
        self.active
            .lock()
            .expect("schedule registry lock poisoned")
            .insert((due.path.clone(), due.id.clone()));
        self.pending = Some(due);
    }

    pub(crate) fn schedule(&mut self, path: PathBuf, id: String, message: String, delay: Duration) {
        let due = Due {
            generation: self.generation,
            path: path.clone(),
            id,
            message,
        };
        let key = (path.clone(), due.id.clone());
        self.active
            .lock()
            .expect("schedule registry lock poisoned")
            .insert(key.clone());
        let active = self.active.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let online = wait_until_online().await;
            if !online {
                active
                    .lock()
                    .expect("schedule registry lock poisoned")
                    .remove(&key);
                return;
            }
            // 磁盘操作留在阻塞池；用户取消则在移除登记后阻止到期事件入队。
            let id = due.id.clone();
            let taken =
                tokio::task::spawn_blocking(move || trigger_scheduler::take_trigger_at(&path, &id))
                    .await
                    .unwrap_or(false);
            let registered = active
                .lock()
                .expect("schedule registry lock poisoned")
                .contains(&key);
            if taken && registered {
                let _ = tx.send(due).await;
            } else {
                active
                    .lock()
                    .expect("schedule registry lock poisoned")
                    .remove(&key);
            }
        });
    }
}

/// 离线时最多等 30 分钟，不在后台绕过用户的离线选择。
async fn wait_until_online() -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1_800);
    while offline::is_offline() {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(15).min(deadline - now)).await;
    }
    true
}

pub(crate) fn poll(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>) -> bool {
    if state.should_exit
        || state.pending_session_restore.is_some()
        || !state.pending_user_messages.is_empty()
    {
        return false;
    }
    let schedules = &mut state.session_schedules;
    let due = loop {
        let Some(due) = schedules
            .pending
            .take()
            .or_else(|| schedules.rx.try_recv().ok())
        else {
            return false;
        };
        if schedules.is_active(&due) {
            break due;
        }
    };
    if due.generation != schedules.generation {
        schedules
            .active
            .lock()
            .expect("schedule registry lock poisoned")
            .remove(&(due.path.clone(), due.id.clone()));
        let message = i18n::t1("ui.schedule.session_changed").replace("{id}", &due.id);
        state.push_toast(&message, ToastKind::Info);
        return true;
    }
    if state.is_processing
        || state.is_streaming
        || state.cancelling_since.is_some()
        || state.network_offline
        || crate::core::offline::is_offline()
    {
        schedules.pending = Some(due);
        return false;
    }
    match agent_tx.try_reserve() {
        Ok(permit) => {
            schedules
                .active
                .lock()
                .expect("schedule registry lock poisoned")
                .remove(&(due.path.clone(), due.id.clone()));
            let message_id = state.next_message_id;
            state.next_message_id += 1;
            state
                .chat_history
                .push(ChatEntry::user(due.message.clone()));
            state.last_turn_failed = false;
            state.is_processing = true;
            state.processing_started_at = Some(std::time::Instant::now());
            permit.send(AgentRequest::SendMessage {
                message_id,
                message: due.message,
            });
            true
        }
        Err(mpsc::error::TrySendError::Full(())) => {
            schedules.pending = Some(due);
            false
        }
        Err(mpsc::error::TrySendError::Closed(())) => {
            schedules
                .active
                .lock()
                .expect("schedule registry lock poisoned")
                .remove(&(due.path.clone(), due.id.clone()));
            let message = i18n::t1("ui.schedule.worker_closed").replace("{id}", &due.id);
            state.push_toast(&message, ToastKind::Error);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::trigger_scheduler;
    use crate::runtime::messages::AgentRequest;
    use crate::ui::state::ChatState;

    struct TestTriggerStore {
        path: PathBuf,
        previous: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestTriggerStore {
        fn new(tag: &str) -> Self {
            static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
            let guard = ENV_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .expect("schedule test environment lock poisoned");
            let path = std::env::temp_dir().join(format!(
                "starcode-schedule-{}-{}.json",
                tag,
                std::process::id()
            ));
            let previous = std::env::var_os("STAR_TRIGGERS_FILE");
            std::env::set_var("STAR_TRIGGERS_FILE", &path);
            let _ = std::fs::remove_file(&path);
            Self {
                path,
                previous,
                _guard: guard,
            }
        }
    }

    impl Drop for TestTriggerStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            if let Some(previous) = self.previous.take() {
                std::env::set_var("STAR_TRIGGERS_FILE", previous);
            } else {
                std::env::remove_var("STAR_TRIGGERS_FILE");
            }
        }
    }

    async fn run_remove(state: &mut ChatState, agent_tx: &mpsc::Sender<AgentRequest>, id: &str) {
        crate::commands::parity::schedule(
            crate::commands::execution::CommandContext { state, agent_tx },
            vec!["remove".into(), id.into()],
        )
        .await
        .expect("schedule remove command should succeed");
    }

    #[tokio::test]
    async fn due_trigger_can_be_cancelled_after_take_before_worker_send() {
        let _store = TestTriggerStore::new("pending-cancel");
        let path = _store.path.clone();
        let id = trigger_scheduler::add_trigger_at(&path, "delayed prompt", 3_600);

        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        agent_tx
            .try_send(AgentRequest::Abort)
            .expect("fill worker channel for cancellation test");
        state.session_schedules.schedule(
            path.clone(),
            id.clone(),
            "delayed prompt".into(),
            Duration::ZERO,
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !poll(&mut state, &agent_tx) {
                    if state.session_schedules.pending.is_some() {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("due trigger should reach the blocked worker queue");

        assert!(
            !trigger_scheduler::take_trigger_at(&path, &id),
            "the due task must already be removed from persistent storage"
        );
        run_remove(&mut state, &agent_tx, &id).await;
        assert!(state.session_schedules.pending.is_none());
        assert!(state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::Assistant
                && entry.content == format!("Removed trigger `{}`.", id)
        }));
        assert!(!state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::User
                && entry.content == "delayed prompt"
        }));
        assert!(matches!(agent_rx.try_recv(), Ok(AgentRequest::Abort)));
        assert!(agent_rx.try_recv().is_err());

        assert!(!poll(&mut state, &agent_tx));
        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn queued_command_is_not_overtaken_by_due_schedule() {
        let store = TestTriggerStore::new("queued-command-priority");
        let id = trigger_scheduler::add_trigger_at(&store.path, "scheduled prompt", 3_600);
        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::command_eligible(format!(
                "/schedule remove {}",
                id
            )));
        state
            .session_schedules
            .mark_taken_for_test(store.path.clone(), &id, "scheduled prompt");

        assert!(!poll(&mut state, &agent_tx));
        assert!(agent_rx.try_recv().is_err());
        assert!(state.session_schedules.pending.is_none());
        assert_eq!(state.session_schedules.rx.len(), 1);
        assert!(state
            .pending_user_messages
            .front()
            .is_some_and(|input| input.text == format!("/schedule remove {}", id)));

        let queued = state
            .pending_user_messages
            .pop_front()
            .expect("queued command exists");
        run_remove(
            &mut state,
            &agent_tx,
            queued.text.split_whitespace().last().expect("id"),
        )
        .await;
        assert!(state.pending_user_messages.is_empty());
        assert!(!poll(&mut state, &agent_tx));
        assert!(agent_rx.try_recv().is_err());
        assert!(state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::Assistant
                && entry.content == format!("Removed trigger `{}`.", id)
        }));
    }

    #[tokio::test]
    async fn due_schedule_is_delivered_after_queued_input_is_cleared() {
        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        state
            .pending_user_messages
            .push_back(crate::types::UserInput::command_eligible("/help".into()));
        state
            .session_schedules
            .mark_due_for_test("due-id", "scheduled prompt");

        assert!(!poll(&mut state, &agent_tx));
        assert!(agent_rx.try_recv().is_err());

        state.pending_user_messages.clear();
        assert!(poll(&mut state, &agent_tx));
        assert!(matches!(
            agent_rx.try_recv(),
            Ok(AgentRequest::SendMessage { message, .. }) if message == "scheduled prompt"
        ));
        assert!(state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::User
                && entry.content == "scheduled prompt"
        }));
        assert!(
            !poll(&mut state, &agent_tx),
            "due event must only be delivered once"
        );
        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn cancelled_due_event_still_in_rx_is_filtered_by_poll() {
        let store = TestTriggerStore::new("queued-cancel");
        let path = store.path.clone();
        let id = trigger_scheduler::add_trigger_at(&path, "delayed prompt", 3_600);
        let mut state = ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        state
            .session_schedules
            .mark_taken_for_test(path, &id, "delayed prompt");

        run_remove(&mut state, &agent_tx, &id).await;
        assert!(state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::Assistant
                && entry.content == format!("Removed trigger `{}`.", id)
        }));
        assert!(!poll(&mut state, &agent_tx));
        assert!(agent_rx.try_recv().is_err());
        assert!(!state.chat_history.iter().any(|entry| {
            entry.entry_type == crate::types::ChatEntryType::User
                && entry.content == "delayed prompt"
        }));
    }
}
