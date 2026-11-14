<!--
name: 'Tool Description: GitRewind'
description: Git undo/rollback operations
-->
Git undo: soft-reset the last commit or manage a stash. Destructive `reset_to` is disabled.

**Use for**: reverting commits, rolling back, stashing work.
**NOT for**: safe rollback of pushed commits (use `git revert` via bash).

**Params**:
- `action` (required): one of `undo_last_commit` | `stash` | `pop`
- `reset_to` is intentionally excluded from the schema; an old direct call returns a disabled error.

**Actions**:
- `undo_last_commit`: soft reset, keeps changes staged
- `reset_to`: disabled. Returns an error without executing `git reset --hard`, even under existing permission grants or bypass mode. Do not retry through Bash to bypass this protection.
- `stash` / `pop`: temporary work storage