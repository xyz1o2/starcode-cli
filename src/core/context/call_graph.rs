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

use super::symbol::{FileSymbols, SymbolDef, SymbolId};
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
    /// 按名称匹配符号，**按匹配质量降序**返回。
    ///
    /// 排序键（越靠前越好）：精确相等 > 前缀匹配 > 子串匹配；同档内名字短者优先
    /// （`ApplyParamOverride` 比 `ApplyParamOverrideWithExtraChecks` 更接近查询本身），
    /// 再按路径稳定排序保证可复现。
    ///
    /// 以前这里只 `filter(contains)` 不排序，调用方 `take(5)` 拿到的是符号表插入顺序，
    /// 而输出文案却写着 "Showing top 5 by name match" —— 模型以为看到的是最相关的 5 个。
    pub fn find_symbols(&self, name_hint: &str) -> Vec<&SymbolDef> {
        let lower = name_hint.to_lowercase();
        let mut matched: Vec<&SymbolDef> = self
            .symbols
            .iter()
            .filter(|s| !s.name.is_empty() && s.name.to_lowercase().contains(&lower))
            .collect();
        matched.sort_by_key(|s| {
            let name_lower = s.name.to_lowercase();
            let rank: u8 = if name_lower == lower {
                0
            } else if name_lower.starts_with(&lower) {
                1
            } else {
                2
            };
            (rank, s.name.chars().count(), s.file_path.as_str())
        });
        matched
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
