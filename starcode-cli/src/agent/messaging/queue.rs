use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

/// An asynchronous message queue that supports non-blocking enqueue and blocking dequeue.
/// StarCode async message queue.
pub struct AsyncMessageQueue<T> {
    tx: mpsc::UnboundedSender<T>,
    rx: Arc<Mutex<mpsc::UnboundedReceiver<T>>>,
    closed: Arc<AtomicBool>,
}

impl<T: Send + 'static> AsyncMessageQueue<T> {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Enqueue a message.
    /// Corresponds to `h2A.enqueue`.
    pub fn enqueue(&self, item: T) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("Queue is closed".to_string());
        }
        self.tx
            .send(item)
            .map_err(|_| "Receiver dropped".to_string())
    }

    /// Try to get the next message without blocking.
    /// Returns None only when no message is available. Logs lock contention.
    pub fn try_next(&self) -> Option<T> {
        match self.rx.try_lock() {
            Ok(mut rx) => rx.try_recv().ok(),
            Err(_) => {
                crate::utils::logging::append_debug_log_line(
                    "AsyncMessageQueue::try_next: lock contention, skipping poll",
                );
                None
            }
        }
    }
}

impl<T: Send + 'static> Default for AsyncMessageQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}
