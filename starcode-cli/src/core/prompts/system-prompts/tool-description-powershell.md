<!--
name: 'Tool Description: PowerShell'
description: Execute PowerShell commands (Windows)
-->
Run PowerShell commands on Windows. Includes danger detection.

**Use for**: Windows system admin, automation, Windows-specific tasks.
**NOT for**: Linux/macOS (use `bash`), high-risk commands (Format-Volume).

**Params**: `command`, `timeout` (default: 30s)

**Rules**:
- Windows-only tool
- Dangerous patterns detected and flagged
- Quote paths with spaces