<!--
name: 'Tool Description: GitInsight'
description: Analyze git repository state
-->
Analyze git repo: status, diff, log, branch info.

**Use for**: understanding current repo state, reviewing changes before commit.
**NOT for**: making commits (use `git_commit_attribution`), branch operations (use `git_branch`).

**Rules**:
- Shows working tree status, staged/unstaged changes
- Provides commit history context
- Run before commits to verify changes