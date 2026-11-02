// ── Cross-file Call Graph ─────────────────────────────────────────────────────
//
// P3 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Builds a bidirectional call graph from the symbol index, enabling:
//   - callers_of(symbol)  — what calls into this function?
//   - callees_of(symbol)  — what does this function call?
//   - call_chain(symbol)  — full upstream and downstream trace
//
// The graph is built on top of the symbol definitions and call edges extracted
// by `symbol.rs`, resolving call-site names to cross-file symbol definitions.

use super::symbol::{FileSymbols, SymbolDef, SymbolId, SymbolKind};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

// ── Graph structures ──────────────────────────────────────────────────────────

/// A resolved edge in the call graph — both ends are known symbols.
#[derive(Debug, Clone)]
pub struct ResolvedEdge {
    pub caller_id: SymbolId,
    pub callee_id: SymbolId,
    pub callee_name: String,
    pub file_path: String,
    pub line: usize,
}

/// The complete call graph for a project.
#[derive(Debug, Clone)]
pub struct CallGraph {
    /// All symbol definitions, indexed by SymbolId
    symbols: Vec<SymbolDef>,
    /// Fast lookup by name → list of SymbolIds
    name_index: HashMap<String, Vec<SymbolId>>,
    /// Resolved call edges
    edges: Vec<ResolvedEdge>,
    /// caller_id → list of callee_ids
    calls_out: HashMap<SymbolId, Vec<SymbolId>>,
    /// callee_id → list of caller_ids
    calls_in: HashMap<SymbolId, Vec<SymbolId>>,
}

/// Result of a call chain query.
#[derive(Debug, Clone)]
pub struct CallChainResult {
    /// The starting symbol
    pub root: SymbolDef,
    /// Callers of the root (incoming edges, up to depth levels)
    pub callers: Vec<CallChainLevel>,
    /// Callees of the root (outgoing edges, up to depth levels)
    pub callees: Vec<CallChainLevel>,
    /// Total unique symbols in the chain
    pub total_symbols: usize,
    /// Max depth reached
    pub max_depth: usize,
}

/// One level of a call chain traversal.
#[derive(Debug, Clone)]
pub struct CallChainLevel {
    pub depth: usize,
    pub symbols: Vec<SymbolDef>,
    /// (caller → callee) or (callee → caller) edges at this level
    pub edges: Vec<(SymbolId, SymbolId)>,
}

// ── Construction ──────────────────────────────────────────────────────────────

impl CallGraph {
    /// Build a call graph from per-file symbol data.
    pub fn build(file_symbols: &[FileSymbols]) -> Self {
        let mut symbols = Vec::new();
        let mut name_index: HashMap<String, Vec<SymbolId>> = HashMap::new();

        // Collect all symbols
        for fs in file_symbols {
            for sym in &fs.symbols {
                name_index.entry(sym.name.clone()).or_default().push(sym.id);
                if sym.id >= symbols.len() {
                    symbols.resize(sym.id + 1, SymbolDef::placeholder());
                }
                symbols[sym.id] = sym.clone();
            }
        }

        // Resolve edges: match callee_name → symbol definitions
        let mut edges = Vec::new();
        let mut calls_out: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        let mut calls_in: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();

        for fs in file_symbols {
            for edge in &fs.edges {
                // Find matching symbols by name (cross-file resolution)
                let matches = name_index.get(&edge.callee_name);

                if let Some(matched_ids) = matches {
                    for &callee_id in matched_ids {
                        let resolved = ResolvedEdge {
                            caller_id: edge.caller_id,
                            callee_id,
                            callee_name: edge.callee_name.clone(),
                            file_path: edge.file_path.clone(),
                            line: edge.line,
                        };
                        calls_out.entry(edge.caller_id).or_default().push(callee_id);
                        calls_in.entry(callee_id).or_default().push(edge.caller_id);
                        edges.push(resolved);
                    }
                }
            }
        }

        // Deduplicate edges
        for ids in calls_out.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        for ids in calls_in.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }

