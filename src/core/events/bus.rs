//! Event bus for publishing and subscribing to events

use super::types::SessionEvent;
use tokio::sync::broadcast;

/// Event bus for publishing and subscribing to session events
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<SessionEvent>,
}

impl EventBus {
    /// Create a new event bus with the given capacity
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an event to all subscribers
    pub fn publish(
        &self,
        event: SessionEvent,
    ) -> Result<(), broadcast::error::SendError<SessionEvent>> {
        self.sender.send(event)?;
        Ok(())
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.sender.subscribe()
    }

    /// Get the number of active receivers
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1000)
    }
}
