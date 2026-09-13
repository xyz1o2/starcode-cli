//! Context-window selection, resolution, and threshold policy.
//!
//! This module is deliberately pure: callers provide provider/catalog/environment
//! evidence instead of resolving policy by reading global runtime state. That keeps
//! a session's requested value stable and makes the UI and worker report the same
//! effective capacity.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Product fallback when neither the user nor capability evidence supplies a size.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;

/// A user's context-window intent. `Auto` remains distinct from an explicit size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextWindowSelection {
    #[default]
    Auto,
    Fixed(u32),
}

impl ContextWindowSelection {
    pub fn parse(input: &str) -> Result<Self, ContextWindowParseError> {
        if input.trim().eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else {
            parse_context_window(input).map(Self::Fixed)
        }
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Fixed(tokens) => tokens > 0,
        }
    }

    pub fn as_fixed(self) -> Option<u32> {
        match self {
            Self::Auto => None,
            Self::Fixed(tokens) => Some(tokens),
        }
    }

    pub fn display(self) -> String {
        match self {
            Self::Auto => "Auto".to_string(),
            Self::Fixed(tokens) => format_context_window(tokens),
        }
    }
}

/// A validation failure from a context-window input field or environment value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextWindowParseError {
    Empty,
    Zero,
    Invalid,
    Overflow,
}

impl fmt::Display for ContextWindowParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "context window size is required",
            Self::Zero => "context window size must be greater than zero",
            Self::Invalid => "use a positive token count, or a whole number followed by k or m",
            Self::Overflow => "context window size is too large",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ContextWindowParseError {}

/// Parses a positive raw token count or a whole-number `k` / `m` value.
pub fn parse_context_window(input: &str) -> Result<u32, ContextWindowParseError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(ContextWindowParseError::Empty);
    }

    let normalized = raw.to_ascii_lowercase();
    let (digits, multiplier) = match normalized.strip_suffix('k') {
        Some(digits) => (digits, 1_000),
        None => match normalized.strip_suffix('m') {
            Some(digits) => (digits, 1_000_000),
            None => (normalized.as_str(), 1),
        },
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ContextWindowParseError::Invalid);
    }

    let value = digits
        .parse::<u32>()
        .map_err(|_| ContextWindowParseError::Overflow)?;
    if value == 0 {
        return Err(ContextWindowParseError::Zero);
    }
    value
        .checked_mul(multiplier)
        .ok_or(ContextWindowParseError::Overflow)
}

/// Formats the two product presets while preserving every custom value exactly.
pub fn format_context_window(tokens: u32) -> String {
    match tokens {
        1_000_000 => "1M".to_string(),
        2_000_000 => "2M".to_string(),
        _ => tokens.to_string(),
    }
}

/// Where the nominal capacity came from. Fixed user choices are separate from
/// provider capability evidence so a later model refresh cannot erase intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextWindowSource {
    FixedSelection,
    ProviderCapability,
    KnownModelCapability,
    Environment,
    ConfiguredDefault,
    BuiltInFallback,
}

impl ContextWindowSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::FixedSelection => "selected",
            Self::ProviderCapability => "provider capability",
            Self::KnownModelCapability => "known model capability",
            Self::Environment => "STAR_CONTEXT_WINDOW",
            Self::ConfiguredDefault => "configured default",
            Self::BuiltInFallback => "built-in default",
        }
    }
}

/// Concrete evidence available while resolving one active model/provider scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextWindowEvidence {
    pub provider_capability: Option<u32>,
    pub known_model_capability: Option<u32>,
    pub environment_default: Option<u32>,
    pub configured_default: Option<u32>,
    /// A provider-limit response recorded for this session and exact scope.
    pub provider_safe_cap: Option<u32>,
}

/// Outcome communicated to the UI for an individual setting mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSettingOutcome {
    Applied,
    Degraded,
    Rejected,
    Failed,
    Superseded,
}

