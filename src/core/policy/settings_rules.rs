//! Claude Code 风格的权限规则：`Tool(specifier)` 字符串。
//!
//! 规则写在 settings.json 的 `permissions` 段里（对标 Claude Code 的
//! `~/.claude/settings.json`）：
//!
//! ```json
//! {
//!   "permissions": {
//!     "allow": ["Bash(cargo test:*)", "Read(~/.cargo/config.toml)"],
//!     "ask":   ["Bash(git push:*)"],
//!     "deny":  ["Read(./.env)", "WebFetch(domain:evil.example)"],
//!     "defaultMode": "acceptEdits"
//!   }
//! }
//! ```
//!
//! 判定优先级 deny > ask > allow：deny 连 yolo 模式都挡得住，这是有意的 ——
//! 用户写下 deny 是为了兜住"任何情况下都别碰"，而不是给一个能被模式覆盖的建议。
//!
//! 与 Claude Code 的一处**故意偏差**：allow 规则匹配 shell 命令时，`&&`/`||`/`;`/
//! 管道拆出来的每一段都必须命中才算通过，且带命令替换（`$(...)`、反引号）的命令
//! 一概不给 allow。Claude Code 只做朴素前缀匹配并在文档里提示这可被绕过；这里选择
//! 直接挡住，因为 allow 的语义是"免确认执行"，绕过的代价由用户承担。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::policy::types::ApprovalMode;

/// 单条规则的判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleVerdict {
    Allow,
    Ask,
    Deny,
}

/// 解析后的一条 `Tool(specifier)` 规则
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRuleSpec {
    /// 原始字符串，用于回显和去重
    pub raw: String,
    /// 工具名，`*` 表示任意工具
    pub tool: String,
    /// 括号内的限定符；`None` 表示该工具的所有调用
    pub specifier: Option<String>,
}

impl PermissionRuleSpec {
    /// 解析 `Tool` 或 `Tool(specifier)`；空串等非法输入返回 `None`。
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (tool, specifier) = match (raw.find('('), raw.ends_with(')')) {
            // 限定符里可能自带括号，所以取第一个 `(` 到末尾 `)` 之间的全部内容
            (Some(open), true) if open > 0 => {
                let spec = raw[open + 1..raw.len() - 1].trim().to_string();
                (
                    raw[..open].trim().to_string(),
                    if spec.is_empty() { None } else { Some(spec) },
                )
            }
            _ => (raw.to_string(), None),
        };
        if tool.is_empty() {
            return None;
        }
        Some(Self {
            raw: raw.to_string(),
            tool,
            specifier,
        })
    }

    /// `strict = true` 走 allow 规则的口径（对 shell 命令更严），deny/ask 用 false。
    pub fn matches(&self, tool: &str, args: &Value, strict: bool) -> bool {
        if !self.tool_matches(tool) {
            return false;
        }
        match self.specifier.as_deref() {
            None | Some("*") => true,
            Some(spec) => self.specifier_matches(spec, tool, args, strict),
        }
    }

    fn tool_matches(&self, tool: &str) -> bool {
        if self.tool == "*" {
            return true;
        }
        if self.tool.eq_ignore_ascii_case(tool) {
            return true;
        }
        // 别名归一：规则写 `Read`，实际调用可能叫 `view_file`
        let actual = canonical(tool);
        if self.tool.eq_ignore_ascii_case(&actual) || canonical(&self.tool) == actual {
            return true;
        }
        if family_covers(&self.tool, &actual) {
            return true;
        }
        // `mcp__server` 覆盖该 server 下全部工具；`Prefix__*` 沿用旧 PolicyRule 语法
        let wildcard = self.tool.ends_with("__*");
        let prefix = self.tool.strip_suffix("__*").unwrap_or(&self.tool);
        if wildcard || prefix.starts_with("mcp__") {
            return tool == prefix || tool.starts_with(&format!("{}__", prefix));
        }
        false
    }

    fn specifier_matches(&self, spec: &str, tool: &str, args: &Value, strict: bool) -> bool {
        if is_shell_family(&canonical(&self.tool)) || is_shell_family(&canonical(tool)) {
            return command_specifier_matches(spec, args, strict);
        }
        if let Some(domain) = spec.strip_prefix("domain:") {
            return url_domain_matches(domain, args);
        }
        if let Some(path) = extract_path_arg(args) {
            return path_specifier_matches(spec, &path);
        }
        // 兜底：对序列化后的 args 做 glob，等价于旧 PolicyRule 的 args_pattern
        glob_matches(spec, &args.to_string(), false)
    }
}

