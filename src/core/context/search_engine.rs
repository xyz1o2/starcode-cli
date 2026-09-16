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

/// 单个已索引文档。`doc_id` 就是它在 `SearchEngine::documents` 里的下标，
/// 因此删除时必须做 swap-remove + 重映射（见 `remove_document`）。
#[derive(Clone)]
struct DocumentEntry {
    path: String,
    chunks: Vec<CodeChunk>,
}

#[derive(Clone)]
pub struct SearchEngine {
    // Inverted index: word -> list of (doc_id, chunk_id)
    index: HashMap<String, Vec<(usize, usize)>>,
    // Document storage: doc_id (positional) -> entry
    documents: Vec<DocumentEntry>,
    // 路径 → doc_id 反查表，供增量 upsert/remove 定位文档。
    // 不变量：`path_to_doc` 的键集与 `documents` 的 path 集一一对应。
    path_to_doc: HashMap<String, usize>,
    // Document frequencies: word -> count of documents containing it
    doc_freqs: HashMap<String, usize>,
    // 注意：没有 `total_docs` 字段 —— 它由 `documents.len()` 派生。
    // 维护一个独立计数器在增量删除下极易与实际文档数脱节，而 IDF 依赖它。
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
            path_to_doc: HashMap::new(),
            doc_freqs: HashMap::new(),
        }
    }

    /// 当前文档数。IDF 用它做分母，所以必须是派生值而非独立计数器。
    pub fn total_docs(&self) -> usize {
        self.documents.len()
    }

    /// 文档数（别名，供状态展示）。
    pub fn doc_count(&self) -> usize {
        self.documents.len()
    }

    /// 所有已索引文档内容的总字节数（供预算核算与状态展示）。
    pub fn total_bytes(&self) -> u64 {
        self.documents
            .iter()
            .flat_map(|doc| doc.chunks.iter())
            .map(|chunk| chunk.content.len() as u64)
            .sum()
    }

    /// 该路径是否已在索引中。
    pub fn contains_document(&self, path: &str) -> bool {
        self.path_to_doc.contains_key(path)
    }

    /// 单个文档占用的字节数（0 表示不在索引里）。
    ///
    /// 增量补丁用它算"重写会先释放多少预算" —— 没有这个数字，
    /// 把 5 MB 文件重写成同样 5 MB 就会被误判成"新增 5 MB"而撞上限。
    pub fn document_bytes(&self, path: &str) -> u64 {
        self.path_to_doc
            .get(path)
            .and_then(|&doc_id| self.documents.get(doc_id))
            .map(|doc| doc.chunks.iter().map(|c| c.content.len() as u64).sum())
            .unwrap_or(0)
    }

    /// 索引某个文档的**全部**去重词集（所有 chunk 的并集）。
    /// doc_freqs 的增减口径必须与插入时一致，否则删除会留下幽灵词频。
    fn document_words(&self, doc_id: usize) -> HashSet<String> {
        let Some(doc) = self.documents.get(doc_id) else {
            return HashSet::new();
        };
        let mut words = HashSet::new();
        for chunk in &doc.chunks {
            words.extend(self.chunk_words(&doc.path, chunk));
        }
        words
    }

    /// 单个 chunk 的可检索词集。
    ///
    /// 插入与删除**共用**这一个分词口径 —— 两边各写一份是增量索引最经典的
    /// bug 来源（加入的词和移除的词不一致 → doc_freqs 单向漂移）。
    fn chunk_words(&self, path: &str, chunk: &CodeChunk) -> HashSet<String> {
        let mut searchable_text = String::new();
        searchable_text.push_str(path);
        searchable_text.push('\n');
        if let Some(header) = &chunk.context_header {
            searchable_text.push_str(header);
            searchable_text.push('\n');
        }
        searchable_text.push_str(&chunk.content);

        self.tokenize(&searchable_text).into_iter().collect()
    }

    /// 插入一个文档，不做去重（内部用）。
    fn insert_document(&mut self, file_path: String, chunks: Vec<CodeChunk>) {
        let doc_id = self.documents.len();
        self.path_to_doc.insert(file_path.clone(), doc_id);

        let mut doc_words = HashSet::new();
        for (chunk_id, chunk) in chunks.iter().enumerate() {
            for word in self.chunk_words(&file_path, chunk) {
                self.index
                    .entry(word.clone())
                    .or_default()
                    .push((doc_id, chunk_id));
                doc_words.insert(word);
            }
        }
        for word in doc_words {
            *self.doc_freqs.entry(word).or_default() += 1;
        }

        self.documents.push(DocumentEntry {
            path: file_path,
            chunks,
        });
    }

    /// 加入文档。同路径重复调用等价于替换（upsert），不会产生重复文档。
    pub fn add_document(&mut self, file_path: String, chunks: Vec<CodeChunk>) {
        self.upsert_document(file_path, chunks);
    }

    /// 插入或替换一个文档。已存在则先移除再插入。
    pub fn upsert_document(&mut self, file_path: String, chunks: Vec<CodeChunk>) {
        self.remove_document(&file_path);
        self.insert_document(file_path, chunks);
    }

    /// 替换已存在文档的内容。文档不存在时插入并返回 `false`。
    pub fn replace_document(&mut self, file_path: String, chunks: Vec<CodeChunk>) -> bool {
        let existed = self.remove_document(&file_path);
        self.insert_document(file_path, chunks);
        existed
    }

    /// 移除一个文档。返回它此前是否存在。
    ///
    /// 用 swap-remove 保持 `documents` 稠密（读路径 `search_with_options` 直接
    /// 下标访问，不能有墓碑），代价是必须重映射被换到 `removed_id` 位置的文档。
    pub fn remove_document(&mut self, file_path: &str) -> bool {
        let Some(removed_id) = self.path_to_doc.remove(file_path) else {
            return false;
        };

        // 1. 递减 doc_freqs，并清掉该文档在倒排表里的 posting。
        for word in self.document_words(removed_id) {
            if let Some(df) = self.doc_freqs.get_mut(&word) {
                *df = df.saturating_sub(1);
                if *df == 0 {
                    self.doc_freqs.remove(&word);
                }
            }
            if let Some(postings) = self.index.get_mut(&word) {
                postings.retain(|&(doc_id, _)| doc_id != removed_id);
                if postings.is_empty() {
                    self.index.remove(&word);
                }
            }
        }

        // 2. swap-remove：末尾元素填坑，并修正它的 doc_id。
        let last_id = self.documents.len() - 1;
        self.documents.swap_remove(removed_id);
        if removed_id != last_id {
            // 被移动的文档现在位于 removed_id，倒排表里的 doc_id 必须跟着改。
            let moved_path = self.documents[removed_id].path.clone();
            self.path_to_doc.insert(moved_path.clone(), removed_id);
            self.remap_postings(last_id, removed_id);
        }

        true
    }

    /// 把倒排表里所有 `old_id` 的 posting 改写成 `new_id`。
    fn remap_postings(&mut self, old_id: usize, new_id: usize) {
        for postings in self.index.values_mut() {
            for posting in postings.iter_mut() {
                if posting.0 == old_id {
                    posting.0 = new_id;
                }
            }
        }
    }

    /// 测试用：doc_freqs 快照（有序，便于断言全量与增量口径一致）。
    #[cfg(test)]
    pub(crate) fn doc_freqs_snapshot(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = self
            .doc_freqs
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        out.sort();
        out
    }

    /// 测试用：已索引路径列表（有序）。比只比数量强得多 —— 数量相同但
    /// 张冠李戴（swap-remove 重映射写错的典型症状）会被它抓出来。
    #[cfg(test)]
    pub(crate) fn document_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.documents.iter().map(|d| d.path.clone()).collect();
        out.sort();
        out
    }

    /// 自检：倒排表、doc_freqs、path_to_doc 三者与 documents 一致。
    /// 仅测试使用 —— 增量删除的簿记错误全在这里暴露。
    #[cfg(test)]
    pub(crate) fn verify_invariants(&self) -> Result<(), String> {
        if self.path_to_doc.len() != self.documents.len() {
            return Err(format!(
                "path_to_doc 有 {} 项，documents 有 {} 项",
                self.path_to_doc.len(),
                self.documents.len()
            ));
        }
        for (expected_id, doc) in self.documents.iter().enumerate() {
            match self.path_to_doc.get(&doc.path) {
                Some(&id) if id == expected_id => {}
                other => {
                    return Err(format!(
                        "路径 {} 期望 doc_id {}，实际 {:?}",
                        doc.path, expected_id, other
                    ))
                }
            }
        }

        // 倒排表里不能出现越界或指向错误文档的 posting。
        for (word, postings) in &self.index {
            if postings.is_empty() {
                return Err(format!("词 {} 有空 posting 列表", word));
            }
            for &(doc_id, chunk_id) in postings {
                let doc = self
                    .documents
                    .get(doc_id)
                    .ok_or_else(|| format!("词 {} 的 posting 指向越界 doc_id {}", word, doc_id))?;
                if chunk_id >= doc.chunks.len() {
                    return Err(format!(
                        "词 {} 的 posting 指向越界 chunk_id {}（文档 {} 只有 {} 个 chunk）",
                        word,
                        chunk_id,
                        doc.path,
                        doc.chunks.len()
                    ));
                }
            }
        }

        // doc_freqs 必须等于"包含该词的文档数"。
        let mut expected_df: HashMap<String, usize> = HashMap::new();
        for doc_id in 0..self.documents.len() {
            for word in self.document_words(doc_id) {
                *expected_df.entry(word).or_default() += 1;
            }
        }
        if expected_df != self.doc_freqs {
            return Err(format!(
                "doc_freqs 不一致：期望 {} 项，实际 {} 项",
                expected_df.len(),
                self.doc_freqs.len()
            ));
        }

        Ok(())
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
                let idf = ((self.total_docs() as f64 + 1.0) / (df as f64 + 1.0)).ln() + 1.0;

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
                let DocumentEntry { path, chunks } = &self.documents[doc_id];
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
                let chunks = match self.documents.get(doc_id) {
                    Some(d) => &d.chunks,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::chunking::CodeChunk;

    fn chunk(content: &str) -> CodeChunk {
        CodeChunk {
            content: content.to_string(),
            start_line: 1,
            end_line: 1,
            context_header: None,
        }
    }

    fn doc(engine: &mut SearchEngine, path: &str, words: &[&str]) {
        engine.add_document(path.to_string(), vec![chunk(&words.join(" "))]);
    }

    #[test]
    fn upsert_does_not_duplicate_documents() {
        let mut engine = SearchEngine::new();
        doc(&mut engine, "a.rs", &["alpha"]);
        doc(&mut engine, "a.rs", &["beta"]);

        assert_eq!(engine.doc_count(), 1, "同路径重复 add 必须是替换，不是追加");
        assert_eq!(engine.total_docs(), 1);
        // 旧内容的词必须被清掉。漏清的症状是"改了文件还能搜到旧实现" ——
        // 而且 doc_freqs 会单向上漂，永不回落。
        assert!(
            engine.search("alpha", 10).is_empty(),
            "替换后旧内容不该还能搜到"
        );
        assert_eq!(engine.search("beta", 10).len(), 1);
        engine.verify_invariants().unwrap();
    }

    /// swap-remove 的重映射 —— 整个增量索引里最容易写错的一处。
    /// 删除中间文档时末尾文档会被换到坑位，倒排表里的 doc_id 必须跟着改；
    /// 写错的症状是"剩余文档搜不到"或"搜到的 chunk 属于别的文件"。
    #[test]
    fn swap_remove_remaps_the_moved_document() {
        let mut engine = SearchEngine::new();
        doc(&mut engine, "a.rs", &["alpha"]);
        doc(&mut engine, "b.rs", &["bravo"]);
        doc(&mut engine, "c.rs", &["charlie"]);
        doc(&mut engine, "d.rs", &["delta"]);

        assert!(engine.remove_document("b.rs"));
        engine.verify_invariants().unwrap();

        assert_eq!(engine.document_paths(), vec!["a.rs", "c.rs", "d.rs"]);
        // d.rs 被 swap 进了 b.rs 原来的槽位，必须仍然可搜到且归属正确。
        let hits = engine.search("delta", 10);
        assert_eq!(hits.len(), 1, "被 swap 的文档不能丢");
        assert_eq!(hits[0].file_path, "d.rs", "被 swap 的文档不能张冠李戴");
    }

    #[test]
    fn removing_unknown_document_is_a_noop() {
        let mut engine = SearchEngine::new();
        doc(&mut engine, "a.rs", &["alpha"]);

        assert!(!engine.remove_document("nope.rs"));
        assert_eq!(engine.doc_count(), 1);
        engine.verify_invariants().unwrap();
    }

    #[test]
    fn doc_freqs_drop_to_zero_and_word_is_dropped() {
        let mut engine = SearchEngine::new();
        doc(&mut engine, "a.rs", &["uniquetoken"]);

        assert!(engine
            .doc_freqs_snapshot()
            .iter()
            .any(|(w, _)| w == "uniquetoken"));
        assert!(engine.remove_document("a.rs"));
        // 递减到 0 必须删 key —— 否则留下永不归零的幽灵词频，
        // IDF 分母被抬高，查询打分整体漂移。
        assert!(
            !engine
                .doc_freqs_snapshot()
                .iter()
                .any(|(w, _)| w == "uniquetoken"),
            "词频归零后必须删掉 key"
        );
        engine.verify_invariants().unwrap();
    }

    #[test]
    fn document_bytes_tracks_content_and_vanishes_on_remove() {
        let mut engine = SearchEngine::new();
        let content = "fn sized() {}";
        doc(&mut engine, "a.rs", &[content]);

        // 预算口径 = chunk content 字节，不是文件字节。
        assert_eq!(engine.document_bytes("a.rs"), content.len() as u64);
        assert_eq!(engine.total_bytes(), content.len() as u64);
        assert_eq!(engine.document_bytes("missing.rs"), 0);

        engine.remove_document("a.rs");
        assert_eq!(engine.document_bytes("a.rs"), 0);
        assert_eq!(engine.total_bytes(), 0);
    }
}
