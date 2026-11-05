use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tokio::fs;

use crate::core::config::storage::Storage;
use sha2::{Digest, Sha256};

const SHELL_EXEC_TIMEOUT: Duration = Duration::from_secs(5);

/// Argument definition for a skill
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: Option<String>,
}

/// Full metadata parsed from SKILL.md frontmatter
///
/// 字段语义对齐 Claude Code（<https://code.claude.com/docs/en/skills>），
/// 尤其是两个正交的调用开关：
///
/// | frontmatter | 用户可调 | 模型可自动调 |
/// |---|---|---|
/// | （默认） | ✓ | ✓ |
/// | `disable-model-invocation: true` | ✓ | ✗ |
/// | `user-invocable: false` | ✗ | ✓ |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub when_to_use: Option<String>,
    /// 本轮免确认放行的工具（调用 skill 的那一轮有效，下条消息即清）。
    pub allowed_tools: Vec<String>,
    /// 本轮从可用池移除的工具。
    pub disallowed_tools: Vec<String>,
    pub argument_hint: Option<String>,
    pub arguments: Vec<SkillArgument>,
    pub model: Option<String>,
    pub context: Option<String>,
    pub effort: Option<String>,
    /// `context: fork` 时的子代理类型（默认 general-purpose）。
    pub agent: Option<String>,
    /// `context: fork` 时是否后台运行（默认 true）。
    pub background: Option<bool>,
    /// `!cmd` 块用的 shell（bash / powershell）。
    pub shell: Option<String>,
    pub paths: Vec<String>,
    pub disabled: bool,
    /// 模型不得自动调用这个 skill（用户仍可显式调用）。
    pub disable_model_invocation: bool,
    /// 用户不可显式调用（仅模型可用）。
    pub user_invocable: bool,
    pub version: Option<String>,
}

impl SkillMetadata {
    /// 这个 skill 是否允许模型自动调用。
    ///
    /// `disable-model-invocation: true` → 不得自动调用。
    pub fn model_may_invoke(&self) -> bool {
        !self.disable_model_invocation
    }

    /// 这个 skill 是否允许用户显式调用。
    ///
    /// `user-invocable: false` → 不可显式调用。默认 true。
    pub fn user_may_invoke(&self) -> bool {
        self.user_invocable
    }

    /// 是否跑在隔离子代理里（`context: fork`）。
    pub fn is_forked(&self) -> bool {
        self.context
            .as_deref()
            .map(|c| c.trim().eq_ignore_ascii_case("fork"))
            .unwrap_or(false)
    }

    /// fork 是否后台运行（默认 true —— 与 Claude Code 一致）。
    pub fn fork_in_background(&self) -> bool {
        self.background.unwrap_or(true)
    }
}

/// `SkillMetadata` 的默认值。
///
/// **手写而不是 derive** —— `user_invocable` 的默认必须是 `true`
/// （Claude Code 语义：默认用户可调），而 `bool` 的 derive 默认是 `false`。
/// 用 derive 的话，「没写 `user-invocable`」会被当成「用户不可调」，
/// 把用户的 skill 全部藏起来 —— 那是**静默失效**，最难查。
impl Default for SkillMetadata {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            when_to_use: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            argument_hint: None,
            arguments: Vec::new(),
            model: None,
            context: None,
            effort: None,
            agent: None,
            background: None,
            shell: None,
            paths: Vec::new(),
            disabled: false,
            disable_model_invocation: false,
            user_invocable: true,
            version: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub location: String,
    pub body: String,
    pub disabled: bool,
    pub version: Option<String>,
    pub metadata: SkillMetadata,
}

pub struct SkillLoader;

