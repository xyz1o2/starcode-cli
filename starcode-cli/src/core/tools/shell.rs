use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::normalization::{normalize_to_size, NormalizationConfig};
use crate::core::utils::paths::{normalize_cross_platform_path, resolve_tool_path};
use crate::llm::client::StarClient;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, BufReader};

pub const OUTPUT_UPDATE_INTERVAL_MS: u64 = 1000;
const EXTERNAL_DIR_SIGNATURE_PREFIX: &str = "__external_dir__:";

static EXTERNAL_DIR_ALLOW_SESSION: Lazy<Mutex<HashSet<PathBuf>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

const WINDOWS_DESCRIPTION: &str = r#"Execute a PowerShell command on the local machine. Use PowerShell syntax.

Usage notes:
1. Directory Verification:
   - If creating directories/files, verify parent directory exists using `ls` first.
2. Command Execution:
   - Always quote file paths with spaces (e.g., cd "path with spaces").
   - Use `Set-Location -LiteralPath "C:\path"; python script.py`, not cmd.exe-only `cd /d C:\path && python script.py`.
   - Capture output is automatic.
3. Restrictions:
   - AVOID using `find`, `grep`, `Select-String` for searching. Use `search`, `glob`, or `todo` tools.
   - AVOID using `cat`, `type`, `gc` (Get-Content) to read files. Use `view_file` or `read_many_files`.
   - Maintain current working directory; use absolute paths where possible.
"#;

const UNIX_DESCRIPTION: &str = r#"Execute a shell command on the local machine. Use POSIX sh/bash syntax.

Usage notes:
1. Directory Verification:
   - If creating directories/files, verify parent directory exists using `ls` first.
2. Command Execution:
   - Always quote file paths with spaces (e.g., cd "path with spaces").
   - Capture output is automatic.
3. Restrictions:
   - AVOID using `find`, `grep` for searching. Use `search`, `glob`, or `todo` tools.
   - AVOID using `cat`, `head`, `tail` to read files. Use `view_file` or `read_many_files`.
   - Maintain current working directory; use absolute paths where possible.
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolParams {
    pub command: String,
    pub description: Option<String>,
    #[serde(rename = "dir_path")]
    pub dir_path: Option<String>,
    /// Disable sandbox for this command execution (auto-set when retrying after sandbox restriction)
    #[serde(default)]
    pub dangerously_disable_sandbox: bool,
}

#[derive(Clone)]
pub struct ShellTool {
    config: Arc<crate::core::config::Config>,
    client: Option<StarClient>,
}

impl ShellTool {
    pub fn new(config: Arc<crate::core::config::Config>, client: Option<StarClient>) -> Self {
        Self { config, client }
    }
}

pub struct ShellToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: ShellToolParams,
    client: Option<StarClient>,
}