        CallGraph {
            symbols,
            name_index,
            edges,
            calls_out,
            calls_in,
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Find symbols whose name contains the given string.
    /// 按名称匹配**打分排序**返回（对标 codegraph 的查询评分层）。
    ///
    /// 排序信号（从强到弱）：
    /// 1. **名称匹配形态**：精确（区分大小写）> 精确（忽略大小写）> 前缀 >
    ///    词边界（camelCase / snake_case / 分隔符后）> 任意子串 > 模糊匹配；
    /// 2. **kind**：函数/方法 > trait/interface > 类型 —— 调用图查询的主场景；
    /// 3. **路径相关性**：查询词出现在文件路径里（`search_engine` →
    ///    `src/core/context/search_engine.rs`）；
    /// 4. **名字长度**：越短越贴近查询本身；
    /// 5. **模糊兜底**：查询 ≥4 字符且首字母一致时，允许 1~2 个编辑距离的
    ///    拼写误差（`handle_eror` → `handle_error`）。
    ///
    /// 同分时按名字长度 → 路径 → id 稳定排序，保证两次查询顺序一致。
    pub fn find_symbols(&self, name_hint: &str) -> Vec<&SymbolDef> {
        let query = name_hint.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let query_lower = query.to_lowercase();

        let mut scored: Vec<(f32, &SymbolDef)> = self
            .symbols
            .iter()
            .filter(|symbol| !symbol.name.is_empty())
            .filter_map(|symbol| {
                score_symbol(symbol, query, &query_lower).map(|score| (score, symbol))
            })
            .collect();

        scored.sort_by(|(score_a, a), (score_b, b)| {
            score_b
                .partial_cmp(score_a)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.name.chars().count().cmp(&b.name.chars().count()))
                .then_with(|| a.file_path.cmp(&b.file_path))
                .then_with(|| a.id.cmp(&b.id))
        });

        scored.into_iter().map(|(_, symbol)| symbol).collect()
    }

    /// Get the symbol definition by ID.
    pub fn get_symbol(&self, id: SymbolId) -> Option<&SymbolDef> {
        self.symbols.get(id)
    }

