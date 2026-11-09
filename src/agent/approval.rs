//! Approval mode - user approval workflow
//!
//! Responsibilities:
//! - Set approval mode

use std::sync::{Arc, Mutex};

use crate::core::confirmation_bus::MessageBus;
use crate::core::policy::types::ApprovalMode as PolicyApprovalMode;
use crate::types::ApprovalMode;

fn spawn_and_log_panic(fut: impl std::future::Future<Output = ()> + Send + 'static) {
    let handle = tokio::spawn(fut);
    tokio::spawn(async move {
        if let Err(e) = handle.await {
            if e.is_panic() {
                crate::utils::logging::append_debug_log_line(&format!(
                    "Panic in approval spawn: {:?}",
                    e
                ));
            }
        }
    });
}

/// Set approval mode
pub fn set_approval_mode(
    mode: ApprovalMode,
    approval_mode_lock: &Arc<Mutex<ApprovalMode>>,
    message_bus: &Arc<MessageBus>,
) {
    // Skip if mode is already set to avoid unnecessary async updates
    {
        let current = approval_mode_lock.lock().unwrap_or_else(|e| e.into_inner());
        if *current == mode {
            return;
        }
    }

    let mut m = approval_mode_lock.lock().unwrap_or_else(|e| e.into_inner());
    *m = mode.clone();

    // Update MessageBus policy engine
    let mb = message_bus.clone();
    let mode_clone = mode.clone().into();
    spawn_and_log_panic(async move {
        mb.set_approval_mode(mode_clone).await;
    });
}
