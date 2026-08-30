//! Integration tests for core tool infrastructure.
//!
//! Tests that the tool registry, tool names, and built-in tool definitions
//! are consistent.

use starcode_cli::core::tools::tool_names::ToolName;

#[test]
fn builtin_tool_names_are_unique() {
    // All known builtin ToolName variants should have unique display strings
    use ToolName::*;
    let names: Vec<String> = [Read, Write, Edit, Bash, Glob, Grep, Task]
        .iter()
        .map(|t| t.to_string())
        .collect();

    let mut dedup = names.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(
        names.len(),
        dedup.len(),
        "Tool names must be unique: {:?}",
        names
    );
}

#[test]
fn tool_name_from_string_is_stable() {
    assert_eq!(ToolName::from("Read"), ToolName::Read);
    assert_eq!(ToolName::from("Write"), ToolName::Write);
    assert_eq!(ToolName::from("Glob"), ToolName::Glob);
    assert_eq!(ToolName::from("Grep"), ToolName::Grep);
    assert_eq!(ToolName::from("Bash"), ToolName::Bash);
}

#[test]
fn unknown_string_maps_to_unknown() {
    assert_eq!(
        ToolName::from("some_made_up_tool"),
        ToolName::Unknown
    );
}
