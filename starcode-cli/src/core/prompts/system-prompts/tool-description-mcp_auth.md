<!--
name: 'Tool Description: MCP Auth'
description: Manage OAuth authentication for MCP servers.
-->
Manage OAuth authentication flows for MCP servers.

**Use for**: authenticating MCP servers that require OAuth, refreshing expired tokens.
**NOT for**: listing MCP resources (use `mcp_list_resources`).

**Rules**:
- Only invoke when a server reports an auth error