    /// Get IDs of functions that call this symbol.
    pub fn callers_of(&self, symbol_id: SymbolId) -> Vec<&SymbolDef> {
        self.calls_in
            .get(&symbol_id)
            .map(|ids| ids.iter().filter_map(|&id| self.symbols.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get IDs of functions that this symbol calls.
    pub fn callees_of(&self, symbol_id: SymbolId) -> Vec<&SymbolDef> {
        self.calls_out
            .get(&symbol_id)
            .map(|ids| ids.iter().filter_map(|&id| self.symbols.get(id)).collect())
            .unwrap_or_default()
    }

    /// Trace the full call chain for a symbol, both upstream (callers) and
    /// downstream (callees), up to `max_depth` levels.
    pub fn call_chain(&self, symbol_id: SymbolId, max_depth: usize) -> CallChainResult {
        let root = match self.symbols.get(symbol_id) {
            Some(s) => s.clone(),
            None => {
                return CallChainResult {
                    root: SymbolDef::placeholder(),
                    callers: Vec::new(),
                    callees: Vec::new(),
                    total_symbols: 0,
                    max_depth: 0,
                };
            }
        };

        let callers = self.trace_up(symbol_id, max_depth);
        let callees = self.trace_down(symbol_id, max_depth);

        let mut seen = HashSet::new();
        seen.insert(symbol_id);
        for level in &callers {
            for sym in &level.symbols {
                seen.insert(sym.id);
            }
        }
        for level in &callees {
            for sym in &level.symbols {
                seen.insert(sym.id);
            }
        }

        let max_depth = callers
            .len()
            .max(callees.len())
            .saturating_sub(1)
            .min(max_depth);

        CallChainResult {
            root,
            callers,
            callees,
            total_symbols: seen.len(),
            max_depth,
        }
    }

    /// Total number of symbols in the graph.
    pub fn len(&self) -> usize {
        self.symbols.iter().filter(|s| !s.name.is_empty()).count()
    }

    /// Total number of resolved call edges.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    // ── Traversal helpers ─────────────────────────────────────────────────

    fn trace_up(&self, start: SymbolId, max_depth: usize) -> Vec<CallChainLevel> {
        self.trace(start, max_depth, true)
    }

    fn trace_down(&self, start: SymbolId, max_depth: usize) -> Vec<CallChainLevel> {
        self.trace(start, max_depth, false)
    }

    fn trace(&self, start: SymbolId, max_depth: usize, upstream: bool) -> Vec<CallChainLevel> {
        if max_depth == 0 {
            return Vec::new();
        }

        let mut levels = Vec::new();
        let mut visited = HashSet::new();
        visited.insert(start);
        let mut frontier: VecDeque<(SymbolId, usize)> = VecDeque::new();
        frontier.push_back((start, 1));

        while let Some((current, depth)) = frontier.pop_front() {
            if depth > max_depth {
                continue;
            }

            let neighbors: Vec<SymbolId> = if upstream {
                self.calls_in.get(&current).cloned().unwrap_or_default()
            } else {
                self.calls_out.get(&current).cloned().unwrap_or_default()
            };

            if neighbors.is_empty() {
                continue;
            }

            let mut level_syms = Vec::new();
            let mut level_edges = Vec::new();

            for neighbor_id in &neighbors {
                if visited.insert(*neighbor_id) {
                    if let Some(sym) = self.symbols.get(*neighbor_id) {
                        level_syms.push(sym.clone());
                    }
                    // Add the edge
                    if upstream {
                        level_edges.push((*neighbor_id, current));
                    } else {
                        level_edges.push((current, *neighbor_id));
                    }
                    frontier.push_back((*neighbor_id, depth + 1));
                }
            }

            if !level_syms.is_empty() {
                // Ensure we have a level entry for this depth
                while levels.len() < depth {
                    levels.push(CallChainLevel {
                        depth: levels.len() + 1,
                        symbols: Vec::new(),
                        edges: Vec::new(),
                    });
                }
                if let Some(level) = levels.get_mut(depth - 1) {
                    level.symbols.extend(level_syms);
                    level.edges.extend(level_edges);
                }
            }
        }

        levels
    }
}

// ── Query scoring ─────────────────────────────────────────────────────────────

/// 给一个符号按查询打分；`None` 表示不匹配（不会出现在结果里）。
fn score_symbol(symbol: &SymbolDef, query: &str, query_lower: &str) -> Option<f32> {
    let name_lower = symbol.name.to_lowercase();
    let query_len = query_lower.chars().count();
    if query_len == 0 {
        return None;
    }

    let mut score = if symbol.name == query {
        100.0
    } else if name_lower == query_lower {
        95.0
    } else if name_lower.starts_with(query_lower) {
        85.0
    } else if camel_case_boundary_match(&symbol.name, query_lower)
        || separator_boundary_match(&name_lower, query_lower)
    {
        75.0
    } else if let Some(pos) = name_lower.find(query_lower) {
        55.0 - (pos as f32 * 0.5)
    } else {
        // 模糊兜底：短查询噪声比收益大，不参与。
        let max_distance = match query_len {
            4..=7 => 1,
            8.. => 2,
            _ => return None,
        };
        if !same_first_char(&name_lower, query_lower) {
            return None;
        }
        if !within_bounded_edit_distance(&name_lower, query_lower, max_distance) {
            return None;
        }
        30.0
    };

    score += match symbol.kind {
        SymbolKind::Function | SymbolKind::Method => 6.0,
        SymbolKind::Trait | SymbolKind::Interface => 3.0,
        SymbolKind::Struct | SymbolKind::Class | SymbolKind::Enum => 2.0,
        _ => 0.0,
    };

    // 路径相关性：查询词出现在文件路径里（`search_engine` → search_engine.rs）。
    if query_len >= 3 && symbol.file_path.to_lowercase().contains(query_lower) {
        score += 8.0;
    }
    // 测试文件里的同名 helper 轻微降权，别挤掉实现。
    if is_test_path(&symbol.file_path) {
        score -= 3.0;
    }

    // 名字越短越贴近查询本身（同档次级信号）。
    score -= (symbol.name.chars().count().saturating_sub(query_len)) as f32 * 0.2;
    Some(score)
}

/// camelCase 词边界：`search` 命中 `CodebaseSearch` 的 `Search` 段。
fn camel_case_boundary_match(name: &str, query_lower: &str) -> bool {
    let mut chars = query_lower.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let mut capitalized = String::with_capacity(query_lower.len());
    capitalized.extend(first.to_uppercase());
    capitalized.push_str(chars.as_str());
    name.contains(&capitalized)
}

/// snake_case / 分隔符词边界：`engine` 命中 `search_engine` / `search-engine`。
fn separator_boundary_match(name_lower: &str, query_lower: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = name_lower[from..].find(query_lower) {
        let idx = from + pos;
        if idx > 0 {
            let prev = name_lower[..idx].chars().next_back().unwrap_or('\0');
            if matches!(prev, '_' | '-' | '.' | ':' | '/') {
                return true;
            }
        }
        from = idx + query_lower.len().max(1);
        if from >= name_lower.len() {
            break;
        }
    }
    false
}

fn same_first_char(a: &str, b: &str) -> bool {
    a.chars().next() == b.chars().next()
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("/tests/")
        || lower.contains("\\tests\\")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_tests.rs")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
}

/// 限界编辑距离：只回答"是否 ≤ `max_distance`"，超限立即返回。
///
/// 逐字符 DP；`row_min > max_distance` 时提前退出（名字通常比查询长得多，
/// 提前退出让 1.4 万符号的模糊扫描仍是毫秒级）。
fn within_bounded_edit_distance(a: &str, b: &str, max_distance: usize) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max_distance {
        return false;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            row_min = row_min.min(current[j]);
        }
        if row_min > max_distance {
            return false;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()] <= max_distance
}

impl SymbolDef {
    fn placeholder() -> Self {
        Self {
            id: usize::MAX,
            name: String::new(),
            kind: super::symbol::SymbolKind::Unknown,
            file_path: String::new(),
            start_line: 0,
            end_line: 0,
            signature: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::symbol::SymbolKind;

    fn sym(id: usize, name: &str, file: &str) -> SymbolDef {
        SymbolDef {
            id,
            name: name.to_string(),
            kind: SymbolKind::Function,
            file_path: file.to_string(),
            start_line: id + 1,
            end_line: id + 1,
            signature: format!("fn {name}()"),
        }
    }

    fn graph_of(symbols: Vec<SymbolDef>) -> CallGraph {
        CallGraph::build(&[FileSymbols {
            file_path: "x.rs".to_string(),
            symbols,
            edges: Vec::new(),
        }])
    }

    /// 事故记录（docs/tool-io-token-audit.md §1.4）：`trace_call_chain` 输出
    /// "Showing top 5 by name match"，但 `find_symbols` 只 `filter(contains)`
    /// **不排序** —— 调用方 `take(5)` 拿到的是符号表的插入顺序。
    ///
    /// 文案说"最相关的 5 个"，实际给的是"最先被索引的 5 个"。模型据此判断
    /// "这个函数在项目里有 82 处定义"并去读第一个 —— 而那个可能只是同名 wrapper。
    #[test]
    fn find_symbols_ranks_by_name_match_quality() {
        // 故意按"最不像"到"最像"的顺序插入：插入顺序与匹配质量相反。
        let graph = graph_of(vec![
            sym(0, "ApplyParamOverrideWithExtraChecks", "z.rs"),
            sym(1, "ApplyParamOverrideV2", "y.rs"),
            sym(2, "ApplyParamOverride", "x.rs"),
        ]);

        let found = graph.find_symbols("ApplyParamOverride");
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                // 精确相等排第一
                "ApplyParamOverride",
                // 前缀匹配排第二
                "ApplyParamOverrideV2",
                // 子串匹配排最后
                "ApplyParamOverrideWithExtraChecks",
            ],
            "精确 > 前缀 > 子串，与插入顺序无关：{names:?}"
        );
    }

    /// 同档内名字短者优先 —— `ApplyParamOverrideWithChecks` 比
    /// `ApplyParamOverrideWithExtraChecks` 更接近查询本身。
    #[test]
    fn find_symbols_prefers_shorter_names_within_the_same_rank() {
        let graph = graph_of(vec![
            sym(0, "ApplyParamOverrideWithExtraChecks", "z.rs"),
            sym(1, "ApplyParamOverrideWithChecks", "y.rs"),
        ]);

        let found = graph.find_symbols("ApplyParamOverride");
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "ApplyParamOverrideWithChecks",
                "ApplyParamOverrideWithExtraChecks"
            ],
            "同档内短名优先：{names:?}"
        );
    }

