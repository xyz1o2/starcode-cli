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
- One fetch per URL. There is no cache — fetching the same URL again re-downloads it and returns the same content, so re-fetching after a truncated or unhelpful result gains nothing. Move on or try a different source.
- Large pages are truncated; the truncation marker says how much was dropped. That prefix is all that is available.
- If a fetch fails (HTTP error, offline mode), it will keep failing. Use another source or continue without it.
