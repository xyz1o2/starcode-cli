//! Deferred pattern implementation for async waiting

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

/// Deferred manager for async request-response pattern
pub struct DeferredManager<T: Send + 'static, E: Send + 'static> {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<T, E>>>>>,
}

impl<T: Send + 'static, E: Send + 'static> DeferredManager<T, E> {
    /// Create a new deferred manager
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new deferred and return the receiver
    pub async fn create(&self, id: String) -> oneshot::Receiver<Result<T, E>> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().await;
        pending.insert(id, tx);
        rx
    }

    /// Resolve a deferred with a success value
    pub async fn resolve(&self, id: &str, value: T) -> Result<(), DeferredError> {
        let mut pending = self.pending.lock().await;
        if let Some(tx) = pending.remove(id) {
            tx.send(Ok(value)).map_err(|_| DeferredError::SendFailed)?;
            Ok(())
        } else {
            Err(DeferredError::NotFound)
        }
    }

    /// Reject a deferred with an error
    pub async fn reject(&self, id: &str, error: E) -> Result<(), DeferredError> {
        let mut pending = self.pending.lock().await;
        if let Some(tx) = pending.remove(id) {
            tx.send(Err(error)).map_err(|_| DeferredError::SendFailed)?;
            Ok(())
        } else {
            Err(DeferredError::NotFound)
        }
    }

    /// Check if a deferred is pending
    pub async fn has_pending(&self, id: &str) -> bool {
        let pending = self.pending.lock().await;
        pending.contains_key(id)
    }

    /// Get all pending IDs
    pub async fn pending_ids(&self) -> Vec<String> {
        let pending = self.pending.lock().await;
        pending.keys().cloned().collect()
    }

    /// Get the number of pending deferreds
    pub async fn pending_count(&self) -> usize {
        let pending = self.pending.lock().await;
        pending.len()
    }
}

impl<T: Send + 'static, E: Send + 'static> Default for DeferredManager<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Deferred errors
#[derive(Debug, thiserror::Error)]
pub enum DeferredError {
    #[error("Deferred not found")]
    NotFound,
    #[error("Failed to send value")]
    SendFailed,
}
