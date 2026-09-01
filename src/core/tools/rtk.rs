//! RTK (Rust Token Killer) integration.
//!
//! When RTK is installed and enabled, shell commands are transparently prefixed
//! with `rtk` before execution. RTK filters and compresses the output, reducing
//! token consumption by 60-90% on common dev commands.
//!
//! Homepage: <https://www.rtk-ai.app>
//! Repository: <https://github.com/rtk-ai/rtk>

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// Environment variable to explicitly enable or disable RTK.
const ENV_RTK_ENABLED: &str = "STAR_RTK_ENABLED";

/// Commands that RTK can filter effectively.
/// Maps command name → `some(true)` if wrapping is recommended by default.
const RTK_COMPATIBLE_COMMANDS: &[&str] = &[
    // Version control
    "git",
    // Build & test
    "cargo",
    "pytest",
    "jest",
    "vitest",
    "go",
    "rake",
    "rspec",
    "mix",
    // Lint & type-check
    "eslint",
    "tsc",
    "ruff",
    "golangci-lint",
    "rubocop",
    "prettier",
    "biome",
    // Package managers
    "npm",
    "pnpm",
    "yarn",
    "pip",
    "bundle",
    "prisma",
    // File & directory
    "ls",
    "find",
    "cat",
    "head",
    "tail",
    "grep",
    "rg",
    // Containers
    "docker",
    "kubectl",
    // Cloud
    "aws",
    "gcloud",
    // Package listing
    "apt",
    "dpkg",
    "rpm",
];

/// Commands that should NOT be wrapped with RTK even if the base command
/// matches — these are interactive, short-lived, or pass-through operations.
const RTK_EXCLUDED_SUBCOMMANDS: &[&str] = &[
    "git add",
    "git commit",
    "git push",
    "git pull",
    "git fetch",
    "git checkout",
    "git switch",
    "git branch",
    "git tag",
    "git merge",
    "git rebase",
    "git stash",
];

/// Cached RTK availability check (lazy, runs once per process).
static RTK_AVAILABLE: Lazy<Mutex<Option<bool>>> = Lazy::new(|| Mutex::new(None));

/// Check whether RTK is installed and enabled.
///
/// Checks (in order):
/// 1. `STAR_RTK_ENABLED` env var — `"1"`/`"true"` forces on, `"0"`/`"false"` forces off
/// 2. Fall back to detecting the `rtk` binary on PATH
pub fn is_rtk_available() -> bool {
    let mut cache = RTK_AVAILABLE.lock().unwrap();
    if let Some(cached) = *cache {
        return cached;
    }

    let available = detect_rtk();
    *cache = Some(available);
    available
}

fn detect_rtk() -> bool {
    // Explicit env-var override
    if let Ok(val) = std::env::var(ENV_RTK_ENABLED) {
        let v = val.trim().to_ascii_lowercase();
        if v == "1" || v == "true" || v == "on" || v == "yes" {
            return true;
        }
        if v == "0" || v == "false" || v == "off" || v == "no" {
            return false;
        }
    }

    // Detect binary on PATH
    which::which("rtk").is_ok()
}

/// Reset the RTK availability cache. Useful after installation.
pub fn reset_rtk_cache() {
    let mut cache = RTK_AVAILABLE.lock().unwrap();
    *cache = None;
}

/// Decide whether the given shell command should be wrapped with `rtk`.
///
/// Returns `Some(rewritten_command)` if the answer is yes, or `None` if
/// the command should be executed as-is.
pub fn maybe_rtk_wrap(raw_command: &str) -> Option<String> {
    if !is_rtk_available() {
        return None;
    }

    let trimmed = raw_command.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Already prefixed — don't double-wrap
    if trimmed.starts_with("rtk ") || trimmed == "rtk" {
        return None;
    }

    // Extract the first word (the command name)
    let first_word = match trimmed.split_whitespace().next() {
        Some(w) => w,
        None => return None,
    };

    // Resolve common aliases (e.g. `python -m pytest ...` → treat as pytest)
    let effective_cmd = resolve_command_alias(trimmed, first_word);

    // Check if this command is in the RTK-compatible list
    if !RTK_COMPATIBLE_COMMANDS.contains(&effective_cmd) {
        return None;
    }

    // Exclude certain subcommands (e.g. git push is too short to benefit)
    let first_two: String = trimmed
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    if RTK_EXCLUDED_SUBCOMMANDS.contains(&first_two.as_str()) {
        return None;
    }

    // Reconstruct: rtk <original command>
    Some(format!("rtk {}", trimmed))
}

/// Token savings summary from RTK analytics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RtkStats {
    pub available: bool,
    pub version: Option<String>,
    pub total_commands: Option<u64>,
    pub tokens_saved: Option<u64>,
    pub savings_pct: Option<f64>,
}

/// Gather RTK token savings statistics by running `rtk gain --format json`.
/// Returns `None` if RTK is not available or the command fails.
pub async fn fetch_rtk_stats() -> Option<RtkStats> {
    if !is_rtk_available() {
        return None;
    }

    let output = tokio::process::Command::new("rtk")
        .args(["gain", "--all", "--format", "json"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return Some(RtkStats {
            available: true,
            version: get_rtk_version().await,
            total_commands: None,
            tokens_saved: None,
            savings_pct: None,
        });
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(RtkStats {
        available: true,
        version: get_rtk_version().await,
        total_commands: json.get("total_commands").and_then(|v| v.as_u64()),
        tokens_saved: json
            .get("tokens_saved")
            .or_else(|| json.get("total_tokens_saved"))
            .and_then(|v| v.as_u64()),
        savings_pct: json
            .get("savings_pct")
            .or_else(|| json.get("avg_savings_pct"))
            .and_then(|v| v.as_f64()),
    })
}

/// Get the installed RTK version string.
pub async fn get_rtk_version() -> Option<String> {
    let output = tokio::process::Command::new("rtk")
        .arg("--version")
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    None
}

/// Resolve common command aliases to their canonical form for RTK matching.
fn resolve_command_alias<'a>(raw_command: &'a str, first_word: &'a str) -> &'a str {
    // python -m pytest → pytest
    if first_word == "python" || first_word == "python3" {
        let parts: Vec<&str> = raw_command.split_whitespace().collect();
        if parts.len() >= 3 && parts[1] == "-m" {
            let module = parts[2];
            if RTK_COMPATIBLE_COMMANDS.contains(&module) {
                return module;
            }
        }
    }

    first_word
}
