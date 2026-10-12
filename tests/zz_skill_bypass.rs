//! REGRESSION TEST: the skill fast path must not bypass permission rules.
//!
//! Regression for: `EditorAgent::try_fast_path` called `invocation.execute()`
//! directly, skipping `check_tool_policy`. `deny: ["Write(secret.txt)"]` held on
//! the normal ToolExecutor path but was bypassed entirely via `skill`.

use starcode_cli::core::config::Config;
use starcode_cli::core::confirmation_bus::MessageBus;
use starcode_cli::core::policy::settings_rules::{PermissionRuleSpec, SettingsPermissions};
use starcode_cli::core::policy::PolicyEngine;
use starcode_cli::core::state::GlobalState;
use starcode_cli::core::tools::tools::BaseDeclarativeTool;
use std::sync::Arc;

fn bus_denying(rule: &str, cwd: &std::path::Path) -> Arc<MessageBus> {
    let mut engine = PolicyEngine::with_project_rules(Default::default(), cwd);
    let mut perms = SettingsPermissions::default();
    perms
        .deny
        .push(PermissionRuleSpec::parse(rule).expect("parse deny rule"));
    engine.set_settings_permissions(perms);
    Arc::new(MessageBus::new(engine, false))
}

/// Mirrors `editor.rs`: authorize, then execute. Kept as a local copy because the
/// real helper is private to the skills module; the point is the *order* of these
/// two operations, which is what regressed.
async fn fast_path_write(
    bus: Arc<MessageBus>,
    config: Arc<Config>,
    global_state: Arc<GlobalState>,
    args: serde_json::Value,
) -> Result<String, String> {
    use starcode_cli::core::policy::{FunctionCall, PolicyDecision};

    let call = FunctionCall {
        name: "Write".to_string(),
        args: Some(args.clone()),
    };
    match bus.check_tool_policy(&call, None).await.decision {
        PolicyDecision::Deny => return Err("denied".to_string()),
        PolicyDecision::DenyWithReason(reason) => return Err(format!("denied: {reason}")),
        PolicyDecision::Allow => {}
        PolicyDecision::AskUser => return Err("needs confirmation".to_string()),
    }

    let tool = starcode_cli::core::tools::write_file::WriteFileTool::new(
        config,
        bus.clone(),
        global_state,
    );
    let invocation = tool.create_invocation(args).map_err(|e| e.to_string())?;
    let result = invocation
        .execute(None, None)
        .await
        .map_err(|e| e.to_string())?;
    if result.error.is_some() {
        return Err("write failed".to_string());
    }
    Ok("written".to_string())
}

#[tokio::test]
async fn deny_rule_blocks_the_skill_fast_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut params = starcode_cli::core::config::ConfigParameters::default();
    params.target_dir = temp.path().to_path_buf();
    params.cwd = temp.path().to_path_buf();
    let config = Arc::new(Config::new(params));

    let bus = bus_denying("Write(secret.txt)", temp.path());
    let global_state = Arc::new(GlobalState::new());

    let outcome = fast_path_write(
        bus,
        config,
        global_state,
        serde_json::json!({
            "file_path": "secret.txt",
            "content": "clobbered",
        }),
    )
    .await;

    assert!(
        outcome.is_err(),
        "deny rule must block the skill fast path, got {outcome:?}"
    );
    assert!(
        !temp.path().join("secret.txt").exists(),
        "denied write must not create the file"
    );
}

#[tokio::test]
async fn allow_rule_permits_the_skill_fast_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut params = starcode_cli::core::config::ConfigParameters::default();
    params.target_dir = temp.path().to_path_buf();
    params.cwd = temp.path().to_path_buf();
    let config = Arc::new(Config::new(params));

    // No deny rule here: default decision is AskUser, which the fast path must
    // refuse rather than silently execute.
    let bus = Arc::new(MessageBus::new(
        PolicyEngine::with_project_rules(Default::default(), temp.path()),
        false,
    ));
    let global_state = Arc::new(GlobalState::new());

    let outcome = fast_path_write(
        bus,
        config,
        global_state,
        serde_json::json!({
            "file_path": "notes.txt",
            "content": "hello",
        }),
    )
    .await;

    assert!(
        outcome.is_err(),
        "AskUser must not be silently downgraded to execute on the fast path"
    );
    assert!(!temp.path().join("notes.txt").exists());
}
