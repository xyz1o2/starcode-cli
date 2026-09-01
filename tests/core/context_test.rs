//! Core module integration tests.

use starcode_cli::core::context::call_graph::{CallNode, FunctionDef, extract_calls_from_function};
use starcode_cli::core::config::providers::ALL_PROVIDERS;

#[test]
fn call_graph_extract_basic_function_calls() {
    // Simulate extracting calls from a simple function body
    let nodes = vec![
        CallNode {
            name: "do_work".to_string(),
            line: 42,
        },
        CallNode {
            name: "log_result".to_string(),
            line: 43,
        },
    ];

    let func = FunctionDef {
        name: "run_pipeline".to_string(),
        file: "pipeline.rs".to_string(),
    };

    let calls = extract_calls_from_function(&func, &nodes, "pipeline.rs");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].caller, "run_pipeline");
    assert_eq!(calls[0].callee, "do_work");
    assert_eq!(calls[1].callee, "log_result");
}

#[test]
fn all_providers_count_matches_expectation() {
    // 5 core providers + 4 Xiaomi regions + OpenAI Compatible = 10
    let core_providers: Vec<_> = ALL_PROVIDERS
        .iter()
        .filter(|p| !p.id.starts_with("xiaomi-") && p.id != "openai-compatible")
        .collect();
    assert!(
        core_providers.len() >= 6,
        "Expected at least 6 core providers, got {}",
        core_providers.len()
    );

    let xiaomi_regions: Vec<_> = ALL_PROVIDERS
        .iter()
        .filter(|p| p.id.starts_with("xiaomi-"))
        .collect();
    assert_eq!(xiaomi_regions.len(), 3, "Expected 3 Xiaomi regions");

    let total = ALL_PROVIDERS.len();
    assert_eq!(total, 10, "Expected 10 total providers, got {}", total);
}
