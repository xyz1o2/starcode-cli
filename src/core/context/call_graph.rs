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
                name_index
                    .entry(sym.name.clone())
                    .or_default()
                    .push(sym.id);
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
                        calls_out
                            .entry(edge.caller_id)
                            .or_default()
                            .push(callee_id);
                        calls_in
                            .entry(callee_id)
                            .or_default()
                            .push(edge.caller_id);
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
    pub fn find_symbols(&self, name_hint: &str) -> Vec<&SymbolDef> {
        let lower = name_hint.to_lowercase();
        self.symbols
            .iter()
            .filter(|s| !s.name.is_empty() && s.name.to_lowercase().contains(&lower))
            .collect()
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

    fn trace(
        &self,
        start: SymbolId,
        max_depth: usize,
        upstream: bool,
    ) -> Vec<CallChainLevel> {
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
                self.calls_in
                    .get(&current)
                    .cloned()
                    .unwrap_or_default()
            } else {
                self.calls_out
                    .get(&current)
                    .cloned()
                    .unwrap_or_default()
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
 