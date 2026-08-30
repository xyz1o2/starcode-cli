// ── Symbol Extraction ─────────────────────────────────────────────────────────
//
// P3 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Extracts function/method/class definitions and call sites from source files
// using Tree-sitter AST parsing.  Feeds the cross-file call graph.

use tree_sitter::{Language, Node, Parser};

// ── Symbol types ─────────────────────────────────────────────────────────────

/// Unique identifier for a symbol in the graph.
pub type SymbolId = usize;

/// The kind of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Trait,
    Interface,
    Enum,
    Module,
    Unknown,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Enum => "enum",
            SymbolKind::Module => "module",
            SymbolKind::Unknown => "unknown",
        }
    }
}

/// A function/class/struct definition extracted from source.
#[derive(Debug, Clone)]
pub struct SymbolDef {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    /// First line of the definition (signature)
    pub signature: String,
}

/// A function call from one symbol to another (resolved or unresolved).
#[derive(Debug, Clone)]
pub struct CallEdge {
    pub caller_id: SymbolId,
    /// The name as written at the call site
    pub callee_name: String,
    /// Resolved to a symbol ID if found in the index
    pub callee_id: Option<SymbolId>,
    pub file_path: String,
    pub line: usize,
}

/// All symbols and edges extracted from a single file.
#[derive(Debug, Clone)]
pub struct FileSymbols {
    pub file_path: String,
    pub symbols: Vec<SymbolDef>,
    pub edges: Vec<CallEdge>,
}

// ── Tree-sitter node type patterns ────────────────────────────────────────────

/// Node kinds that define a function/method/class.
const DEF_KINDS: &[(&str, SymbolKind)] = &[
    ("function_item", SymbolKind::Function),
    ("function_declaration", SymbolKind::Function),
    ("function_definition", SymbolKind::Function),
    ("method_definition", SymbolKind::Method),
    ("method_declaration", SymbolKind::Method),
    ("constructor_declaration", SymbolKind::Method),
    ("class_declaration", SymbolKind::Class),
    ("class_definition", SymbolKind::Class),
    ("struct_item", SymbolKind::Struct),
    ("trait_item", SymbolKind::Trait),
    ("interface_declaration", SymbolKind::Interface),
    ("enum_item", SymbolKind::Enum),
    ("enum_declaration", SymbolKind::Enum),
    ("impl_item", SymbolKind::Unknown),
    ("mod_item", SymbolKind::Module),
    ("type_declaration", SymbolKind::Unknown),
    ("arrow_function", SymbolKind::Function),
    ("generator_function_declaration", SymbolKind::Function),
];

/// Node kinds that represent a function/method call.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "new_expression",
];

/// Maximum depth to descend into call expressions for name extraction.
const MAX_CALL_NAME_DEPTH: usize = 4;

// ── Public API ────────────────────────────────────────────────────────────────

/// Extract all symbol definitions and call edges from source using Tree-sitter.
///
/// Returns `None` if tree-sitter is unavailable for the language or parsing fails.
pub fn extract_symbols(
    content: &str,
    file_path: &str,
    _file_ext: &str,
    language: &Language,
    symbol_id_base: &mut SymbolId,
) -> Option<FileSymbols> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    let tree = parser.parse(content, None)?;

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    let root = tree.root_node();

    walk_for_symbols(
        &root,
        content,
        file_path,
        &mut symbols,
        &mut edges,
        symbol_id_base,
    );

    Some(FileSymbols {
        file_path: file_path.to_string(),
        symbols,
        edges,
    })
}

// ── AST walking ───────────────────────────────────────────────────────────────

