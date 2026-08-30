<!--
name: 'Tool Description: GitBranch'
description: Manage Git branches
-->
Git branch operations: list, create, switch, delete, merge, rebase.

**Use for**: branch lifecycle management.
**NOT for**: viewing diffs (use `git_insight`), cherry-pick (use `bash`).

**Actions**: `list` | `create` | `switch` | `delete` | `merge` | `rebase`

**Rules**:
- `delete` fails on unmerged branches
- `merge`/`rebase` requires `target` branch
- Check `git_insight` before destructive operations