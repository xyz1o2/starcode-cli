// ── Tree-sitter AST-aware chunking ─────────────────────────────────────────────
//
// P1 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Replaces brace-balancing / indentation heuristics with real AST boundaries,
// raising context_header accuracy from ~70% → ~95%.
//
// Strategy:
//  1. Map file extension → tree-sitter Language
//  2. Parse source → AST
//  3. Walk top-level named children for definition nodes
//  4. Chunk at definition boundaries; sub-chunk large definitions (>120 lines)
//  5. Fall back to SmartChunker on any parse failure

use super::chunking::{CodeChunk, SmartChunker};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use tree_sitter::{Language, Node, Parser, Tree};

// ── Language registry ─────────────────────────────────────────────────────────

/// Map file extension to the corresponding tree-sitter Language.
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

// ── Parser pool ───────────────────────────────────────────────────────────────
//
// Parser::new() + set_language() has non-trivial overhead per file.  Reusing
// parsers per language avoids this during bulk indexing.  parking_lot::Mutex
// is not poisoned — a panic during parsing releases the lock cleanly.

/// Thread-safe pool of tree-sitter parsers, lazily created per language.
static PARSER_POOL: Lazy<Mutex<HashMap<String, Parser>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Execute `f` with a pooled parser for the given file extension.
/// The parser is created on first use per language and reused thereafter.
fn with_pooled_parser<T>(ext: &str, f: impl FnOnce(&mut Parser) -> T) -> Option<T> {
    let _language = language_for_ext(ext)?;
    let mut pool = PARSER_POOL.lock();
    if !pool.contains_key(ext) {
        let mut parser = Parser::new();
        parser.set_language(&language_for_ext(ext).unwrap()).ok()?;
        pool.insert(ext.to_string(), parser);
    }
    let parser = pool.get_mut(ext)?;
    Some(f(parser))
}

/// Node kinds that represent a named definition boundary across languages.
///
/// These are the AST node types that map to semantic chunks — functions, classes,
/// structs, etc.  Everything else (imports, comments, module-level expressions)
/// is collected as preamble or attached to the nearest definition.
const DEFINITION_KINDS: &[&str] = &[
    // Rust
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "mod_item",
    "const_item",
    "static_item",
    "type_item",
    "macro_definition",
    // Python
    "function_definition",
    "class_definition",
    // JavaScript
    "function_declaration",
    "class_declaration",
    "method_definition",
    "generator_function_declaration",
    "lexical_declaration", // const/let at module level
    // TypeScript
    "interface_declaration",
    "type_alias_declaration",
    "enum_declaration",
    "export_statement",
    // Go
    "function_declaration",
    "type_declaration",
    "method_declaration",
    "const_declaration",
    "var_declaration",
    // C
    "function_definition",
    "struct_specifier",
    "enum_specifier",
    "union_specifier",
    "type_definition",
    // C++
    "namespace_definition",
    "template_declaration",
    "linkage_specification",
    // Java
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "method_declaration",
    "constructor_declaration",
    "field_declaration",
];

/// Maximum lines in a single chunk before sub-chunking by inner definitions.
const MAX_CHUNK_LINES: usize = 120;

// ── Public API ────────────────────────────────────────────────────────────────

/// Stack size for the tree-sitter worker thread.
///
/// Tree-sitter's C parser is recursive and can use significant stack space in
/// debug builds (where the C code is compiled at -O0).  8 MiB is generous enough
/// for files up to several thousand lines while keeping thread-spawn overhead low.
#[cfg(debug_assertions)]
const TREE_SITTER_THREAD_STACK: usize = 8 * 1024 * 1024; // 8 MiB
#[cfg(not(debug_assertions))]
const TREE_SITTER_THREAD_STACK: usize = 2 * 1024 * 1024; // 2 MiB — enough in release

