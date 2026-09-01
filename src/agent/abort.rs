//! Abort control - operation cancellation

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Abort the current operation
pub fn abort(abort_flag: &Arc<AtomicBool>) {
    abort_flag.store(true, Ordering::SeqCst);
}

/// Reset abort flag
pub fn reset_abort(abort_flag: &Arc<AtomicBool>) {
    abort_flag.store(false, Ordering::SeqCst);
}
