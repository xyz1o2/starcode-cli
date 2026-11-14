<!--
name: 'Tool Description: WebFetch'
description: Fetch and read web page content
-->
Fetch a URL, convert it to markdown, and either return the page or answer a question about it.

**Use for**: reading documentation, release notes, issue threads, API references.
**NOT for**: searching the web (use `WebSearch`), local files (use `Read`).

**Params**: `url`, `prompt` (optional — what to extract)

**Rules**:
- Pass `prompt` by default: the page is read for you and only the answer comes back. Omitting it drops the whole page into context, which costs a great deal more and stays there for the rest of the session.
- Avoid redundant fetches of the same URL. For large pages, use the available offset/range mechanism if the tool exposes one; otherwise try a more specific source.
- Offline mode remains unavailable until the user disables it. A transient network error, rate limit, or server error may recover; retry only when appropriate rather than claiming all failures are permanent.