impl ShellToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: ShellToolParams,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
        client: Option<StarClient>,
    ) -> Self {
        Self {
            config,
            params,
            client,
        }
    }

    fn normalize_windows_shell_command(command: &str) -> String {
        let command = command.trim();
        if command.is_empty() {
            return String::new();
        }

        let segments = Self::split_outside_quotes(command, "&&");
        if segments.iter().any(|segment| segment.trim().is_empty()) {
            return Self::rewrite_cmd_cd_drive_segment(command)
                .unwrap_or_else(|| command.to_string());
        }

        let rewritten = segments
            .iter()
            .map(|segment| {
                Self::rewrite_cmd_cd_drive_segment(segment)
                    .unwrap_or_else(|| segment.trim().to_string())
            })
            .collect::<Vec<_>>();

        if rewritten.len() == 1 {
            rewritten.into_iter().next().unwrap_or_default()
        } else {
            Self::join_powershell_success_chain(&rewritten)
        }
    }

    fn split_outside_quotes(input: &str, delimiter: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut in_single = false;
        let mut in_double = false;
        let mut escaped = false;
        let mut iter = input.char_indices().peekable();

        while let Some((idx, ch)) = iter.next() {
            if escaped {
                escaped = false;
                continue;
            }

            if ch == '`' {
                escaped = true;
                continue;
            }

            if ch == '\'' && !in_double {
                if in_single && input[idx + ch.len_utf8()..].starts_with('\'') {
                    let _ = iter.next();
                    continue;
                }
                in_single = !in_single;
                continue;
            }

            if ch == '"' && !in_single {
                in_double = !in_double;
                continue;
            }

            if !in_single && !in_double && input[idx..].starts_with(delimiter) {
                parts.push(input[start..idx].trim().to_string());
                start = idx + delimiter.len();
                for _ in 1..delimiter.chars().count() {
                    let _ = iter.next();
                }
            }
        }

        parts.push(input[start..].trim().to_string());
        parts
    }

    fn rewrite_cmd_cd_drive_segment(segment: &str) -> Option<String> {
        let rest = Self::strip_cmd_cd_drive_prefix(segment)?;
        let path = Self::parse_windows_cd_path(rest)?;
        if path.trim().is_empty() {
            return None;
        }

        Some(format!(
            "Set-Location -LiteralPath {}",
            Self::powershell_single_quoted_literal(&path)
        ))
    }

    fn strip_cmd_cd_drive_prefix(command: &str) -> Option<&str> {
        let trimmed = command.trim_start();
        let after_cd = Self::strip_case_insensitive_word(trimmed, "cd")
            .or_else(|| Self::strip_case_insensitive_word(trimmed, "chdir"))?;
        let after_cd = after_cd.trim_start();
        let after_drive = Self::strip_case_insensitive_word(after_cd, "/d")?;
        Some(after_drive.trim_start())
    }

    fn strip_case_insensitive_word<'a>(input: &'a str, word: &str) -> Option<&'a str> {
        if input.len() < word.len() {
            return None;
        }

        let (head, tail) = input.split_at(word.len());
        if !head.eq_ignore_ascii_case(word) {
            return None;
        }

        if tail
            .chars()
            .next()
            .map(|ch| !ch.is_whitespace())
            .unwrap_or(false)
        {
            return None;
        }

        Some(tail)
    }

    fn parse_windows_cd_path(input: &str) -> Option<String> {
        let input = input.trim_start();
        if input.is_empty() {
            return None;
        }

        let mut chars = input.char_indices();
        let (_, first) = chars.next()?;
        if first == '"' || first == '\'' {
            let start = first.len_utf8();
            for (idx, ch) in chars {
                if ch == first {
                    return Some(input[start..idx].to_string());
                }
            }
            return Some(input[start..].to_string());
        }

        input.split_whitespace().next().map(|path| path.to_string())
    }

    fn powershell_single_quoted_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn join_powershell_success_chain(segments: &[String]) -> String {
        let mut command = String::new();

        for (idx, segment) in segments.iter().enumerate() {
            if idx > 0 {
                command.push_str("; ");
            }
            command.push_str(segment);

            if idx + 1 < segments.len() {
                command.push_str("; if (-not $?) { exit 1 }");
            }
        }

        command
    }

    fn non_interactive_env_enabled() -> bool {
        std::env::var("STAR_ENABLE_NON_INTERACTIVE_ENV")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(true)
    }

    fn apply_non_interactive_env(cmd: &mut tokio::process::Command) {
        if !Self::non_interactive_env_enabled() {
            return;
        }

        cmd.env("PAGER", "cat");
        cmd.env("GIT_PAGER", "cat");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("CI", "1");
    }

    fn llm_injection_check_enabled() -> bool {
        std::env::var("STAR_ENABLE_LLM_INJECTION_CHECK")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                v == "1" || v == "true" || v == "on"
            })
            .unwrap_or(false)
    }

    fn split_command_args(command: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut in_single = false;
        let mut in_double = false;
        let mut escape = false;

        for ch in command.chars() {
            if escape {
                current.push(ch);
                escape = false;
                continue;
            }

            if ch == '\\' && !in_single {
                escape = true;
                continue;
            }

            if ch == '\'' && !in_double {
                in_single = !in_single;
                continue;
            }

            if ch == '"' && !in_single {
                in_double = !in_double;
                continue;
            }

            if ch.is_whitespace() && !in_single && !in_double {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
                continue;
            }

            current.push(ch);
        }

        if !current.is_empty() {
            args.push(current);
        }

        args
    }

    fn looks_like_path(token: &str) -> bool {
        if token.is_empty() {
            return false;
        }

        if token == "." || token == ".." {
            return true;
        }

        if token.starts_with("./") || token.starts_with("../") || token.starts_with("~") {
            return true;
        }

        if token.contains('/') || token.contains('\\') {
            return true;
        }

        let bytes = token.as_bytes();
        if bytes.len() > 1 && bytes[1] == b':' {
            return true;
        }

        false
    }

    fn strip_glob_prefix(token: &str) -> &str {
        if let Some(idx) = token.find(|c| c == '*' || c == '?' || c == '[') {
            return &token[..idx];
        }
        token
    }

    fn resolve_path_from(base: &Path, token: &str) -> Option<PathBuf> {
        let token = token.trim();
        if token.is_empty() {
            return None;
        }

        let expanded = shellexpand::full(token)
            .ok()
            .map(|v| v.to_string())
            .unwrap_or_else(|| token.to_string());
        let expanded = expanded.trim();
        if expanded.is_empty() {
            return None;
        }

        let path = PathBuf::from(expanded);
        let absolute = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        Some(absolute.canonicalize().unwrap_or(absolute))
    }

    fn external_paths_in_command(
        command: &str,
        base_dir: &Path,
        project_root: &Path,
    ) -> Vec<PathBuf> {
        let args = Self::split_command_args(command);
        if args.is_empty() {
            return Vec::new();
        }

        let mut candidates: Vec<String> = Vec::new();
        let mut iter = args.iter().peekable();
        while let Some(token) = iter.next() {
            if token == ">" || token == ">>" || token == "1>" || token == "2>" || token == "2>>" {
                if let Some(next) = iter.next() {
                    candidates.push(next.clone());
                }
                continue;
            }

            if token == "&&" || token == "||" || token == "|" || token == ";" {
                continue;
            }

            if token.starts_with('-') {
                if let Some(eq_pos) = token.find('=') {
                    let value = &token[eq_pos + 1..];
                    if Self::looks_like_path(value) {
                        candidates.push(value.to_string());
                    }
                }
                continue;
            }

            if Self::looks_like_path(token) {
                candidates.push(token.clone());
            }
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        let mut seen = HashSet::new();
        let mut external = Vec::new();
        for raw in candidates {
            let raw = Self::strip_glob_prefix(&raw);
            if raw.is_empty() {
                continue;
            }

            if let Some(resolved) = Self::resolve_path_from(base_dir, raw) {
                if !crate::core::utils::paths::is_subpath(&resolved, project_root)
                    && seen.insert(resolved.clone())
                {
                    external.push(resolved);
                }
            }
        }

        external
    }

    fn normalize_external_dir(path: &Path) -> PathBuf {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        dir.canonicalize().unwrap_or(dir)
    }

    fn external_dir_signature(path: &Path) -> String {
        format!(
            "{}{}",
            EXTERNAL_DIR_SIGNATURE_PREFIX,
            path.to_string_lossy()
        )
    }

    fn load_permission_signatures(path: &Path) -> HashSet<String> {
        let mut set = HashSet::new();
        let Ok(content) = fs::read_to_string(path) else {
            return set;
        };
        let Ok(list) = serde_json::from_str::<Vec<String>>(&content) else {
            return set;
        };
        for item in list {
            set.insert(item);
        }
        set
    }

    fn save_permission_signatures(path: &Path, set: &HashSet<String>) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let list: Vec<String> = set.iter().cloned().collect();
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = fs::write(path, json);
        }
    }
}

