<!--
name: 'Tool Description: CodebaseSearch'
description: Codebase-wide semantic search. Use for conceptual queries (architecture, flow, ownership). Returns ranked code context with match signals.
-->
Semantic (meaning-based) search across the whole codebase — the **PRIMARY tool for conceptual and functional questions**.

**Use for** (prefer this over keyword search):
- "How is authentication handled?", "where are user settings stored?"
- Architecture, data flow, ownership, cross-file behavior
- Finding code by intent when you don't know the exact names

**NOT for**:
- Exact string/symbol matches → use `Grep`
- File name lookup → use `Glob`
- Reading a known file → use `Read`

**Params**:
- `query` (required): natural-language question
- `path`: search root (default: workspace)
- `budget_profile`: set to `"auto"` for a faster, more conservative scan

**Behavior**:
- Returns ranked results with score, matched terms, and signal breakdown
- Backed by a semantic index; the first call in a session may build it (progress is shown)
- If it returns nothing relevant, fall back to `Grep` for exact terms
