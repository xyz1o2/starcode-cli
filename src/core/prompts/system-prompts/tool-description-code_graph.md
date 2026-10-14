<!--
name: 'Tool Description: CodeGraph'
description: Precise call-graph query — who-calls / callees / call chains for a symbol via tree-sitter + resolved edges. For structural "what calls X" questions.
routing: call-structure
-->
Precise **call-graph** query over the project. Give it a symbol name; it returns who-calls, callees, and resolved call chains built from tree-sitter parsing — the structural relationships `Grep` can't follow across dynamic dispatch.

**Use for**:
- "What calls `foo`?", "how does `X` reach `Y`?", blast-radius before a change
- Tracing caller/callee chains for a function, method, or type

**NOT for**:
- Exact string / regex matches → use `Grep`
- You don't know the symbol name yet, only rough domain words, and want ranked regions across the
  repo → use `CodebaseSearch` first to find candidate symbols, then come back here with the name
- Reading a known file → use `Read`

**Params**:
- `symbol` (required): function/method/type name to trace
- `path`: root directory to scan (default: current directory)

**Behavior**:
- Builds the call graph on demand (parses call-graph-capable languages: Rust/Python/JS/TS/Go/Java/C/C++)
- Returns up to 5 matching symbols with their call chains; narrow with a more specific name
- Read-only; makes no changes
