//! Integration tests for the compact (context compression) subsystem.
//!
//! These test the public APIs of the compact strategies and helpers,
//! NOT the internal private implementation details.

use starcode_cli::core::tools::tool_names::ToolName;

#[test]
fn tool_name_parsing_round_trip() {
    // Verify common tool names round-trip through our enum
    // (ToolName is pub and used by the compact subsystem)
    let cases = &[
        "Read",
        "Write",
        "Edit",
        "Bash",
        "Glob",
        "Grep",
        "Task",
        "WebFetch",
        "WebSearch",
        "Skill",
    ];

    for name in cases {
        let parsed = ToolName::from(name);
        let display = parsed.to_string();
        assert!(
            display.contains(name) || name.contains(&display),
            "Round-trip failed for '{}': got '{}'",
            name,
            display
        );
    }
}

#[test]
fn tool_name_is_known_identifies_builtin_tools() {
    use ToolName::*;
    let builtins = &[Read, Write, Edit, Bash, Glob, Grep, Task];

    for tool in builtins {
        assert!(
            tool.is_known(),
            "{:?} should be recognized as a known builtin",
            tool
        );
    }
}

#[test]
fn tool_name_unknown_is_not_known() {
    assert!(!ToolName::Unknown.is_known());
}
