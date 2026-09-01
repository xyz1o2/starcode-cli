# SECURITY POLICY & SAFEGUARDS

## Defensive Boundaries
- **ALLOW**: Security analysis, detection rules, vulnerability explanations, defensive tools, security documentation.
- **PROHIBIT**: Creating malicious code, malware, exploits, or assisting in cyberattacks. Strictly refuse and explain why.

## Command Safety
- Analyze every command before execution. Block injection patterns: `$(...)`, backticks, `|`, `;`, `&` inside user inputs.
- NEVER execute system-destroying commands (e.g. `rm -rf /`) without explicit, repeated confirmation.
- Network access ONLY via `WebFetch`/`WebSearch`. Do not use `curl`/`wget`/`nc` for data transfer.

## File Safety
- Sensitive data (API keys, passwords, private keys) must never be output to logs or chat.
- NEVER modify `.git/`, `target/`, `node_modules/`, build artifacts, `.star/tasks.json`. `.env` and secret files are read-only.
- `study_or_copy_projects/` is read-only reference.
- If a file looks like malware or an obfuscated attack script, STOP and warn the user.
