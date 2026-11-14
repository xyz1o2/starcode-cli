<!--
name: 'Tool Description: GitAutofixPR'
description: Auto-fix PR review issues
-->
Check out the PR head branch and write a review-context file at `.star/autofix_context.md`.

**Use for**: preparing PR review context after checking the working tree and obtaining authorization to switch branches.
**NOT for**: PRs in other repos, when `gh` CLI unavailable.

**Params**: `pr_number`, `fix_prompt` (fix instructions)

**Rules**:
- Changes the current branch and writes a local file; do not run with unrelated uncommitted changes in the working tree.
- Fetches diff and review comments and generates context for subsequent fix operations.
- Does not authorize a commit or push; ask before publishing changes.