<!--
name: 'Tool Description: CodeGraph'
description: Compound code investigation — verbatim source, direct callers/callees and upstream/downstream impact for a symbol, in one call.
routing: call-structure
-->
Compound **code investigation** for one symbol. Give it a function/method/type name; it returns, in a single call:

1. the symbol's **verbatim source** (the definition's line range, capped),
2. its **direct callers and callees** (name + file),
3. an **impact summary** — how many symbols are reachable up/downstream at `depth`.

This replaces the usual `Grep` → `Read` → call-graph chain. The graph is pre-indexed in `.star/context/callgraph.db` and synced incrementally, so repeat calls are cheap.

**Use for**:
- "What calls `foo`?", "how does `X` reach `Y`?", blast radius before a change
- Understanding a symbol without opening its file first
- Tracing caller/callee chains for a function, method, or type

**NOT for**:
- Exact string / regex matches → use `Grep`
- You don't know the symbol name yet, only rough domain words, and want ranked regions across the
  repo → use `CodebaseSearch` first to find candidate symbols, then come back here with the name
- Reading a whole file → use `Read`
- Editing → use `Edit` / `MultiEdit`

**Params**:
- `symbol` (required): function/method/type name; ranked exact > camel/snake word boundary > prefix > substring > typo tolerance (1–2 edits for queries of 4+ chars)
- `path`: root directory to scan (default: current directory)
- `depth`: call-chain depth for the impact summary (default 2, max 4)
- `max_symbols`: how many matches to expand with source and call flow (default 3, max 5)

**Behavior**:
- Parses call-graph-capable languages: Rust/Python/JS/TS/Go/Java/C/C++
- Source snippets are capped per symbol; the output says how many lines were omitted and where to `Read` the rest
- Read-only; makes no changes