fn canonical(tool: &str) -> String {
    crate::core::tools::constants::canonical_tool_name(tool)
}

/// 工具族：用户按 Claude Code 的口径写 `Read` / `Edit`，得覆盖 StarCode 里做同一件事的
/// 全部工具名。否则 `deny: ["Edit(./secrets/**)"]` 会被 `smart_edit` 从旁边绕过去。
/// 方向是单向的 —— `Read` 覆盖 `read_many_files`，反过来不成立。
const TOOL_FAMILIES: &[(&str, &[&str])] = &[
    (
        "Read",
        &[
            "view_file",
            "read_many_files",
            "notebook_read",
            "Glob",
            "ListDir",
        ],
    ),
    (
        "Edit",
        &[
            "smart_edit",
            "multi_edit",
            "str_replace_editor",
            "notebook_edit",
            "next_edit",
        ],
    ),
    ("Write", &["create_file", "write_file"]),
    (
        "Bash",
        &[
            "shell",
            "run_shell_command",
            "execute_command",
            "powershell",
        ],
    ),
    ("WebFetch", &["web_fetch", "web_browser"]),
    ("WebSearch", &["web_search"]),
];

fn family_covers(rule_tool: &str, tool: &str) -> bool {
    TOOL_FAMILIES.iter().any(|(family, members)| {
        rule_tool.eq_ignore_ascii_case(family)
            && members.iter().any(|m| m.eq_ignore_ascii_case(tool))
    })
}

fn is_shell_family(name: &str) -> bool {
    matches!(
        name,
        "Bash" | "bash" | "shell" | "run_shell_command" | "execute_command" | "powershell"
    )
}

fn command_specifier_matches(spec: &str, args: &Value, strict: bool) -> bool {
    let Some(command) = extract_command_arg(args) else {
        return false;
    };
    let segments = split_shell_segments(&command);
    if segments.is_empty() {
        return false;
    }
    if strict {
        // 命令替换能把任意子命令偷带进来，免确认这条路直接堵掉
        if command.contains("$(") || command.contains('`') {
            return false;
        }
        segments.iter().all(|seg| segment_matches(spec, seg))
    } else {
        segments.iter().any(|seg| segment_matches(spec, seg))
    }
}

fn extract_command_arg(args: &Value) -> Option<String> {
    ["command", "cmd", "script", "shell_command"]
        .iter()
        .filter_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(|v| v.to_string())
}

/// 按 `&&` `||` `;` `|` 和换行切分；引号与转义内部的分隔符不算分隔符。
fn split_shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quote != Some('\'') => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '\'' | '"' => {
                match quote {
                    Some(q) if q == c => quote = None,
                    None => quote = Some(c),
                    _ => {}
                }
                current.push(c);
            }
            '&' | '|' if quote.is_none() => {
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                push_segment(&mut segments, &mut current);
            }
            ';' | '\n' if quote.is_none() => push_segment(&mut segments, &mut current),
            _ => current.push(c),
        }
    }
    push_segment(&mut segments, &mut current);
    segments
}

fn push_segment(segments: &mut Vec<String>, current: &mut String) {
    let seg = current.trim().to_string();
    current.clear();
    if !seg.is_empty() {
        segments.push(seg);
    }
}

