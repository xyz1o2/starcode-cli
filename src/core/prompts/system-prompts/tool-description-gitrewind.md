<!--
name: 'Tool Description: GitRewind'
description: Git undo/rollback operations
-->
Git undo: undo commit, hard reset, stash, pop.

**Use for**: reverting commits, rolling back, stashing work.
**NOT for**: safe rollback of pushed commits (use `git revert` via bash).

**Params**:
- `action` (required): one of `undo_last_commit` | `reset_to` | `stash` | `pop`
- `target`: commit/ref/branch — **required when `action` is `reset_to`**

**Actions**:
- `undo_last_commit`: soft reset, keeps changes staged
- `reset_to`: HARD reset to the target — **discards all changes**. Inspect the target and obtain explicit user authorization first. This tool does not itself guarantee a separate confirmation dialog.
- `stash` / `pop`: temporary work storage