<!--
name: 'Tool Description: GitRewind'
description: Git undo/rollback operations
-->
Git undo: undo commit, hard reset, stash, pop.

**Use for**: reverting commits, rolling back, stashing work.
**NOT for**: safe rollback of pushed commits (use `git revert` via bash).

**Actions**: `undo_last_commit` | `reset_to` | `stash` | `pop`

**Rules**:
- `undo_last_commit`: soft reset, keeps changes staged
- `reset_to`: HARD reset — **discards all changes**, confirm first
- `stash`/`pop`: temporary work storage