/// Try to chunk `content` using tree-sitter for the given file extension.
/// Falls back to `SmartChunker` when tree-sitter is unavailable for the language
/// or parsing fails (including stack overflow in debug builds or Rust-level panics).
///
/// Defense-in-depth:
///   1. Dedicated thread with large stack (8 MiB debug / 2 MiB release)
///   2. `catch_unwind` around the parse call — catches Rust binding panics
///      (unwrap failures, integer overflows).  C-level `abort()` from tree-sitter
///      assertions cannot be caught and will kill the process.
///   3. Fallback to heuristic SmartChunker on any failure path.
pub fn chunk_with_tree_sitter(content: &str, file_ext: &str) -> Vec<CodeChunk> {
    let Some(language) = language_for_ext(file_ext) else {
        return SmartChunker::chunk(content, file_ext);
    };

    let content_owned = content.to_string();
    let file_ext_owned = file_ext.to_string();

    let result = std::thread::Builder::new()
        .name("tree-sitter-chunker".into())
        .stack_size(TREE_SITTER_THREAD_STACK)
        .spawn(move || {
            // catch_unwind to survive Rust-level panics from tree-sitter bindings
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse_and_chunk(&content_owned, &file_ext_owned, language)
            }))
            .ok()
            .flatten()
        })
        .ok()
        .and_then(|handle| handle.join().ok())
        .flatten();

    result.unwrap_or_else(|| SmartChunker::chunk(content, file_ext))
}

/// Parse content and produce chunks — runs on the dedicated tree-sitter thread.
///
/// Uses the pooled parser (parking_lot::Mutex) to reuse Parser objects across
/// files for the same language, avoiding repeated Parser::new() + set_language().
fn parse_and_chunk(content: &str, file_ext: &str, _language: Language) -> Option<Vec<CodeChunk>> {
    let tree = with_pooled_parser(file_ext, |parser| parser.parse(content, None))??;

    let chunks = chunk_from_tree(content, &tree);

    if chunks.is_empty() || chunks.iter().all(|c| c.content.trim().is_empty()) {
        None
    } else {
        Some(chunks)
    }
}

// ── AST walking ───────────────────────────────────────────────────────────────

fn chunk_from_tree(source: &str, tree: &Tree) -> Vec<CodeChunk> {
    let root = tree.root_node();
    let named_children: Vec<Node<'_>> = root
        .named_children(&mut root.walk())
        .filter(|n| n.is_named())
        .collect();

    if named_children.is_empty() {
        // Empty or trivial file — use line-based fallback
        return trivial_chunks(source);
    }

    let mut chunks = Vec::new();
    let mut pending_lines: Vec<&str> = Vec::new(); // lines not yet assigned to a chunk
    let mut pending_start: usize = 1;
    let lines = source_lines(source);

    let mut i = 0;
    while i < named_children.len() {
        let node = named_children[i];
        let node_start_line = byte_to_line(source, node.start_byte());
        let node_end_line = byte_to_line(source, node.end_byte());

        // If there are pending lines before this node, flush or merge
        if !pending_lines.is_empty() {
            // If this node is a definition AND pending lines are brief, merge
            // them into the preamble (they're likely imports/comments above a fn).
            let is_definition = DEFINITION_KINDS.contains(&node.kind());
            if is_definition && pending_lines.len() <= 6 {
                // Extend pending to include this node — attach preamble to definition
                let def_text = lines_in_range(&lines, node_start_line, node_end_line);
                pending_lines.extend(def_text);
                let content = pending_lines.join("\n");
                let header = definition_header(node, source);
                chunks.push(CodeChunk {
                    content,
                    start_line: pending_start,
                    end_line: node_end_line,
                    context_header: header,
                });
                pending_lines.clear();
                i += 1;
                continue;
            }

            // Flush pending as preamble chunk
            let content = pending_lines.join("\n");
            if !content.trim().is_empty() {
                chunks.push(CodeChunk {
                    content,
                    start_line: pending_start,
                    end_line: node_start_line.saturating_sub(1),
                    context_header: None,
                });
            }
            pending_lines.clear();
        }

        if DEFINITION_KINDS.contains(&node.kind()) {
            // Check if the definition is too large and needs sub-chunking
            let node_lines = node_end_line - node_start_line + 1;
            if node_lines > MAX_CHUNK_LINES {
                chunks.extend(sub_chunk_definition(source, node, &lines));
            } else {
                let def_text = lines_in_range(&lines, node_start_line, node_end_line);
                let header = definition_header(node, source);
                chunks.push(CodeChunk {
                    content: def_text.join("\n"),
                    start_line: node_start_line,
                    end_line: node_end_line,
                    context_header: header,
                });
            }
        } else {
            // Non-definition top-level node — collect as pending
            let text = lines_in_range(&lines, node_start_line, node_end_line);
            if pending_lines.is_empty() {
                pending_start = node_start_line;
            }
            pending_lines.extend(text);
        }

        i += 1;
    }

    // Flush remaining pending lines
    if !pending_lines.is_empty() {
        let content = pending_lines.join("\n");
        if !content.trim().is_empty() {
            chunks.push(CodeChunk {
                content,
                start_line: pending_start,
                end_line: lines.len(),
                context_header: None,
            });
        }
    }

    chunks
}