impl RuntimeSettingOutcome {
    pub fn is_terminal(self) -> bool {
        true
    }
}

/// Fully resolved, worker-authoritative context policy for the next turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContextPolicy {
    pub selection: ContextWindowSelection,
    pub nominal_tokens: u32,
    pub effective_tokens: u32,
    pub source: ContextWindowSource,
    pub outcome: RuntimeSettingOutcome,
    pub reason: Option<String>,
    pub pre_send_threshold: u32,
    pub budget_nudge_threshold: u32,
    pub compact_max_tokens: u32,
    pub compact_target_tokens: u32,
}

impl ResolvedContextPolicy {
    /// The late compressor remains independent and intentionally runs nearer
    /// the boundary; these thresholds drive proactive admission and compaction.
    pub fn from_evidence(
        selection: ContextWindowSelection,
        evidence: ContextWindowEvidence,
    ) -> Self {
        let usable = |tokens: Option<u32>| tokens.filter(|tokens| *tokens > 0);
        let (nominal_tokens, source) = match selection {
            ContextWindowSelection::Fixed(tokens) if tokens > 0 => {
                (tokens, ContextWindowSource::FixedSelection)
            }
            ContextWindowSelection::Fixed(_) | ContextWindowSelection::Auto => {
                usable(evidence.provider_capability)
                    .map(|tokens| (tokens, ContextWindowSource::ProviderCapability))
                    .or_else(|| {
                        usable(evidence.known_model_capability)
                            .map(|tokens| (tokens, ContextWindowSource::KnownModelCapability))
                    })
                    .or_else(|| {
                        usable(evidence.environment_default)
                            .map(|tokens| (tokens, ContextWindowSource::Environment))
                    })
                    .or_else(|| {
                        usable(evidence.configured_default)
                            .map(|tokens| (tokens, ContextWindowSource::ConfiguredDefault))
                    })
                    .unwrap_or((DEFAULT_CONTEXT_WINDOW, ContextWindowSource::BuiltInFallback))
            }
        };

        let effective_tokens = usable(evidence.provider_safe_cap)
            .map(|cap| nominal_tokens.min(cap))
            .unwrap_or(nominal_tokens);
        let degraded = effective_tokens < nominal_tokens;
        let reason = degraded.then(|| {
            format!(
                "Provider limited this session to {} (requested {}).",
                format_context_window(effective_tokens),
                format_context_window(nominal_tokens)
            )
        });

        Self {
            selection,
            nominal_tokens,
            effective_tokens,
            source,
            outcome: if degraded {
                RuntimeSettingOutcome::Degraded
            } else {
                RuntimeSettingOutcome::Applied
            },
            reason,
            pre_send_threshold: fraction(effective_tokens, 80),
            budget_nudge_threshold: fraction(effective_tokens, 90),
            compact_max_tokens: fraction(effective_tokens, 75),
            compact_target_tokens: fraction(effective_tokens, 50),
        }
    }
}