fn walk_for_symbols(
    node: &Node,
    source: &str,
    file_path: &str,
    symbols: &mut Vec<SymbolDef>,
    edges: &mut Vec<CallEdge>,
    next_id: &mut SymbolId,
) {
    let kind = node.kind();

    // ── Definition extraction ────────────────────────────────────────────
    if let Some(&(_, sym_kind)) = DEF_KINDS.iter().find(|(k, _)| *k == kind) {
        if let Some(name) = extract_def_name(node, source) {
            let start_line = byte_to_line(source, node.start_byte());
            let end_line = byte_to_line(source, node.end_byte());
            let signature = node_text_first_line(&node, source);

            symbols.push(SymbolDef {
                id: *next_id,
                name,
                kind: sym_kind,
                file_path: file_path.to_string(),
                start_line,
                end_line,
                signature,
            });
            *next_id += 1;
        }
    }

    // ── Call extraction ──────────────────────────────────────────────────
    if CALL_KINDS.contains(&kind) {
        if let Some(callee_name) = extract_call_name(node, source) {
            // Find the enclosing function/method to attribute this call to
            if let Some(caller_sym) = symbols.last() {
                let line = byte_to_line(source, node.start_byte());
                edges.push(CallEdge {
                    caller_id: caller_sym.id,
                    callee_name,
                    callee_id: None, // resolved later in cross-file pass
                    file_path: file_path.to_string(),
                    line,
                });
            }
        }
    }

    // Recurse into children
    for child in node.named_children(&mut node.walk()) {
        walk_for_symbols(&child, source, file_path, symbols, edges, next_id);
    }
}

// ── Name extraction helpers ───────────────────────────────────────────────────

/// Extract the definition name from a definition node.
fn extract_def_name(node: &Node, source: &str) -> Option<String> {
    // Try the `name` field first (most tree-sitter grammars have this)
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(&name_node, source);
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Fallback: find the first identifier child
    for child in node.named_children(&mut node.walk()) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            let name = node_text(&child, source);
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    None
}

/// Extract the function/method name from a call expression.
fn extract_call_name(node: &Node, source: &str) -> Option<String> {
    extract_call_name_depth(node, source, 0)
}

fn extract_call_name_depth(node: &Node, source: &str, depth: usize) -> Option<String> {
    if depth > MAX_CALL_NAME_DEPTH {
        return None;
    }

    // Try the `function` field first
    if let Some(func_node) = node.child_by_field_name("function") {
        return extract_name_from_func_node(&func_node, source, depth);
    }

    // Try the `object` field (for method calls like obj.method())
    // The method name is usually in the `function` field of the method_invocation

    // Fallback: get the first child and try to extract name
    for child in node.named_children(&mut node.walk()) {
        let kind = child.kind();
        match kind {
            "identifier" => return Some(node_text(&child, source).to_string()),
            "field_expression" | "member_expression" | "attribute" => {
                // For obj.method(), the method name is the property/field name
                return extract_property_name(&child, source);
            }
            "call_expression" | "call" | "method_invocation" => {
                // Chained calls: foo()() or obj.method()()
                return extract_call_name_depth(&child, source, depth + 1);
            }
            _ => {}
        }
    }

    None
}

fn extract_name_from_func_node(node: &Node, source: &str, depth: usize) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, source).to_string()),
        "field_expression" | "member_expression" | "attribute" => {
            extract_property_name(node, source)
        }
        "call_expression" | "call" | "method_invocation" => {
            extract_call_name_depth(node, source, depth + 1)
        }
        _ => {
            // Search deeper
            for child in node.named_children(&mut node.walk()) {
                if let Some(name) = extract_name_from_func_node(&child, source, depth + 1) {
                    return Some(name);
                }
            }
            None
        }
    }
}

/// Extract the property/method name from a field_expression (obj.method).
fn extract_property_name(node: &Node, source: &str) -> Option<String> {
    // The property name is the last named child (or the `property` field)
    if let Some(prop) = node.child_by_field_name("property") {
        return Some(node_text(&prop, source).to_string());
    }

    // Fallback: last named child
    let children: Vec<Node> = node.named_children(&mut node.walk()).collect();
    children.last().map(|c| node_text(&c, source).to_string())
}

// ── Text helpers ──────────────────────────────────────────────────────────────

fn node_text<'a>(node: &Node<'a>, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn node_text_first_line<'a>(node: &Node<'a>, source: &'a str) -> String {
    let text = node_text(node, source);
    text.lines()
        .find(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with("//")
                && !t.starts_with("#")
                && !t.starts_with("/*")
                && !t.starts_with("*")
        })
        .unwrap_or("")
        .trim()
        .to_string()
}

fn byte_to_line(source: &str, byte_offset: usize) -> usize {
    let prefix = &source[..byte_offset.min(source.len())];
    prefix.lines().count().max(1)
}

 