impl BaseDeclarativeTool for ShellTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn display_name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        if cfg!(windows) {
            WINDOWS_DESCRIPTION
        } else {
            UNIX_DESCRIPTION
        }
    }

    fn parameter_schema(&self) -> serde_json::Value {
        let command_desc = if cfg!(windows) {
            "The PowerShell command line to execute."
        } else {
            "The shell command line to execute."
        };

        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": command_desc
                },
                "description": {
                    "type": "string",
                    "description": "Optional description of what the command does."
                },
                "dir_path": {
                    "type": "string",
                    "description": "Optional directory to run the command in."
                }
            },
            "required": ["command"]
        })
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ShellToolParams = serde_json::from_value(params)?;
        Ok(Box::new(ShellToolInvocation::new(
            self.config.clone(),
            params,
            None,
            None,
            self.client.clone(),
        )))
    }
}

fn check_interactive_command(command: &str) -> Option<crate::core::tools::tools::ToolCallConfirmationDetails> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let first = parts.first()?;
    let cmd_lower = first.to_lowercase();

    if cmd_lower == "sudo" {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Admin privileges required".to_string(),
            prompt: format!("Command '{}' requires sudo privileges.\n\nPlease run this command manually in the terminal, or configure sudoers for passwordless access.\n\nContinuing will wait for password input (may timeout).", command),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }

    if ["vim", "vi", "nano", "emacs", "top", "htop", "less", "more"].contains(&cmd_lower.as_str()) {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Danger,
            title: "Interactive Command Detected".to_string(),
            prompt: format!("The command '{}' appears to be interactive and may hang the session. Do you want to proceed?", command),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }
    if (cmd_lower == "python" || cmd_lower == "python3" || cmd_lower == "node") && parts.len() == 1 {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Danger,
            title: "Interactive Interpreter Detected".to_string(),
            prompt: format!("The command '{}' starts an interactive interpreter which will hang. Proceed?", command),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }
    None
}

