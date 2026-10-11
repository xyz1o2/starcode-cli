<!--
name: 'Tool Description: CodebaseSearch'
description: Ranked lexical code search (tokens, not meaning): ranking + cross-file signals over raw Grep. For keyword/structural locate; exact strings use Grep.
-->
Ranked **lexical** search across the whole codebase. It matches tokens, not meaning — but layers ranking, query expansion, and cross-file structure signals on top of raw text search, so it's a stronger first pass than `Grep` when you want the *most relevant* code rather than every match.

**Use for**:
- Locating where a keyword / symbol / concept-expressed-in-code-words lives, ranked by relevance
- A fast cross-file "where is this handled" sweep when you know roughly the terms the code uses
- Ranked snippets with match signals instead of a flat grep dump

**NOT for**:
- True conceptual Q&A where your words won't appear in the code (this is lexical, not semantic) — read the project overview / key files instead
- Exact string / symbol matches → use `Grep`
- File name lookup → use `Glob`
- Reading a known file → use `Read`

**Params**:
- `query` (required): keywords or a short phrase whose terms appear in the code
- `path`: search root (default: workspace)
- `budget_profile`: set to `"auto"` for a faster, more conservative scan

**Behavior**:
- Returns ranked results with score, matched terms, and signal breakdown
- Backed by a lexical index (TF-IDF + query expansion + rank fusion); the first call in a session may build it
- If it returns nothing relevant, fall back to `Grep` for exact terms