/// `prefix:*` 走前缀匹配（且必须停在词边界），含通配符走 glob，否则按空白归一化精确比较。
fn segment_matches(spec: &str, segment: &str) -> bool {
    let spec = spec.trim();
    let segment = segment.trim();
    if spec == "*" {
        return true;
    }
    if let Some(prefix) = spec.strip_suffix(":*") {
        let prefix = prefix.trim();
        if prefix.is_empty() || segment == prefix {
            return true;
        }
        // `git diff:*` 不该顺手放过 `git diffoscope`，所以要求下一个字符是空白
        return segment.starts_with(prefix)
            && segment[prefix.len()..].starts_with(char::is_whitespace);
    }
    if has_glob_meta(spec) {
        return glob_matches(spec, segment, false);
    }
    normalize_ws(spec) == normalize_ws(segment)
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn glob_matches(pattern: &str, text: &str, literal_separator: bool) -> bool {
    glob::Pattern::new(pattern)
        .map(|p| {
            p.matches_with(
                text,
                glob::MatchOptions {
                    case_sensitive: true,
                    require_literal_separator: literal_separator,
                    require_literal_leading_dot: false,
                },
            )
        })
        .unwrap_or(false)
}

/// 路径限定符走 gitignore 风味：`//abs` 绝对，`~/x` 家目录，其余按项目相对。
fn path_specifier_matches(spec: &str, path: &str) -> bool {
    let candidates = path_candidates(path);
    expand_path_patterns(spec)
        .iter()
        .any(|pattern| candidates.iter().any(|c| path_pattern_matches(pattern, c)))
}

fn normalize_sep(s: &str) -> String {
    s.replace('\\', "/")
}

/// 同一个路径的多种写法都参与匹配：cwd 相对、绝对、去掉 `./` 的原样写法。
fn path_candidates(path: &str) -> Vec<String> {
    let mut out = vec![normalize_sep(path.trim_start_matches("./"))];
    let p = Path::new(path);
    let cwd = std::env::current_dir().ok();
    let abs = if p.is_absolute() {
        Some(p.to_path_buf())
    } else {
        cwd.as_ref().map(|c| c.join(p))
    };
    if let Some(abs) = abs {
        let abs_s = normalize_sep(&abs.to_string_lossy());
        if let Some(cwd_s) = cwd.as_ref().map(|c| normalize_sep(&c.to_string_lossy())) {
            let prefix = format!("{}/", cwd_s.trim_end_matches('/'));
            if let Some(rel) = abs_s.strip_prefix(&prefix) {
                out.push(rel.to_string());
            }
        }
        out.push(abs_s);
    }
    out.dedup();
    out
}

/// 一条路径规则展开成多个候选 pattern。
fn expand_path_patterns(spec: &str) -> Vec<String> {
    let spec = normalize_sep(spec.trim());
    if let Some(rest) = spec.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|home| vec![normalize_sep(&home.join(rest).to_string_lossy())])
            .unwrap_or_default();
    }
    if let Some(rest) = spec.strip_prefix("//") {
        return vec![format!("/{}", rest)];
    }
    if spec.starts_with('/') {
        // Claude Code 里单个 `/` 前缀是"相对 settings 文件所在目录"，
        // 这里两种口径都试，免得用户写 `/src/**` 反而什么都匹配不上
        return vec![spec.clone(), spec.trim_start_matches('/').to_string()];
    }
    let bare = spec.trim_start_matches("./").to_string();
    let mut out = vec![bare.clone()];
    if !bare.contains('/') {
        // gitignore：不含 `/` 的 pattern 在任意层级生效
        out.push(format!("**/{}", bare));
    }
    out
}

fn path_pattern_matches(pattern: &str, candidate: &str) -> bool {
    if has_glob_meta(pattern) {
        return glob_matches(pattern, candidate, true);
    }
    // 目录规则覆盖目录下的一切
    let pattern = pattern.trim_end_matches('/');
    candidate == pattern || candidate.starts_with(&format!("{}/", pattern))
}

/// `domain:example.com` 命中 host 本身及其子域。
fn url_domain_matches(domain: &str, args: &Value) -> bool {
    let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    let Some(raw) = ["url", "uri", "link"]
        .iter()
        .filter_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .map(str::trim)
        .find(|v| !v.is_empty())
    else {
        return false;
    };
    let host = url::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_else(|| {
            // 没带 scheme 时 Url::parse 会失败，手工兜一把
            raw.trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        });
    !host.is_empty() && (host == domain || host.ends_with(&format!(".{}", domain)))
}

fn extract_path_arg(args: &Value) -> Option<String> {
    [
        "file_path",
        "path",
        "target_file",
        "absolute_path",
        "notebook_path",
        "filename",
        "file",
        "dir_path",
        "directory",
    ]
    .iter()
    .filter_map(|key| args.get(*key).and_then(|v| v.as_str()))
    .map(str::trim)
    .find(|v| !v.is_empty())
    .map(|v| v.to_string())
}

