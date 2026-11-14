<!--
name: 'Tool Description: GitPRSubscribe'
description: Save a PR subscription record and inspect current state
-->
Save a local PR subscription record and query the PR's current state and check count.

**Use for**: recording a PR to track later.
**NOT for**: claiming that reviews, comments, or CI are being monitored continuously; this call only inspects the current snapshot. For a one-time check, use `gh` via Bash.

**Params**: repo (owner/repo), PR number.

**Rules**:
- Requires an authenticated `gh` CLI.
- Writes a record under `.star/pr_subscriptions/`; do not treat it as read-only.
- Confirm that some monitor is running before promising future notifications.