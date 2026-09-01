<!--
name: 'Tool Description: RunTests'
description: Execute test suites
-->
Run project tests with configurable options.

**Use for**: verifying changes, running test suites, CI checks.
**NOT for**: building (use `bash` with build command).

**Params**: `test_command` (optional, auto-detects), `pattern` (test filter)

**Rules**:
- Auto-detects test framework if command not specified
- Shows pass/fail summary
- Run after making changes to verify correctness