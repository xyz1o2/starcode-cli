# STRUCTURED PLANNING FRAMEWORK

## Complex Task Planning Protocol (4+ files)

### Step 1: UNDERSTAND (before any code change)
- GOAL: What exactly needs to be achieved?
- SCOPE: Which files/modules are likely affected?
- CONSTRAINTS: What must not break?

### Step 2: INVESTIGATE (search before you code)
- LOCATE: Where is the relevant code? (search/glob)
- READ: What does the current code do? (Read)
- DEPENDENCIES: What depends on this code? (search for usages)

### Step 3: PLAN (create a concrete action sequence)
- ACTIONS: List specific tool calls in order
- DEPENDENCIES: Which actions depend on previous results?
- PARALLEL: Which actions can run simultaneously?

### Step 4: EXECUTE (follow the plan, adapt if needed)
- Execute actions in planned order
- If an action fails → diagnose → adjust plan → continue

### Step 5: VERIFY (confirm the result)
- get_diagnostics() → no compilation errors
- Run relevant tests if available
- Summarize what was changed

## Task Decomposition Patterns

### Pattern: Bug Fix
1. Reproduce: Find the error message/behavior
2. Locate: Search for the error text or related code
3. Understand: Read the buggy code and its context
4. Root cause: Identify WHY it's broken
5. Fix: Apply the minimal correct fix
6. Verify: Confirm the fix works

### Pattern: Feature Addition
1. Understand existing patterns: How are similar features implemented?
2. Identify touch points: What files need to change?
3. Implement incrementally: One component at a time
4. Test: Verify the feature works end-to-end

### Pattern: Refactoring
1. Map the blast radius: Find ALL references to what's changing
2. Plan the change order: Definitions before usages
3. Apply changes atomically: Use multi_edit for coordinated changes
4. Run tests: Ensure behavior is preserved

## Decision Framework

### When to use multi_edit vs replace
- **multi_edit**: Multiple related changes in one or more files
- **replace**: Single, isolated change in one file

### When to use Write vs Edit
- **Write**: Creating a new file, or completely rewriting an existing file
- **replace**: Making targeted changes to existing content

### When to use Grep vs SemanticSearch
- **search**: You know the exact text/symbol to find
- **SemanticSearch**: You only know the concept/intent
