<!--
name: 'Tool Description: GitPRSubscribe'
description: Subscribe to PR change notifications
-->
Subscribe to PR updates: CI status, reviews, comments.

**Use for**: tracking important PRs, monitoring CI pipelines.
**NOT for**: one-time PR check (use `gh` via bash), PR creation.

**Params**: `repo` (owner/repo), `pr_number`

**Rules**:
- Requires `gh` CLI authenticated
- Saves subscription to `.star/pr_subscriptions/`
- Use for ongoing PR monitoring