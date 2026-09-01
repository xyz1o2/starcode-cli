# TOOL GUIDANCE

## EFFICIENCY RULE — READ THIS FIRST

**NEVER use bash for tasks that have dedicated tools.** This is the #1 efficiency mistake.
Dedicated tools use fewer tokens, execute faster, and produce better structured output.

| Task | Tool | NEVER via bash | Why |
|------|------|----------------|-----|
| Read file | `Read` | `cat`/`head`/`tail` | Dedicated tool returns structured output |
| Edit file | `Edit`/`replace` | `sed`/`awk` | Dedicated tool handles indentation correctly |
| Write file | `Write` | `echo > file` | Dedicated tool is safer and faster |
| Search content | `Grep` | `grep` via bash | Dedicated tool has better regex support |
| Find files | `Glob` | `find` via bash | Dedicated tool is optimized for file patterns |
| List directory | `ls` | `ls` via bash | Dedicated tool returns structured data |

**ONLY use Bash for operations with NO dedicated tool**: package installs, test runners, build commands, git operations, system commands.

## Quick Reference

## Tool Selection Flow

### File Operations
1. **Find** → `glob` (by pattern) or `ls` (by directory)
2. **Read** → `Read` (with offset/limit for large files)
3. **Edit** → `replace` (single) or `multi_edit` (batch)
4. **Create** → `Write` (new files only)

### Code Search
- **Exact match** → `grep` (fast, regex support)
- **Semantic/concept** → `SemanticSearch` (natural language)
- **Structure** → `ProjectMap` (architecture overview)

### ProjectMap Trigger Conditions
Call `ProjectMap` ONLY when:
- User explicitly asks about project structure/architecture
- User is new to the project and needs a codebase overview
- Before cross-module architectural changes to understand module relationships

**Do NOT call when**:
- Looking for specific files (use `glob`)
- Reading file content (use `Read`)
- Making changes within a single file
- User already has sufficient context

### Task Management
- **Multi-step (3+)** → `Todo` for tracking
- **Complex/broad** → `Agent` for autonomous execution
- **Sequential edits** → `multi_edit` for coordinated changes
