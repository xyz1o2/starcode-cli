//! Integration tests for the built-in LLM provider registry.
//!
//! These verify that ALL_PROVIDERS is correctly populated and that
//! every built-in provider has a resolvable base URL + env-var hint.

use starcode_cli::core::config::providers::{
    api_key_env_candidates, get_provider_by_id, provider_requires_manual_base_url,
    resolve_provider_base_url, ALL_PROVIDERS,
};

#[test]
fn all_builtin_providers_are_present() {
    // Every provider that the website advertises must exist
    let expected_ids = [
        "anthropic",
        "openai",
        "deepseek",
        "minimax",
        "stepfun",
        "xiaomi",
        "xiaomi-cn",
        "xiaomi-sgp",
        "xiaomi-ams",
        "openai-compatible",
    ];

    for id in &expected_ids {
        let p = get_provider_by_id(id);
        assert!(
            p.is_some(),
            "Built-in provider '{}' not found in ALL_PROVIDERS",
            id
        );
    }
}

#[test]
fn every_provider_has_name_and_description() {
    for p in ALL_PROVIDERS {
        assert!(
            !p.name.is_empty(),
            "Provider '{}' has empty name",
            p.id
        );
        assert!(
            !p.description.is_empty(),
            "Provider '{}' has empty description",
            p.id
        );
    }
}

#[test]
fn api_key_providers_have_env_hint() {
    for p in ALL_PROVIDERS.iter().filter(|p| p.requires_api_key) {
        let candidates = api_key_env_candidates(p.id);
        assert!(
            !candidates.is_empty(),
            "Provider '{}' requires API key but has no env-var hint",
            p.id
        );
    }
}

#[test]
fn provider_base_url_resolves_to_default() {
    // anthropic has default_base_url and no stored override
    let url = resolve_provider_base_url("anthropic", None);
    assert!(url.is_some(), "anthropic should resolve a base URL");
    assert!(url.unwrap().contains("anthropic"));
}

#[test]
fn openai_compatible_requires_manual_base_url() {
    assert!(provider_requires_manual_base_url("openai-compatible"));
    assert!(!provider_requires_manual_base_url("openai"));
    assert!(!provider_requires_manual_base_url("anthropic"));
}

#[test]
fn unknown_provider_returns_none() {
    assert!(get_provider_by_id("nonexistent-xyz").is_none());
}