impl SkillLoader {
    /// 从指定目录加载所有 SKILL.md
    pub async fn load_skills_from_dir(dir: &Path) -> Vec<SkillDefinition> {
        let mut skills = Vec::new();

        if !dir.exists() {
            return skills;
        }

        let mut read_dir = match fs::read_dir(dir).await {
            Ok(rd) => rd,
            Err(_) => return skills,
        };

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() {
                    if let Some(skill) = Self::load_skill_from_file(&skill_file).await {
                        skills.push(skill);
                    }
                }
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".skill.md") {
                    if let Some(skill) = Self::load_skill_from_file(&path).await {
                        skills.push(skill);
                    }
                }
            }
        }

        skills
    }

    /// 从 GitHub 加载 Skill (通过 git clone，支持缓存)
    pub async fn load_skill_from_github(
        repo_url: &str,
        path_in_repo: Option<&str>,
    ) -> Option<SkillDefinition> {
        let hash = crate::core::utils::paths::sha256_hex(repo_url.as_bytes());

        let cache_base = Storage::global_star_dir().join("skills_cache");
        let cache_dir = cache_base.join(&hash);

        if std::fs::create_dir_all(&cache_base).is_err() {
            return None;
        }

        if cache_dir.exists() {
            let _ = Command::new("git")
                .current_dir(&cache_dir)
                .args(&["pull"])
                .status();
        } else {
            let status = Command::new("git")
                .args(&[
                    "clone",
                    "--depth",
                    "1",
                    repo_url,
                    cache_dir.to_str().unwrap(),
                ])
                .status()
                .ok()?;

            if !status.success() {
                let _ = std::fs::remove_dir_all(&cache_dir);
                return None;
            }
        }

        let mut skill_path = cache_dir.clone();
        if let Some(p) = path_in_repo {
            skill_path.push(p);
        }

        if skill_path.is_dir() {
            skill_path.push("SKILL.md");
        }

        Self::load_skill_from_file(&skill_path).await
    }

    pub async fn load_skill_from_file(path: &Path) -> Option<SkillDefinition> {
        let content = fs::read_to_string(path).await.ok()?;
        Self::parse_skill_definition(&content, &path.to_string_lossy())
    }

    pub fn parse_skill_definition(content: &str, location: &str) -> Option<SkillDefinition> {
        let (metadata, body) = Self::parse_frontmatter(content);

        if metadata.disabled {
            return None;
        }

        Some(SkillDefinition {
            name: if metadata.name.is_empty() {
                "unknown".to_string()
            } else {
                metadata.name.clone()
            },
            description: metadata.description.clone(),
            location: location.to_string(),
            body,
            disabled: metadata.disabled,
            version: metadata.version.clone(),
            metadata,
        })
    }

    /// Full frontmatter parser supporting YAML arrays and nested fields
    pub fn parse_frontmatter(content: &str) -> (SkillMetadata, String) {
        let mut meta = SkillMetadata::default();
        let mut body = content.to_string();

        let Some(after_first) = content.strip_prefix("---") else {
            return (meta, body);
        };

        let Some(end_idx) = after_first.find("---") else {
            return (meta, body);
        };

        let yaml_str = &after_first[..end_idx];
        body = after_first[end_idx + 3..].trim().to_string();

        let lines: Vec<&str> = yaml_str.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                i += 1;
                continue;
            }

            // Detect indented array items (part of a multi-line list)
            // Handled by the parent key's parser below

            if let Some((k, v)) = trimmed.split_once(':') {
                let key = k.trim().to_string();
                let value = v.trim();

                // Check if this key starts a YAML array (next lines are indented items)
                if value.is_empty() {
                    // Collect indented array items
                    let mut items = Vec::new();
                    i += 1;
                    while i < lines.len() {
                        let next = lines[i];
                        let next_trimmed = next.trim();
                        if next_trimmed.is_empty() {
                            i += 1;
                            continue;
                        }
                        // Array item starts with "- "
                        if next_trimmed.starts_with("- ") {
                            let item = next_trimmed
                                .trim_start_matches("- ")
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string();
                            items.push(item);
                            i += 1;
                        } else if next.starts_with("  ") || next.starts_with("\t") {
                            // Still indented but not an array item — skip
                            i += 1;
                        } else {
                            break;
                        }
                    }

                    match key.as_str() {
                        "allowed-tools" | "allowed_tools" => meta.allowed_tools = items,
                        "disallowed-tools" | "disallowed_tools" => meta.disallowed_tools = items,
                        "paths" => meta.paths = items,
                        "arguments" => {
                            // Parse structured arguments from collected items
                            // Each item might be "name: value" or we parse them as argument blocks
                            meta.arguments = parse_argument_items(&lines, &mut i, &items);
                        }
                        _ => {
                            // For unknown keys with arrays, just skip
                        }
                    }
                    continue;
                }

                // Scalar value
                let cleaned = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();

                match key.as_str() {
                    "name" => meta.name = cleaned,
                    "description" => meta.description = cleaned,
                    "when_to_use" | "when-to-use" => meta.when_to_use = Some(cleaned),
                    "argument_hint" | "argument-hint" => meta.argument_hint = Some(cleaned),
                    "model" => meta.model = Some(cleaned),
                    "context" => meta.context = Some(cleaned),
                    "effort" => meta.effort = Some(cleaned),
                    "agent" => meta.agent = Some(cleaned),
                    "shell" => meta.shell = Some(cleaned),
                    "background" => meta.background = Some(is_truthy(&cleaned)),
                    "disabled" => meta.disabled = is_truthy(&cleaned),
                    // 调用闸门（Claude Code 语义）：默认两个都允许。
                    "disable-model-invocation" | "disable_model_invocation" => {
                        meta.disable_model_invocation = is_truthy(&cleaned)
                    }
                    "user-invocable" | "user_invocable" => {
                        // 默认 true —— 只有显式写成假值才禁用。
                        meta.user_invocable = !is_falsy(&cleaned);
                    }
                    "version" => meta.version = Some(cleaned),
                    "allowed-tools" | "allowed_tools" => {
                        // Inline array like: allowed-tools: Bash, Read, Write
                        meta.allowed_tools = parse_inline_csv(&cleaned);
                    }
                    "disallowed-tools" | "disallowed_tools" => {
                        meta.disallowed_tools = parse_inline_csv(&cleaned);
                    }
                    "paths" => {
                        meta.paths = parse_inline_csv(&cleaned);
                    }
                    _ => {}
                }
            }

            i += 1;
        }

        (meta, body)
    }

    /// Perform parameter substitution on skill body content.
    /// Supports `$1`, `$2`, ... for positional args and `${argName}` for named args.
    pub fn substitute_parameters(
        body: &str,
        positional_args: &[String],
        named_args: &HashMap<String, String>,
    ) -> String {
        let mut result = body.to_string();

        // Replace ${argName} first (named args)
        for (name, value) in named_args {
            let placeholder = format!("${{{}}}", name);
            result = result.replace(&placeholder, value);
        }

        // Replace $1, $2, ... (positional args, 1-indexed)
        for (i, arg) in positional_args.iter().enumerate() {
            let placeholder = format!("${}", i + 1);
            result = result.replace(&placeholder, arg);
        }

        result
    }

    /// Check if a skill should be activated based on its `paths` patterns
    /// and the current file paths being worked on.
    /// Returns true if the skill has no paths constraint (always active)
    /// or if any file path matches any pattern.
    pub fn should_activate_skill(skill_paths: &[String], file_paths: &[String]) -> bool {
        if skill_paths.is_empty() {
            return true;
        }

        for pattern in skill_paths {
            let Ok(glob_pattern) = glob::Pattern::new(pattern) else {
                continue;
            };
            for file_path in file_paths {
                if glob_pattern.matches(file_path) {
                    return true;
                }
            }
        }

        false
    }

    pub fn render_skill_prompt(skill: &SkillDefinition) -> String {
        let body = execute_shell_in_prompt(&skill.body);
        body
    }
}

