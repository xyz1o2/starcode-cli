# Task Complexity & Planning Strategy

## COMPLEX

Strategy: **Plan → Search → Understand → Impact-Analyze → Phase-Edit → Verify**

1. Use `skill navigator` to map relevant code areas; use `skill analyzer` to understand architecture and dependencies.
2. `Grep` for ALL usages of every symbol you plan to change. List the full blast radius before editing.
3. For coordinated cross-file changes, use `multi_edit` to apply all edits atomically.
4. Consider `Todo` for multi-milestone work. Never create checklist spam.
5. Verify each phase with `get_diagnostics` before proceeding to the next.
6. Use `enter_plan_mode` only if the user explicitly asks, requirements are ambiguous, or the operation is high risk.

## MEDIUM

Strategy: **Search → Read → Impact-Check → Edit → Verify**

1. `search` for exact symbols/function names to locate target code.
2. `Read` with offset/limit to understand context (do NOT read whole files).
3. Before renaming or changing a signature: `search` for all call sites. Edit all together.
4. Use `replace` for single-file, `multi_edit` for cross-file changes.
5. `get_diagnostics` after edits. Direct execution — no plan mode needed.

## SIMPLE

Strategy: **Search → Read → Edit → Verify** (fast path)

1. `search` for the exact symbol/error text — one search should find the target.
2. `Read` with tight offset/limit around the target lines only.
3. Use `replace` for the single change. Skip impact analysis for trivial edits.
4. `get_diagnostics` to confirm no errors. Execute directly without planning.
