<!--
name: 'Tool Description: Extra Tools'
description: Search for and execute deferred/extra tools. Used to discover and load lazily-loaded tools.
-->
Search for and execute deferred tools.

**Use for**: discovering lazily-loaded tools and delegating parameters to them.
**NOT for**: the core file/edit/search tools (always directly visible).

**Rules**:
- `search_extra_tools` first to discover what is available
- Then `execute_extra_tool` with the discovered tool name
