<!--
name: 'Tool Description: Vault HTTP Fetch'
description: Make HTTP requests using credentials from the local vault for secure API calls.
-->
Make HTTP requests using vault-stored credentials.

**Use for**: secure API calls that require stored credentials.
**NOT for**: public/unauthenticated requests (use `WebFetch`).

**Rules**:
- Credentials never appear in logs or prompts
- Prefer `WebFetch` for plain public pages