fn check_dangerous_patterns(command: &str) -> Option<crate::core::tools::tools::ToolCallConfirmationDetails> {
    let dangerous_commands = [
        "rm -rf /", ":(){ :|:& };:", "mkfs", "dd if=/dev/zero", "chmod -R 777 /",
    ];
    for dangerous in dangerous_commands {
        if command.contains(dangerous) {
            return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Danger,
                title: "⛔ CRITICAL: Dangerous Command Detected".to_string(),
                prompt: format!("The command '{}' contains a pattern explicitly blocked for safety ('{}'). Execution is highly discouraged.", command, dangerous),
                on_confirm: std::sync::Arc::new(|_| {}),
            });
        }
    }

    let sensitive_patterns = [
        ".ssh", ".env", ".aws", ".kube", "id_rsa", ".pem", "/etc/shadow", "/etc/passwd",
    ];
    for pattern in sensitive_patterns {
        if command.contains(pattern) {
            return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                title: "Sensitive Resource Access".to_string(),
                prompt: format!("The command references a sensitive resource '{}'. Do you want to proceed?", pattern),
                on_confirm: std::sync::Arc::new(|_| {}),
            });
        }
    }
    None
}

fn check_tool_substitution(command: &str) -> Option<crate::core::tools::tools::ToolCallConfirmationDetails> {
    let cmd_parts: Vec<&str> = command.split_whitespace().collect();
    let cmd_lower = cmd_parts.first()?.to_lowercase();

    if cmd_lower == "Grep" || cmd_lower == "egrep" || cmd_lower == "fgrep" {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Tool Substitution Suggestion".to_string(),
            prompt: format!("Use the 'Grep' tool instead of shell '{}'. It is safer, faster, and provides structured output for the agent.", cmd_parts.first().unwrap()),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }

    if cmd_lower == "find" {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Tool Substitution Suggestion".to_string(),
            prompt: "Use the 'glob' tool instead of 'find'. It is optimized for codebase exploration.".to_string(),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }

    if ["cat", "head", "tail", "more", "less"].contains(&cmd_lower.as_str()) {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Tool Substitution Suggestion".to_string(),
            prompt: format!("Use the 'Read' tool instead of '{}'. It handles large files better and tracks file access context.", cmd_parts.first().unwrap()),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }

    if (cmd_lower == "ListDir" || cmd_lower == "dir") && (command.contains("-R") || command.contains("/s")) {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Tool Substitution Suggestion".to_string(),
            prompt: "Use the 'ListDir' (list_directory) tool for recursive or detailed directory listing. It provides file types and sizes in a parsed format.".to_string(),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }
    None
}