/// settings.json `permissions` 段的解析结果（多层设置合并后的并集）。
#[derive(Debug, Clone, Default)]
pub struct SettingsPermissions {
    pub deny: Vec<PermissionRuleSpec>,
    pub ask: Vec<PermissionRuleSpec>,
    pub allow: Vec<PermissionRuleSpec>,
    /// `defaultMode`：default | acceptEdits | plan | bypassPermissions
    pub default_mode: Option<String>,
    /// `additionalDirectories`：允许越出 cwd 访问的目录
    pub additional_directories: Vec<String>,
}

impl SettingsPermissions {
    /// 收集全部 settings 层的权限规则。判定时 deny > ask > allow，所以各层取并集即可，
    /// 不需要"后面的层覆盖前面的层"。
    pub fn from_project(cwd: &Path) -> Self {
        Self::from_paths(&Self::candidate_files(cwd))
    }

    /// 只读给定文件，不碰全局设置 —— 单测靠这个入口保持与运行机器无关。
    pub fn from_paths(paths: &[PathBuf]) -> Self {
        let mut out = Self::default();
        for path in paths {
            out.merge_file(path);
        }
        out
    }

    /// 从低优先级到高优先级；`defaultMode` 后写的赢。
    pub fn candidate_files(cwd: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Some(explicit) = std::env::var("STAR_PERMISSIONS_FILE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            files.push(PathBuf::from(explicit));
        }
        files.push(crate::core::config::storage::Storage::global_star_dir().join("settings.json"));
        let project = crate::core::config::storage::Storage::new(cwd.to_path_buf()).star_dir();
        files.push(project.join("settings.json"));
        files.push(project.join("settings.local.json"));
        files
    }

    fn merge_file(&mut self, path: &Path) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        match crate::core::config::json_with_comments::parse_json_with_comments::<Value>(&content) {
            Ok(value) => self.merge_json(&value),
            Err(e) => crate::utils::logging::append_debug_log_line(&format!(
                "[Policy] Skipping unparsable settings {}: {}",
                path.display(),
                e
            )),
        }
    }

    /// 从一个 settings JSON 对象里合并 `permissions` 段。
    pub fn merge_json(&mut self, settings: &Value) {
        let Some(perms) = settings.get("permissions") else {
            return;
        };
        push_rules(&mut self.allow, perms.get("allow"));
        push_rules(&mut self.ask, perms.get("ask"));
        push_rules(&mut self.deny, perms.get("deny"));
        if let Some(mode) = pick(perms, &["defaultMode", "default_mode"]).and_then(|v| v.as_str()) {
            self.default_mode = Some(mode.trim().to_string());
        }
        if let Some(dirs) = pick(perms, &["additionalDirectories", "additional_directories"])
            .and_then(|v| v.as_array())
        {
            for dir in dirs.iter().filter_map(|v| v.as_str()) {
                let dir = dir.trim().to_string();
                if !dir.is_empty() && !self.additional_directories.contains(&dir) {
                    self.additional_directories.push(dir);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.ask.is_empty() && self.deny.is_empty()
    }

    pub fn rule_count(&self) -> usize {
        self.allow.len() + self.ask.len() + self.deny.len()
    }

    /// deny > ask > allow。没有任何规则命中时返回 `None`，交给上层继续走审批模式。
    pub fn evaluate(&self, tool: &str, args: &Value) -> Option<(RuleVerdict, String)> {
        for rule in &self.deny {
            if rule.matches(tool, args, false) {
                return Some((RuleVerdict::Deny, rule.raw.clone()));
            }
        }
        for rule in &self.ask {
            if rule.matches(tool, args, false) {
                return Some((RuleVerdict::Ask, rule.raw.clone()));
            }
        }
        for rule in &self.allow {
            if rule.matches(tool, args, true) {
                return Some((RuleVerdict::Allow, rule.raw.clone()));
            }
        }
        None
    }
}

fn pick<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| value.get(*k))
}

