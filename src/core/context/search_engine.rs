use super::chunking::CodeChunk;
use std::collections::{HashMap, HashSet};

// ── Scoring weights ───────────────────────────────────────────────────────────
// All tunable multipliers live here.  Change a constant once; it applies
// everywhere the formula is used.
mod weights {
    // ── Phrase-match boosts ──────────────────────────────────────────────────
    pub const PHRASE_IN_CONTENT: f64 = 1.8;
    pub const PHRASE_IN_HEADER: f64 = 1.6;
    pub const PHRASE_IN_PATH: f64 = 1.45;

    // ── Core-term coverage boosts ────────────────────────────────────────────
    pub const FULL_CORE_COVERAGE: f64 = 1.35;
    pub const PARTIAL_COVERAGE_BASE: f64 = 0.85;
    pub const PARTIAL_COVERAGE_SCALE: f64 = 0.35;

    // ── Structural boosts ────────────────────────────────────────────────────
    pub const HEADER_TERM_MATCH: f64 = 1.5;
    pub const PATH_TERM_MATCH: f64 = 1.2;
    pub const NAMED_DEFINITION_BOOST: f64 = 1.2; // chunk has a context_header

    // ── Penalties ────────────────────────────────────────────────────────────
    pub const LARGE_CHUNK: f64 = 0.92;
    pub const LARGE_CHUNK_LINES: usize = 120;
    pub const SMALL_SNIPPET_PENALTY: f64 = 0.5;
    pub const SMALL_SNIPPET_LINES: usize = 3;

    // ── Result shaping ───────────────────────────────────────────────────────
    pub const MAX_MATCHED_TERMS: usize = 12;
    pub const MAX_SIGNALS: usize = 6;

    // ── File diversity decay (diminishing returns per file) ──────────────────
    pub const FILE_DIVERSITY_DECAY: [f64; 5] = [1.00, 0.85, 0.65, 0.40, 0.25];

    // ── Indexing ─────────────────────────────────────────────────────────────
    pub const MIN_TOKEN_LEN: usize = 2;
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub chunk: CodeChunk,
    pub score: f64,
    pub matches: Vec<String>,
    pub signals: Vec<String>,
}

#[derive(Clone)]
pub struct SearchEngine {
    // Inverted index: word -> list of (file_index, chunk_index)
    index: HashMap<String, Vec<(usize, usize)>>,
    // Document storage: file_index -> (file_path, chunks)
    documents: Vec<(String, Vec<CodeChunk>)>,
    // Document frequencies: word -> count of documents containing it
    doc_freqs: HashMap<String, usize>,
    total_docs: usize,
}

#[derive(Debug)]
struct QueryProfile {
    base_terms: Vec<String>,
    expanded_terms: Vec<String>,
    phrases: Vec<String>,
}

/// Options controlling the search behaviour.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// When true, co-occurrence expansion adds related terms to the query.
    /// When false, only the base query tokens are used (exact mode).
    pub expand: bool,
    /// When true, the per-file diversity decay is skipped, keeping raw scores.
    /// Useful when results will be fused with other strategies via RRF.
    pub skip_diversity: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            expand: true,
            skip_diversity: false,
        }
    }
}

impl SearchOptions {
    /// Exact match mode: no expansion, no diversity decay (for RRF fusion).
    pub const fn exact() -> Self {
        Self {
            expand: false,
            skip_diversity: true,
        }
    }