fn check_dangerous_operators(command: &str) -> Option<crate::core::tools::tools::ToolCallConfirmationDetails> {
    let dangerous_ops = ["&&", "||", ";", "|", ">"];
    if dangerous_ops.iter().any(|op| command.contains(op)) {
        return Some(crate::core::tools::tools::ToolCallConfirmationDetails {
            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
            title: "Complex Command".to_string(),
            prompt: format!("The command contains shell operators/chaining. Review carefully: {}", command),
            on_confirm: std::sync::Arc::new(|_| {}),
        });
    }
    None
}

impl ToolInvocation for ShellToolInvocation {
    fn get_description(&self) -> String {
        let mut description = self.params.command.clone();

        if let Some(dir_path) = &self.params.dir_path {
            description.push_str(&format!(" [in {}]", dir_path));
        } else {
            description.push_str(&format!(
                " [current working directory {:?}]",
                std::env::current_dir()
            ));
        }

        if let Some(desc) = &self.params.description {
            description.push_str(&format!(" ({})", desc.replace('\n', " ")));
        }

        description
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        if let Some(dir_path) = &self.params.dir_path {
            vec![ToolLocation {
                path: normalize_cross_platform_path(dir_path),
                location_type: crate::core::tools::tools::LocationType::Execute,
            }]
        } else {
            vec![]
        }
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let path = if let Some(dir) = &params.dir_path {
            resolve_tool_path(config.target_dir(), dir)
        } else {
            config.target_dir().to_path_buf()
        };
        let client = self.client.clone();

        Box::pin(async move {
            let command = &params.command;

            // 1. Interactive Command Check
            if let Some(details) = check_interactive_command(command) {
                return Ok(Some(details));
            }

            // 1.5 Static Dangerous Patterns & Sensitive Files Check
            if let Some(details) = check_dangerous_patterns(command) {
                return Ok(Some(details));
            }

            // 2. LLM Command Injection Detection (Optional, disabled by default)
            if Self::llm_injection_check_enabled() {
                if let Some(client) = &client {
                    let suspicious_chars = ['$', ';', '&', '|', '`', '(', ')', '<', '>', '\n'];
                    if command.contains(&suspicious_chars[..]) {
                        let system_prompt =
                            crate::core::policy::security_prompts::bash_injection_detection_prompt(
                            );
                        let messages = vec![
                            crate::types::StarMessage::system(system_prompt),
                            crate::types::StarMessage::user(command.clone()),
                        ];

                        let check_timeout_secs =
                            std::env::var("STAR_LLM_INJECTION_CHECK_TIMEOUT_SECS")
                                .ok()
                                .and_then(|v| v.parse::<u64>().ok())
                                .unwrap_or(8);
                        if let Ok(Ok(response)) = tokio::time::timeout(
                            Duration::from_secs(check_timeout_secs),
                            client.chat(messages, None, None, None),
                        )
                        .await
                        {
                            if let Some(choice) = response.choices.first() {
                                if let Some(content) = &choice.message.content {
                                    if content.contains("command_injection_detected") {
                                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                                         confirmation_type: crate::core::tools::tools::ConfirmationType::Danger,
                                         title: "⚠️ Security Alert: Command Injection Detected".to_string(),
                                         prompt: format!("The security system detected a potential command injection risk in the command:\n\n`{}`\n\nThis command attempts to execute code beyond the detected prefix. Proceed with extreme caution.", command),
                                         on_confirm: std::sync::Arc::new(|_| {}),
                                     }));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 3.5 External Path Check (Opencode-style)
            let external_paths =
                Self::external_paths_in_command(command, &path, config.target_dir());
            if !external_paths.is_empty() {
                let persist_path = config.storage().project_permissions_path();
                let persisted = Self::load_permission_signatures(&persist_path);
                let session_allowed = EXTERNAL_DIR_ALLOW_SESSION.lock().unwrap();
                let mut pending: Vec<PathBuf> = Vec::new();

                for p in external_paths {
                    let dir = Self::normalize_external_dir(&p);
                    let sig = Self::external_dir_signature(&dir);
                    if persisted.contains(&sig) || session_allowed.contains(&dir) {
                        continue;
                    }
                    pending.push(dir);
                }
                drop(session_allowed);

                if !pending.is_empty() {
                    let mut lines = Vec::new();
                    for (idx, p) in pending.iter().enumerate() {
                        if idx >= 6 {
                            let remaining = pending.len().saturating_sub(idx);
                            if remaining > 0 {
                                lines.push(format!("... 还有 {} 个路径省略", remaining));
                            }
                            break;
                        }
                        lines.push(format!("- {}", p.display()));
                    }

                    let pending_for_confirm = pending.clone();
                    let persist_path_for_confirm = persist_path.clone();
                    return Ok(Some(
                        crate::core::tools::tools::ToolCallConfirmationDetails {
                            confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                            title: "External Path Access".to_string(),
                            prompt: format!(
                                "Command references paths outside the project:\n{}\n\nPlease confirm whether to proceed.",
                                lines.join("\n")
                            ),
                            on_confirm: std::sync::Arc::new(move |outcome| match outcome {
                                crate::types::ToolConfirmationOutcome::AllowSession
                                | crate::types::ToolConfirmationOutcome::ProceedAlways => {
                                    let mut set = EXTERNAL_DIR_ALLOW_SESSION.lock().unwrap();
                                    for dir in &pending_for_confirm {
                                        set.insert(dir.clone());
                                    }
                                }
                                crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave => {
                                    let mut set = ShellToolInvocation::load_permission_signatures(
                                        &persist_path_for_confirm,
                                    );
                                    for dir in &pending_for_confirm {
                                        set.insert(ShellToolInvocation::external_dir_signature(
                                            dir,
                                        ));
                                    }
                                    ShellToolInvocation::save_permission_signatures(
                                        &persist_path_for_confirm,
                                        &set,
                                    );
                                }
                                _ => {}
                            }),
                        },
                    ));
                }
            }
            // 3. Tool Substitution Check
            if let Some(details) = check_tool_substitution(command) {
                return Ok(Some(details));
            }

            // 4. Untrusted Folder Check
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let is_trusted = tf.is_path_trusted(&path).unwrap_or(false);
                    if !is_trusted {
                        let path_clone = path.clone();
                        let config_clone = config.clone();
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Folder".to_string(),
                             prompt: format!("Security: Execution in untrusted path {:?} is blocked. Do you want to proceed?", path),
                             on_confirm: std::sync::Arc::new(move |outcome| {
                                 if let crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave = outcome {
                                     if let Some(tf) = config_clone.trusted_folders() {
                                         let folder_to_trust = if path_clone.is_dir() {
                                              path_clone.clone()
                                          } else {
                                              path_clone.parent().unwrap_or(&path_clone).to_path_buf()
                                          };
                                         let _ = tf.set_trust_level(&folder_to_trust, crate::core::config::trusted_folders::TrustLevel::TrustFolder);
                                     }
                                 }
                             }),
                         }));
                    }
                }
            }

            // 4. Dangerous Operators Check
            if let Some(details) = check_dangerous_operators(command) {
                return Ok(Some(details));
            }

            Ok(None)
        })
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let params = self.params.clone();
        let original_command = self.params.command.clone();
        let signal = signal.cloned();
        Box::pin(async move {
            if let Some(signal) = &signal {
                if signal.is_cancelled() {
                    return Ok(ToolResult {
                        llm_content: Some(
                            "Command was cancelled by user before it could start.".to_string(),
                        ),
                        return_display: Some("Command cancelled by user.".to_string()),
                        output: "Command cancelled by user.".to_string(),
                        error: None,
                        data: None,
                    });
                }
            }

            let stripped_command = params.command.trim().to_string();
            // RTK integration: transparently prefix with `rtk` when available,
            // reducing token consumption by 60-90% on common dev commands.
            let stripped_command = crate::core::tools::rtk::maybe_rtk_wrap(&stripped_command)
                .unwrap_or(stripped_command);
            let execution_command = if cfg!(windows) {
                Self::normalize_windows_shell_command(&stripped_command)
            } else {
                stripped_command.clone()
            };

            let cwd = if let Some(dir_path) = &params.dir_path {
                resolve_tool_path(config.target_dir(), dir_path)
            } else {
                config.target_dir().clone()
            };

            let mut cmd = if cfg!(windows) {
                let mut c = tokio::process::Command::new("powershell.exe");
                // Use UTF-8 for PowerShell output to avoid encoding issues
                // We wrap the command in a block that sets the encoding first
                c.arg("-NoProfile")
                .arg("-Command")
                .arg(format!(
                    "$OutputEncoding = [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {}", 
                    execution_command
                ));
                c
            } else {
                let mut c = tokio::process::Command::new("sh");
                c.arg("-c").arg(&execution_command);
                c
            };

            cmd.current_dir(&cwd);
            cmd.kill_on_drop(true);
            Self::apply_non_interactive_env(&mut cmd);
            cmd.stdin(Stdio::null());
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(e) => {
                    return Ok(ToolResult {
                        llm_content: None,
                        return_display: None,
                        output: String::new(),
                        error: Some(crate::core::tools::tools::ToolError {
                            error_type: "execution_error".to_string(),
                            message: format!("Failed to spawn command: {}", e),
                        }),
                        data: None,
                    });
                }
            };

            let stdout = child.stdout.take().expect("Failed to open stdout");
            let stderr = child.stderr.take().expect("Failed to open stderr");

            let mut stdout_output = String::new();
            let mut stderr_output = String::new();

            let (tx, mut rx) = tokio::sync::mpsc::channel(100);
            let tx_out = tx.clone();
            let tx_err = tx.clone();

            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    // We force UTF-8 via $OutputEncoding on Windows PowerShell,
                    // so always decode as UTF-8 with lossy fallback.
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = tx_out.send((true, s)).await;
                }
            });

            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let s = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = tx_err.send((false, s)).await;
                }
            });

            drop(tx);

            let shell_timeout_secs = std::env::var("STAR_SHELL_TIMEOUT")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(120);
            let inactivity_timeout_ms = std::env::var("STAR_SHELL_INACTIVITY_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or_else(|| config.shell_tool_inactivity_timeout());
            let started_at = Instant::now();
            let mut last_output_at = Instant::now();
            let mut output_channel_closed = false;
            let mut status: Option<std::process::ExitStatus> = None;
            let mut poll = tokio::time::interval(Duration::from_millis(200));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let _ = poll.tick().await;

            loop {
                tokio::select! {
                    chunk = rx.recv(), if !output_channel_closed => {
                        match chunk {
                            Some((is_stdout, s)) => {
                                last_output_at = Instant::now();
                                if is_stdout {
                                    stdout_output.push_str(&s);
                                } else {
                                    stderr_output.push_str(&s);
                                }

                                if let Some(ref cb) = update_output {
                                    cb(s);
                                }
                            }
                            None => {
                                output_channel_closed = true;
                            }
                        }
                    }
                    _ = poll.tick() => {
                        if let Some(signal) = &signal {
                            if signal.is_cancelled() {
                                let _ = child.kill().await;
                                let _ = child.wait().await;
                                return Ok(ToolResult {
                                    llm_content: Some("Command was cancelled by user.".to_string()),
                                    return_display: Some("Command cancelled by user.".to_string()),
                                    output: "Command cancelled by user.".to_string(),
                                    error: None,
                                    data: None,
                                });
                            }
                        }

                        if status.is_none() {
                            status = child.try_wait()?;
                        }

                        if shell_timeout_secs > 0
                            && started_at.elapsed() >= Duration::from_secs(shell_timeout_secs)
                            && status.is_none()
                        {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            return Ok(ToolResult {
                                llm_content: Some(format!(
                                    "Command timed out after {}s: {}\nPartial stdout: {}\nPartial stderr: {}",
                                    shell_timeout_secs, original_command,
                                    if stdout_output.is_empty() { "(empty)" } else { &stdout_output },
                                    if stderr_output.is_empty() { "(none)" } else { &stderr_output },
                                )),
                                return_display: Some(format!("Command timed out after {}s", shell_timeout_secs)),
                                output: format!("Command timed out after {}s.\nPartial output: {}", shell_timeout_secs, stdout_output),
                                error: Some(crate::core::tools::ToolError {
                                    error_type: "timeout".to_string(),
                                    message: format!("Command timed out after {}s", shell_timeout_secs),
                                }),
                                data: None,
                            });
                        }

                        if inactivity_timeout_ms > 0
                            && last_output_at.elapsed() >= Duration::from_millis(inactivity_timeout_ms)
                            && status.is_none()
                        {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            return Ok(ToolResult {
                                llm_content: Some(format!(
                                    "Command inactive for {}ms and was terminated: {}\nPartial stdout: {}\nPartial stderr: {}",
                                    inactivity_timeout_ms, original_command,
                                    if stdout_output.is_empty() { "(empty)" } else { &stdout_output },
                                    if stderr_output.is_empty() { "(none)" } else { &stderr_output },
                                )),
                                return_display: Some(format!("Command stopped after inactivity ({}ms)", inactivity_timeout_ms)),
                                output: format!("Command stopped after inactivity ({}ms).\nPartial output: {}", inactivity_timeout_ms, stdout_output),
                                error: Some(crate::core::tools::ToolError {
                                    error_type: "inactivity_timeout".to_string(),
                                    message: format!("Command inactive for {}ms", inactivity_timeout_ms),
                                }),
                                data: None,
                            });
                        }
                    }
                }

                if status.is_some() && output_channel_closed {
                    break;
                }
            }

            let status = if let Some(status) = status {
                status
            } else {
                child.wait().await?
            };

            Ok(Self::format_command_result(
                &original_command,
                &cwd,
                &stdout_output,
                &stderr_output,
                status,
            ))
        })
    }
}