fn push_rules(out: &mut Vec<PermissionRuleSpec>, value: Option<&Value>) {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return;
    };
    for rule in items
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(PermissionRuleSpec::parse)
    {
        if !out.iter().any(|existing| existing.raw == rule.raw) {
            out.push(rule);
        }
    }
}

/// `defaultMode` / `/permissions <mode>` / `--permission-mode` 共用的一套别名口径。
pub fn approval_mode_from_str(raw: &str) -> Option<ApprovalMode> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', '_'], "");
    match normalized.as_str() {
        "default" | "acceptedits" | "ask" => Some(ApprovalMode::Default),
        "plan" | "readonly" => Some(ApprovalMode::Plan),
        "yolo" | "bypasspermissions" | "dangerouslyskippermissions" => Some(ApprovalMode::Yolo),
        _ => None,
    }
}

/// `/permissions` 写规则的落点：项目级 `.star/settings.local.json`。
///
/// 对标 Claude Code —— 交互式加的规则进 local 层而不是 `settings.json`，
/// 免得把个人偏好提交进仓库。
pub fn local_settings_path(cwd: &Path) -> PathBuf {
    crate::core::config::storage::Storage::new(cwd.to_path_buf())
        .star_dir()
        .join("settings.local.json")
}

fn read_local_settings(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| {
            crate::core::config::json_with_comments::parse_json_with_comments::<Value>(&c).ok()
        })
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

fn write_local_settings(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {}", parent.display(), e))?;
    }
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| format!("serialize {}: {}", path.display(), e))?;
    std::fs::write(path, body + "\n").map_err(|e| format!("write {}: {}", path.display(), e))
}

/// 往 `.star/settings.local.json` 的 `permissions.<bucket>` 里追加一条规则。
/// 返回落盘路径；规则已存在时返回 `Ok(None)`。
pub fn add_local_rule(
    cwd: &Path,
    bucket: RuleVerdict,
    rule: &str,
) -> Result<Option<PathBuf>, String> {
    let spec = PermissionRuleSpec::parse(rule).ok_or_else(|| format!("Invalid rule: {}", rule))?;
    let path = local_settings_path(cwd);
    let mut settings = read_local_settings(&path);
    let bucket_key = bucket_key(bucket);

    let array = settings
        .as_object_mut()
        .expect("read_local_settings 保证是 object")
        .entry("permissions")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| "`permissions` in settings.local.json is not an object".to_string())?
        .entry(bucket_key)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| format!("`permissions.{}` is not an array", bucket_key))?;

    if array.iter().any(|v| v.as_str() == Some(spec.raw.as_str())) {
        return Ok(None);
    }
    array.push(Value::String(spec.raw.clone()));
    write_local_settings(&path, &settings)?;
    Ok(Some(path))
}

/// 从 `.star/settings.local.json` 的所有桶里删掉一条规则；返回删掉的桶。
pub fn remove_local_rule(cwd: &Path, rule: &str) -> Result<Vec<RuleVerdict>, String> {
    let rule = rule.trim();
    let path = local_settings_path(cwd);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut settings = read_local_settings(&path);
    let mut removed = Vec::new();
    if let Some(perms) = settings
        .as_object_mut()
        .and_then(|o| o.get_mut("permissions"))
        .and_then(|v| v.as_object_mut())
    {
        for bucket in [RuleVerdict::Allow, RuleVerdict::Ask, RuleVerdict::Deny] {
            if let Some(array) = perms
                .get_mut(bucket_key(bucket))
                .and_then(|v| v.as_array_mut())
            {
                let before = array.len();
                array.retain(|v| v.as_str() != Some(rule));
                if array.len() != before {
                    removed.push(bucket);
                }
            }
        }
    }
    if !removed.is_empty() {
        write_local_settings(&path, &settings)?;
    }
    Ok(removed)
}

pub fn bucket_key(bucket: RuleVerdict) -> &'static str {
    match bucket {
        RuleVerdict::Allow => "allow",
        RuleVerdict::Ask => "ask",
        RuleVerdict::Deny => "deny",
    }
}

