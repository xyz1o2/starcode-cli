//! 仓库符号地图（PageRank 排序）。
//!
//! 对标 aider 的 repo map：给模型一份"这个仓库里最值得知道的符号"清单 ——
//! 名字 + 签名 + 位置 + 排名。它解决的正是"中文需求落不到英文标识符上"：
//! 模型看到 `retry_backoff` / `parse_config` 这类**真实符号名**，才知道该拿
//! 什么去喂词法引擎。
//!
//! 排序信号：
//! 1. **文件 PageRank**：调用图投影成文件图（caller 文件 → callee 文件），
//!    PageRank 给出"被依赖得最多"的文件（核心模块）；
//! 2. **符号入度**：被调用得多的符号更重要；
//! 3. **kind 权重**：函数/方法优先于类型；
//! 4. **focus 偏置**：调用方给了提示词时，相关文件/符号加权。

use super::call_graph::CallGraph;
use super::symbol::SymbolKind;
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

/// 地图的输出预算。
#[derive(Debug, Clone, Copy)]
pub struct RepoMapLimits {
    /// 最多列多少个符号。
    pub max_entries: usize,
    /// 每个文件最多列多少个符号（防止一个大文件占满整张图）。
    pub max_per_file: usize,
    /// 签名截断长度。
    pub max_signature_len: usize,
}

impl Default for RepoMapLimits {
    fn default() -> Self {
        Self {
            max_entries: 60,
            max_per_file: 8,
            max_signature_len: 110,
        }
    }
}

/// 地图里的一个条目。
#[derive(Debug, Clone)]
pub struct RepoMapEntry {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_line: usize,
    pub signature: String,
    pub score: f32,
}

/// 构建仓库符号地图（文本形式，直接喂给模型）。
pub fn build_repo_map(
    root: &Path,
    focus: Option<&str>,
    limits: RepoMapLimits,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let graph = crate::core::context::call_graph_db::get_call_graph(root)?;
    Ok(format_repo_map(&graph, focus, limits))
}

/// 把调用图渲染成符号地图文本。
pub fn format_repo_map(graph: &CallGraph, focus: Option<&str>, limits: RepoMapLimits) -> String {
    let entries = rank_symbols(graph, focus, limits);

    if entries.is_empty() {
        return "The repository symbol map is empty (no symbols indexed).\n".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Repo map — {} of {} symbols, ranked by call-graph centrality\n",
        entries.len(),
        graph.len()
    ));
    if let Some(focus) = focus.filter(|focus| !focus.trim().is_empty()) {
        out.push_str(&format!("Focus: {focus}\n"));
    }
    out.push_str("These are real identifiers — use them (or part of them) as CodebaseSearch/Grep queries.\n\n");

    let mut current_file = String::new();
    for entry in &entries {
        if entry.file_path != current_file {
            current_file = entry.file_path.clone();
            out.push_str(&format!("── {current_file}\n"));
        }
        let signature = if entry.signature.trim().is_empty() {
            entry.name.clone()
        } else {
            truncate(&entry.signature, limits.max_signature_len)
        };
        out.push_str(&format!(
            "   {} {}  (line {})\n",
            kind_tag(entry.kind),
            signature,
            entry.start_line
        ));
    }

    out
}

