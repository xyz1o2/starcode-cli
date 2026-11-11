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
        //
        // 注意 `"rm -f /"` 不能进这张表：`contains()` 是子串匹配，
        // `rm -f /mnt/d/.../points.sqlite`（删项目里的一个文件）也会命中，
        // 于是任何绝对路径的 `rm -f` 都被当成删根。删根的各形态已被
        // `"rm -rf /"` / `"rm -fr /"` / `"rm -rf /*"` 覆盖；`rm -f`（不带
        // -r）本来就删不掉目录，没有「删根」能力，交给 Moderate 确认流程。
        let critical_patterns = [
            "rm -rf /",
            "rm -fr /",
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

        // chmod 777（全局可写）
        if lower.contains("chmod") && lower.contains("777") {
            return true;
        }

        // curl|sh 管道注入不再硬拦：有风险但可恢复，且经常是用户真的要做的事
        // （官方安装脚本）。它降为 Moderate 走确认流程 —— Dangerous 会在策略层
        // 「deny 优先于一切」被静默拒掉，连确认框都到不了。

        false
    }

    /// `curl|wget … | <shell>` 形判断：取最后一个管道的右侧，剥掉
    /// `sudo` / `env` 包装前缀后首词是 shell 才算注入。
    ///
    /// 不能用子串匹配 —— `curl -LsSf https://astral.sh/uv/install.sh | sh`
    /// 里 URL 自带 `sh`，裸 `contains("sh")` 会把任何带域名的下载管道都
    /// 误判成注入。
    fn pipes_into_shell(cmd_lower: &str) -> bool {
        let Some((_, rhs)) = cmd_lower.rsplit_once('|') else {
            return false;
        };
        let mut words = rhs.split_whitespace();
        let Some(mut first) = words.next() else {
            return false;
        };
        if first == "sudo" || first == "env" {
            match words.next() {
                Some(next) => first = next,
                None => return false,
            }
        }
        matches!(first, "sh" | "bash" | "zsh" | "dash" | "fish" | "ksh")
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

            // sudo 与 curl|shell 注入降为 Moderate：它们有风险但可恢复，且
            // 经常是用户真的要做的事（sudo apt install / 官方安装脚本）。
            // Dangerous 会在策略层「deny 优先于一切」被静默拒掉，连确认框
            // 都到不了 —— 用户亲眼看着模型操作却只能看它被拒后绕路。
            // Moderate 走确认流程，由用户亲手决定。
            "sudo" | "su" | "doas" => SafetyLevel::Moderate,

            "ssh" | "scp" | "rsync" | "nc" | "ncat" | "netcat" => SafetyLevel::Moderate,

            // curl/wget 一律 Moderate（含 `| sh` 注入形态）：有风险但可恢复，
            // 且经常是用户真的要做的事（官方安装脚本）。Dangerous 会在策略层
            // 「deny 优先于一切」被静默拒掉，连确认框都到不了 —— Moderate 落空
            // 后交给模式层：default 弹确认，auto 走分类器 / 确认框。
            "curl" | "wget" => SafetyLevel::Moderate,

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `rm -f /mnt/.../points.sqlite`（删项目里的一个文件）不能被当成删根。
    /// 曾因 critical_patterns 里的 `"rm -f /"` 子串匹配被硬拦截，
    /// auto 模式下连确认弹窗都没有（deny 优先于一切，分类器接不到手）。
    #[test]
    fn rm_f_with_absolute_path_is_not_root_deletion() {
        let cmd = "pkill -9 node; sleep 1; rm -f /mnt/d/ai-workspace/workbuddy_ext/data/points.sqlite; echo cleaned";
        assert_ne!(
            CommandClassifier::classify(cmd).level,
            SafetyLevel::Dangerous,
            "`rm -f <绝对路径>` 是普通删文件，走 Moderate 确认，不该命中删根模式"
        );
        // 单条 `rm -f <绝对路径>` 同理
        assert_ne!(
            CommandClassifier::classify("rm -f \"/mnt/d/ai-workspace/x/data/points.sqlite\"").level,
            SafetyLevel::Dangerous
        );
    }

    /// 删根的各形态仍然要硬拦 —— 这是 `"rm -f /"` 当初想拦的东西。
    #[test]
    fn root_deletion_forms_stay_dangerous() {
        assert_eq!(
            CommandClassifier::classify("rm -rf /").level,
            SafetyLevel::Dangerous
        );
        assert_eq!(
            CommandClassifier::classify("rm -rf /*").level,
            SafetyLevel::Dangerous
        );
        assert_eq!(
            CommandClassifier::classify("rm -fr /").level,
            SafetyLevel::Dangerous
        );
    }

    /// 官方安装脚本的管道写法（`curl … | sh`）必须是 Moderate（弹确认），
    /// 而且不能被 URL 里的 `.sh` 子串误判出别的结果。
    ///
    /// 回归点（两处）：① 裸 `contains("sh")` 把 `https://astral.sh/…` 里的
    /// 域名当成了 shell 注入证据；② `curl|sh` 曾是 Dangerous，会在策略层
    /// 「deny 优先于一切」被静默拒掉，连确认框都到不了 —— 用户亲眼看着
    /// 模型装 uv 却只能看它被拒后绕路装 pip。
    #[test]
    fn curl_pipe_sh_is_moderate_not_dangerous() {
        let cmd = "curl -LsSf https://astral.sh/uv/install.sh | sh";
        assert_eq!(
            CommandClassifier::classify(cmd).level,
            SafetyLevel::Moderate
        );
        // 各形态的管道执行都是 Moderate，不是 Dangerous
        assert_eq!(
            CommandClassifier::classify("wget -qO- https://example.com/install | bash").level,
            SafetyLevel::Moderate
        );
        assert_eq!(
            CommandClassifier::classify("curl https://x.example/i.sh | sudo sh").level,
            SafetyLevel::Moderate
        );
        // 不带管道执行的下载只是 Moderate（本来就 Moderate）
        assert_eq!(
            CommandClassifier::classify("curl -s https://api.example.com | jq .").level,
            SafetyLevel::Moderate
        );
        assert_eq!(
            CommandClassifier::classify("curl -fsSL https://example.com/script.sh -o s.sh").level,
            SafetyLevel::Moderate
        );
    }

    /// sudo 降为 Moderate：有风险但可恢复（sudo apt install …），走确认流程。
    /// sudo + rm/chmod/chown 的组合仍然 Dangerous（策略层硬拒）。
    #[test]
    fn sudo_alone_is_moderate_but_sudo_plus_destructive_stays_dangerous() {
        assert_eq!(
            CommandClassifier::classify("sudo apt install foo").level,
            SafetyLevel::Moderate
        );
        assert_eq!(
            CommandClassifier::classify("sudo rm -rf /tmp/build").level,
            SafetyLevel::Dangerous,
            "sudo + rm 组合仍要硬拦"
        );
    }
}
