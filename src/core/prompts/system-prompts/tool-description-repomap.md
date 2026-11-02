<!--
name: 'Tool Description: RepoMap'
description: Repository symbol map — the most important functions/methods/types ranked by call-graph centrality, with signatures and locations.
routing: repo-map
-->
A **map of the repository's real identifiers**: the functions, methods and types that matter most (ranked by call-graph centrality), each with its signature and file location.

Search is lexical — it matches the words the code actually uses. When the request is in a language the code isn't written in (or you don't know the names yet), this tool is the dictionary: `retry_backoff`, `parse_config`, `resolve_engine` with their real spellings.

**Use for**:
- "Where is X?" when X is described in prose and you don't know the identifier yet
- Learning a codebase's vocabulary before running `CodebaseSearch` / `Grep`

**NOT for**:
- Exact string / regex matches → `Grep`; ranked regions for a known query → `CodebaseSearch`
- Caller/callee structure for one symbol → `CodeGraph`; reading a known file → `Read`

**Params**: `focus` (optional word from the request), `path` (root, default cwd), `max_entries` (default 60)

**Behavior**: ranking = file-level PageRank × symbol in-degree × kind; read-only
