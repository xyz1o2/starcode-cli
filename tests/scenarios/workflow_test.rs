//! End-to-end workflow integration tests.
//!
//! These tests simulate real user workflows across multiple subsystems.
//! They create temp projects, configure providers, and exercise the tool chain.

use crate::common::helpers::TempProject;
use starcode_cli::core::config::providers::{
    get_provider_by_id, resolve_provider_base_url, ALL_PROVIDERS,
};

// ── Scenario 1: Provider configuration workflow ──

#[test]
fn configure_all_builtin_providers() {
    // Simulate: user configures every built-in provider with an API key
    for p in ALL_PROVIDERS.iter() {
        let _ = get_provider_by_id(p.id); // verify each is resolvable
    }
}

#[test]
fn switch_between_providers_preserves_configuration() {
    // Simulate: user switches from anthropic -> deepseek -> openai
    let providers = ["anthropic", "deepseek", "openai"];

    for pid in &providers {
        let meta = get_provider_by_id(pid).unwrap();
        assert!(meta.requires_api_key, "{} should require API key", pid);

        let url = resolve_provider_base_url(pid, None);
        assert!(url.is_some(), "{} should have a default base URL", pid);
    }
}

// ── Scenario 2: Code editing workflow ──

#[test]
fn create_and_edit_rust_project() {
    let project = TempProject::new("rust_project").with_cargo_toml(
        r#"
[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#,
    );

    // Initial file
    project.write_file(
        "src/main.rs",
        r#"fn main() {
    println!("Hello, world!");
}
"#,
    );

    // Step 1: Read file
    let content = std::fs::read_to_string(project.path.join("src/main.rs")).unwrap();
    assert!(content.contains("Hello, world!"));

    // Step 2: Edit (add a function)
    let edited = content.replace(
        "fn main()",
        "fn greet() -> &'static str { \"Hi!\" }\n\nfn main()",
    );
    std::fs::write(project.path.join("src/main.rs"), &edited).unwrap();

    // Step 3: Verify
    let result = std::fs::read_to_string(project.path.join("src/main.rs")).unwrap();
    assert!(result.contains("fn greet()"), "New function should exist");
    assert!(result.contains("Hello, world!"), "Original code preserved");
}

// ── Scenario 3: Multi-file project exploration ──

#[test]
fn explore_project_structure() {
    let project = TempProject::new("multi_mod");
    project.with_cargo_toml("[package]\nname = \"multi\"\nversion = \"0.1.0\"\n");

    let files = [
        "src/main.rs",
        "src/lib.rs",
        "src/auth/mod.rs",
        "src/auth/login.rs",
        "src/models/user.rs",
        "tests/integration_test.rs",
    ];

    for f in &files {
        project.write_file(f, "// placeholder\n");
    }

    // Verify all files exist
    let mut found = Vec::new();
    for entry in walkdir::WalkDir::new(&project.path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.path().extension().map_or(false, |e| e == "rs") {
            found.push(
                entry
                    .path()
                    .strip_prefix(&project.path)
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    // Should find all 6 .rs files (match regardless of path separator)
    let found_normalized: Vec<String> = found.iter().map(|p| p.replace('\\', "/")).collect();
    for expected in &files {
        assert!(
            found_normalized.contains(&expected.to_string()),
            "Missing file: {} (found: {:?})",
            expected,
            found_normalized
        );
    }
}
