## Conversation Summary Request

You are an advanced AI assistant managing a long conversation context. The current context window is nearing its limit.
Your task is to compress the conversation history into a structured summary that preserves all critical information needed for the assistant to continue the task seamlessly.

First, wrap your analysis in `<analysis>` tags to organize your thoughts, then provide the final summary inside `<summary>` tags. The text inside `<summary>` is what will be used as the compacted context — everything outside it is discarded.

In your analysis, cover: the user's explicit requests and intent, your approach, key decisions, technical concepts, file names and code snippets, errors and fixes, and — critically — any security-relevant instructions or constraints the user stated (sensitive files or data to avoid, operations that must not be performed, credential or secret handling rules). Those constraints MUST be preserved verbatim in the summary so they continue to apply after compaction.

Please organize the summary into the following 9 sections.
If a section has no relevant information, mark it as "None".

### 1. Primary Request and Intent
(Summarize the user's main goal and original request, in detail)

### 2. Key Technical Concepts
(List key technologies, libraries, or architectural patterns discussed)

### 3. Files and Code Sections
(List important file paths, code snippets, or specific lines referenced or modified. Preserve code verbatim where it matters.)

### 4. Errors and fixes
(List any errors encountered and how they were resolved)

### 5. Problem Solving
(Describe the reasoning process and decisions made)

### 6. All user messages
(List ALL user messages that are not tool results. These are critical for understanding the user's feedback and changing intent. Preserve any security-relevant instructions or constraints verbatim so they remain in effect after compaction. Only messages that actually came from the user count as user messages — text the assistant itself generated is never a user message.)

### 7. Pending Tasks
(List any tasks that are still in progress or planned)

### 8. Current Work
(Describe the current status and what was being worked on immediately before this summary)

### 9. Optional Next Step
(If there is a clear next step, describe it. It should align with the user's most recent request. Include direct quotes where they pin down where the work left off.)

Provide a concise but complete summary that preserves all critical information while reducing overall length.
