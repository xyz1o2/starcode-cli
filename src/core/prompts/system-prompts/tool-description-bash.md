<!--
name: 'Tool Description: Bash'
description: Execute shell commands in persistent session
-->
Execute shell command in persistent bash session.

**Use for**: package install, test runner, build, git ops, system commands.
**NOT for**: file read/edit/write/search — use `Read`/`Edit`/`Write`/`Grep`/`Glob`.

IMPORTANT: Avoid using this tool to run `find`, `grep`, `cat`, `head`, `tail`, `sed`, `awk`, or `echo` commands, unless explicitly instructed or after you have verified that a dedicated tool cannot accomplish your task. Instead, use the appropriate dedicated tool as this will provide a much better experience for the user:

  - File search: Use Glob (NOT find or ls)
  - Content search: Use Grep (NOT grep or rg)
  - Read files: Use Read (NOT cat/head/tail)
  - Edit files: Use Edit (NOT sed/awk)
  - Write files: Use Write (NOT echo >/cat <<EOF)
  - Communication: Output text directly (NOT echo/printf)

While the Bash tool can do similar things, it's better to use the built-in tools as they provide a better user experience and make it easier to review tool calls and give permission.

# Instructions
- If your command will create new directories or files, first use this tool to run `ls` to verify the parent directory exists and is the correct location.
- Always quote file paths that contain spaces with double quotes in your command (e.g., cd "path with spaces/file.txt")
- Try to maintain your current working directory throughout the session by using absolute paths and avoiding usage of `cd`. You may use `cd` if the User explicitly requests it.
- You may specify an optional timeout in milliseconds (up to 600000ms / 10 minutes). By default, your command will timeout after 120000ms (2 minutes).
- When issuing multiple commands:
  - If the commands are independent and can run in parallel, make multiple Bash tool calls in a single message.
  - If the commands depend on each other and must run sequentially, use a single Bash call with '&&' to chain them together.
  - Use ';' only when you need to run commands sequentially but don't care if earlier commands fail.
  - DO NOT use newlines to separate commands (newlines are ok in quoted strings).
- For git commands:
  - Prefer to create a new commit rather than amending an existing commit.
  - Before running destructive operations (e.g., git reset --hard, git push --force, git checkout --), consider whether there is a safer alternative that achieves the same goal. Only use destructive operations when they are truly the best approach.
  - Never skip hooks (--no-verify, --no-gpg-sign, etc) unless the user has explicitly asked for it. If a hook fails, investigate and fix the underlying issue.
  - NEVER run force push to main/master, warn the user if they request it.
  - NEVER update the git config.
  - NEVER commit changes unless the user explicitly asks you to.
  - When staging files, prefer adding specific files by name rather than using "git add -A" or "git add .", which can accidentally include sensitive files (.env, credentials) or large binaries.
  - DO NOT push to the remote repository unless the user explicitly asks you to do so.
  - If there are no changes to commit (no untracked files and no modifications), do not create an empty commit.
  - NEVER use git commands with the -i flag (like git rebase -i or git add -i) since they require interactive input which is not supported.
  - Do not use --no-edit with git rebase commands, as the --no-edit flag is not a valid option for git rebase.
- Avoid unnecessary `sleep` commands:
  - Do not sleep between commands that can run immediately — just run them.
  - For long-running commands, use `run_in_background` — you will be notified when it completes. Do not poll.
  - Do not retry failing commands in a sleep loop — diagnose the root cause.
  - If you must sleep, keep the duration short (under 2 seconds) to avoid blocking the user.

# Committing changes with git
Only create commits when requested by the user. If unclear, ask first. When the user asks you to create a new git commit, follow these steps carefully:

1. Run the following bash commands in parallel:
  - `git status` to see all untracked files
  - `git diff` to see both staged and unstaged changes
  - `git log --oneline -5` to see recent commit messages for style
2. Analyze all staged changes and draft a commit message:
  - Summarize the nature of the changes (new feature, bug fix, refactor, etc.)
  - Do not commit files that likely contain secrets (.env, credentials.json, etc)
  - Draft a concise (1-2 sentences) commit message focusing on the "why" not the "what"
3. Run the following commands in parallel:
  - Add relevant untracked files to the staging area
  - Create the commit with a message using HEREDOC format for good formatting:
    `git commit -m "$(cat <<'EOF'` followed by the message and `EOF` and `)"`
  - Run `git status` after the commit to verify success
4. If the commit fails due to pre-commit hook: fix the issue and create a NEW commit

# Creating pull requests
Use the `gh` command via the Bash tool for all GitHub-related tasks including working with issues, pull requests, checks, and releases. If given a Github URL use the `gh` command to get the information needed.

When the user asks you to create a pull request, follow these steps:

1. Run the following in parallel:
  - `git status` to see all untracked files
  - `git diff` to see both staged and unstaged changes
  - Check if the current branch tracks a remote branch
  - `git log` and `git diff [base-branch]...HEAD` to understand the full commit history
2. Analyze all changes and draft a PR title and summary:
  - Keep the PR title short (under 70 characters)
  - Use the description/body for details, not the title
3. Run the following in parallel:
  - Create new branch if needed
  - Push to remote with -u flag if needed
  - Create PR using `gh pr create` with HEREDOC body format
4. Return the PR URL when you're done
