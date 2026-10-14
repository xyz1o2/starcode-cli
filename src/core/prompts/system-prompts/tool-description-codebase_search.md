<!--
name: 'Tool Description: CodebaseSearch'
description: Ranked cross-file search — top relevant regions with match signals, not every raw match. Use when you know roughly the code's words; for exact strings use Grep.
routing: ranked-cross-file-locate
-->
Ranked **cross-file** search over the whole codebase. It is lexical (matches tokens, not
meaning), but it layers TF-IDF ranking, query expansion and rank fusion on top of raw text
search — so when several files mention roughly the same thing, it hands you the *most
relevant* regions rather than a flat dump of every match.

**Use for**:
- "Where is X handled?" when you know the domain words but not the exact symbol — one ranked
  sweep across the repo instead of three guessed `Grep` patterns
- Cross-file sweeps where the answer is "this touches these 4 places" and you want them ranked
- Ranking noisy matches: a `Grep` that returns 200 hits is a to-do list, this returns the top 10

**NOT for**:
- Exact string / symbol / regex matches → use `Grep` (faster, exact, and it is the right default)
- File name lookup → use `Glob`
- Reading a known file → use `Read`
- Caller / callee structure → use `CodeGraph`

**Params**:
- `query` (required): **keywords**, not a sentence. "retry backoff", "parse config toml". Full
  natural language questions dilute the ranking — pass the terms the code would actually use.
- `path`: search root (default: workspace)
- `budget_profile`: set to `"auto"` for a faster, more conservative scan

**Behavior**:
- Returns ranked results with score, matched terms, and signal breakdown
- Backed by a lexical index (TF-IDF + query expansion + rank fusion); the first call in a
  session may build it, and that first call can come back empty while the index warms up —
  re-run it once before falling back to `Grep`