use serde::{Deserialize, Serialize};

/// Safe cache/build directories that can be recursively deleted without risk.
/// These are auto-generated, reproducible, and contain no user data.
const SAFE_CACHE_DIRS: &[&str] = &[
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    "node_modules/.cache",
    ".cache",
    ".gradle/caches",
    ".npm/_cacache",
    ".yarn/cache",
    "target/debug",
    "target/release",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".parcel-cache",
    ".turbo",
    ".eslintcache",
    ".tsbuildinfo",
    "*.pyc",
    "*.pyo",
    ".DS_Store",
    "Thumbs.db",
];

/// Check if a `rm -rf` command targets only safe paths.
/// Returns true if all targets are relative paths (within working directory).
/// Returns false if any target is an absolute path or contains `..` traversal.
fn is_safe_working_dir_deletion(command: &str) -> bool {
    // Extract the arguments after rm -rf / rm -fr
    let args_part = if let Some(pos) = command.find("rm -rf") {
        &command[pos + 6..]
    } else if let Some(pos) = command.find("rm -fr") {
        &command[pos + 6..]
    } else if let Some(pos) = command.find("rm -r -f") {
        &command[pos + 8..]
    } else if let Some(pos) = command.find("rm -f -r") {
        &command[pos + 8..]
    } else {
        return false;
    };

    let args = args_part.trim();
    if args.is_empty() {
        return false;
    }

    // Check each argument
    for arg in args.split_whitespace() {
        // Skip flags
        if arg.starts_with('-') {
            continue;
        }
        // Absolute paths are dangerous
        if arg.starts_with('/') {
            return false;
        }
        // Parent directory traversal is dangerous
        if arg.contains("..") {
            return false;
        }
        // System directories are dangerous
        let lower = arg.to_lowercase();
        if lower.starts_with("etc")
            || lower.starts_with("var")
            || lower.starts_with("usr")
            || lower.starts_with("bin")
            || lower.starts_with("sbin")
            || lower.starts_with("boot")
            || lower.starts_with("dev")
            || lower.starts_with("proc")
            || lower.starts_with("sys")
            || lower.starts_with("tmp")
            || lower.starts_with("opt")
            || lower.starts_with("mnt")
            || lower.starts_with("media")
            || lower.starts_with("root")
            || lower.starts_with("home")
        {
            return false;
        }
    }
    true
}