    /// 排序必须是**稳定且可复现**的：同样的图两次查询必须给出同样的顺序，
    /// 否则模型看到的"top 5"每次都不一样，无法据此做判断。
    #[test]
    fn find_symbols_order_is_reproducible() {
        let graph = graph_of(vec![
            sym(0, "ApplyParamOverrideHelper", "b.rs"),
            sym(1, "ApplyParamOverrideHelper", "a.rs"),
        ]);

        let first: Vec<String> = graph
            .find_symbols("ApplyParamOverride")
            .iter()
            .map(|s| s.file_path.clone())
            .collect();
        let second: Vec<String> = graph
            .find_symbols("ApplyParamOverride")
            .iter()
            .map(|s| s.file_path.clone())
            .collect();

        assert_eq!(first, second, "同分同名必须按路径稳定排序");
        assert_eq!(first, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    fn sym_of_kind(id: usize, name: &str, kind: SymbolKind, file: &str) -> SymbolDef {
        SymbolDef {
            id,
            name: name.to_string(),
            kind,
            file_path: file.to_string(),
            start_line: id + 1,
            end_line: id + 1,
            signature: String::new(),
        }
    }

    /// 拼错一个字符（长查询）必须还能兜住 —— 这是 boundedEditDistance 的
    /// 直接收益，记不清完整标识符时尤其常见。
    #[test]
    fn find_symbols_tolerates_a_typo_in_long_queries() {
        let graph = graph_of(vec![sym(0, "handle_error", "x.rs")]);
        let found = graph.find_symbols("handle_eror");
        assert_eq!(found.len(), 1, "一个编辑距离内的拼写误差应命中");
        assert_eq!(found[0].name, "handle_error");
    }

    /// 短查询不参与模糊匹配：3 个字符的窗口太容易撞上无关符号。
    #[test]
    fn find_symbols_does_not_fuzzy_match_tiny_queries() {
        let graph = graph_of(vec![sym(0, "handler", "x.rs")]);
        assert!(graph.find_symbols("hnl").is_empty());
        // 前缀匹配不受影响
        assert_eq!(graph.find_symbols("hand").len(), 1);
    }

    /// camelCase / snake_case 的词边界匹配要排在任意子串之前。
    #[test]
    fn find_symbols_prefers_word_boundaries_over_plain_substrings() {
        let graph = graph_of(vec![
            sym(0, "Research", "r.rs"),
            sym(1, "CodebaseSearch", "s.rs"),
        ]);
        let found = graph.find_symbols("search");
        assert_eq!(found[0].name, "CodebaseSearch", "camel 词边界应排第一");

        let graph = graph_of(vec![
            sym(0, "researchagent", "r.rs"),
            sym(1, "thing_search", "t.rs"),
        ]);
        let found = graph.find_symbols("search");
        assert_eq!(found[0].name, "thing_search", "分隔符词边界应排第一");
    }

    /// 调用图的主场景是可调用符号：同名时 function/method 排在类型前面。
    #[test]
    fn find_symbols_prefers_callables_over_types() {
        let graph = graph_of(vec![
            sym_of_kind(0, "Auth", SymbolKind::Struct, "a.rs"),
            sym_of_kind(1, "Auth", SymbolKind::Function, "z.rs"),
        ]);
        let found = graph.find_symbols("Auth");
        assert_eq!(found[0].kind, SymbolKind::Function);
    }

    /// 查询词出现在路径里时加权：`search` → `src/core/search_engine.rs`。
    #[test]
    fn find_symbols_rewards_path_relevance() {
        let graph = graph_of(vec![
            sym(0, "SearchEngine", "src/other/model.rs"),
            sym(1, "SearchEngine", "src/core/search_engine.rs"),
        ]);
        let found = graph.find_symbols("search");
        assert_eq!(found[0].file_path, "src/core/search_engine.rs");
    }

    /// 测试文件里的同名 helper 不应挤掉实现。
    #[test]
    fn find_symbols_downranks_test_paths() {
        let graph = graph_of(vec![
            sym(0, "compute_hash", "src/tests/util.rs"),
            sym(1, "compute_hash", "src/core/util.rs"),
        ]);
        let found = graph.find_symbols("compute_hash");
        assert_eq!(found[0].file_path, "src/core/util.rs");
    }
}

// ── Formatting ────────────────────────────────────────────────────────────────

/// Format a CallChainResult as a human-readable string for the LLM.
pub fn format_call_chain(result: &CallChainResult) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "Call chain for `{}` ({})\n",
        result.root.name, result.root.signature
    ));
    out.push_str(&format!(
        "  File: {}:{}\n",
        result.root.file_path, result.root.start_line
    ));
    out.push_str(&format!(
        "  Total symbols in chain: {}\n\n",
        result.total_symbols
    ));

    // Upstream (callers)
    if !result.callers.is_empty() {
        out.push_str("Callers (who calls this):\n");
        for level in &result.callers {
            for sym in &level.symbols {
                out.push_str(&format!(
                    "  {} {} → `{}` ({})\n",
                    "  ".repeat(level.depth - 1),
                    sym.kind.as_str(),
                    sym.name,
                    sym.file_path
                ));
            }
        }
        out.push('\n');
    }

    // Downstream (callees)
    if !result.callees.is_empty() {
        out.push_str("Callees (what this calls):\n");
        for level in &result.callees {
            for sym in &level.symbols {
                out.push_str(&format!(
                    "  {} {} → `{}` ({})\n",
                    "  ".repeat(level.depth - 1),
                    sym.kind.as_str(),
                    sym.name,
                    sym.file_path
                ));
            }
        }
        out.push('\n');
    }

    if result.callers.is_empty() && result.callees.is_empty() {
        out.push_str("No callers or callees found (isolated symbol)\n");
    }

    out
}