fn execute_single_shell(cmd: &str) -> String {
    let mut child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("[shell error: failed to spawn: {}]", e),
    };

    let deadline = std::time::Instant::now() + SHELL_EXEC_TIMEOUT;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(ref mut out) = child.stdout {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(ref mut err) = child.stderr {
                    let _ = err.read_to_string(&mut stderr);
                }

                let mut result = stdout;
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&format!("[stderr]\n{}", stderr));
                }
                if !status.success() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&format!("[exit code: {}]", status.code().unwrap_or(-1)));
                }
                return result;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return format!(
                        "[shell error: command timed out after {}s]",
                        SHELL_EXEC_TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return format!("[shell error: {}]", e);
            }
        }
    }
}

pub fn execute_shell_in_prompt(content: &str) -> String {
    let mut result = String::new();
    let mut remaining = content;

    while let Some(start) = remaining.find("!(") {
        result.push_str(&remaining[..start]);

        let after_open = &remaining[start + 2..];
        if let Some(end) = find_matching_paren(after_open) {
            let cmd = &after_open[..end];
            let output = execute_single_shell(cmd);
            result.push_str(&output);
            remaining = &after_open[end + 1..];
        } else {
            result.push_str("!(");
            remaining = after_open;
        }
    }

    result.push_str(remaining);
    result
}

fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev_char = '\0';

    for (i, c) in s.char_indices() {
        match c {
            '\'' if !in_double_quote && prev_char != '\\' => in_single_quote = !in_single_quote,
            '"' if !in_single_quote && prev_char != '\\' => in_double_quote = !in_double_quote,
            '(' if !in_single_quote && !in_double_quote => depth += 1,
            ')' if !in_single_quote && !in_double_quote => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
        prev_char = c;
    }
    None
}

/// Parse argument items from YAML block.
/// Supports both simple list items and structured blocks.
fn parse_argument_items(_lines: &[&str], _pos: &mut usize, items: &[String]) -> Vec<SkillArgument> {
    let mut args = Vec::new();

    // Try to parse structured arguments: each item could be "name: description"
    // or we look for multi-line blocks with name/description sub-keys
    for item in items {
        if let Some((name, desc)) = item.split_once(':') {
            args.push(SkillArgument {
                name: name.trim().to_string(),
                description: desc.trim().to_string(),
                required: false,
                default: None,
            });
        } else {
            // Single word as argument name
            args.push(SkillArgument {
                name: item.clone(),
                description: String::new(),
                required: false,
                default: None,
            });
        }
    }

    args
}

fn parse_inline_csv(value: &str) -> Vec<String> {
    let normalized = value.trim().trim_start_matches('[').trim_end_matches(']');
    normalized
        .split(',')
        .map(|s| {
            s.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// frontmatter 布尔值的真值判定。
///
/// Claude Code 的口径（<https://code.claude.com/docs/en/skills>）：
/// "Boolean fields accept `yes`, `no`, `on`, `off`, `1`, and `0` in any letter
/// case, in addition to `true` and `false`."
///
/// 所以真值 = `true` / `yes` / `on` / `1`（任意大小写）。
fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// frontmatter 布尔值的假值判定（`is_truthy` 的反面）。
///
/// 单独一个函数而不是 `!is_truthy(...)`：**空值与无法识别的值不是"假"**。
/// `user-invocable: `（写了 key 但没给值）应当按"没写"处理 —— 也就是
/// 保持默认的 true，而不是当成 false 把用户的 skill 藏起来。
fn is_falsy(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}