/// 符号排序：文件 PageRank × 入度 × kind 权重，再按 focus 偏置。
pub fn rank_symbols(
    graph: &CallGraph,
    focus: Option<&str>,
    limits: RepoMapLimits,
) -> Vec<RepoMapEntry> {
    let (files, out_links) = file_graph(graph);
    let ranks = pagerank(&out_links, 0.85, 24);

    let average_rank = if ranks.is_empty() {
        0.0
    } else {
        ranks.iter().sum::<f32>() / ranks.len() as f32
    };
    let mut file_index: HashMap<&str, usize> = HashMap::new();
    for (index, path) in files.iter().enumerate() {
        file_index.insert(path.as_str(), index);
    }

    let focus_lower = focus
        .map(|focus| focus.trim().to_lowercase())
        .filter(|focus| !focus.is_empty());

    let mut entries: Vec<RepoMapEntry> = Vec::new();
    for symbol in graph.symbols() {
        if symbol.name.is_empty() {
            continue;
        }
        let file_rank = file_index
            .get(symbol.file_path.as_str())
            .and_then(|index| ranks.get(*index))
            .copied()
            .unwrap_or(average_rank);

        let in_degree = graph.caller_count(symbol.id) as f32;
        // 基线分不能是 0：没有调用边的孤立符号也要有可比较的底分，
        // 否则 focus 偏置（乘系数）无从生效。
        let mut score =
            (file_rank + 0.001) * (1.0 + (1.0 + in_degree).ln()) * kind_weight(symbol.kind);

        if let Some(focus) = &focus_lower {
            if symbol.name.to_lowercase().contains(focus.as_str()) {
                score *= 2.0;
            }
            if symbol.file_path.to_lowercase().contains(focus.as_str()) {
                score *= 1.5;
            }
        }

        entries.push(RepoMapEntry {
            name: symbol.name.clone(),
            kind: symbol.kind,
            file_path: symbol.file_path.clone(),
            start_line: symbol.start_line,
            signature: symbol.signature.clone(),
            score,
        });
    }

    entries.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.name.cmp(&b.name))
    });

    // 每文件限额：一张地图不该被单个大文件占满。
    let mut per_file: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<RepoMapEntry> = Vec::new();
    for entry in entries {
        let count = per_file.entry(entry.file_path.clone()).or_insert(0);
        if *count >= limits.max_per_file {
            continue;
        }
        *count += 1;
        kept.push(entry);
        if kept.len() >= limits.max_entries {
            break;
        }
    }
    kept
}

/// 调用图投影成文件图：`caller 文件 → callee 文件`。
fn file_graph(graph: &CallGraph) -> (Vec<String>, Vec<Vec<usize>>) {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for edge in graph.resolved_edges() {
        let Some(caller) = graph.get_symbol(edge.caller_id) else {
            continue;
        };
        let Some(callee) = graph.get_symbol(edge.callee_id) else {
            continue;
        };
        if caller.file_path == callee.file_path {
            continue;
        }
        pairs.push((caller.file_path.clone(), callee.file_path.clone()));
    }

    let mut files: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (from, to) in &pairs {
        for path in [from, to] {
            if !index.contains_key(path) {
                index.insert(path.clone(), files.len());
                files.push(path.clone());
            }
        }
    }

    let mut out_links: Vec<Vec<usize>> = vec![Vec::new(); files.len()];
    for (from, to) in &pairs {
        out_links[index[from]].push(index[to]);
    }
    (files, out_links)
}

/// 朴素 PageRank（均匀 teleport + 悬挂节点回流）。
fn pagerank(out_links: &[Vec<usize>], damping: f32, iterations: usize) -> Vec<f32> {
    let count = out_links.len();
    if count == 0 {
        return Vec::new();
    }

    let mut rank = vec![1.0 / count as f32; count];
    for _ in 0..iterations {
        let mut next = vec![0.0f32; count];
        let mut dangling = 0.0f32;
        for (node, links) in out_links.iter().enumerate() {
            if links.is_empty() {
                dangling += rank[node];
                continue;
            }
            let share = rank[node] / links.len() as f32;
            for target in links {
                next[*target] += share;
            }
        }
        let teleport = (1.0 - damping) / count as f32 + damping * dangling / count as f32;
        for node in 0..count {
            next[node] = teleport + damping * next[node];
        }
        rank = next;
    }
    rank
}

fn kind_weight(kind: SymbolKind) -> f32 {
    match kind {
        SymbolKind::Function | SymbolKind::Method => 1.0,
        SymbolKind::Trait | SymbolKind::Interface => 0.9,
        SymbolKind::Struct | SymbolKind::Class | SymbolKind::Enum => 0.8,
        SymbolKind::Module => 0.6,
        SymbolKind::Unknown => 0.5,
    }
}

