use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod classifier;
pub mod deny_log;
pub mod path_rules;
pub mod rule_parser;

use deny_log::DenyLog;
use path_rules::{PathPermission, PathRuleMatcher};
use rule_parser::{ParsedRule, RuleParser};
use classifier::{CommandClassifier, SafetyLevel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionAction {
    Allow,
    Deny,
    Ask(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleSource {
    User,
    Project,
    Enterprise,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub id: String,
    pub name: String,
    pub tool_pattern: String,
    pub path_pattern: Option<String>,
    pub action: PermissionAction,
    pub priority: i32,
    pub source: RuleSource,
}

pub struct PermissionRuleEngine {
    rules: Vec<PermissionRule>,
    deny_log: DenyLog,
    path_matcher: PathRuleMatcher,
}

#[derive(Debug, Clone)]
pub struct PermissionDecision {
    pub allowed: bool,
    pub action: PermissionAction,
    pub rule_id: Option<String>,
    pub reason: String,
}

impl PermissionRuleEngine {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            deny_log: DenyLog::new(),
            path_matcher: PathRuleMatcher::new(PathBuf::from(".")),
        }
    }

    pub fn with_deny_log(deny_log: DenyLog) -> Self {
        Self {
            rules: Vec::new(),
            deny_log,
            path_matcher: PathRuleMatcher::new(PathBuf::from(".")),
        }
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.path_matcher = PathRuleMatcher::new(dir);
        self
    }

    pub fn load_rules_from_json(&mut self, content: &str) -> Result<(), String> {
        let parsed = RuleParser::parse_json(content)?;
        self.apply_parsed_rules(parsed);
        Ok(())
    }

    pub fn load_rules_from_toml(&mut self, content: &str) -> Result<(), String> {
        let parsed = RuleParser::parse_toml(content)?;
        self.apply_parsed_rules(parsed);
        Ok(())
    }

    pub fn load_rules_from_files(&mut self, project_dir: &PathBuf, home_dir: &PathBuf) {
        let project_rules = project_dir.join(".star").join("permissions.json");
        let user_rules = home_dir.join(".star").join("permissions.json");

        if project_rules.exists() {
            if let Ok(content) = std::fs::read_to_string(&project_rules) {
                let _ = self.load_rules_from_json_with_source(&content, RuleSource::Project);
            }
        }

        let project_toml = project_dir.join(".star").join("permissions.toml");
        if project_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&project_toml) {
                let _ = self.load_rules_from_toml_with_source(&content, RuleSource::Project);
            }
        }

        if user_rules.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_rules) {
                let _ = self.load_rules_from_json_with_source(&content, RuleSource::User);
            }
        }

        let user_toml = home_dir.join(".star").join("permissions.toml");
        if user_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&user_toml) {
                let _ = self.load_rules_from_toml_with_source(&content, RuleSource::User);
            }
        }

        self.sort_rules();
    }

    pub fn load_rules_from_json_with_source(
        &mut self,
        content: &str,
        source: RuleSource,
    ) -> Result<(), String> {
        let parsed = RuleParser::parse_json(content)?;
        self.apply_parsed_rules_with_source(parsed, source);
        Ok(())
    }

    pub fn load_rules_from_toml_with_source(
        &mut self,
        content: &str,
        source: RuleSource,
    ) -> Result<(), String> {
        let parsed = RuleParser::parse_toml(content)?;
        self.apply_parsed_rules_with_source(parsed, source);
        Ok(())
    }

    fn apply_parsed_rules(&mut self, parsed: Vec<ParsedRule>) {
        self.apply_parsed_rules_with_source(parsed, RuleSource::Project);
    }

    fn apply_parsed_rules_with_source(&mut self, parsed: Vec<ParsedRule>, source: RuleSource) {
        for (i, parsed) in parsed.into_iter().enumerate() {
            let id = format!("rule_{}", i);
            let action = match parsed.action.as_str() {
                "allow" => PermissionAction::Allow,
                "deny" => PermissionAction::Deny,
                other => PermissionAction::Ask(other.to_string()),
            };

            self.rules.push(PermissionRule {
                id,
                name: parsed.tool.clone(),
                tool_pattern: parsed.tool,
                path_pattern: parsed.path,
                action,
                priority: parsed.priority.unwrap_or(0),
                source: source.clone(),
            });
        }
        self.sort_rules();
    }

    pub fn check_permission(&self, tool: &str, args: &serde_json::Value) -> PermissionDecision {
        for rule in &self.rules {
            if self.tool_matches(&rule.tool_pattern, tool) {
                if let Some(ref path_pattern) = rule.path_pattern {
                    if let Some(file_path) = self.extract_path_from_args(tool, args) {
                        if !self.path_matches(path_pattern, &file_path) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                return match &rule.action {
                    PermissionAction::Allow => PermissionDecision {
                        allowed: true,
                        action: PermissionAction::Allow,
                        rule_id: Some(rule.id.clone()),
                        reason: format!("Allowed by rule '{}'", rule.name),
                    },
                    PermissionAction::Deny => {
                        self.deny_log.record(
                            tool,
                            args,
                            &format!("Denied by rule '{}'", rule.name),
                            Some(rule.id.clone()),
                        );
                        PermissionDecision {
                            allowed: false,
                            action: PermissionAction::Deny,
                            rule_id: Some(rule.id.clone()),
                            reason: format!("Denied by rule '{}'", rule.name),
                        }
                    }
                    PermissionAction::Ask(msg) => PermissionDecision {
                        allowed: false,
                        action: PermissionAction::Ask(msg.clone()),
                        rule_id: Some(rule.id.clone()),
                        reason: msg.clone(),
                    },
                };
            }
        }

        if tool == "Bash" || tool == "shell" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let classification = CommandClassifier::classify(cmd);
                if classification.level == SafetyLevel::Dangerous {
                    self.deny_log.record(
                        tool,
                        args,
                        &format!("Dangerous command: {}", classification.reason),
                        None,
                    );
                    return PermissionDecision {
                        allowed: false,
                        action: PermissionAction::Deny,
                        rule_id: None,
                        reason: format!("Dangerous command detected: {}", classification.reason),
                    };
                }
            }
        }

        PermissionDecision {
            allowed: true,
            action: PermissionAction::Allow,
            rule_id: None,
            reason: "No matching rule, default allow".to_string(),
        }
    }

    fn tool_matches(&self, pattern: &str, tool: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if pattern.ends_with("__*") {
            let prefix = &pattern[..pattern.len() - 3];
            return tool.starts_with(&format!("{}__", prefix)) || tool == prefix;
        }
        pattern == tool
    }

    fn path_matches(&self, _pattern: &str, path: &str) -> bool {
        let matcher = PathRuleMatcher::new(PathBuf::from("."));
        matcher.check_permission(
            std::path::Path::new(path),
            &PathPermission::Read,
        )
    }

    fn extract_path_from_args(&self, tool: &str, args: &serde_json::Value) -> Option<String> {
        match tool {
            "Edit" | "str_replace_editor" | "smart_edit" | "create_file"
            | "Write" | "Read" | "view_file" | "read_many_files" => {
                args.get("path")
                    .or_else(|| args.get("file_path"))
                    .or_else(|| args.get("target_file"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
            "Bash" | "shell" => {
                if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                    self.extract_path_from_command(cmd)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn extract_path_from_command(&self, command: &str) -> Option<String> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        for part in parts {
            if part.starts_with('/') || part.starts_with("./") || part.starts_with("../") {
                return Some(part.to_string());
            }
            if part.ends_with(".rs") || part.ends_with(".py") || part.ends_with(".js") {
                return Some(part.to_string());
            }
        }
        None
    }

    pub fn add_rule(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
        self.sort_rules();
    }

    pub fn remove_rule(&mut self, id: &str) {
        self.rules.retain(|r| r.id != id);
    }

    pub fn get_rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    pub fn get_rules_for_tool(&self, tool: &str) -> Vec<&PermissionRule> {
        self.rules
            .iter()
            .filter(|r| self.tool_matches(&r.tool_pattern, tool))
            .collect()
    }

    pub fn get_deny_log(&self) -> &DenyLog {
        &self.deny_log
    }

    pub fn clear_rules(&mut self) {
        self.rules.clear();
    }

    fn sort_rules(&mut self) {
        self.rules.sort_by(|a, b| {
            let source_order = |s: &RuleSource| match s {
                RuleSource::Enterprise => 0,
                RuleSource::Project => 1,
                RuleSource::User => 2,
            };
            let so = source_order(&a.source).cmp(&source_order(&b.source));
            so.then(b.priority.cmp(&a.priority))
        });
    }
}

impl Default for PermissionRuleEngine {
    fn default() -> Self {
        Self::new()
    }
}
 