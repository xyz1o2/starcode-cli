# Context Scope Strategy

Editing involves {file_count} active file(s).

- **Tight scope** (1-2 files): `Grep`→`Read`→`Edit` directly. Fast path.
- **Moderate scope** (3-5 files): `Grep` all usages first, then `multi_edit` or sequential `Edit`; verify each file.
- **Broad scope** (5+ files): map the blast radius before touching anything. Plan edit order (definitions→call sites), use `multi_edit` for coordinated changes, run `get_diagnostics` after each phase.
