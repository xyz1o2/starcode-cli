<!--
name: 'Tool Description: ProjectMap'
description: Generate a flat codebase structure inventory (languages, key files, top-level layout, optional symbols)
-->
Generate a **flat inventory** of the project's structure: file counts by language, notable files, and the top-level directory layout.

This is a statistical overview, **not** a dependency or call-graph analysis. It does not trace imports, module relationships, or entry points — for those use `CodebaseSearch` (conceptual/flow questions) or `Read` on specific files.

**Output sections**:
- Summary — scanned file count, depth, and limits reached
- Languages / File Types — counts, top 12
- Key Files — well-known filenames (main, lib, config, README, …)
- Top-Level Layout — top 24 directories with sample files
- Symbols — only when `include_symbols: true`; sampled class/function names

**Params**:
- `path`: root directory to map (default: workspace)
- `max_depth`: traversal depth (default 4)
- `include_symbols`: set `true` to include sampled symbols (default false, faster)
- `force_refresh`: bypass cache and rebuild
- `max_files`: scan cap; raise it for large repos

**Use for**: onboarding to an unfamiliar codebase, confirming a project's shape before diving in.
**NOT for**: finding a specific file (`Glob`), reading content (`Read`), or understanding how modules depend on each other (`CodebaseSearch`).
