use crate::core::policy::types::*;
use crate::core::permission_rules::PermissionRuleEngine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub decision: PolicyDecision,
    pub rule: Option<PolicyRule>,
}

pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
    checkers: Vec<SafetyCheckerRule>,
    hook_checkers: Vec<HookCheckerRule>,
    default_decision: PolicyDecision,
    non_interactive: bool,
    allow_hooks: bool,
    approval_mode: ApprovalMode,
    session_allowances: std::collections::HashSet<String>, // tool_name
    permission_rule_engine: PermissionRuleEngine,
}

impl PolicyEngine {
    pub fn new(config: PolicyEngineConfig) -> Self {
        let mut rules = config.rules.unwrap_or_default();
        let mut checkers = config.checkers.unwrap_or_default();
        let mut hook_checkers = config.hook_checkers.unwrap_or_default();

        rules.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
        checkers.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
        hook_checkers.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));

        Self {
            rules,
            checkers,
            hook_checkers,
            default_decision: config.default_decision.unwrap_or(PolicyDecision::AskUser),
            non_interactive: config.non_interactive.unwrap_or(false),
            allow_hooks: config.allow_hooks.unwrap_or(true),
            approval_mode: config.approval_mode.unwrap_or(ApprovalMode::Default),
            session_allowances: std::collections::HashSet::new(),
            permission_rule_engine: PermissionRuleEngine::new(),
        }
    }

    pub fn load_permission_rules(&mut self, project_dir: &std::path::PathBuf, home_dir: &std::path::PathBuf) {
        self.permission_rule_engine.load_rules_from_files(project_dir, home_dir);
    }

    pub fn get_permission_rule_engine(&self) -> &PermissionRuleEngine {
        &self.permission_rule_engine
    }

    pub fn get_permission_rule_engine_mut(&mut self) -> &mut PermissionRuleEngine {
        &mut self.permission_rule_engine
    }

    pub fn allow_tool_for_session(&mut self, tool_name: String) {
        self.session_allowances.insert(tool_name);
    }

    pub fn set_approval_mode(&mut self, mode: ApprovalMode) {
        self.approval_mode = mode;
    }

    pub fn get_approval_mode(&self) -> ApprovalMode {
        self.approval_mode.clone()
    }

    pub async fn check(&self, tool_call: &FunctionCall, server_name: Option<&str>) -> CheckResult {
        // 0. Check session allowances first
        if self.session_allowances.contains(&tool_call.name) {
            return CheckResult {
                decision: PolicyDecision::Allow,
                rule: None, // Implicit rule
            };
        }

        // 0.5. Check permission rule engine first
        let args = tool_call.args.clone().unwrap_or(serde_json::Value::Null);
        let permission_decision = self.permission_rule_engine.check_permission(&tool_call.name, &args);
        if !permission_decision.allowed {
            let reason = if permission_decision.reason.is_empty() {
                format!("Tool '{}' denied by permission rules", tool_call.name)
            } else {
                permission_decision.reason
            };
            return CheckResult {
                decision: PolicyDecision::DenyWithReason(reason),
                rule: None,
            };
        }

        // 1. Handle Approval Modes (YOLO / Plan)
        match self.approval_mode {
            ApprovalMode::Yolo => {
                // In YOLO mode, allow everything unless explicitly denied by a high-priority rule?
                // For true YOLO, we just allow everything.
                return CheckResult {
                    decision: PolicyDecision::Allow,
                    rule: None,
                };
            }
            ApprovalMode::Plan => {
                // In Plan mode, only allow safe read-only tools and exit_plan_mode
                // We use is_safe_query_tool helper from types
                let is_safe = crate::types::is_safe_query_tool(&tool_call.name);
                let is_exit = tool_call.name == "exit_plan_mode";

                if is_safe || is_exit {
                    // Safe tools are allowed automatically in Plan mode (to facilitate planning)
                    return CheckResult {
                        decision: PolicyDecision::Allow,
                        rule: None,
                    };
                } else {
                    // Unsafe tools are DENIED in Plan mode. Re-entering plan mode is not useful
                    // and can trap the agent in enter/exit loops; use exit_plan_mode when ready.
                    return CheckResult {
                        decision: PolicyDecision::Deny,
                        rule: None,
                    };
                }
            }
            ApprovalMode::Default => {} // Continue to rules
        }

        let stringified_args = if tool_call.args.is_some() {
            serde_json::to_string(&tool_call.args).ok()
        } else {
            None
        };

        let mut matched_rule: Option<PolicyRule> = None;
        let mut decision: Option<PolicyDecision> = None;

        for rule in &self.rules {
            if self.rule_matches(rule, tool_call, stringified_args.as_deref(), server_name) {
                decision = Some(self.apply_non_interactive_mode(rule.decision.clone()));
                matched_rule = Some(rule.clone());
                break;
            }
        }

        if decision.is_none() {
            decision = Some(self.apply_non_interactive_mode(self.default_decision.clone()));
        }

        CheckResult {
            decision: decision.unwrap(),
            rule: matched_rule,
        }
    }

    fn rule_matches(
        &self,
        rule: &PolicyRule,
        tool_call: &FunctionCall,
        stringified_args: Option<&str>,
        server_name: Option<&str>,
    ) -> bool {
        if let Some(modes) = &rule.modes {
            if !modes.contains(&self.approval_mode) {
                return false;
            }
        }

        if let Some(tool_name) = &rule.tool_name {
            if tool_name.ends_with("__*") {
                let prefix = &tool_name[..tool_name.len() - 3];
                if let Some(server) = server_name {
                    if server != prefix {
                        return false;
                    }
                }
                if !tool_call.name.starts_with(&format!("{}__", prefix)) {
                    return false;
                }
            } else if tool_call.name != *tool_name {
                return false;
            }
        }

        if let Some(pattern) = &rule.args_pattern {
            if tool_call.args.is_none() {
                return false;
            }
            if let Some(args) = stringified_args {
                let re = regex::Regex::new(pattern);
                if let Ok(regex) = re {
                    if !regex.is_match(args) {
                        return false;
                    }
                }
            }
        }

        true
    }

    fn apply_non_interactive_mode(&self, decision: PolicyDecision) -> PolicyDecision {
        if self.non_interactive && decision == PolicyDecision::AskUser {
            PolicyDecision::Deny
        } else {
            decision
        }
    }

    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.rules.push(rule);
        self.rules
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn add_checker(&mut self, checker: SafetyCheckerRule) {
        self.checkers.push(checker);
        self.checkers
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn remove_rules_for_tool(&mut self, tool_name: &str) {
        self.rules
            .retain(|rule| rule.tool_name.as_deref() != Some(tool_name));
    }

    pub fn get_rules(&self) -> &[PolicyRule] {
        &self.rules
    }

    pub fn get_checkers(&self) -> &[SafetyCheckerRule] {
        &self.checkers
    }

    pub fn add_hook_checker(&mut self, checker: HookCheckerRule) {
        self.hook_checkers.push(checker);
        self.hook_checkers
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn get_hook_checkers(&self) -> &[HookCheckerRule] {
        &self.hook_checkers
    }

    pub async fn check_hook(&self, context: &HookExecutionContext) -> PolicyDecision {
        if !self.allow_hooks {
            return PolicyDecision::Deny;
        }

        if context.trusted_folder == Some(false) && context.hook_source == Some(HookSource::Project)
        {
            return PolicyDecision::Deny;
        }

        PolicyDecision::Allow
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new(PolicyEngineConfig::default())
    }
}

impl Default for PolicyEngineConfig {
    fn default() -> Self {
        Self {
            rules: None,
            checkers: None,
            hook_checkers: None,
            default_decision: None,
            non_interactive: None,
            allow_hooks: None,
            approval_mode: None,
        }
    }
}
