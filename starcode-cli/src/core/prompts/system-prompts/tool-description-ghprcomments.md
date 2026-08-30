<!--
name: 'Tool Description: GHPRComments'
description: Get GitHub PR review comments
-->
Fetch PR review comments and discussions.

**Use for**: reviewing PR feedback, understanding review context.
**NOT for**: creating PRs (use `suggest_pr`), managing issues (use `github_issue`).

**Rules**:
- Requires `gh` CLI
- Shows inline code comments and discussions
- Use with `git_autofix_pr` for automated fixes