impl ShellToolInvocation {
    fn format_command_result(
        original_command: &str,
        cwd: &Path,
        stdout_output: &str,
        stderr_output: &str,
        status: std::process::ExitStatus,
    ) -> ToolResult {
        let mut llm_content = format!(
            "Command: {}\n\
             Directory: {}\n\
             Output: {}\n\
             Error: {}\n\
             Exit Code: {}",
            original_command,
            cwd.display(),
            if stdout_output.is_empty() { "(empty)" } else { stdout_output },
            if stderr_output.is_empty() { "(none)" } else { stderr_output },
            status.code().unwrap_or(-1)
        );

        let normalized_value = normalize_to_size(
            json!(llm_content),
            Some(NormalizationConfig {
                target_size: 20 * 1024,
                ..Default::default()
            }),
        );
        if let Some(s) = normalized_value.as_str() {
            llm_content = s.to_string();
        }

        let return_display = if !stdout_output.is_empty() {
            stdout_output.to_string()
        } else if !stderr_output.is_empty() {
            format!("Command failed: {}", stderr_output)
        } else if status.success() {
            String::new()
        } else {
            format!("Command exited with code: {}", status.code().unwrap_or(-1))
        };

        let output_for_display = if return_display.len() > 100 * 1024 {
            format!(
                "{}... (output truncated, total length: {})",
                &return_display[..100 * 1024],
                return_display.len()
            )
        } else {
            return_display.clone()
        };

        let max_raw_output = 100 * 1024;
        let raw_output = if stdout_output.is_empty() && !stderr_output.is_empty() {
            stderr_output
        } else {
            stdout_output
        };
        let display_output = if raw_output.len() > max_raw_output {
            let truncated: String = raw_output.chars().take(max_raw_output).collect();
            format!(
                "{}...\n(output truncated to 100KB, total length: {})",
                truncated,
                raw_output.len()
            )
        } else {
            raw_output.to_string()
        };

        ToolResult {
            llm_content: Some(llm_content),
            return_display: Some(output_for_display),
            output: display_output,
            error: if !status.success() {
                Some(crate::core::tools::tools::ToolError {
                    error_type: "execution_error".to_string(),
                    message: format!("Exit code: {}", status.code().unwrap_or(-1)),
                })
            } else {
                None
            },
            data: None,
        }
    }
}
