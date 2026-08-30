//! LLM module integration tests.
//!
//! Tests the public LLM API types and behaviors.
//! NOTE: These are offline tests — no real API calls are made.

use starcode_cli::types::StarMessage;

// ── Message structure tests ──

#[test]
fn system_message_has_correct_role() {
    let msg = StarMessage::system("You are a helpful assistant".to_string());
    assert_eq!(msg.role, "system");
    assert!(!msg.content.is_empty());
}

#[test]
fn user_message_has_correct_role() {
    let msg = StarMessage::user("Hello".to_string());
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "Hello");
}

#[test]
fn assistant_message_has_correct_role() {
    let msg = StarMessage::assistant("Hi there!".to_string());
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "Hi there!");
}

#[test]
fn tool_message_stores_name_and_id() {
    let msg = StarMessage::tool(
        "read_file".to_string(),
        "tool-001".to_string(),
        "File contents here".to_string(),
    );
    assert_eq!(msg.role, "tool");
    assert_eq!(msg.tool_name, Some("read_file".to_string()));
    assert_eq!(msg.tool_call_id, Some("tool-001".to_string()));
}

// ── Message round-trip sanity ──

#[test]
fn messages_can_roundtrip_through_serialization() {
    let input = vec![
        StarMessage::system("System prompt".to_string()),
        StarMessage::user("Question".to_string()),
        StarMessage::assistant("Answer".to_string()),
    ];

    let json = serde_json::to_string(&input).unwrap();
    let output: Vec<StarMessage> = serde_json::from_str(&json).unwrap();

    assert_eq!(input.len(), output.len());
    for (a, b) in input.iter().zip(output.iter()) {
        assert_eq!(a.role, b.role);
        assert_eq!(a.content, b.content);
    }
}
