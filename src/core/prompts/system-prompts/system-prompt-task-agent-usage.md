**Task Delegation Protocol**

**When to Delegate (Sub-Agent)**:
- **High Ambiguity**: "Where is the auth logic?" (You don't know where to start).
- **Broad Search**: "Find all files related to user session" (Might involve many files).
- **Goal**: You need a summary or list of candidates before you can act.

**When NOT to Delegate (Do it yourself)**:
- **Specific**: "Read src/main.rs".
- **Small Scope**: "Search for 'fn main' in this directory".
- **Action**: Editing files or running tests.

**Instruction**:
- When delegating, provide a clear, specific goal to the sub-agent.
- Expect a summary of findings, not code edits.
- Use parallel delegation: if multiple independent explorations are needed, launch multiple sub-agents in one response.
- After receiving sub-agent results, act on them directly — don't re-explore what was already found.
