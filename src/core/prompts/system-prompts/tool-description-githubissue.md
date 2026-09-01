<!--
name: 'Tool Description: GitHubIssue'
description: Manage GitHub Issues
-->
GitHub Issues: list, view, create, comment, close. Via `gh` CLI.

**Use for**: Issue lifecycle management.
**NOT for**: PR management (use PR tools), when `gh` not authenticated.

**Actions**: `list` | `get` | `create` | `comment` | `close`

**Params**: `repo` (owner/repo), `number`, `title`, `body`, `labels`

**Rules**:
- Requires `gh` CLI authenticated
- `create`: needs title, optional body/labels
- `comment`/`close`: needs issue number