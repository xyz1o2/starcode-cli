// ── AST symbol overview for read_file ────────────────────────────────────────
//
// When read_file returns text content for a supported language, this module
// extracts a compact symbol index (function/class/struct definitions with line
// numbers) using tree-sitter and appends it to the output.
//
// This gives the LLM AST-level awareness without requiring a separate tool call.

use tree_sitter::{Language, Node, Parser};

/// Maximum symbols to include in the overview (avoid context bloat).
const MAX_SYMBOL_OVERVIEW_ENTRIES: usize = 50;

/// Map file extension to tree-sitter Language.
fn language_for_ext(ext: &str) -> Option<Language> {
    match ext {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" | "pyi" => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        _ => None,
    }
}

/// Top-level definition kinds worth listing in the overview.
const DEF_KINDS: &[(&str, &str)] = &[
    ("function_item", "fn"),
    ("function_declaration", "fn"),
    ("function_definition", "fn"),
    ("method_declaration", "fn"),
    ("method_definition", "fn"),
    ("constructor_declaration", "fn"),
    ("class_declaration", "class"),
    ("class_definition", "class"),
    ("struct_item", "struct"),
    ("trait_item", "trait"),
    ("interface_declaration", "interface"),
    ("enum_item", "enum"),
    ("enum_declaration", "enum"),
    ("impl_item", "impl"),
    ("mod_item", "mod"),
    ("type_item", "type"),
    ("type_alias_declaration", "type"),
    ("type_declaration", "type"),
    ("const_item", "const"),
    ("static_item", "static"),
    ("macro_definition", "macro"),
    ("namespace_definition", "namespace"),
    ("export_statement", "export"),
    ("var_declaration", "var"),
    ("field_declaration", "field"),
];

/// Build a compact AST symbol overview from file content.
///
/// Returns `None` if tree-sitter is unavailable for the language or parsing fails.
pub fn build_symbol_overview(content: &str, file_ext: &str) -> Option<String> {
    let language = language_for_ext(file_ext)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(content, None)?;

    let root = tree.root_node();
    let mut symbols: Vec<(String, usize)> = Vec::new();

    for node in root.named_children(&mut root.walk()) {
        if !node.is_named() {
            continue;
        }
        if let Some((label, name)) = extract_def_info(&node, content) {
            let line = node.start_position().row + 1; // tree-sitter rows are 0-based
            symbols.push((format!("{} {} @ L{}", label, name, line), line));
        }
    }

    if symbols.is_empty() {
        return None;
    }

    // Sort by line number; cap at max entries
    symbols.sort_by_key(|(_, line)| *line);
    symbols.truncate(MAX_SYMBOL_OVERVIEW_ENTRIES);

    let index = symbols
        .iter()
        .map(|(s, _)| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        "\n\n--- AST SYMBOLS ({} definitions) ---\n{}\n",
        symbols.len(),
        index
    ))
}

/// Extract definition kind label + name from a tree-sitter node.
fn extract_def_info<'a>(node: &Node<'a>, source: &'a str) -> Option<(&'static str, String)> {
    let kind = node.kind();
    let &(_, label) = DEF_KINDS.iter().find(|(k, _)| *k == kind)?;
    let name = extract_def_name(node, source)?;
    Some((label, name))
}

/// Extract the definition name (identifier) from a definition node.
fn extract_def_name(node: &Node, source: &str) -> Option<String> {
    // Try the `name` field first (most tree-sitter grammars have this)
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(&name_node, source);
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    // Fallback: find the first identifier/type_identifier child
    for child in node.named_children(&mut node.walk()) {
        let ck = child.kind();
        if ck == "identifier" || ck == "type_identifier" {
            let name = node_text(&child, source);
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    None
}

fn node_text<'a>(node: &Node<'a>, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}
