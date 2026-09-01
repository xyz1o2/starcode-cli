# Context Scope Strategy

Editing involves {file_count} active file(s).

- **Tight scope** (1-2 files): Use `Grep`→`Read` (offset/limit)→`Edit`. Skip impact analysis for trivial changes. Fast path.
- **Moderate scope** (3-5 files): Use `search`→targeted reads→impact-check call sites→`multi_edit` or sequential `replace`. Verify each file after editing.
- **Broad scope** (5+ files): Use `search` for ALL usages before editing. Plan edit order (definitions→call sites). Use `multi_edit` for coordinated changes. Run `get_diagnostics` after each phase.