fn fraction(tokens: u32, percent: u32) -> u32 {
    ((tokens as u64 * percent as u64) / 100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_suffixed_and_auto_selections() {
        assert_eq!(parse_context_window("200000"), Ok(200_000));
        assert_eq!(parse_context_window(" 128K "), Ok(128_000));
        assert_eq!(parse_context_window("2m"), Ok(2_000_000));
        assert_eq!(
            ContextWindowSelection::parse("AUTO"),
            Ok(ContextWindowSelection::Auto)
        );
        assert_eq!(
            ContextWindowSelection::parse("1M"),
            Ok(ContextWindowSelection::Fixed(1_000_000))
        );
    }

    #[test]
    fn rejects_invalid_zero_and_overflow_values() {
        assert_eq!(
            parse_context_window(""),
            Err(ContextWindowParseError::Empty)
        );
        assert_eq!(
            parse_context_window("0"),
            Err(ContextWindowParseError::Zero)
        );
        assert_eq!(
            parse_context_window("0k"),
            Err(ContextWindowParseError::Zero)
        );
        assert_eq!(
            parse_context_window("1.5m"),
            Err(ContextWindowParseError::Invalid)
        );
        assert_eq!(
            parse_context_window("500g"),
            Err(ContextWindowParseError::Invalid)
        );
        assert_eq!(
            parse_context_window("4294968k"),
            Err(ContextWindowParseError::Overflow)
        );
    }

    #[test]
    fn formatter_abbreviates_only_the_product_presets() {
        assert_eq!(format_context_window(1_000_000), "1M");
        assert_eq!(format_context_window(2_000_000), "2M");
        assert_eq!(format_context_window(128_000), "128000");
        assert_eq!(format_context_window(3_000_000), "3000000");
        assert_eq!(format_context_window(1_048_576), "1048576");
    }

    #[test]
    fn auto_uses_evidence_in_priority_order() {
        let cases = [
            (
                ContextWindowEvidence {
                    provider_capability: Some(1_000_000),
                    known_model_capability: Some(900_000),
                    environment_default: Some(800_000),
                    configured_default: Some(700_000),
                    provider_safe_cap: None,
                },
                1_000_000,
                ContextWindowSource::ProviderCapability,
            ),
            (
                ContextWindowEvidence {
                    known_model_capability: Some(900_000),
                    environment_default: Some(800_000),
                    configured_default: Some(700_000),
                    ..Default::default()
                },
                900_000,
                ContextWindowSource::KnownModelCapability,
            ),
            (
                ContextWindowEvidence {
                    environment_default: Some(800_000),
                    configured_default: Some(700_000),
                    ..Default::default()
                },
                800_000,
                ContextWindowSource::Environment,
            ),
            (
                ContextWindowEvidence {
                    configured_default: Some(700_000),
                    ..Default::default()
                },
                700_000,
                ContextWindowSource::ConfiguredDefault,
            ),
            (
                ContextWindowEvidence::default(),
                DEFAULT_CONTEXT_WINDOW,
                ContextWindowSource::BuiltInFallback,
            ),
        ];

        for (evidence, tokens, source) in cases {
            let policy =
                ResolvedContextPolicy::from_evidence(ContextWindowSelection::Auto, evidence);
            assert_eq!((policy.effective_tokens, policy.source), (tokens, source));
        }
    }

    #[test]
    fn fixed_selection_survives_capability_changes_but_reports_degradation() {
        let policy = ResolvedContextPolicy::from_evidence(
            ContextWindowSelection::Fixed(1_000_000),
            ContextWindowEvidence {
                provider_capability: Some(200_000),
                provider_safe_cap: Some(256_000),
                ..ContextWindowEvidence::default()
            },
        );
        assert_eq!(policy.nominal_tokens, 1_000_000);
        assert_eq!(policy.effective_tokens, 256_000);
        assert_eq!(policy.source, ContextWindowSource::FixedSelection);
        assert_eq!(policy.outcome, RuntimeSettingOutcome::Degraded);
        assert_eq!(
            policy.reason.as_deref(),
            Some("Provider limited this session to 256000 (requested 1M).")
        );
        assert_eq!(policy.pre_send_threshold, 204_800);
        assert_eq!(policy.budget_nudge_threshold, 230_400);
        assert_eq!(policy.compact_max_tokens, 192_000);
        assert_eq!(policy.compact_target_tokens, 128_000);
    }

    #[test]
    fn ignores_zero_valued_evidence() {
        let policy = ResolvedContextPolicy::from_evidence(
            ContextWindowSelection::Auto,
            ContextWindowEvidence {
                provider_capability: Some(0),
                known_model_capability: Some(0),
                environment_default: Some(0),
                configured_default: Some(0),
                provider_safe_cap: Some(0),
            },
        );

        assert_eq!(policy.effective_tokens, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(policy.source, ContextWindowSource::BuiltInFallback);
        assert_eq!(policy.outcome, RuntimeSettingOutcome::Applied);
    }
}