pub fn bucket_from_str(raw: &str) -> Option<RuleVerdict> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "allow" => Some(RuleVerdict::Allow),
        "ask" => Some(RuleVerdict::Ask),
        "deny" => Some(RuleVerdict::Deny),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn perms(allow: &[&str], ask: &[&str], deny: &[&str]) -> SettingsPermissions {
        let mut p = SettingsPermissions::default();
        p.merge_json(&json!({
            "permissions": {
                "allow": allow,
                "ask": ask,
                "deny": deny,
            }
        }));
        p
    }

    #[test]
    fn parses_tool_and_specifier() {
        let bare = PermissionRuleSpec::parse("Bash").unwrap();
        assert_eq!(bare.tool, "Bash");
        assert_eq!(bare.specifier, None);

        let spec = PermissionRuleSpec::parse("Bash(cargo test:*)").unwrap();
        assert_eq!(spec.tool, "Bash");
        assert_eq!(spec.specifier.as_deref(), Some("cargo test:*"));

        // 限定符里自带括号也要完整保留
        let nested = PermissionRuleSpec::parse("Bash(echo (hi))").unwrap();
        assert_eq!(nested.specifier.as_deref(), Some("echo (hi)"));

        assert!(PermissionRuleSpec::parse("   ").is_none());
    }

    #[test]
    fn bash_prefix_rules_stop_at_word_boundary() {
        let p = perms(&["Bash(git diff:*)"], &[], &[]);
        let hit = |cmd: &str| p.evaluate("Bash", &json!({"command": cmd}));

        assert_eq!(hit("git diff").map(|(v, _)| v), Some(RuleVerdict::Allow));
        assert_eq!(
            hit("git diff --stat src/").map(|(v, _)| v),
            Some(RuleVerdict::Allow)
        );
        // `git diffoscope` 不是 `git diff` 的子命令，不能被顺手放过
        assert!(hit("git diffoscope").is_none());
        assert!(hit("git push").is_none());
    }

    /// allow 规则下每一段都要命中；deny/ask 只要有一段命中就算命中
    #[test]
    fn compound_commands_are_strict_for_allow_and_loose_for_deny() {
        let allow = perms(&["Bash(cargo test:*)"], &[], &[]);
        let call = |cmd: &str| json!({"command": cmd});

        assert!(allow.evaluate("Bash", &call("cargo test --lib")).is_some());
        // 第二段没被 allow 覆盖 → 整条命令不给免确认
        assert!(allow
            .evaluate("Bash", &call("cargo test --lib && rm -rf /"))
            .is_none());
        // 命令替换一律不给免确认
        assert!(allow
            .evaluate("Bash", &call("cargo test $(rm -rf /)"))
            .is_none());
        // 引号里的 && 不是分隔符
        assert!(allow
            .evaluate("Bash", &call("cargo test --lib -- 'a && b'"))
            .is_some());

        let deny = perms(&[], &[], &["Bash(rm:*)"]);
        assert_eq!(
            deny.evaluate("Bash", &call("cargo build && rm -rf target"))
                .map(|(v, _)| v),
            Some(RuleVerdict::Deny)
        );
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        let p = perms(&["Bash"], &["Bash(git push:*)"], &["Bash(curl:*)"]);
        let verdict = |cmd: &str| p.evaluate("Bash", &json!({"command": cmd})).map(|(v, _)| v);
        assert_eq!(verdict("curl http://x"), Some(RuleVerdict::Deny));
        assert_eq!(verdict("git push origin main"), Some(RuleVerdict::Ask));
        assert_eq!(verdict("ls"), Some(RuleVerdict::Allow));
    }

    #[test]
    fn path_rules_follow_gitignore_flavor() {
        let p = perms(&[], &[], &["Read(.env)", "Edit(src/generated/**)"]);
        let read = |path: &str| p.evaluate("Read", &json!({"file_path": path}));
        let edit = |path: &str| p.evaluate("Edit", &json!({"file_path": path}));

        // 不含 `/` 的 pattern 在任意层级生效
        assert!(read(".env").is_some());
        assert!(read("./.env").is_some());
        assert!(read("config/.env").is_some());
        assert!(read("src/main.rs").is_none());

        assert!(edit("src/generated/api.rs").is_some());
        assert!(edit("src/generated/deep/api.rs").is_some());
        assert!(edit("src/main.rs").is_none());
    }

    /// 工具族要归一：规则写 `Read`/`Edit`，模型换个同义工具也得管住
    #[test]
    fn tool_aliases_and_wildcards_resolve() {
        let p = perms(&[], &[], &["Read(.env)", "Edit(secrets/**)"]);
        assert!(p
            .evaluate("view_file", &json!({"file_path": ".env"}))
            .is_some());
        assert!(p
            .evaluate("smart_edit", &json!({"file_path": "secrets/key.pem"}))
            .is_some());
        // 反向不成立：写 `read_many_files` 不该顺带覆盖 `Read`
        let narrow = perms(&[], &[], &["read_many_files(.env)"]);
        assert!(narrow
            .evaluate("Read", &json!({"file_path": ".env"}))
            .is_none());

        let star = perms(&["*"], &[], &[]);
        assert_eq!(
            star.evaluate("AnythingAtAll", &json!({})).map(|(v, _)| v),
            Some(RuleVerdict::Allow)
        );

        let mcp = perms(&[], &[], &["mcp__github"]);
        assert!(mcp
            .evaluate("mcp__github__create_issue", &json!({}))
            .is_some());
        assert!(mcp
            .evaluate("mcp__gitlab__create_issue", &json!({}))
            .is_none());
    }

    #[test]
    fn webfetch_domain_matches_host_and_subdomains() {
        let p = perms(&["WebFetch(domain:example.com)"], &[], &[]);
        let hit = |url: &str| p.evaluate("WebFetch", &json!({"url": url})).is_some();
        assert!(hit("https://example.com/a/b"));
        assert!(hit("https://docs.example.com/a"));
        assert!(!hit("https://notexample.com/a"));
        assert!(!hit("https://example.com.evil.test/a"));
    }

    #[test]
    fn unmatched_calls_leave_the_decision_to_the_approval_mode() {
        let p = perms(&["Bash(ls:*)"], &[], &[]);
        assert!(p.evaluate("Write", &json!({"file_path": "a.rs"})).is_none());
        assert!(SettingsPermissions::default().is_empty());
    }

    #[test]
    fn default_mode_uses_the_cli_alias_vocabulary() {
        let mut p = SettingsPermissions::default();
        p.merge_json(&json!({"permissions": {"defaultMode": "acceptEdits"}}));
        assert_eq!(p.default_mode.as_deref(), Some("acceptEdits"));
        assert_eq!(
            approval_mode_from_str("acceptEdits"),
            Some(ApprovalMode::Default)
        );
        assert_eq!(
            approval_mode_from_str("bypassPermissions"),
            Some(ApprovalMode::Yolo)
        );
        assert_eq!(approval_mode_from_str("nonsense"), None);
    }

    /// `/permissions allow|deny|remove` 的落盘往返：写进 settings.local.json 再读回来
    #[test]
    fn local_rules_round_trip_through_settings_local_json() {
        let dir = std::env::temp_dir().join(format!("starcode-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".star")).unwrap();

        assert!(
            add_local_rule(&dir, RuleVerdict::Allow, "Bash(cargo test:*)")
                .unwrap()
                .is_some()
        );
        // 重复添加是幂等的
        assert!(
            add_local_rule(&dir, RuleVerdict::Allow, "Bash(cargo test:*)")
                .unwrap()
                .is_none()
        );
        add_local_rule(&dir, RuleVerdict::Deny, "Read(.env)").unwrap();

        let only_local = vec![local_settings_path(&dir)];
        let loaded = SettingsPermissions::from_paths(&only_local);
        assert_eq!(
            loaded.allow.len(),
            1,
            "allow 桶应有 1 条: {:?}",
            loaded.allow
        );
        assert_eq!(loaded.deny.len(), 1);
        assert_eq!(
            loaded
                .evaluate("Bash", &json!({"command": "cargo test --lib"}))
                .map(|(v, _)| v),
            Some(RuleVerdict::Allow)
        );

        assert_eq!(
            remove_local_rule(&dir, "Bash(cargo test:*)").unwrap(),
            vec![RuleVerdict::Allow]
        );
        assert!(SettingsPermissions::from_paths(&only_local)
            .allow
            .is_empty());
        assert!(remove_local_rule(&dir, "Nope(x)").unwrap().is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
