//! Checkpoint - save and restore agent state
//!
//! Responsibilities:
//! - List available file-history snapshots (new API, Claude Code parity)
//!
//! Legacy note: the old `list_checkpoints()` walked `tmp/checkpoints/*.json`,
//! but `create_checkpoint` had zero callers, so that dir was never populated.
//! This wrapper now forwards to the new file-history API
//! (`checkpoint_manager::list_snapshots`).

use crate::utils::checkpoint_manager;

/// List available file-history snapshot ids (newest last).
pub async fn list_checkpoints() -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(checkpoint_manager::list_snapshots(None)
        .await?
        .into_iter()
        .map(|s| s.snapshot_id)
        .collect())
}