fn kind_tag(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "fn",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "iface",
        SymbolKind::Enum => "enum",
        SymbolKind::Module => "mod",
        SymbolKind::Unknown => "sym",
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::symbol::{CallEdge, FileSymbols, SymbolDef};

    fn symbol(id: usize, name: &str, file: &str, kind: SymbolKind) -> SymbolDef {
        SymbolDef {
            id,
            name: name.to_string(),
            kind,
            file_path: file.to_string(),
            start_line: id + 1,
            end_line: id + 1,
            signature: format!("fn {name}()"),
        }
    }

    /// 核心文件（被依赖得多）必须排在前面 —— 这就是 PageRank 的意义。
    #[test]
    fn ranks_the_most_called_file_first() {
        // core.rs 定义 hub；a.rs / b.rs 都调它；a.rs / b.rs 互不相连。
        let files = vec![
            FileSymbols {
                file_path: "a.rs".to_string(),
                symbols: vec![symbol(0, "a_entry", "a.rs", SymbolKind::Function)],
                edges: vec![CallEdge {
                    caller_id: 0,
                    callee_name: "hub".to_string(),
                    callee_id: None,
                    file_path: "a.rs".to_string(),
                    line: 1,
                }],
            },
            FileSymbols {
                file_path: "b.rs".to_string(),
                symbols: vec![symbol(1, "b_entry", "b.rs", SymbolKind::Function)],
                edges: vec![CallEdge {
                    caller_id: 1,
                    callee_name: "hub".to_string(),
                    callee_id: None,
                    file_path: "b.rs".to_string(),
                    line: 1,
                }],
            },
            FileSymbols {
                file_path: "core.rs".to_string(),
                symbols: vec![symbol(2, "hub", "core.rs", SymbolKind::Function)],
                edges: Vec::new(),
            },
        ];
        let graph = CallGraph::build(&files);

        let entries = rank_symbols(&graph, None, RepoMapLimits::default());
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names.first().copied(),
            Some("hub"),
            "被依赖最多的符号应排第一：{names:?}"
        );
    }

    /// 单个文件不能占满整张图。
    #[test]
    fn caps_entries_per_file() {
        let symbols: Vec<SymbolDef> = (0..10)
            .map(|id| symbol(id, &format!("sym_{id}"), "big.rs", SymbolKind::Function))
            .collect();
        let graph = CallGraph::build(&[FileSymbols {
            file_path: "big.rs".to_string(),
            symbols,
            edges: Vec::new(),
        }]);

        let limits = RepoMapLimits {
            max_entries: 20,
            max_per_file: 3,
            max_signature_len: 60,
        };
        let entries = rank_symbols(&graph, None, limits);
        assert_eq!(entries.len(), 3, "每文件上限必须生效：{}", entries.len());
    }

    /// focus 提示必须能把相关符号顶上来。
    #[test]
    fn focus_biases_the_map() {
        let files = vec![FileSymbols {
            file_path: "a.rs".to_string(),
            symbols: vec![
                symbol(0, "generic_helper", "a.rs", SymbolKind::Function),
                symbol(1, "retry_backoff", "b.rs", SymbolKind::Function),
            ],
            edges: Vec::new(),
        }];
        let graph = CallGraph::build(&files);

        let focused = rank_symbols(&graph, Some("retry"), RepoMapLimits::default());
        assert_eq!(
            focused.first().map(|entry| entry.name.as_str()),
            Some("retry_backoff"),
            "focus 命中的符号必须排第一"
        );
    }

    /// 地图文本必须带上"这些是真标识符"的提示 —— 它就是给模型当字典用的。
    #[test]
    fn the_map_tells_the_model_to_use_the_names_as_queries() {
        let graph = CallGraph::build(&[FileSymbols {
            file_path: "a.rs".to_string(),
            symbols: vec![symbol(0, "parse_config", "a.rs", SymbolKind::Function)],
            edges: Vec::new(),
        }]);

        let text = format_repo_map(&graph, None, RepoMapLimits::default());
        assert!(text.contains("parse_config"), "{text}");
        assert!(text.contains("CodebaseSearch"), "{text}");
        assert!(text.contains("a.rs"), "{text}");
    }
}
