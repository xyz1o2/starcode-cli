<!--
name: 'Tool Description: GitAutofixPR'
description: Auto-fix PR review issues
-->
Prepare fix context from PR review comments. Saves to `.star/autofix_context.md`.

**Use for**: addressing PR review feedback automatically.
**NOT for**: PRs in other repos, when `gh` CLI unavailable.

**Params**: `pr_number`, `fix_prompt` (fix instructions)

**Rules**:
- Checks out PR head branch
- Fetches diff and review comments
- Generates context for subsequent fix operations