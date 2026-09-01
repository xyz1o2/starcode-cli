//! Permission system for fine-grained access control

pub mod evaluator;
pub mod manager;
pub mod rules;

pub use evaluator::PermissionEvaluator;
pub use manager::PermissionManager;
pub use rules::*;

// Re-export legacy types for backward compatibility
pub use legacy::{PermissionHit, SessionPermissionManager};

/// Legacy permission types for backward compatibility
pub mod legacy {
    use serde_json::Value;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::RwLock;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PermissionHit {
        None,
        ToolSession,
        ToolPersisted,
        SignatureSession,
        SignaturePersisted,
    }

    #[derive(Debug, Default)]
    pub struct SessionPermissionManager {
        allowed_actions: RwLock<HashSet<String>>,
        allowed_tools: RwLock<HashSet<String>>,
        persisted_actions: RwLock<HashSet<String>>,
        persist_path: Option<PathBuf>,
        persisted_loaded: AtomicBool,
    }

    const TOOL_PERSISTED_PREFIX: &str = "__tool__:";

    impl SessionPermissionManager {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_persistence(path: PathBuf) -> Self {
            Self {
                persist_path: Some(path),
                ..Self::default()
            }
        }

        pub fn allow_action(&self, tool_name: &str, args: &Value) {
            self.allow_action_with_identity(tool_name, args, None);
        }

        pub fn allow_action_with_identity(
            &self,
            tool_name: &str,
            args: &Value,
            identity: Option<&str>,
        ) {
            let signature = self.get_signature(tool_name, args);
            let signature = self.rewrite_signature(signature, identity);
            let mut set = self.allowed_actions.write().unwrap();
            set.insert(signature);
        }

        pub fn allow_tool_session(&self, tool_name: &str) {
            self.allow_tool_session_with_identity(tool_name, None);
        }

        pub fn allow_tool_session_with_identity(&self, tool_name: &str, identity: Option<&str>) {
            let mut set = self.allowed_tools.write().unwrap();
            set.insert(identity.unwrap_or(tool_name).to_string());
        }

        pub fn allow_action_persisted(&self, tool_name: &str, args: &Value) {
            self.allow_action_persisted_with_identity(tool_name, args, None);
        }

        pub fn allow_action_persisted_with_identity(
            &self,
            tool_name: &str,
            args: &Value,
            identity: Option<&str>,
        ) {
            self.ensure_persisted_loaded();
            let signature = self.get_signature(tool_name, args);
            let signature = self.rewrite_signature(signature, identity);
            {
                let mut set = self.persisted_actions.write().unwrap();
                if !set.insert(signature) {
                    return;
                }
            }
            self.save_persisted();
        }

        pub fn allow_tool_persisted(&self, tool_name: &str) {
            self.allow_tool_persisted_with_identity(tool_name, None);
        }

        pub fn allow_tool_persisted_with_identity(&self, tool_name: &str, identity: Option<&str>) {
            self.ensure_persisted_loaded();
            let signature = Self::tool_persisted_signature(identity.unwrap_or(tool_name));
            {
                let mut set = self.persisted_actions.write().unwrap();
                if !set.insert(signature) {
                    return;
                }
            }
            self.save_persisted();
        }

        pub fn check_allowed(&self, tool_name: &str, args: &Value) -> PermissionHit {
            self.check_allowed_with_identity(tool_name, args, None)
        }

        pub fn check_allowed_with_identity(
            &self,
            tool_name: &str,
            args: &Value,
            identity: Option<&str>,
        ) -> PermissionHit {
            // Check tool-level session
            {
                let set = self.allowed_tools.read().unwrap();
                if set.contains(identity.unwrap_or(tool_name)) {
                    return PermissionHit::ToolSession;
                }
            }

            // Check tool-level persisted
            {
                self.ensure_persisted_loaded();
                let set = self.persisted_actions.read().unwrap();
                let sig = Self::tool_persisted_signature(identity.unwrap_or(tool_name));
                if set.contains(&sig) {
                    return PermissionHit::ToolPersisted;
                }
            }

            // Check signature-level session
            let signature = self.get_signature(tool_name, args);
            let signature = self.rewrite_signature(signature, identity);
            {
                let set = self.allowed_actions.read().unwrap();
                if set.contains(&signature) {
                    return PermissionHit::SignatureSession;
                }
            }

            // Check signature-level persisted
            {
                self.ensure_persisted_loaded();
                let set = self.persisted_actions.read().unwrap();
                if set.contains(&signature) {
                    return PermissionHit::SignaturePersisted;
                }
            }

            PermissionHit::None
        }

        fn get_signature(&self, tool_name: &str, args: &Value) -> String {
            format!("{}:{}", tool_name, args)
        }

        fn rewrite_signature(&self, signature: String, identity: Option<&str>) -> String {
            if let Some(id) = identity {
                format!("{}:{}", id, signature)
            } else {
                signature
            }
        }

        fn tool_persisted_signature(tool_name: &str) -> String {
            format!("{}{}", TOOL_PERSISTED_PREFIX, tool_name)
        }

        fn ensure_persisted_loaded(&self) {
            if self.persisted_loaded.load(Ordering::Acquire) {
                return;
            }
            if let Some(path) = &self.persist_path {
                if let Ok(data) = std::fs::read_to_string(path) {
                    if let Ok(set) = serde_json::from_str::<HashSet<String>>(&data) {
                        let mut persisted = self.persisted_actions.write().unwrap();
                        *persisted = set;
                    }
                }
            }
            self.persisted_loaded.store(true, Ordering::Release);
        }

        fn save_persisted(&self) {
            if let Some(path) = &self.persist_path {
                let set = self.persisted_actions.read().unwrap();
                if let Ok(data) = serde_json::to_string(&*set) {
                    let _ = std::fs::write(path, data);
                }
            }
        }
    }
}
