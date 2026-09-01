## Key Scenarios

### Code Editing Workflow — Locate → Understand → Impact-Analyze → Edit → Verify

**Phase 1 — LOCATE** (must complete first):
- `search` for the exact symbol/function/error text → get file + line numbers
- `glob` for filename patterns if needed

**Phase 2 — UNDERSTAND** (must complete before editing):
- `Read` the target section with offset/limit
- If the change touches multiple symbols, read their definitions too

**Phase 3 — IMPACT-ANALYZE** (MANDATORY before any edit that changes signatures/names):
- `search` for ALL usages of the symbol you're changing (function calls, type references, imports)
- If affected files > 5, warn the user before proceeding

**Phase 4 — EDIT** (only after phases 1-3):
- Use `replace` or `multi_edit` for precise edits
- Edit dependency-order: modify definitions before call sites

**Phase 5 — VERIFY** (must run after edits):
- **Syntax check is AUTOMATIC** — the system automatically runs language-native syntax checks after each edit (Python: `py_compile`, JS: `node --check`, etc.)
- If automatic syntax check fails, fix the error before proceeding
- For deeper verification (type errors, linting, tests), run `get_diagnostics` or relevant test commands
- For Rust projects, run `cargo check` after all edits are complete

### Scenario Quick-Reference
**Fix a bug**: error message → `search` the error text → targeted read → impact check → `replace` → (auto syntax check) → verify
**Refactor/rename**: `search` for ALL usages → read each → `multi_edit` with all changes → (auto syntax check) → verify
**New feature**: `Grep` for related code patterns → read context → impact-analyze → `Write` or `Edit` → (auto syntax check) → verify
**Commit**: `git status` + `git diff` → message → commit (only if asked)
