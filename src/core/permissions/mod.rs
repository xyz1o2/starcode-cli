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
    use std::collections::{HashMap, HashSet};
    use std::io::{self, Write};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, RwLock};

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
        /// 同一授权身份的门禁串行决策，实际工具执行仍可并行。
        approval_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
        persisted: Mutex<PersistedGrants>,
        persist_path: Option<PathBuf>,
    }

    #[derive(Debug, Default)]
    struct PersistedGrants {
        loaded: bool,
        actions: HashSet<String>,
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

        /// 与授权缓存共用生命周期，跨执行器/子代理也不能提前发出重复确认。
        pub(crate) fn approval_lock(
            &self,
            tool_name: &str,
            identity: Option<&str>,
        ) -> Arc<tokio::sync::Mutex<()>> {
            self.approval_locks
                .lock()
                .expect("approval lock map poisoned")
                .entry(identity.unwrap_or(tool_name).to_string())
                .or_default()
                .clone()
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

        pub fn allow_action_persisted(&self, tool_name: &str, args: &Value) -> io::Result<()> {
            self.allow_action_persisted_with_identity(tool_name, args, None)
        }

        pub fn allow_action_persisted_with_identity(
            &self,
            tool_name: &str,
            args: &Value,
            identity: Option<&str>,
        ) -> io::Result<()> {
            let signature = self.get_signature(tool_name, args);
            let signature = self.rewrite_signature(signature, identity);
            self.persist_signature(signature)
        }

        pub fn allow_tool_persisted(&self, tool_name: &str) -> io::Result<()> {
            self.allow_tool_persisted_with_identity(tool_name, None)
        }

        pub fn allow_tool_persisted_with_identity(
            &self,
            tool_name: &str,
            identity: Option<&str>,
        ) -> io::Result<()> {
            self.persist_signature(Self::tool_persisted_signature(
                identity.unwrap_or(tool_name),
            ))
        }

        fn persist_signature(&self, signature: String) -> io::Result<()> {
            if self.persist_path.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "permission persistence path is not configured",
                ));
            }
            // 首次加载、候选集合和磁盘提交共用一把锁，避免并发写入覆盖彼此。
            let mut persisted = self
                .persisted
                .lock()
                .expect("persisted grants lock poisoned");
            self.ensure_persisted_loaded(&mut persisted)?;
            if persisted.actions.contains(&signature) {
                return Ok(());
            }
            let mut candidate = persisted.actions.clone();
            candidate.insert(signature);
            self.save_persisted(&candidate)?;
            persisted.actions = candidate;
            Ok(())
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

            let signature = self.get_signature(tool_name, args);
            let signature = self.rewrite_signature(signature, identity);
            let signature_persisted = {
                let mut persisted = self
                    .persisted
                    .lock()
                    .expect("persisted grants lock poisoned");
                if let Err(err) = self.ensure_persisted_loaded(&mut persisted) {
                    crate::utils::logging::append_debug_log_line(&format!(
                        "[PERMISSIONS] failed to load persisted grants from {:?}: {}",
                        self.persist_path, err
                    ));
                }
                let sig = Self::tool_persisted_signature(identity.unwrap_or(tool_name));
                if persisted.actions.contains(&sig) {
                    return PermissionHit::ToolPersisted;
                }
                persisted.actions.contains(&signature)
            };

            // Check signature-level session
            {
                let set = self.allowed_actions.read().unwrap();
                if set.contains(&signature) {
                    return PermissionHit::SignatureSession;
                }
            }

            if signature_persisted {
                return PermissionHit::SignaturePersisted;
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

        fn ensure_persisted_loaded(&self, persisted: &mut PersistedGrants) -> io::Result<()> {
            if persisted.loaded {
                return Ok(());
            }
            if let Some(path) = &self.persist_path {
                match std::fs::read_to_string(path) {
                    Ok(data) => {
                        persisted.actions = serde_json::from_str(&data)
                            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    }
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
            persisted.loaded = true;
            Ok(())
        }

        fn save_persisted(&self, actions: &HashSet<String>) -> io::Result<()> {
            let path = self.persist_path.as_ref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "permission persistence path is not configured",
                )
            })?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            std::fs::create_dir_all(parent)?;
            // 临时文件与目标同目录，写入并同步成功后才原子替换旧授权。
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            let data = serde_json::to_vec(actions)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            temporary.write_all(&data)?;
            temporary.as_file().sync_all()?;
            temporary.persist(path).map_err(|err| err.error)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::legacy::SessionPermissionManager;
    use std::path::PathBuf;

    #[test]
    fn approval_locks_follow_grant_identity_and_manager_scope() {
        use std::sync::Arc;
        let manager = SessionPermissionManager::new();
        let lock = manager.approval_lock("Read", Some("plugin:a"));
        assert!(Arc::ptr_eq(
            &lock,
            &manager.approval_lock("Read", Some("plugin:a"))
        ));
        assert!(!Arc::ptr_eq(
            &lock,
            &manager.approval_lock("Read", Some("plugin:b"))
        ));
        assert!(!Arc::ptr_eq(&lock, &manager.approval_lock("Read", None)));
        assert!(!Arc::ptr_eq(
            &manager.approval_lock("Read", None),
            &manager.approval_lock("Write", None),
        ));
        assert!(!Arc::ptr_eq(
            &lock,
            &SessionPermissionManager::new().approval_lock("Read", Some("plugin:a")),
        ));
    }

    /// 回归：点一次"总是允许"不能摧毁手写的权限规则。
    ///
    /// 两种格式曾共用 `.star/permissions.json`：授权签名写 JSON 数组，
    /// 而 `PermissionRuleEngine` 按 `{"rules":[...]}` 对象读。写入方整份覆盖，
    /// 读取方解析失败又被 `let _ =` 吞掉 —— 用户的手写 deny 规则无声消失。
    /// 现在写入方只碰 `permission_grants.json`，规则文件原封不动。
    #[test]
    fn always_allow_never_overwrites_hand_written_rules() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let star_dir = temp.path().join(".star");
        std::fs::create_dir_all(&star_dir).expect("create .star");

        let rules_path: PathBuf = star_dir.join("permissions.json");
        let grants_path: PathBuf = star_dir.join("permission_grants.json");

        let handwritten =
            r#"{"rules":[{"tool":"Bash","action":"deny","path":null,"priority":10}]}"#;
        std::fs::write(&rules_path, handwritten).expect("seed hand-written rules");

        // 用户点击"3. 永久允许"。
        let manager = SessionPermissionManager::with_persistence(grants_path.clone());
        manager
            .allow_tool_persisted("Bash")
            .expect("persist tool grant");

        assert_eq!(
            std::fs::read_to_string(&rules_path).expect("rules file still readable"),
            handwritten,
            "hand-written rules must be untouched"
        );

        let grants = std::fs::read_to_string(&grants_path).expect("grants file written");
        let parsed: Vec<String> = serde_json::from_str(&grants).expect("grants is a string array");
        assert!(parsed.iter().any(|s| s.contains("Bash")));
    }

    #[test]
    fn persisted_grants_retry_after_parent_file_is_repaired() {
        use super::PermissionHit;
        let temp = tempfile::tempdir().expect("create temporary directory");
        for variant in 0..4 {
            let parent = temp.path().join(format!("parent-{variant}"));
            std::fs::write(&parent, "not a directory").expect("block grant directory");
            let path = parent.join("permission_grants.json");
            let manager = SessionPermissionManager::with_persistence(path.clone());
            let args = serde_json::json!({"path": "src/lib.rs"});
            let identity = (variant % 2 == 1).then_some("plugin:trusted");
            let grant = || match variant {
                0 => manager.allow_action_persisted("Read", &args),
                1 => manager.allow_action_persisted_with_identity("Read", &args, identity),
                2 => manager.allow_tool_persisted("Read"),
                _ => manager.allow_tool_persisted_with_identity("Read", identity),
            };
            assert!(grant().is_err(), "variant {variant} must report failure");
            assert_eq!(
                manager.check_allowed_with_identity("Read", &args, identity),
                PermissionHit::None
            );
            std::fs::remove_file(&parent).expect("remove blocking file");
            grant().expect("retry persists the same grant");
            let expected = if variant < 2 {
                PermissionHit::SignaturePersisted
            } else {
                PermissionHit::ToolPersisted
            };
            assert_eq!(
                manager.check_allowed_with_identity("Read", &args, identity),
                expected
            );
            let reloaded = SessionPermissionManager::with_persistence(path);
            assert_eq!(
                reloaded.check_allowed_with_identity("Read", &args, identity),
                expected
            );
        }
    }

    #[test]
    fn persisted_grants_failed_write_does_not_publish_candidate_in_memory() {
        use super::PermissionHit;
        let temp = tempfile::tempdir().expect("create temporary directory");
        let parent = temp.path().join("grants");
        let path = parent.join("permission_grants.json");
        let manager = SessionPermissionManager::with_persistence(path.clone());
        manager.allow_tool_persisted("Read").expect("seed grant");
        std::fs::remove_file(&path).expect("remove grant file");
        std::fs::remove_dir(&parent).expect("remove grant directory");
        std::fs::write(&parent, "not a directory").expect("block replacement directory");
        assert!(manager.allow_tool_persisted("Edit").is_err());
        let args = serde_json::json!({});
        assert_eq!(manager.check_allowed("Edit", &args), PermissionHit::None);
        assert_eq!(
            manager.check_allowed("Read", &args),
            PermissionHit::ToolPersisted
        );
        std::fs::remove_file(&parent).expect("repair parent");
        manager.allow_tool_persisted("Edit").expect("retry grant");
        let reloaded = SessionPermissionManager::with_persistence(path);
        assert_eq!(
            reloaded.check_allowed("Read", &args),
            PermissionHit::ToolPersisted
        );
        assert_eq!(
            reloaded.check_allowed("Edit", &args),
            PermissionHit::ToolPersisted
        );
    }

    #[test]
    fn persisted_grants_atomic_replace_failure_cleans_temp_and_can_retry() {
        use super::PermissionHit;
        let temp = tempfile::tempdir().expect("create temporary directory");
        let path = temp.path().join("permission_grants.json");
        let manager = SessionPermissionManager::with_persistence(path.clone());
        let args = serde_json::json!({});
        assert_eq!(manager.check_allowed("Edit", &args), PermissionHit::None);
        std::fs::create_dir(&path).expect("block replacement with directory");
        assert!(manager.allow_action_persisted("Edit", &args).is_err());
        assert_eq!(manager.check_allowed("Edit", &args), PermissionHit::None);
        assert_eq!(
            std::fs::read_dir(temp.path()).expect("read parent").count(),
            1,
            "failed replacement must remove its temporary file"
        );
        std::fs::remove_dir(&path).expect("repair target");
        manager
            .allow_action_persisted("Edit", &args)
            .expect("retry replacement");
        let grants: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read JSON grants"))
                .expect("existing string-array format");
        assert_eq!(grants, vec![format!("Edit:{args}")]);
        let reloaded = SessionPermissionManager::with_persistence(path);
        assert_eq!(
            reloaded.check_allowed("Edit", &args),
            PermissionHit::SignaturePersisted
        );
    }

    #[test]
    fn persisted_grants_concurrent_first_load_and_writes_preserve_every_entry() {
        use super::PermissionHit;
        use std::collections::HashSet;
        use std::sync::{Arc, Barrier};
        let temp = tempfile::tempdir().expect("create temporary directory");
        let path = temp.path().join("permission_grants.json");
        std::fs::write(&path, r#"["__tool__:Existing"]"#).expect("seed existing grant");
        let manager = Arc::new(SessionPermissionManager::with_persistence(path.clone()));
        let barrier = Arc::new(Barrier::new(16));
        std::thread::scope(|scope| {
            for index in 0..16 {
                let manager = Arc::clone(&manager);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    assert_eq!(
                        manager.check_allowed("Existing", &serde_json::json!({})),
                        PermissionHit::ToolPersisted
                    );
                    let identity = format!("plugin:{index}");
                    let args = serde_json::json!({"index": index});
                    manager
                        .allow_action_persisted_with_identity("Read", &args, Some(&identity))
                        .expect("persist concurrent action");
                    manager
                        .allow_tool_persisted_with_identity("Edit", Some(&identity))
                        .expect("persist concurrent tool");
                });
            }
        });
        let grants: HashSet<String> =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read concurrent grants"))
                .expect("complete JSON document");
        assert_eq!(grants.len(), 33);
        let reloaded = SessionPermissionManager::with_persistence(path);
        assert_eq!(
            reloaded.check_allowed("Existing", &serde_json::json!({})),
            PermissionHit::ToolPersisted
        );
        for index in 0..16 {
            let identity = format!("plugin:{index}");
            let args = serde_json::json!({"index": index});
            assert_eq!(
                reloaded.check_allowed_with_identity("Read", &args, Some(&identity)),
                PermissionHit::ToolPersisted
            );
            assert!(
                grants.contains(&format!("{identity}:Read:{args}")),
                "action {index}"
            );
            assert!(
                grants.contains(&format!("__tool__:{identity}")),
                "tool {index}"
            );
        }
    }

    #[test]
    fn persisted_grants_failed_first_load_retries_without_overwriting_existing_data() {
        use super::PermissionHit;
        let temp = tempfile::tempdir().expect("create temporary directory");
        let path = temp.path().join("permission_grants.json");
        std::fs::write(&path, "incomplete JSON").expect("seed invalid file");
        let manager = SessionPermissionManager::with_persistence(path.clone());
        assert!(manager.allow_tool_persisted("Edit").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("read original file"),
            "incomplete JSON"
        );
        std::fs::write(&path, r#"["__tool__:Read"]"#).expect("repair persisted grants");
        manager
            .allow_tool_persisted("Edit")
            .expect("retry first load");
        let reloaded = SessionPermissionManager::with_persistence(path);
        for tool in ["Read", "Edit"] {
            assert_eq!(
                reloaded.check_allowed(tool, &serde_json::json!({})),
                PermissionHit::ToolPersisted
            );
        }
    }

    #[test]
    fn persisted_grants_without_path_return_error_instead_of_false_success() {
        use super::PermissionHit;
        let manager = SessionPermissionManager::new();
        let args = serde_json::json!({});
        assert!(manager.allow_action_persisted("Read", &args).is_err());
        assert!(manager.allow_tool_persisted("Edit").is_err());
        assert_eq!(manager.check_allowed("Read", &args), PermissionHit::None);
        assert_eq!(manager.check_allowed("Edit", &args), PermissionHit::None);
    }
}
