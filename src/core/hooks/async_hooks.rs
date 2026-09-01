use super::types::{HookEvent, HookResult};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub struct AsyncHookTask {
    pub id: String,
    pub event: HookEvent,
    pub started_at: i64,
    pub future: BoxFuture<HookResult>,
    pub completed: Option<HookResult>,
}

pub struct AsyncHookRegistry {
    pending: HashMap<String, AsyncHookTask>,
}

impl AsyncHookRegistry {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    pub fn register(&mut self, event: HookEvent, future: BoxFuture<HookResult>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let task = AsyncHookTask {
            id: id.clone(),
            event,
            started_at: chrono::Utc::now().timestamp(),
            future,
            completed: None,
        };
        self.pending.insert(id.clone(), task);
        id
    }

    pub fn poll(&mut self, id: &str) -> Option<HookResult> {
        let task = self.pending.get_mut(id)?;

        if let Some(result) = task.completed.take() {
            self.pending.remove(id);
            return Some(result);
        }

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        match task.future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => {
                self.pending.remove(id);
                Some(result)
            }
            Poll::Pending => None,
        }
    }

    pub fn cancel(&mut self, id: &str) {
        self.pending.remove(id);
    }

    pub fn cancel_all(&mut self) {
        self.pending.clear();
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn pending_ids(&self) -> Vec<String> {
        self.pending.keys().cloned().collect()
    }

    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }
}

impl Default for AsyncHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn noop_waker() -> std::task::Waker {
    use std::task::{RawWaker, RawWakerVTable};

    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { std::task::Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}
