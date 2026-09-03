<!--
name: 'Tool Description: ToolSearch'
description: Discover tools outside the current tool list — keyword query returns ranked matches with JSON schemas, `select:<tool_name>` returns one tool's full schema. Tools found here are callable directly by name.
-->
Discover tools that are not in the current tool list (long-tail built-ins and MCP tools).

**Use for**: finding a capability you suspect exists, getting the parameter schema of a tool you know by name.
**NOT for**: searching code (use `grep`), searching files (use `glob`).

**Params**: `query` (keywords, or `select:<tool_name>`), `max_results` (keyword mode, default 10)

**Rules**:
- Keyword mode: describe the capability, not the tool name — "rename a git branch", "subscribe to PR comments". Matching is per-word, so natural phrasing works.
- Ranked matches come back with name, description, and a full JSON Schema for the top few. Query `select:<tool_name>` to get the schema for any of the rest.
- Anything returned here can be called directly by name in the same turn — there is no separate "execute discovered tool" step.
- Tools you find stay available for the remainder of the session; you do not need to search for the same tool twice.
