// ── Reciprocal Rank Fusion (RRF) ─────────────────────────────────────────────
//
// P2 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Merges ranked result lists from multiple search strategies into a single
// consensus ranking.  RRF is the standard hybrid-search fusion algorithm used
// by ACE, OCE, and other production search engines — it requires no training
// and no score normalisation.
//
// Formula:  RRF_score(d) = Σ 1 / (k + rank_i(d))
//   where k = 60 (standard constant) and rank_i(d) is the position of document
//   d in result-set i (1-based).

use std::collections::HashMap;

/// A search result from a single strategy, before fusion.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Stable identifier — typically "file_path:chunk_start_line"
    pub id: String,
    /// Original score from the search strategy (for tie-breaking)
    pub score: f64,
}

/// A fused result with its combined RRF score.
#[derive(Debug, Clone)]
pub struct FusedResult {
    pub id: String,
    pub rrf_score: f64,
    /// Which result sets contributed to this fusion
    pub sources: Vec<usize>,
}

/// Parameters for Reciprocal Rank Fusion.
pub struct RrfParams {
    /// The k constant — higher values dampen rank differences.
    /// Standard value is 60 (used by ACE, Elasticsearch, etc.).
    pub k: f64,
    /// Optional weight per result set.  If provided, must match the number of
    /// result sets; set weights[0] = 2.0 to double-weight the first strategy.
    pub weights: Option<Vec<f64>>,
}

impl Default for RrfParams {
    fn default() -> Self {
        Self {
            k: 60.0,
            weights: None,
        }
    }
}

impl RrfParams {
    pub const fn new(k: f64) -> Self {
        Self { k, weights: None }
    }
}

/// Fuse multiple ranked result lists into a single ranking via RRF.
///
/// Each `result_sets[i]` is assumed to be sorted by relevance (best first).
/// The returned vector is sorted by descending RRF score.
pub fn fuse(result_sets: &[Vec<Candidate>], params: &RrfParams) -> Vec<FusedResult> {
    // Map: candidate id → accumulated RRF score + contributing source indices
    let mut fused: HashMap<String, (f64, Vec<usize>)> = HashMap::new();

    for (set_idx, results) in result_sets.iter().enumerate() {
        let weight = params
            .weights
            .as_ref()
            .map(|w| w.get(set_idx).copied().unwrap_or(1.0))
            .unwrap_or(1.0);

        for (rank, candidate) in results.iter().enumerate() {
            // 1-based rank
            let rank_1based = (rank as f64) + 1.0;
            let rrf_score = weight / (params.k + rank_1based);

            let entry = fused
                .entry(candidate.id.clone())
                .or_insert_with(|| (0.0, Vec::new()));
            entry.0 += rrf_score;
            entry.1.push(set_idx);
        }
    }

    let mut output: Vec<FusedResult> = fused
        .into_iter()
        .map(|(id, (score, sources))| FusedResult {
            id,
            rrf_score: score,
            sources,
        })
        .collect();

    // Sort by descending RRF score
    output.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    output
}
