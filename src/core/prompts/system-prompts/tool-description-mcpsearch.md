<!--
name: 'Tool Description: MCPSearch'
description: Search MCP server tools
-->
Search for tools available on MCP servers.

**Use for**: discovering MCP capabilities, finding MCP tools.
**NOT for**: local tool search (use `ToolSearch`).

**Params**: `query` (search term), `server` (optional filter)

**Rules**:
- Searches across configured MCP servers
- Use `McpListResources` for resource discovery
- Use discovered tools via `mcp__<server>__<tool>` naming