/// Check if a `rm -rf` command targets only safe cache directories.
/// Returns true if all targets are in the safe list.
fn is_safe_cache_deletion(command: &str) -> bool {
    // Extract the arguments after rm -rf / rm -fr
    let args_part = if let Some(pos) = command.find("rm -rf") {
        &command[pos + 6..]
    } else if let Some(pos) = command.find("rm -fr") {
        &command[pos + 6..]
    } else if let Some(pos) = command.find("rm -r -f") {
        &command[pos + 8..]
    } else if let Some(pos) = command.find("rm -f -r") {
        &command[pos + 8..]
    } else {
        return false;
    };

    let args = args_part.trim();
    if args.is_empty() {
        return false;
    }

    // Check each argument
    for arg in args.split_whitespace() {
        // Skip flags
        if arg.starts_with('-') {
            continue;
        }
        // Get the basename for comparison
        let basename = arg.rsplit('/').next().unwrap_or(arg);
        let is_safe = SAFE_CACHE_DIRS
            .iter()
            .any(|safe| *safe == basename || *safe == arg || arg.ends_with(safe));
        if !is_safe {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SafetyLevel {
    Safe,
    Moderate,
    Dangerous,
}

#[derive(Debug, Clone)]
pub struct CommandClassification {
    pub level: SafetyLevel,
    pub reason: String,
    pub command_name: String,
}

pub struct CommandClassifier;

impl CommandClassifier {
    pub fn classify(command: &str) -> CommandClassification {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return CommandClassification {
                level: SafetyLevel::Safe,
                reason: "Empty command".to_string(),
                command_name: String::new(),
            };
        }

        if Self::is_dangerous_pattern(trimmed) {
            return CommandClassification {
                level: SafetyLevel::Dangerous,
                reason: "Contains dangerous pattern".to_string(),
                command_name: Self::extract_command_name(trimmed),
            };
        }

        let cmd_name = Self::extract_command_name(trimmed);
        let level = Self::classify_by_name(&cmd_name, trimmed);

        CommandClassification {
            level: level.clone(),
            reason: Self::reason_for_level(&level, &cmd_name),
            command_name: cmd_name,
        }
    }

    fn is_dangerous_pattern(command: &str) -> bool {
        let lower = command.to_lowercase();

        // 真正危险的模式（应该硬拦截）：
        // - 删除根目录
        // - sudo + 危险命令
        // - 磁盘操作
        // - Fork bomb
        // - 管道注入
        let critical_patterns = [
            "rm -rf /",
            "rm -fr /",
            "rm -f /",
            "rm -rf /*",
            "mkfs",
            "fdisk",
            ":(){:|:&};:",
            "fork bomb",
            "> /dev/",
            "dd if=/dev/zero",
            "dd if=/dev/random",
        ];

        for pattern in &critical_patterns {
            if lower.contains(pattern) {
                return true;
            }
        }

        // sudo + 危险命令
        if lower.contains("sudo")
            && (lower.contains("rm") || lower.contains("chmod") || lower.contains("chown"))
        {
            return true;
        }

        // 管道注入
        if (lower.contains("curl") || lower.contains("wget"))
            && lower.contains("|")
            && (lower.contains("Bash") || lower.contains("sh"))
        {
            return true;
        }

        // chmod 777（全局可写）
        if lower.contains("chmod") && lower.contains("777") {
            return true;
        }

        false
    }

    fn extract_command_name(command: &str) -> String {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return String::new();
        }

        let first = parts[0];

        if first.ends_with('/') {
            let path_parts: Vec<&str> = first.rsplit('/').collect();
            path_parts[0].to_string()
        } else {
            first.to_string()
        }
    }

    fn classify_by_name(cmd_name: &str, full_cmd: &str) -> SafetyLevel {
        match cmd_name {
            "ListDir" | "cat" | "Grep" | "find" | "echo" | "pwd" | "whoami" | "date" | "which"
            | "whereis" | "file" | "stat" | "wc" | "head" | "tail" | "less" | "more" | "sort"
            | "uniq" | "diff" | "tree" | "du" | "df" | "env" | "printenv" | "history" | "type"
            | "realpath" | "basename" | "dirname" => SafetyLevel::Safe,

            "git" | "npm" | "yarn" | "pnpm" | "cargo" | "pip" | "pip3" | "make" | "cmake"
            | "docker" | "podman" | "kubectl" | "helm" | "terraform" | "ansible" | "gradle"
            | "mvn" | "dotnet" | "go" | "python" | "python3" | "node" | "ruby" | "java"
            | "javac" | "gcc" | "g++" | "clang" | "rustc" | "rustup" => SafetyLevel::Moderate,

            "mkdir" => {
                // mkdir -p in working directory is safe
                if full_cmd.contains("-p") {
                    SafetyLevel::Safe
                } else {
                    SafetyLevel::Moderate
                }
            }

            "rm" | "rmdir" | "mv" | "cp" | "chmod" | "chown" | "chgrp" | "ln" | "unlink"
            | "truncate" | "shred" => {
                // rm -rf 不再硬拦截，改为 Moderate 走权限确认流程
                // 真正危险的（rm -rf /）已在 is_dangerous_pattern 中拦截
                SafetyLevel::Moderate
            }

            "sudo" | "su" | "doas" => SafetyLevel::Dangerous,

            "ssh" | "scp" | "rsync" | "nc" | "ncat" | "netcat" => SafetyLevel::Moderate,

            "curl" | "wget" => {
                if full_cmd.contains("|") && (full_cmd.contains("Bash") || full_cmd.contains("sh"))
                {
                    SafetyLevel::Dangerous
                } else {
                    SafetyLevel::Moderate
                }
            }

            _ => {
                if cmd_name.starts_with('.') || cmd_name.starts_with('/') {
                    SafetyLevel::Moderate
                } else {
                    SafetyLevel::Moderate
                }
            }
        }
    }

    fn reason_for_level(level: &SafetyLevel, cmd_name: &str) -> String {
        match level {
            SafetyLevel::Safe => format!("{} is a read-only/safe command", cmd_name),
            SafetyLevel::Moderate => format!("{} may modify system state", cmd_name),
            SafetyLevel::Dangerous => format!("{} is potentially destructive", cmd_name),
        }
    }

    /// Get detailed reason for command classification
    pub fn classify_detailed(command: &str) -> (SafetyLevel, String) {
        let result = Self::classify(command);
        (result.level, result.reason.clone())
    }

    pub fn classify_with_pipes(command: &str) -> Vec<CommandClassification> {
        let segments: Vec<&str> = command.split('|').map(|s| s.trim()).collect();
        segments.iter().map(|seg| Self::classify(seg)).collect()
    }

    pub fn overall_safety(classifications: &[CommandClassification]) -> SafetyLevel {
        if classifications
            .iter()
            .any(|c| c.level == SafetyLevel::Dangerous)
        {
            SafetyLevel::Dangerous
        } else if classifications
            .iter()
            .any(|c| c.level == SafetyLevel::Moderate)
        {
            SafetyLevel::Moderate
        } else {
            SafetyLevel::Safe
        }
    }
}
