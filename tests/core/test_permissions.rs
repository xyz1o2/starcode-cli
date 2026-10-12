//! Permission system tests

use starcode_cli::core::events::EventBus;
use starcode_cli::core::permissions::*;

#[test]
fn test_permission_evaluator_no_rules() {
    let evaluator = PermissionEvaluator::new();
    assert_eq!(
        evaluator.evaluate("bash", "ls", None),
        PermissionEffect::Ask
    );
}

#[test]
fn test_permission_evaluator_allow_rule() {
    let evaluator = PermissionEvaluator::with_rules(vec![PermissionRule::allow("bash", "ls")]);
    assert_eq!(
        evaluator.evaluate("bash", "ls", None),
        PermissionEffect::Allow
    );
}

#[test]
fn test_permission_evaluator_deny_rule() {
    let evaluator = PermissionEvaluator::with_rules(vec![PermissionRule::deny("bash", "rm *")]);
    assert_eq!(
        evaluator.evaluate("bash", "rm -rf /", None),
        PermissionEffect::Deny
    );
}

#[test]
fn test_permission_evaluator_wildcard() {
    let evaluator = PermissionEvaluator::with_rules(vec![PermissionRule::allow("bash", "*")]);
    assert_eq!(
        evaluator.evaluate("bash", "ls", None),
        PermissionEffect::Allow
    );
    assert_eq!(
        evaluator.evaluate("bash", "rm -rf /", None),
        PermissionEffect::Allow
    );
}

#[test]
fn test_permission_evaluator_prefix_wildcard() {
    let evaluator = PermissionEvaluator::with_rules(vec![PermissionRule::allow("bash", "/tmp/*")]);
    assert_eq!(
        evaluator.evaluate("bash", "/tmp/test", None),
        PermissionEffect::Allow
    );
    assert_eq!(
        evaluator.evaluate("bash", "/home/test", None),
        PermissionEffect::Ask
    );
}

#[test]
fn test_permission_evaluator_last_match_wins() {
    let evaluator = PermissionEvaluator::with_rules(vec![
        PermissionRule::allow("bash", "*"),
        PermissionRule::deny("bash", "rm *"),
    ]);
    assert_eq!(
        evaluator.evaluate("bash", "rm -rf /", None),
        PermissionEffect::Deny
    );
    assert_eq!(
        evaluator.evaluate("bash", "ls", None),
        PermissionEffect::Allow
    );
}

#[test]
fn test_permission_evaluator_saved_rules() {
    let mut evaluator = PermissionEvaluator::new();
    evaluator.add_saved_rule("project-1", PermissionRule::allow("bash", "ls"));

    assert_eq!(
        evaluator.evaluate("bash", "ls", Some("project-1")),
        PermissionEffect::Allow
    );
    assert_eq!(
        evaluator.evaluate("bash", "ls", Some("project-2")),
        PermissionEffect::Ask
    );
}

#[test]
fn test_permission_rule_creation() {
    let rule = PermissionRule::new("bash", "ls", PermissionEffect::Allow);
    assert_eq!(rule.action, "bash");
    assert_eq!(rule.resource, "ls");
    assert_eq!(rule.effect, PermissionEffect::Allow);

    let rule = PermissionRule::allow("write_file", "*.rs");
    assert_eq!(rule.effect, PermissionEffect::Allow);

    let rule = PermissionRule::deny("bash", "rm *");
    assert_eq!(rule.effect, PermissionEffect::Deny);

    let rule = PermissionRule::ask("bash", "*");
    assert_eq!(rule.effect, PermissionEffect::Ask);
}

#[tokio::test]
async fn test_permission_manager_creation() {
    let events = EventBus::new(100);
    let manager = PermissionManager::new(events);

    assert_eq!(manager.pending_requests().await.len(), 0);
}

#[tokio::test]
async fn test_permission_manager_with_rules() {
    let events = EventBus::new(100);
    let manager = PermissionManager::with_rules(events, vec![PermissionRule::allow("bash", "ls")]);

    assert_eq!(manager.pending_requests().await.len(), 0);
}
