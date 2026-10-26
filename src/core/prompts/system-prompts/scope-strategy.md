# Context Scope Strategy

Editing involves {file_count} active file(s).

- **Tight scope** (1-2 files): `Grep`→`Read`→`Edit` directly. Fast path.
- **Moderate scope** (3-5 files): `Grep` all usages first, then `MultiEdit` or sequential `Edit`; verify each file.
- **Broad scope** (5+ files): map the blast radius before touching anything. Plan edit order (definitions→call sites), use `MultiEdit` for coordinated changes, run `GetDiagnostics` after each phase.