/// Sub-chunk a large definition by its inner definition children.
fn sub_chunk_definition<'a>(
    source: &'a str,
    node: Node<'a>,
    lines: &[&'a str],
) -> Vec<CodeChunk> {
    let node_start_line = byte_to_line(source, node.start_byte());
    let node_end_line = byte_to_line(source, node.end_byte());

    let inner_defs: Vec<Node<'a>> = node
        .named_children(&mut node.walk())
        .filter(|n| DEFINITION_KINDS.contains(&n.kind()))
        .collect();

    if inner_defs.is_empty() {
        // No inner definitions — just emit the whole thing as one chunk
        let def_text = lines_in_range(lines, node_start_line, node_end_line);
        let header = definition_header(node, source);
        return vec![CodeChunk {
            content: def_text.join("\n"),
            start_line: node_start_line,
            end_line: node_end_line,
            context_header: header,
        }];
    }

    let mut chunks = Vec::new();
    let mut cursor_line = node_start_line;

    for inner in &inner_defs {
        let inner_start = byte_to_line(source, inner.start_byte());
        let inner_end = byte_to_line(source, inner.end_byte());

        // Lines between cursor and this inner definition become preamble
        if inner_start > cursor_line {
            let preamble = lines_in_range(lines, cursor_line, inner_start.saturating_sub(1));
            let content = preamble.join("\n");
            if !content.trim().is_empty() {
                chunks.push(CodeChunk {
                    content,
                    start_line: cursor_line,
                    end_line: inner_start - 1,
                    context_header: None,
                });
            }
        }

        let inner_lines = inner_end - inner_start + 1;
        let def_text = lines_in_range(lines, inner_start, inner_end);
        let header = definition_header(*inner, source);

        if inner_lines > MAX_CHUNK_LINES {
            // Very large inner definition — just emit it
            chunks.push(CodeChunk {
                content: def_text.join("\n"),
                start_line: inner_start,
                end_line: inner_end,
                context_header: header,
            });
        } else {
            chunks.push(CodeChunk {
                content: def_text.join("\n"),
                start_line: inner_start,
                end_line: inner_end,
                context_header: header,
            });
        }

        cursor_line = inner_end + 1;
    }

    // Remaining lines after last inner definition
    if cursor_line <= node_end_line {
        let remainder = lines_in_range(lines, cursor_line, node_end_line);
        let content = remainder.join("\n");
        if !content.trim().is_empty() {
            chunks.push(CodeChunk {
                content,
                start_line: cursor_line,
                end_line: node_end_line,
                context_header: None,
            });
        }
    }

    chunks
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build an array of source lines indexed by 1-based line number.
fn source_lines(source: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = source.lines().collect();
    // Ensure 1-based indexing works: prepend a dummy line 0
    lines.insert(0, "");
    lines
}

/// Convert a byte offset into a 1-based line number.
fn byte_to_line(source: &str, byte_offset: usize) -> usize {
    let prefix = &source[..byte_offset.min(source.len())];
    prefix.lines().count().max(1)
}

/// Extract lines from the source lines array (1-based, inclusive range).
fn lines_in_range<'a>(lines: &[&'a str], start_line: usize, end_line: usize) -> Vec<&'a str> {
    let start = start_line.min(lines.len().saturating_sub(1)).max(1);
    let end = end_line.min(lines.len().saturating_sub(1)).max(1);
    if start > end {
        return Vec::new();
    }
    lines[start..=end].to_vec()
}

/// Extract the definition header (first line of the node) as context_header.
fn definition_header<'a>(node: Node<'a>, source: &'a str) -> Option<String> {
    let node_text = &source[node.start_byte()..node.end_byte()];
    // Use the first non-empty, non-comment line of the node
    node_text
        .lines()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//") && !t.starts_with("#") && !t.starts_with("/*")
        })
        .map(|l| l.trim().to_string())
}

/// Fallback for files where tree-sitter finds no named children.
fn trivial_chunks(source: &str) -> Vec<CodeChunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let lines: Vec<&str> = source.lines().collect();
    vec![CodeChunk {
        content: source.to_string(),
        start_line: 1,
        end_line: lines.len().max(1),
        context_header: None,
    }]
}
 