//! Integration tests for agent routing.

use starcode_cli::agent::router::{route_choice, RouteResult};

#[test]
fn route_to_agent_returns_correct_variant() {
    let result = RouteResult::RouteToAgent("test-agent".to_string());
    match route_choice(&result) {
        Some(id) => assert_eq!(id, "test-agent"),
        None => panic!("Expected RouteToAgent"),
    }
}

#[test]
fn route_stay_returns_none() {
    let result = RouteResult::Stay;
    assert!(
        route_choice(&result).is_none(),
        "Stay should produce None choice"
    );
}
