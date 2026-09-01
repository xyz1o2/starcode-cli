//! Permission rule definitions

use serde::{Deserialize, Serialize};

/// Permission effect
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionEffect {
    /// Allow the action
    Allow,
    /// Deny the action
    Deny,
    /// Ask the user for confirmation
    Ask,
}

/// Permission rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Action pattern (e.g., "Bash", "Write", "*")
    pub action: String,
    /// Resource pattern (e.g., "*", "/tmp/*", "rm *")
    pub resource: String,
    /// Effect when matched
    pub effect: PermissionEffect,
}

impl PermissionRule {
    /// Create a new permission rule
    pub fn new(
        action: impl Into<String>,
        resource: impl Into<String>,
        effect: PermissionEffect,
    ) -> Self {
        Self {
            action: action.into(),
            resource: resource.into(),
            effect,
        }
    }

    /// Create an allow rule
    pub fn allow(action: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::new(action, resource, PermissionEffect::Allow)
    }

    /// Create a deny rule
    pub fn deny(action: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::new(action, resource, PermissionEffect::Deny)
    }

    /// Create an ask rule
    pub fn ask(action: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::new(action, resource, PermissionEffect::Ask)
    }
}

/// Saved permission rule for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPermission {
    pub project_id: String,
    pub action: String,
    pub resource: String,
    pub effect: PermissionEffect,
}

/// Permission request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub session_id: String,
    pub action: String,
    pub resources: Vec<String>,
    pub save: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

/// Permission reply
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionReply {
    Once,
    Always,
    Reject,
}

/// Permission error
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("Permission denied")]
    Denied,
    #[error("Permission rejected by user")]
    Rejected,
    #[error("Permission corrected with feedback: {0}")]
    Corrected(String),
}