    /// Full search with expansion but no diversity decay (for RRF fusion).
    pub const fn expanded_no_diversity() -> Self {
        Self {
            expand: true,
            skip_diversity: true,
        }
    }
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            documents: Vec::new(),
            doc_freqs: HashMap::new(),
            total_docs: 0,
        }
    }

    pub fn add_document(&mut self, file_path: String, chunks: Vec<CodeChunk>) {
        let doc_id = self.documents.len();
        self.documents.push((file_path.clone(), chunks.clone()));
        self.total_docs += 1;

        let mut doc_words = HashSet::new();

        for (chunk_id, chunk) in chunks.iter().enumerate() {
            let mut searchable_text = String::new();
            searchable_text.push_str(&file_path);
            searchable_text.push('\n');
            if let Some(header) = &chunk.context_header {
                searchable_text.push_str(header);
                searchable_text.push('\n');
            }
            searchable_text.push_str(&chunk.content);

            let words = self.tokenize(&searchable_text);
            for word in words {
                self.index
                    .entry(word.clone())
                    .or_default()
                    .push((doc_id, chunk_id));
                doc_words.insert(word);
            }
        }

        // Update document frequencies
        for word in doc_words {
            *self.doc_freqs.entry(word).or_default() += 1;
        }
    }

    /// Search with default options (full expansion, with diversity decay).
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        self.search_with_options(query, limit, &SearchOptions::default())
    }

    /// Search with explicit options to control expansion and result shaping.
    pub fn search_with_options(
        &self,
        query: &str,
        limit: usize,
        options: &SearchOptions,
    ) -> Vec<SearchResult> {
        let profile = self.build_query_profile(query);
        if profile.base_terms.is_empty() {
            return Vec::new();
        }

        // Choose which terms to score: expanded (full) or base (exact).
        let scoring_terms: &[String] = if options.expand {
            &profile.expanded_terms
        } else {
            &profile.base_terms
        };

        // Score map: (doc_id, chunk_id) -> score
        let mut scores: HashMap<(usize, usize), f64> = HashMap::new();

        for word in scoring_terms {
            if let Some(postings) = self.index.get(word) {
                // IDF Calculation
                let df = *self.doc_freqs.get(word).unwrap_or(&1);
                // Smoothed IDF keeps meaningful positive scores even in tiny corpora.
                let idf = ((self.total_docs as f64 + 1.0) / (df as f64 + 1.0)).ln() + 1.0;

                // TF = number of times this term appears in the chunk (postings
                // list may contain the same (doc,chunk) pair multiple times).
                let mut tf_map: HashMap<(usize, usize), f64> = HashMap::new();
                for &(doc_id, chunk_id) in postings {
                    *tf_map.entry((doc_id, chunk_id)).or_default() += 1.0;
                }
                for ((doc_id, chunk_id), tf) in tf_map {
                    *scores.entry((doc_id, chunk_id)).or_default() += tf * idf;
                }
            }
        }

        // Convert scores to results
        let mut results: Vec<SearchResult> = scores
            .into_iter()
            .map(|((doc_id, chunk_id), score)| {
                let (path, chunks) = &self.documents[doc_id];
                let chunk = chunks[chunk_id].clone();

                let mut final_score = score;
                let path_lower = path.to_lowercase();
                let chunk_lower = chunk.content.to_lowercase();
                let header_lower = chunk
                    .context_header
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase();
                let searchable_lower = format!("{}\n{}\n{}", path_lower, header_lower, chunk_lower);

                // Boost score when user phrasing appears directly in code, header, or path.
                for phrase in &profile.phrases {
                    if chunk_lower.contains(phrase) {
                        final_score *= weights::PHRASE_IN_CONTENT;
                    }
                    if header_lower.contains(phrase) {
                        final_score *= weights::PHRASE_IN_HEADER;
                    }
                    if path_lower.contains(phrase) {
                        final_score *= weights::PHRASE_IN_PATH;
                    }
                }

                let matched_core_count = profile
                    .base_terms
                    .iter()
                    .filter(|w| searchable_lower.contains(w.as_str()))
                    .count();
                let core_coverage = matched_core_count as f64 / profile.base_terms.len() as f64;

                // Strongly prefer chunks that cover the full intent, not just one synonym.
                if matched_core_count == profile.base_terms.len() {
                    final_score *= weights::FULL_CORE_COVERAGE;
                } else {
                    final_score *= weights::PARTIAL_COVERAGE_BASE
                        + core_coverage * weights::PARTIAL_COVERAGE_SCALE;
                }

                // Boost score if context header matches query.
                for word in &profile.base_terms {
                    if header_lower.contains(word) {
                        final_score *= weights::HEADER_TERM_MATCH;
                    }
                }

                // Boost score if file path matches query.
                for word in &profile.base_terms {
                    if path_lower.contains(word) {
                        final_score *= weights::PATH_TERM_MATCH;
                    }
                }

                // Tiny snippets rarely carry enough context to be useful alone.
                let chunk_lines = chunk.end_line.saturating_sub(chunk.start_line) + 1;
                if chunk_lines < weights::SMALL_SNIPPET_LINES {
                    final_score *= weights::SMALL_SNIPPET_PENALTY;
                }

                // Large chunks are often less focused than function-sized chunks.
                if chunk_lines > weights::LARGE_CHUNK_LINES {
                    final_score *= weights::LARGE_CHUNK;
                }

                // Boost chunks that represent a named definition (function/class/struct).
                // SmartChunker sets context_header only for identifiable definitions,
                // making this a language-agnostic signal of structural importance.
                if chunk.context_header.is_some() {
                    final_score *= weights::NAMED_DEFINITION_BOOST;
                }

                let mut matched_terms: Vec<String> = profile
                    .base_terms
                    .iter()
                    .chain(profile.expanded_terms.iter())
                    .filter(|w| searchable_lower.contains(w.as_str()))
                    .cloned()
                    .collect();
                matched_terms.sort();
                matched_terms.dedup();
                if matched_terms.len() > weights::MAX_MATCHED_TERMS {
                    matched_terms.truncate(weights::MAX_MATCHED_TERMS);
                }

                let signals = Self::match_signals(
                    &profile,
                    &path_lower,
                    &header_lower,
                    &chunk_lower,
                    matched_core_count,
                    chunk_lines,
                    chunk.context_header.is_some(),
                );

                SearchResult {
                    file_path: path.clone(),
                    chunk,
                    score: final_score,
                    matches: matched_terms,
                    signals,
                }
            })
            .collect();

        // Sort by score desc
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Diversify results: apply diminishing-returns scoring per file.
        // This replaces a hard MAX_HITS_PER_FILE cap with a softer decay,
        // keeping strong matches visible across files while still promoting breadth.
        // Skip diversity when the caller plans to fuse multiple result sets,
        // as RRF naturally handles diversity across strategies.
        if !options.skip_diversity {
            let mut per_file_hits: HashMap<String, usize> = HashMap::new();
            for res in &mut results {
                let hits = per_file_hits.entry(res.file_path.clone()).or_insert(0);
                let decay = weights::FILE_DIVERSITY_DECAY.get(*hits).copied().unwrap_or(
                    weights::FILE_DIVERSITY_DECAY[weights::FILE_DIVERSITY_DECAY.len() - 1],
                );
                res.score *= decay;
                *hits += 1;
            }

            // Re-sort after applying decay
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        results.truncate(limit);
        results
    }

    fn build_query_profile(&self, query: &str) -> QueryProfile {
        let base_terms = self.tokenize(query);
        let expanded_terms = self.expand_query_terms(query, &base_terms);
        let query_lower = query.to_lowercase();
        let phrases = Self::extract_query_phrases(&query_lower, &base_terms);
        QueryProfile {
            base_terms,
            expanded_terms,
            phrases,
        }
    }

    /// Expand query terms using **co-occurrence within the actual index**.
    ///
    /// For each base term we look at the top-N chunks that contain it, collect
    /// all tokens from those chunks, and add the most frequently co-occurring
    /// ones as expansion terms.  This is completely domain-agnostic: the
    /// expansions come from the project being searched, not from a hard-coded
    /// vocabulary list.
    fn expand_query_terms(&self, _query: &str, base_terms: &[String]) -> Vec<String> {
        // Caps that keep expansion fast and focused.
        const MAX_SAMPLE_CHUNKS: usize = 8; // chunks to sample per query term
        const MIN_COOCCURRENCE: usize = 2; // a token must appear in ≥2 sampled chunks
        const MAX_EXPANSIONS: usize = 20; // total extra terms added

        let mut expanded: HashSet<String> = base_terms.iter().cloned().collect();
        let mut cooccur: HashMap<String, usize> = HashMap::new();

        for term in base_terms {
            let postings = match self.index.get(term) {
                Some(p) => p,
                None => continue,
            };

            for &(doc_id, chunk_id) in postings.iter().take(MAX_SAMPLE_CHUNKS) {
                let (_, chunks) = match self.documents.get(doc_id) {
                    Some(d) => d,
                    None => continue,
                };
                let chunk = match chunks.get(chunk_id) {
                    Some(c) => c,
                    None => continue,
                };

                // Collect tokens from chunk content + header
                let mut text = chunk.content.clone();
                if let Some(h) = &chunk.context_header {
                    text.push('\n');
                    text.push_str(h);
                }
                for token in self.tokenize(&text) {
                    if !expanded.contains(&token) {
                        *cooccur.entry(token).or_default() += 1;
                    }
                }
            }
        }

        // Pick the most frequent co-occurring tokens that appear in enough chunks
        let mut ranked: Vec<(String, usize)> = cooccur
            .into_iter()
            .filter(|(_, count)| *count >= MIN_COOCCURRENCE)
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));

        for (token, _) in ranked.into_iter().take(MAX_EXPANSIONS) {
            expanded.insert(token);
        }

        let mut out: Vec<String> = expanded.into_iter().collect();
        out.sort();
        out
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut out = HashSet::new();
        for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
            let raw_lower = raw.to_lowercase();
            Self::push_token(&mut out, &raw_lower);

            for part in raw_lower.split(['_', '-']) {
                Self::push_token(&mut out, part);
            }

            for part in Self::split_camel_case(raw) {
                Self::push_token(&mut out, &part);
            }
        }

        let mut tokens: Vec<String> = out.into_iter().collect();
        tokens.sort();
        tokens
    }

    fn push_token(tokens: &mut HashSet<String>, token: &str) {
        let token = token.trim_matches('_').trim_matches('-');
        if token.chars().count() >= weights::MIN_TOKEN_LEN {
            tokens.insert(token.to_string());
        }
    }

    fn split_camel_case(raw: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut prev_lower_or_digit = false;

        for ch in raw.chars() {
            if ch == '_' || ch == '-' {
                if !current.is_empty() {
                    parts.push(current.to_lowercase());
                    current.clear();
                }
                prev_lower_or_digit = false;
                continue;
            }

            if ch.is_uppercase() && prev_lower_or_digit && !current.is_empty() {
                parts.push(current.to_lowercase());
                current.clear();
            }

            current.push(ch);
            prev_lower_or_digit = ch.is_lowercase() || ch.is_ascii_digit();
        }

        if !current.is_empty() {
            parts.push(current.to_lowercase());
        }

        parts
    }

    fn extract_query_phrases(query_lower: &str, base_terms: &[String]) -> Vec<String> {
        let mut phrases = HashSet::new();
        let normalized = query_lower.trim();
        if normalized.chars().count() > 3 {
            phrases.insert(normalized.to_string());
        }

        for window in base_terms.windows(2) {
            phrases.insert(window.join(" "));
            phrases.insert(window.join("_"));
            phrases.insert(window.join("-"));
        }

        let mut out: Vec<String> = phrases.into_iter().collect();
        out.sort();
        out
    }

    fn match_signals(
        profile: &QueryProfile,
        path_lower: &str,
        header_lower: &str,
        chunk_lower: &str,
        matched_core_count: usize,
        chunk_lines: usize,
        has_named_definition: bool,
    ) -> Vec<String> {
        let mut signals = Vec::new();

        if matched_core_count == profile.base_terms.len() {
            signals.push("covers all core query terms".to_string());
        } else if matched_core_count > 0 {
            signals.push(format!(
                "covers {}/{} core query terms",
                matched_core_count,
                profile.base_terms.len()
            ));
        }
        if profile.phrases.iter().any(|p| header_lower.contains(p)) {
            signals.push("query phrase appears in symbol/header".to_string());
        }
        if profile.phrases.iter().any(|p| chunk_lower.contains(p)) {
            signals.push("query phrase appears in code".to_string());
        }
        if profile.base_terms.iter().any(|t| path_lower.contains(t)) {
            signals.push("file path matches query terms".to_string());
        }
        if chunk_lines < weights::SMALL_SNIPPET_LINES {
            signals.push(format!(
                "tiny snippet < {} lines",
                weights::SMALL_SNIPPET_LINES
            ));
        }
        if has_named_definition {
            signals.push("named definition (function/class/struct)".to_string());
        }

        if signals.len() > weights::MAX_SIGNALS {
            signals.truncate(weights::MAX_SIGNALS);
        }
        signals
    }
}
