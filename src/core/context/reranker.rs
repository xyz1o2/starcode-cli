// ── Heuristic Reranker ────────────────────────────────────────────────────────
//
// P2 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Re-ranks fused search candidates using code-aware heuristics — a lightweight
// alternative to LLM-based reranking that requires no API calls.
//
// When the architecture supports threading an LLM client through the tool layer,
// the `Reranker` trait can be implemented with provider-based scoring for
// maximum quality (the ACE/OCE approach).

use super::search_engine::SearchResult;
use std::collections::HashSet;

/// A candidate passed to the reranker — already scored but needing a second pass.
#[derive(Debug, Clone)]
pub struct RerankCandidate {
    pub id: String,
    pub file_path: String,
    pub content: String,
    pub context_header: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub initial_score: f64,
    pub matched_terms: Vec<String>,
}

impl RerankCandidate {
    /// Build a RerankCandidate from a SearchResult.
    pub fn from_search_result(r: &SearchResult) -> Self {
        Self {
            id: format!("{}:{}", r.file_path, r.chunk.start_line),
            file_path: r.file_path.clone(),
            content: r.chunk.content.clone(),
            context_header: r.chunk.context_header.clone(),
            start_line: r.chunk.start_line,
            end_line: r.chunk.end_line,
            initial_score: r.score,
            matched_terms: r.matches.clone(),
        }
    }
}

/// A reranked result with the final score and explanation.
#[derive(Debug, Clone)]
pub struct RerankedResult {
    pub candidate: RerankCandidate,
    pub final_score: f64,
    pub boost_signals: Vec<String>,
}

/// Trait for reranking strategies.
pub trait Reranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<RerankCandidate>,
        top_n: usize,
    ) -> Vec<RerankedResult>;
}

// ── Heuristic Reranker ────────────────────────────────────────────────────────

/// Scoring weights for the heuristic reranker.
mod weights {
    /// Chunk's context_header contains a query token.
    pub const HEADER_QUERY_MATCH: f64 = 1.8;
    /// File path contains a query token.
    pub const PATH_QUERY_MATCH: f64 = 1.5;
    /// Chunk content contains the full query string.
    pub const FULL_QUERY_IN_CONTENT: f64 = 1.4;
    /// Chunk covers all core query terms.
    pub const FULL_COVERAGE: f64 = 1.3;
    /// Chunk is a named definition.
    pub const NAMED_DEFINITION_BOOST: f64 = 1.25;
}



/// A heuristic code-aware reranker — no API calls needed.
///
/// Re-scores candidates by applying stronger versions of the first-pass signals,
/// plus cross-candidate comparisons (e.g., penalizing duplicates).
pub struct HeuristicReranker;

impl Reranker for HeuristicReranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<RerankCandidate>,
        top_n: usize,
    ) -> Vec<RerankedResult> {
        let query_lower = query.to_lowercase();
        let query_tokens: HashSet<&str> = query_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 2)
            .collect();

        // ── Pass 1: Re-score each candidate ──────────────────────────────────
        let mut scored: Vec<RerankedResult> = candidates
            .iter()
            .map(|c| {
                let mut score = c.initial_score.max(0.01); // keep positive baseline
                let mut signals = Vec::new();
                let content_lower = c.content.to_lowercase();
                let header_lower = c
                    .context_header
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase();
                let path_lower = c.file_path.to_lowercase();

                // Context header contains a query token
                if query_tokens.iter().any(|t| header_lower.contains(t)) {
                    score *= weights::HEADER_QUERY_MATCH;
                    signals.push("header matches query".to_string());
                }

                // File path contains a query token
                if query_tokens.iter().any(|t| path_lower.contains(t)) {
                    score *= weights::PATH_QUERY_MATCH;
                    signals.push("path matches query".to_string());
                }

                // Full query appears verbatim in content
                if content_lower.contains(&query_lower) && query_lower.len() > 3 {
                    score *= weights::FULL_QUERY_IN_CONTENT;
                    signals.push("full query in content".to_string());
                }

                // Covers all query tokens
                if query_tokens.iter().all(|t| content_lower.contains(t)) {
                    score *= weights::FULL_COVERAGE;
                    signals.push("covers all query terms".to_string());
                }

                // Named definition
                if c.context_header.is_some() {
                    score *= weights::NAMED_DEFINITION_BOOST;
                    signals.push("named definition".to_string());
                }

                RerankedResult {
                    candidate: c.clone(),
                    final_score: score,
                    boost_signals: signals,
                }
            })
            .collect();

        // ── Pass 2: Cross-candidate dedup penalty ────────────────────────────
        // Penalize near-duplicate chunks (same file, overlapping line ranges)
        for i in 0..scored.len() {
            for j in (i + 1)..scored.len() {
                if scored[i].candidate.file_path == scored[j].candidate.file_path {
                    let overlap = line_overlap(
                        scored[i].candidate.start_line,
                        scored[i].candidate.end_line,
                        scored[j].candidate.start_line,
                        scored[j].candidate.end_line,
                    );
                    if overlap > 0.5 {
                        // Penalize the lower-scored one
                        if scored[i].final_score >= scored[j].final_score {
                            scored[j].final_score *= 0.7;
                        } else {
                            scored[i].final_score *= 0.7;
                        }
                    }
                }
            }
        }

        // ── Sort and truncate ────────────────────────────────────────────────
        scored.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        scored.truncate(top_n);
        scored
    }
}

/// Fraction of overlap between two line ranges (0.0–1.0).
fn line_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> f64 {
    let overlap_start = a_start.max(b_start);
    let overlap_end = a_end.min(b_end);
    if overlap_end < overlap_start {
        return 0.0;
    }
    let overlap_len = overlap_end - overlap_start + 1;
    let total_len = (a_end - a_start + 1).min(b_end - b_start + 1);
    if total_len == 0 {
        return 0.0;
    }
    overlap_len as f64 / total_len as f64
}
 